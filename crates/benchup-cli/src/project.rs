//! A `.bup` file opened for tool calls. Every change is recorded in the
//! document history (so BenchUp can undo it) and saved atomically.
use benchup_core::Document;
use benchup_tools::{Call, ToolEnv, execute, files};
use serde_json::{Value, json};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};

pub const CHANGED_ON_DISK: &str =
    "The project file changed on disk since it was opened (was it saved in BenchUp?). Run the command again.";

/// Content fingerprint, to refuse overwriting a file somebody else changed.
fn stamp(bytes: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

pub struct Project {
    path: PathBuf,
    pub doc: Document,
    env: ToolEnv,
    stamp: u64,
    counter: u64,
}

/// Result of applying one or more calls.
pub enum Applied {
    /// Every call succeeded; one result per call.
    Ok(Vec<Value>),
    /// Call `index` failed; nothing was changed or saved.
    Failed { index: usize, result: Value },
}

impl Project {
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
        Ok(Self { path, doc, env, stamp: stamp(&bytes), counter: 0 })
    }

    /// Writes a new, empty project. Refuses to replace an existing file unless `force`.
    pub fn create(path: &Path, name: Option<&str>, force: bool) -> Result<Document, String> {
        if path.exists() && !force {
            return Err(format!("{} already exists (pass --force to replace it)", path.display()));
        }
        let mut doc = Document::default();
        doc.name = match name {
            Some(name) => name.to_owned(),
            None => path.file_stem().map_or(doc.name.clone(), |s| s.to_string_lossy().into_owned()),
        };
        files::save_atomic(path, &files::encode_document(&doc)?)?;
        Ok(doc)
    }

    pub fn save(&mut self) -> Result<(), String> {
        let current = std::fs::read(&self.path).map_err(|_| CHANGED_ON_DISK.to_owned())?;
        if stamp(&current) != self.stamp {
            return Err(CHANGED_ON_DISK.to_owned());
        }
        let bytes = files::encode_document(&self.doc)?;
        files::save_atomic(&self.path, &bytes)?;
        self.stamp = stamp(&bytes);
        Ok(())
    }

    /// Applies `calls` in order as one all-or-nothing step: if any call fails,
    /// the document is left as it was. Changes become a single undo entry.
    pub fn apply(&mut self, calls: &[(String, Value)]) -> Result<Applied, String> {
        let original = self.doc.clone();
        let selection_before = self.env.selection.clone();
        self.env.before = None;
        self.env.created.clear();
        let mut results = Vec::with_capacity(calls.len());
        let mut mutated = false;
        for (index, (name, arguments)) in calls.iter().enumerate() {
            self.counter += 1;
            let call = Call::new(self.counter.to_string(), name, arguments.clone());
            let outcome = execute(&mut self.doc.model, &call, &mut self.env);
            if !outcome.ok {
                self.doc = original;
                self.env.selection = selection_before;
                return Ok(Applied::Failed { index, result: outcome.result });
            }
            mutated |= outcome.mutated;
            results.push(outcome.result);
        }
        if mutated {
            let label = match calls {
                [(name, _)] => format!("Agent: {name}"),
                _ => format!("Agent: {} changes", calls.len()),
            };
            self.doc.history.record(label, original.model.clone(), &self.doc.model);
        }
        self.doc.selection = self.env.selection.clone();
        if (mutated || self.env.selection != selection_before)
            && let Err(error) = self.save()
        {
            self.doc = original;
            self.env.selection = selection_before;
            return Err(error);
        }
        Ok(Applied::Ok(results))
    }

    /// Undo (`redo == false`) or redo the last history entry and save.
    pub fn step_history(&mut self, redo: bool) -> Result<Value, String> {
        let label = if redo { self.doc.history.redo_label() } else { self.doc.history.undo_label() }
            .map(str::to_owned);
        let changed =
            if redo { self.doc.history.redo(&mut self.doc.model) } else { self.doc.history.undo(&mut self.doc.model) };
        if !changed {
            let what = if redo { "redo" } else { "undo" };
            return Ok(json!({"ok": false, "error": format!("nothing to {what}")}));
        }
        // Undo may remove entities the saved selection still points at.
        self.doc.selection.retain(|id| self.doc.model.contains_entity(id));
        self.env.selection = self.doc.selection.clone();
        self.save()?;
        let key = if redo { "redone" } else { "undone" };
        let mut result = json!({"ok": true});
        result[key] = json!(label);
        Ok(result)
    }
}

/// MCP: each `tools/call` is applied (and saved) on its own.
impl crate::mcp::Backend for Project {
    fn call(&mut self, _client: &str, name: &str, arguments: Value) -> (Value, bool) {
        match self.apply(&[(name.to_owned(), arguments)]) {
            Ok(Applied::Ok(mut results)) => (results.pop().unwrap_or(Value::Null), true),
            Ok(Applied::Failed { result, .. }) => (result, false),
            Err(error) => (json!({"ok": false, "error": error}), false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_to_overwrite_a_file_changed_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p.bup");
        Project::create(&path, Some("P"), false).unwrap();
        let mut project = Project::open(&path, false).unwrap();
        // BenchUp saves the project while this session has it open.
        let other = Document { name: "Saved elsewhere".into(), ..Document::default() };
        files::save_atomic(&path, &files::encode_document(&other).unwrap()).unwrap();

        let board = json!({"name": "B", "origin_mm": [0, 0, 0], "size_mm": [10, 10, 10]});
        let error = project.apply(&[("create_board".into(), board)]).err().unwrap();
        assert_eq!(error, CHANGED_ON_DISK);
        assert_eq!(files::decode(&std::fs::read(&path).unwrap()).unwrap().name, "Saved elsewhere");
        assert!(project.doc.model.root_entities().is_empty(), "the in-memory change is rolled back");
    }
}
