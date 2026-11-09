//! `file` mode: apply tool calls to a `.bup` file, saving after each one.
use crate::protocol::Backend;
use benchup_core::Document;
use benchup_tools::{Call, ToolEnv, execute, files, is_mutating};
use serde_json::{Value, json};
use std::path::PathBuf;

pub struct FileBackend {
    path: PathBuf,
    doc: Document,
    env: ToolEnv,
    counter: u64,
}

impl FileBackend {
    pub fn open(path: impl Into<PathBuf>, allow_shared: bool) -> Result<Self, String> {
        let path = path.into();
        let bytes = std::fs::read(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let doc = files::decode(&bytes).map_err(|e| format!("cannot load {}: {e}", path.display()))?;
        if doc.shared_project.is_some() && !allow_shared {
            return Err(format!(
                "{} is a shared project; edits made here would diverge from the server. \
                 Pass --allow-shared to edit the local copy anyway.",
                path.display()
            ));
        }
        let env = ToolEnv {
            catalog: doc.model.stock_materials.clone(),
            selection: doc.selection.clone(),
            ..ToolEnv::default()
        };
        Ok(Self { path, doc, env, counter: 0 })
    }

    pub fn document(&self) -> &Document {
        &self.doc
    }

    fn save(&self, doc: &Document) -> Result<(), String> {
        let bytes = files::encode_document(doc)?;
        files::save_atomic(&self.path, &bytes)
    }
}

fn failure(message: String) -> (Value, bool) {
    (json!({"ok": false, "error": message}), false)
}

impl Backend for FileBackend {
    fn call(&mut self, _client: &str, name: &str, arguments: Value) -> (Value, bool) {
        self.counter += 1;
        // Per-turn state starts fresh; each call is its own undo step.
        self.env.before = None;
        self.env.created.clear();
        let backup = is_mutating(name).then(|| self.doc.clone());
        let selection_before = self.env.selection.clone();
        let before = self.doc.model.clone();
        let call = Call::new(self.counter.to_string(), name, arguments);
        let outcome = execute(&mut self.doc.model, &call, &mut self.env);
        if outcome.mutated {
            self.doc.history.record(format!("Agent: {name}"), before, &self.doc.model);
        }
        self.doc.selection = self.env.selection.clone();
        if outcome.mutated || self.env.selection != selection_before {
            if let Err(error) = self.save(&self.doc) {
                if let Some(backup) = backup {
                    self.doc = backup;
                }
                self.doc.selection = selection_before.clone();
                self.env.selection = selection_before;
                return failure(format!("could not save {}: {error}", self.path.display()));
            }
        }
        (outcome.result, outcome.ok)
    }
}
