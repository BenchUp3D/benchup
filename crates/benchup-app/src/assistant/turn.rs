//! One designer request: send the scene and prompt, apply the tool calls the
//! model returns, feed the results back, and repeat until it answers in text.
//! The loop is a state machine advanced by `poll` once per UI frame; nothing
//! here blocks. The caller commits the whole turn as a single history step.
use super::{
    client::{ChatError, ChatRequest, ChatResponse, ChatTransport, Message, Role},
    scene, tools,
};
use benchup_core::{MaterialId, Model, StockMaterial, tr};
use eframe::egui;
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::mpsc::{Receiver, TryRecvError},
};

pub(crate) const MAX_ROUNDS: usize = 12;
/// One tool result message sent back to the model.
const MAX_TOOL_RESULT_CHARS: usize = 8_000;
/// Characters of an oversized result kept in its `preview`.
const TRUNCATED_PREVIEW_CHARS: usize = 7_000;
/// Tool calls executed per model reply; the rest are answered with an error.
pub(crate) const MAX_CALLS_PER_REPLY: usize = 32;
/// Share of the scene budget the `references` block may use.
const REFERENCE_BUDGET: usize = scene::SCENE_BUDGET / 3;

pub(crate) const SYSTEM_PROMPT: &str = "You are MIRA, a woodworking design assistant inside BenchUp, a 3D modeler. \
You change the user's model only by calling the provided tools. \
All lengths you read or write are in millimetres. The Z axis points up; boards are axis-aligned boxes unless rotated. \
Refer to entities by their numeric id, and use the ids in the scene and in the references block. \
Call get_object to read a board's faces before push_pull or any edit that needs a face id; never guess ids. \
Prefer high-level tools (create_board, move, resize_board, add_joint, set_material) over drawing raw geometry. \
Check tool results: if a call fails, read the error and hint, then correct it. \
When the work is complete, answer with one short sentence summarising what you did, without calling more tools.";

/// Everything a turn needs from the app, captured when the user sends.
pub(crate) struct TurnSetup {
    pub prompt: String,
    pub references: BTreeSet<u64>,
    pub selection: BTreeSet<u64>,
    pub include_selection: bool,
    pub catalog: BTreeMap<MaterialId, StockMaterial>,
    pub read_only: bool,
    /// Shared project: the leased entities; `None` when not shared.
    pub lease: Option<BTreeSet<u64>>,
    /// Earlier user/assistant text of this chat.
    pub history: Vec<Message>,
}

pub(crate) struct TurnResult {
    pub prompt: String,
    pub summary: String,
    pub created: Vec<u64>,
    pub modified: Vec<u64>,
    pub erased: Vec<u64>,
    pub joints: Vec<u64>,
    /// Model before the first change; `None` when the turn changed nothing.
    pub before: Option<Model>,
    pub stopped: bool,
    pub error: Option<String>,
    /// Set when the model used `select`; the editor selection should follow.
    pub selection: Option<BTreeSet<u64>>,
    /// The editing scope when the turn began; the app puts the user back in it.
    pub scope: Option<u64>,
}

/// Per-frame conditions the app supplies while a turn runs (shared projects).
#[derive(Default)]
pub(crate) struct Gate<'a> {
    /// Why further edits are not allowed right now; the turn stops with it.
    pub block: Option<String>,
    /// The edit lease to enforce for this round's tool calls.
    pub lease: Option<BTreeSet<u64>>,
    /// Asked again before each mutating tool call of a reply; a reason stops
    /// the turn before that call runs.
    pub recheck: Option<&'a dyn Fn() -> Option<String>>,
}

pub(crate) struct Poll {
    pub finished: Option<TurnResult>,
    /// The scene changed during this poll; refresh the viewport.
    pub mutated: bool,
}

pub(crate) struct Turn {
    base: String,
    key: String,
    model_name: String,
    prompt: String,
    messages: Vec<Message>,
    tools: Vec<serde_json::Value>,
    env: tools::ToolEnv,
    selection_used: bool,
    scope: Option<u64>,
    pending: Option<Receiver<Result<ChatResponse, ChatError>>>,
    rounds: usize,
    created: Vec<u64>,
    modified: Vec<u64>,
    erased: Vec<u64>,
    joints: Vec<u64>,
    /// Names of the tools run so far, for progress display.
    pub activity: Vec<String>,
    pub started_at: f64,
}

