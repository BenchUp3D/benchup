//! The editor side of every agent that edits the model through
//! `benchup_tools`: MIRA's turns and external agents connected through
//! `agent_link`. It builds the tool environment from the app's state and
//! commits one external call as one undo step, so both add-ons obey the same
//! catalog, read-only, lease and edit-scope rules.
// Without both add-ons some helpers have no caller.
#![cfg_attr(not(all(feature = "assistant", feature = "agent-link")), allow(dead_code))]
use super::*;
use benchup_tools::{Call, ToolEnv};
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// What a finished turn or agent call changed, for the change card.
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

/// Put the user back in the editing scope they had before an agent acted, or
/// at the top level if that object is gone.
pub(super) fn restore_scope(model: &mut Model, scope: Option<u64>) {
    model.edit_context = scope.filter(|id| model.objects.contains_key(id));
}

impl BenchUpApp {
    /// Whether "Build with MIRA" is compiled in and turned on.
    pub(super) fn assistant_available(&self) -> bool {
        cfg!(feature = "assistant") && self.general_settings.assistant_enabled
    }
    /// Sections that are hidden (not removed) while their add-on is off, so
    /// saved layouts keep their order.
    pub(super) fn section_hidden(&self, section: sidebar::InspectorSection) -> bool {
        section == sidebar::InspectorSection::Assistant && !self.assistant_available()
    }

    /// The tool environment for the current document: purchasing catalog,
    /// read-only and lease rules for shared projects, and the selection.
    pub(super) fn tool_env(&self, ctx: &egui::Context) -> ToolEnv {
        ToolEnv {
            read_only: !self.shared_edit_allowed(ctx),
            // Deliberate: external agents see the current selection (it lets
            // a user say "change this" and the agent find what "this" is).
            selection: self.selected.clone(),
            hide_selection: false,
            catalog: self
                .general_settings
                .stock
                .wood
                .iter()
                .map(|stock| (stock.id, stock.definition()))
                .collect(),
            before: None,
            lease: self.shared_lease(),
            created: BTreeSet::new(),
        }
    }

    /// Select what an agent just made or changed (top-level, visible entities).
    pub(super) fn focus_touched(&mut self, touched: impl IntoIterator<Item = u64>) {
        let model = &self.doc.model;
        let focus: BTreeSet<u64> = touched
            .into_iter()
            .filter(|id| model.entity_in_context(*id) == Some(*id) && !model.is_hidden(*id))
            .collect();
        if !focus.is_empty() {
            self.selected = focus;
        }
    }
    /// Follow the selection an agent asked for with the `select` tool.
    pub(super) fn apply_agent_selection(&mut self, selection: BTreeSet<u64>) {
        self.selected = selection
            .into_iter()
            .filter(|id| self.doc.model.contains_entity(id) && !self.doc.model.is_hidden(*id))
            .collect();
    }

    /// Run one call from an external agent. A call that edits the model is
    /// committed as exactly one history step, `Agent: <summary>`; if the edit
    /// is refused it is rolled back and the reason is returned.
    pub(super) fn run_agent_call(&mut self, call: Call, client: &str, ctx: &egui::Context) -> (bool, Value) {
        let mut env = self.tool_env(ctx);
        let scope = self.doc.model.edit_context;
        let outcome = benchup_tools::execute(&mut self.doc.model, &call, &mut env);
        let selection = (outcome.ok && call.name == "select").then(|| env.selection.clone());
        let Some(mut before) = env.before.take() else {
            // Nothing was edited (a read, a failure, or a no-op).
            restore_scope(&mut self.doc.model, scope);
            if let Some(selection) = selection {
                self.apply_agent_selection(selection);
            }
            ctx.request_repaint();
            return (outcome.ok, outcome.result);
        };
        // The snapshot belongs to the scope the designer was in, so undo
        // leaves them there whatever enter_object did meanwhile.
        before.edit_context = scope;
        restore_scope(&mut self.doc.model, scope);
        let exists = |ids: &[u64]| -> Vec<u64> {
            ids.iter().copied().filter(|id| self.doc.model.contains_entity(id) || self.doc.model.connections.contains_key(id)).collect()
        };
        let created = exists(&outcome.created);
        let modified: Vec<u64> = exists(&outcome.modified).into_iter().filter(|id| !created.contains(id)).collect();
        let erased: Vec<u64> = outcome.erased.iter().copied().filter(|id| !outcome.created.contains(id)).collect();
        let joints = exists(&outcome.joints);
        let record = self.assistant_available();
        let erased_names = if record { assistant::panel::erased_names(&before, &erased) } else { BTreeMap::new() };
        let summary = if created.is_empty() && modified.is_empty() {
            call.name.clone()
        } else {
            tr!("agent_link.summary", tool = call.name, created = created.len(), modified = modified.len())
        };
        let label = tr!("agent_link.history_label", summary = summary);
        let edited = before != self.doc.model;
        let recorded = self.commit_edit(&label, before, ctx);
        if edited && !recorded {
            // Shared edit access lapsed (or MIRA started): rolled back whole.
            let reason = if !self.shared_edit_allowed(ctx) {
                self.shared_edit_block_reason(ctx)
            } else {
                tr!("assistant.busy_blocked")
            };
            return (false, json!({"ok": false, "error": reason}));
        }
        self.focus_touched(created.iter().chain(&modified).copied());
        if let Some(selection) = selection {
            self.apply_agent_selection(selection);
        }
        let changes = assistant::Changes {
            created,
            modified,
            erased,
            joints,
            stopped: false,
            position: recorded.then(|| self.doc.history.position()),
            erased_names,
        };
        if record {
            self.assistant.record_agent(client, &call.name, changes);
        }
        ctx.request_repaint();
        (outcome.ok, outcome.result)
    }
}
