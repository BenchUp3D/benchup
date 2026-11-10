//! `file` mode: apply tool calls to a `.bup` file, saving after each one.
use crate::protocol::Backend;
use benchup_core::Document;
use benchup_tools::{Call, ToolEnv, execute, files, is_mutating};
use serde_json::{Value, json};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// What the project file looked like when this session last read or wrote it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Stamp {
    modified: Option<SystemTime>,
    len: u64,
    hash: u64,
}

impl Stamp {
    fn of(path: &Path) -> Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok();
        Ok(Self::from_bytes(modified, &bytes))
    }
    fn from_bytes(modified: Option<SystemTime>, bytes: &[u8]) -> Self {
        let mut hasher = DefaultHasher::new();
        bytes.hash(&mut hasher);
        Self { modified, len: bytes.len() as u64, hash: hasher.finish() }
    }
}

const CHANGED_ON_DISK: &str = "The project file changed on disk (was it saved in BenchUp?). \
Reopen it with a new benchup-mcp session.";

pub struct FileBackend {
    path: PathBuf,
    doc: Document,
    env: ToolEnv,
    counter: u64,
    stamp: Stamp,
}

impl FileBackend {
    pub fn open(path: impl Into<PathBuf>, allow_shared: bool) -> Result<Self, String> {
        let path = path.into();
        let bytes = std::fs::read(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        let stamp = Stamp::from_bytes(modified, &bytes);
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
        Ok(Self { path, doc, env, counter: 0, stamp })
    }

    pub fn document(&self) -> &Document {
        &self.doc
    }

    /// Refuses to overwrite a file somebody else (BenchUp, another session) changed.
    fn save(&mut self) -> Result<(), String> {
        if Stamp::of(&self.path).map_err(|_| CHANGED_ON_DISK.to_owned())? != self.stamp {
            return Err(CHANGED_ON_DISK.to_owned());
        }
        let bytes = files::encode_document(&self.doc)?;
        files::save_atomic(&self.path, &bytes)?;
        self.stamp = Stamp::of(&self.path)?;
        Ok(())
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
            if let Err(error) = self.save() {
                let changed = error == CHANGED_ON_DISK;
                if let Some(backup) = backup {
                    self.doc = backup;
                }
                self.doc.selection = selection_before.clone();
                self.env.selection = selection_before;
                if changed {
                    return failure(error);
                }
                return failure(format!("could not save {}: {error}", self.path.display()));
            }
        }
        (outcome.result, outcome.ok)
    }
}
