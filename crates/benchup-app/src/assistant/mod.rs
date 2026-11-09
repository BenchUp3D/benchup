//! "Build with MIRA": an LLM design assistant. The model proposes tool calls
//! (see `tools`), the editor applies them to the live model, and each request
//! becomes one undo step. The provider is any OpenAI-compatible server; the
//! default preset is endlesstokens.ai. `panel` is the chat UI and `window`
//! hosts it docked in the inspector or detached in its own window.
use super::*;
use benchup_core::tr;
use client::{ChatError, ChatTransport, Message, Role};
use std::{collections::BTreeMap, sync::mpsc::Receiver};

#[path = "client.rs"]
pub(crate) mod client;
#[path = "credentials.rs"]
pub(crate) mod credentials;
#[path = "panel.rs"]
pub(crate) mod panel;
#[path = "scene.rs"]
pub(crate) mod scene;
#[path = "tools.rs"]
pub(crate) mod tools;
#[path = "turn.rs"]
pub(crate) mod turn;
#[path = "window.rs"]
pub(crate) mod window;
#[cfg(test)]
#[path = "tests.rs"]
mod tests;


use turn::{Gate, Turn, TurnResult, TurnSetup};

const HISTORY_MESSAGES: usize = 20;
const HISTORY_CHARS: usize = 2000;
const DEFAULT_MODEL_HINT: &str = "qwen";

/// Where the panel lives. Position is the outer window corner and size the
/// inner size, both in points; they are only meaningful while `detached`.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(super) struct WindowPreferences {
    pub detached: bool,
    pub position: Option<[f32; 2]>,
    pub size: Option<[f32; 2]>,
}