fn push_unique(list: &mut Vec<u64>, ids: &[u64]) {
    for id in ids {
        if !list.contains(id) {
            list.push(*id);
        }
    }
}

/// An oversized tool result replaced by valid JSON: the verdict plus a prefix.
pub(crate) fn truncated_result(text: &str, ok: bool) -> String {
    let mut keep = TRUNCATED_PREVIEW_CHARS;
    loop {
        let preview: String = text.chars().take(keep).collect();
        let wrapped = json!({"ok": ok, "truncated": true, "preview": preview}).to_string();
        // Escaping can make the preview longer; shrink it until the whole fits.
        if wrapped.len() <= MAX_TOOL_RESULT_CHARS || keep == 0 {
            return wrapped;
        }
        keep = keep * 3 / 4;
    }
}

impl Turn {
    pub(crate) fn start(
        ctx: &egui::Context,
        transport: &dyn ChatTransport,
        (base, key, model_name): (&str, &str, &str),
        model: &Model,
        setup: TurnSetup,
    ) -> Self {
        let mut references = setup.references.clone();
        // With "Include selection" off the selection is neither listed in the
        // scene nor offered by `get_selection`.
        let mut shared_selection = BTreeSet::new();
        if setup.include_selection {
            references.extend(&setup.selection);
            shared_selection = setup.selection.clone();
        }
        // Selection, tags, references and objects share one budget.
        let references = scene::references(model, &references, REFERENCE_BUDGET);
        let remaining = scene::SCENE_BUDGET.saturating_sub(references.to_string().len());
        let context = json!({
            "scene": scene::summary(model, &shared_selection, &setup.catalog, remaining),
            "references": references,
        });
        let mut messages = vec![
            Message::text(Role::System, SYSTEM_PROMPT),
            Message::text(Role::System, format!("Current model state (JSON):\n{context}")),
        ];
        messages.extend(setup.history);
        messages.push(Message::text(Role::User, setup.prompt.clone()));
        let mut turn = Self {
            base: base.into(),
            key: key.into(),
            model_name: model_name.into(),
            prompt: setup.prompt,
            messages,
            tools: tools::definitions(),
            env: tools::ToolEnv {
                read_only: setup.read_only,
                selection: setup.selection,
                hide_selection: !setup.include_selection,
                catalog: setup.catalog,
                before: None,
                lease: setup.lease,
                created: BTreeSet::new(),
            },
            selection_used: false,
            scope: model.edit_context,
            pending: None,
            rounds: 0,
            created: Vec::new(),
            modified: Vec::new(),
            erased: Vec::new(),
            joints: Vec::new(),
            activity: Vec::new(),
            started_at: ctx.input(|i| i.time),
        };
        turn.request(ctx, transport);
        turn
    }

    fn request(&mut self, ctx: &egui::Context, transport: &dyn ChatTransport) {
        let request = ChatRequest {
            model: self.model_name.clone(),
            messages: self.messages.clone(),
            tools: self.tools.clone(),
            tool_choice: Some("auto"),
            stream: false,
        };
        self.pending = Some(transport.chat(ctx, &self.base, &self.key, &request));
    }