/// Remembered across restarts. The API key is deliberately not here.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(super) struct Preferences {
    pub base_url: String,
    pub model: String,
    pub include_selection: bool,
    /// The one-line "what is sent" notice was dismissed for good.
    pub privacy_dismissed: bool,
    pub window: WindowPreferences,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            base_url: client::DEFAULT_BASE_URL.into(),
            model: String::new(),
            include_selection: true,
            privacy_dismissed: false,
            window: WindowPreferences::default(),
        }
    }
}
impl Preferences {
    fn sanitized(mut self) -> Self {
        let finite = |pair: [f32; 2]| pair.iter().all(|v| v.is_finite());
        self.window.position = self.window.position.filter(|p| finite(*p));
        self.window.size = self
            .window
            .size
            .filter(|s| finite(*s))
            .map(|[w, h]| [w.clamp(window::MIN_SIZE[0], 8192.), h.clamp(window::MIN_SIZE[1], 8192.)]);
        self.base_url = client::normalize_base(&self.base_url);
        if !client::valid_base_url(&self.base_url) {
            self.base_url = client::DEFAULT_BASE_URL.into();
        }
        self.model = self.model.trim().chars().take(200).collect();
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Connection {
    Disconnected,
    Connecting,
    Connected,
    Failed(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum EntryRole {
    User,
    Assistant,
    Error,
}
/// What a finished turn changed, for the change card.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Changes {
    pub created: Vec<u64>,
    pub modified: Vec<u64>,
    pub erased: Vec<u64>,
    pub joints: Vec<u64>,
    pub stopped: bool,
    /// History position right after this turn's step; `None` if nothing was recorded.
    pub position: Option<usize>,
    /// Names of erased entities, which no longer exist to be looked up.
    pub erased_names: BTreeMap<u64, String>,
}
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Entry {
    pub role: EntryRole,
    pub text: String,
    pub changes: Option<Changes>,
}

/// A prompt plus what the user attached to it.
pub(super) struct Prompt {
    pub prompt: String,
    pub references: BTreeSet<u64>,
}

pub(super) struct Assistant {
    pub settings: Preferences,
    pub transcript: Vec<Entry>,
    /// API key for `settings.base_url`, loaded on demand.
    pub key: Option<String>,
    pub models: Vec<String>,
    pub connection: Connection,
    history: Vec<Message>,
    turn: Option<Turn>,
    /// Reply to a model-list request, with the server it was sent to.
    models_receiver: Option<(String, Receiver<Result<Vec<String>, ChatError>>)>,
    key_candidate: Option<String>,
    transport: Box<dyn ChatTransport>,
    /// Session-only panel state: composer, chips, dialogs.
    pub ui: panel::UiState,
}

impl Default for Assistant {
    fn default() -> Self {
        Self::with_transport(Box::new(client::HttpTransport))
    }
}

impl Assistant {
    pub(super) fn with_transport(transport: Box<dyn ChatTransport>) -> Self {
        Self {
            settings: Preferences::default(),
            transcript: Vec::new(),
            key: None,
            models: Vec::new(),
            connection: Connection::Disconnected,
            history: Vec::new(),
            turn: None,
            models_receiver: None,
            key_candidate: None,
            transport,
            ui: panel::UiState::default(),
        }
    }

    pub(super) fn preferences(&self) -> Preferences {
        self.settings.clone()
    }
    pub(super) fn restore(&mut self, preferences: &Preferences) {
        self.settings = preferences.clone().sanitized();
    }

    pub(super) fn busy(&self) -> bool {
        self.turn.is_some()
    }
    /// Tools run so far in the current turn, and when it began.
    pub(super) fn progress(&self) -> Option<(&[String], f64)> {
        self.turn.as_ref().map(|turn| (turn.activity.as_slice(), turn.started_at))
    }

    /// Read the saved key for the selected server (once, on first use).
    pub(super) fn load_key(&mut self) {
        if self.key.is_none() {
            self.key = credentials::load(&self.settings.base_url);
            if self.key.is_some() {
                self.connection = Connection::Connected;
            }
        }
    }
    /// Validate a key by listing models; it is saved only if the server accepts it.
    pub(super) fn connect(&mut self, ctx: &egui::Context, key: &str) {
        let key = key.trim();
        if key.is_empty() {
            self.connection = Connection::Failed(tr!("assistant.error.invalid_api_key").into());
            return;
        }
        self.connection = Connection::Connecting;
        self.key_candidate = Some(key.to_owned());
        self.models_receiver = Some((self.settings.base_url.clone(), self.transport.models(ctx, &self.settings.base_url, key)));
    }
    /// Reload the model list with the stored key.
    pub(super) fn refresh_models(&mut self, ctx: &egui::Context) {
        if let Some(key) = self.key.clone() {
            self.key_candidate = None;
            self.models_receiver = Some((self.settings.base_url.clone(), self.transport.models(ctx, &self.settings.base_url, &key)));
        }
    }
    pub(super) fn forget_key(&mut self) -> Result<(), String> {
        self.key = None;
        self.key_candidate = None;
        self.models.clear();
        self.connection = Connection::Disconnected;
        credentials::forget(&self.settings.base_url)
    }
    pub(super) fn new_chat(&mut self) {
        if !self.busy() {
            self.transcript.clear();
            self.history.clear();
        }
    }

    fn poll_models(&mut self) {
        let Some((_, receiver)) = &self.models_receiver else { return };
        let reply = match receiver.try_recv() {
            Ok(reply) => reply,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(_) => Err(ChatError::Network(tr!("assistant.error.interrupted").into())),
        };
        let base = self.models_receiver.take().map(|(base, _)| base).unwrap_or_default();
        let candidate = self.key_candidate.take();
        if client::normalize_base(&base) != client::normalize_base(&self.settings.base_url) {
            // The server setting changed while this request was in flight: the
            // reply (and any key it validated) belongs to another server.
            if self.connection == Connection::Connecting {
                self.connection = Connection::Disconnected;
            }
            return;
        }
        match reply {
            Ok(models) => {
                if let Some(key) = candidate {
                    match credentials::save(&base, &key) {
                        Ok(()) => {}
                        Err(e) => self.transcript.push(Entry {
                            role: EntryRole::Error,
                            text: tr!("assistant.error.cannot_save_key_detail", e = e),
                            changes: None,
                        }),
                    }
                    self.key = Some(key);
                }
                if !models.iter().any(|m| *m == self.settings.model) {
                    self.settings.model = models
                        .iter()
                        .find(|m| m.to_lowercase().contains(DEFAULT_MODEL_HINT))
                        .or(models.first())
                        .cloned()
                        .unwrap_or_default();
                }
                self.models = models;
                self.connection = Connection::Connected;
            }
            Err(ChatError::InvalidKey) => {
                if candidate.is_none() {
                    self.key = None;
                }
                self.connection = Connection::Failed(ChatError::InvalidKey.user_message());
            }
            Err(error) => self.connection = Connection::Failed(error.user_message()),
        }
    }

    /// Start a turn. Fails fast, with a user-facing message, if it cannot run.
    pub(super) fn send(
        &mut self,
        ctx: &egui::Context,
        model: &Model,
        selection: &BTreeSet<u64>,
        catalog: BTreeMap<benchup_core::MaterialId, benchup_core::StockMaterial>,
        read_only: bool,
        lease: Option<BTreeSet<u64>>,
        request: Prompt,
    ) -> Result<(), String> {
        let prompt = request.prompt.trim().to_owned();
        if self.busy() {
            return Err(tr!("assistant.error.busy").into());
        }
        if prompt.is_empty() {
            return Err(tr!("assistant.error.empty_prompt").into());
        }
        if !client::valid_base_url(&self.settings.base_url) {
            return Err(ChatError::InvalidUrl.user_message());
        }
        let key = self.key.clone().ok_or(tr!("assistant.error.not_connected"))?;
        if self.settings.model.trim().is_empty() {
            return Err(tr!("assistant.error.no_model").into());
        }
        let setup = TurnSetup {
            prompt: prompt.clone(),
            references: request.references,
            selection: selection.clone(),
            include_selection: self.settings.include_selection,
            catalog,
            read_only,
            lease,
            history: self.history.clone(),
        };
        self.transcript.push(Entry { role: EntryRole::User, text: prompt, changes: None });
        self.turn = Some(Turn::start(
            ctx,
            self.transport.as_ref(),
            (&self.settings.base_url, &key, &self.settings.model),
            model,
            setup,
        ));
        Ok(())
    }

    /// Advance connection checks and the running turn. Returns the finished
    /// turn once, for the app to commit with `assistant_commit_turn`.
    pub(super) fn poll(&mut self, ctx: &egui::Context, model: &mut Model, gate: &Gate) -> turn::Poll {
        self.poll_models();
        let Some(turn) = &mut self.turn else { return turn::Poll { finished: None, mutated: false } };
        let poll = turn.poll(ctx, self.transport.as_ref(), model, gate);
        if poll.finished.is_some() {
            self.turn = None;
        } else if !poll.mutated {
            // Keep polling without waiting for input while a reply is pending.
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        poll
    }

    /// Stop the running turn; keep what it already changed.
    pub(super) fn stop(&mut self, model: &Model) -> Option<TurnResult> {
        let mut turn = self.turn.take()?;
        Some(turn.stop(model))
    }

    /// End the running turn and throw its edits away (the model it edited was
    /// replaced): nothing is committed, and the transcript says why.
    pub(super) fn discard_turn(&mut self, model: &Model, error: String) -> bool {
        let Some(mut turn) = self.turn.take() else { return false };
        let mut result = turn.stop(model);
        result.before = None;
        result.selection = None;
        (result.created, result.modified, result.erased, result.joints) = Default::default();
        result.error = Some(error);
        self.record(&result);
        true
    }

    /// Record a finished turn in the transcript and the model-facing history.
    pub(super) fn record(&mut self, result: &TurnResult) {
        let changed = !(result.created.is_empty() && result.modified.is_empty() && result.erased.is_empty() && result.joints.is_empty());
        if !result.summary.is_empty() {
            self.transcript.push(Entry {
                role: EntryRole::Assistant,
                text: result.summary.clone(),
                changes: changed.then(|| Changes {
                    created: result.created.clone(),
                    modified: result.modified.clone(),
                    erased: result.erased.clone(),
                    joints: result.joints.clone(),
                    stopped: result.stopped,
                    ..Changes::default()
                }),
            });
        } else if changed {
            self.transcript.push(Entry {
                role: EntryRole::Assistant,
                text: String::new(),
                changes: Some(Changes {
                    created: result.created.clone(),
                    modified: result.modified.clone(),
                    erased: result.erased.clone(),
                    joints: result.joints.clone(),
                    stopped: result.stopped,
                    ..Changes::default()
                }),
            });
        }
        if let Some(error) = &result.error {
            self.transcript.push(Entry { role: EntryRole::Error, text: error.clone(), changes: None });
        }
        let clip = |text: &str| -> String { text.chars().take(HISTORY_CHARS).collect() };
        self.history.push(Message::text(Role::User, clip(&result.prompt)));
        if !result.summary.is_empty() {
            self.history.push(Message::text(Role::Assistant, clip(&result.summary)));
        }
        let excess = self.history.len().saturating_sub(HISTORY_MESSAGES);
        self.history.drain(..excess);
    }
}

impl BenchUpApp {
    /// Called once per frame from `ui`.
    pub(super) fn poll_assistant(&mut self, ctx: &egui::Context) {
        let gate = if self.assistant.busy() {
            // Shared projects: editing rights can lapse between rounds.
            Gate {
                block: (!self.shared_edit_allowed(ctx)).then(|| self.shared_edit_block_reason(ctx).to_owned()),
                lease: self.shared_lease(),
            }
        } else {
            Gate::default()
        };
        let poll = self.assistant.poll(ctx, &mut self.doc.model, &gate);
        if poll.mutated {
            self.changed(ctx);
        }
        if let Some(result) = poll.finished {
            self.assistant_commit_turn(result, ctx);
        }
    }

    pub(super) fn assistant_send(&mut self, ctx: &egui::Context, request: Prompt) -> Result<(), String> {
        let catalog = self
            .general_settings
            .stock
            .wood
            .iter()
            .map(|stock| (stock.id, stock.definition()))
            .collect();
        let read_only = !self.shared_edit_allowed(ctx);
        let lease = self.shared_lease();
        self.assistant.send(ctx, &self.doc.model, &self.selected, catalog, read_only, lease, request)
    }

    pub(super) fn assistant_stop(&mut self, ctx: &egui::Context) {
        if let Some(result) = self.assistant.stop(&self.doc.model) {
            self.assistant_commit_turn(result, ctx);
        }
    }

    /// Exactly one history entry per turn, however many tool calls it made.
    pub(super) fn assistant_commit_turn(&mut self, mut result: turn::TurnResult, ctx: &egui::Context) {
        let mut recorded = false;
        let erased_names = result
            .before
            .as_ref()
            .map(|before| panel::erased_names(before, &result.erased))
            .unwrap_or_default();
        if let Some(before) = result.before.take() {
            recorded = before != self.doc.model && self.shared_edit_allowed(ctx);
            let label = if result.stopped {
                tr!("assistant.history_stopped").to_owned()
            } else {
                let summary: String = result.summary.chars().take(60).collect();
                tr!("assistant.history_label", summary = summary.trim())
            };
            self.finish_edit(&label, before, ctx);
            // An edit refused for a shared project is rolled back as a whole.
            let touched: BTreeSet<u64> = result.created.iter().chain(&result.modified).copied().collect();
            let model = &self.doc.model;
            let focus: BTreeSet<u64> = touched
                .into_iter()
                .filter(|id| model.entity_in_context(*id) == Some(*id) && !model.is_hidden(*id))
                .collect();
            if !focus.is_empty() {
                self.selected = focus;
            }
        }
        if let Some(selection) = result.selection.take() {
            self.selected = selection
                .into_iter()
                .filter(|id| self.doc.model.contains_entity(id) && !self.doc.model.is_hidden(*id))
                .collect();
        }
        self.assistant.record(&result);
        // Remember where this turn sits in the history, so "Undo turn" is only
        // offered while it is still the newest step.
        let position = recorded.then(|| self.doc.history.position());
        if let Some(changes) = self
            .assistant
            .transcript
            .iter_mut()
            .rev()
            .find_map(|entry| entry.changes.as_mut())
        {
            changes.position = position;
            changes.erased_names = erased_names;
        }
        ctx.request_repaint();
    }
}