    /// Advance at most one model reply. `model` is the live document model.
    pub(crate) fn poll(&mut self, ctx: &egui::Context, transport: &dyn ChatTransport, model: &mut Model, gate: &Gate) -> Poll {
        let idle = Poll { finished: None, mutated: false };
        let Some(receiver) = &self.pending else { return idle };
        let reply = match receiver.try_recv() {
            Ok(reply) => reply,
            Err(TryRecvError::Empty) => return idle,
            Err(TryRecvError::Disconnected) => {
                Err(ChatError::Network(tr!("assistant.error.interrupted").into()))
            }
        };
        self.pending = None;
        let message = match reply {
            Ok(response) => response.choices.into_iter().next().map(|c| c.message).unwrap_or_default(),
            Err(error) => {
                return Poll { finished: Some(self.finish(model, String::new(), Some(error.user_message()), false)), mutated: false };
            }
        };
        let content = message.content.unwrap_or_default();
        let mut calls = message.tool_calls.unwrap_or_default();
        if calls.is_empty()
            && let Some(call) = tools::fallback_tool_call(&content)
        {
            calls.push(call);
        }
        for (index, call) in calls.iter_mut().enumerate() {
            if call.id.is_empty() {
                call.id = format!("call_{}_{index}", self.rounds);
            }
        }
        if !calls.is_empty()
            && let Some(reason) = &gate.block
        {
            // Editing stopped being allowed (shared project) since the last round.
            return Poll { finished: Some(self.finish(model, String::new(), Some(reason.clone()), true)), mutated: false };
        }
        self.env.lease = gate.lease.clone();
        self.messages.push(Message {
            role: Role::Assistant,
            content: content.clone(),
            tool_calls: calls.clone(),
            tool_call_id: None,
        });
        if calls.is_empty() {
            return Poll { finished: Some(self.finish(model, content, None, false)), mutated: false };
        }
        let mut mutated = false;
        for (index, call) in calls.iter().enumerate() {
            if index >= MAX_CALLS_PER_REPLY {
                let refused = json!({"ok": false, "error": "too many calls in one reply"});
                self.messages.push(Message::tool(&call.id, refused.to_string()));
                continue;
            }
            // Edit access can lapse inside a batch: ask again before every edit.
            if tools::is_mutating(&call.function.name)
                && let Some(reason) = gate.recheck.and_then(|recheck| recheck())
            {
                return Poll { finished: Some(self.finish(model, String::new(), Some(reason), true)), mutated };
            }
            let outcome = tools::execute(model, call, &mut self.env);
            self.activity.push(call.function.name.clone());
            mutated |= outcome.mutated;
            if outcome.ok {
                push_unique(&mut self.created, &outcome.created);
                push_unique(&mut self.modified, &outcome.modified);
                push_unique(&mut self.erased, &outcome.erased);
                push_unique(&mut self.joints, &outcome.joints);
                self.selection_used |= call.function.name == "select";
            }
            let mut text = outcome.result.to_string();
            if text.len() > MAX_TOOL_RESULT_CHARS {
                text = truncated_result(&text, outcome.ok);
            }
            self.messages.push(Message::tool(&call.id, text));
        }
        self.rounds += 1;
        if self.rounds >= MAX_ROUNDS {
            let error = Some(tr!("assistant.error.round_limit", count = MAX_ROUNDS));
            return Poll { finished: Some(self.finish(model, content, error, false)), mutated };
        }
        self.request(ctx, transport);
        Poll { finished: None, mutated }
    }

    /// End the turn now. Edits already applied are kept for the caller to commit.
    pub(crate) fn stop(&mut self, model: &Model) -> TurnResult {
        self.pending = None;
        self.finish(model, String::new(), None, true)
    }

    fn finish(&mut self, model: &Model, summary: String, error: Option<String>, stopped: bool) -> TurnResult {
        self.pending = None;
        let exists = |ids: &[u64]| -> Vec<u64> { ids.iter().copied().filter(|id| model.contains_entity(id) || model.connections.contains_key(id)).collect() };
        let created = exists(&self.created);
        let modified: Vec<u64> = exists(&self.modified).into_iter().filter(|id| !created.contains(id)).collect();
        // Ids that were created and then removed in the same turn leave no trace.
        let erased: Vec<u64> = self.erased.iter().copied().filter(|id| !self.created.contains(id)).collect();
        // The snapshot belongs to the scope the designer was in, whatever
        // enter_object/exit_to_root did meanwhile, so undo leaves them there.
        let mut before = self.env.before.take();
        if let Some(before) = &mut before {
            before.edit_context = self.scope;
        }
        let mut summary = summary.trim().to_owned();
        if summary.is_empty() && !stopped && error.is_none() {
            summary = tr!("assistant.default_summary").into();
        }
        TurnResult {
            prompt: self.prompt.clone(),
            summary,
            created,
            modified,
            erased,
            joints: exists(&self.joints),
            before,
            stopped,
            error,
            selection: self.selection_used.then(|| self.env.selection.clone()),
            scope: self.scope,
        }
    }
}
