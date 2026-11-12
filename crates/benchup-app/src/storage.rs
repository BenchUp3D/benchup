use benchup_core::Document;
use benchup_core::tr;

use std::{
    collections::BTreeSet,
    sync::{LazyLock, Mutex},
};
static AUTOSAVE_TOKEN: LazyLock<Mutex<Option<String>>> = LazyLock::new(|| Mutex::new(None));
static EDIT_BASE: LazyLock<Mutex<Option<(benchup_core::Model, benchup_core::History)>>> =
    LazyLock::new(|| Mutex::new(None));
#[derive(serde::Serialize, serde::Deserialize)]
struct SavedEdits {
    token: String,
    checkpoint: benchup_core::DocumentCheckpoint,
}
#[derive(serde::Serialize, serde::Deserialize)]
struct SavedSelection {
    token: String,
    selected: BTreeSet<u64>,
    context: Option<u64>,
}
pub(crate) fn invalidate_selection_source() {
    *AUTOSAVE_TOKEN.lock().unwrap() = None;
    *EDIT_BASE.lock().unwrap() = None;
}
fn source_token(json: &str) -> String {
    if let Some((_, tail)) = json
        .rsplit_once(",\"_benchup_autosave\":\"")
        .or_else(|| json.rsplit_once(",\"_ketchup_autosave\":\""))
        && let Some(token) = tail.strip_suffix("\"}")
    {
        return token.to_owned();
    }
    // Legacy autosaves need no rewrite: bind their overlay to the exact source.
    let hash = json.bytes().fold(0xcbf29ce484222325_u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    });
    format!("legacy:{}:{hash:016x}", json.len())
}
fn autosave_json(doc: &Document) -> Result<(String, String), String> {
    let mut json = serde_json::to_string(doc).map_err(|e| e.to_string())?;
    let token = benchup_core::MaterialId::new_v4().to_string();
    json.pop();
    json.push_str(",\"_benchup_autosave\":\"");
    json.push_str(&token);
    json.push_str("\"}");
    Ok((json, token))
}
#[cfg(test)]
fn restore_selection(json: &str, overlay: Option<&str>) -> Result<Document, String> {
    restore_autosave(json, overlay, None)
}
fn remember_base(doc: &Document) {
    *EDIT_BASE.lock().unwrap() = Some((doc.model.clone(), doc.history.clone()));
}
fn restore_autosave(
    json: &str,
    overlay: Option<&str>,
    edits: Option<&str>,
) -> Result<Document, String> {
    let mut doc = Document::from_json(json)?;
    let token = source_token(json);
    remember_base(&doc);
    if let Some(json) = edits {
        let saved: SavedEdits = serde_json::from_str(json)
            .map_err(|error| tr!("storage.cannot_restore_edits", error = error))?;
        if saved.token == token {
            doc = saved.checkpoint.restore(&doc.model, &doc.history)?;
        }
    }
    if let Some(selection) =
        overlay.and_then(|json| serde_json::from_str::<SavedSelection>(json).ok())
        && selection.token == token
    {
        doc.selection = selection.selected;
        doc.selection_context = selection.context;
    }
    *AUTOSAVE_TOKEN.lock().unwrap() = Some(token);
    Ok(doc)
}
fn edits_json(doc: &Document) -> Result<Option<String>, String> {
    let Some(token) = AUTOSAVE_TOKEN.lock().unwrap().clone() else {
        return Ok(None);
    };
    let base = EDIT_BASE.lock().unwrap();
    let Some((model, history)) = base.as_ref() else {
        return Ok(None);
    };
    let json = serde_json::to_string(&SavedEdits {
        token,
        checkpoint: doc.checkpoint(model, history),
    })
    .map_err(|error| error.to_string())?;
    // Compact large or long-running edit sessions into a complete snapshot.
    Ok((json.len() <= 2 * 1024 * 1024).then_some(json))
}
fn selection_json(doc: &Document) -> Result<Option<String>, String> {
    let Some(token) = AUTOSAVE_TOKEN.lock().unwrap().clone() else {
        return Ok(None);
    };
    serde_json::to_string(&SavedSelection {
        token,
        selected: doc.selection.clone(),
        context: doc.selection_context,
    })
    .map(Some)
    .map_err(|e| e.to_string())
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn data_dir() -> Result<std::path::PathBuf, String> {
    if let Some(path) = crate::identity::data_override() {
        return Ok(path);
    }
    let new = directories::ProjectDirs::from("", "BenchUp", "BenchUp")
        .map(|d| d.data_local_dir().to_path_buf())
        .ok_or_else(|| tr!("storage.cannot_locate_the_application_data_directory").to_owned())?;
    if let Some(old) = directories::ProjectDirs::from("", "Ketchup", "Ketchup") {
        crate::identity::migrate_directory(&new, old.data_local_dir())
            .map_err(|error| tr!("storage.cannot_migrate_workspace", error = error))?;
    }
    Ok(new)
}

#[cfg(not(target_arch = "wasm32"))]
fn path() -> Result<std::path::PathBuf, String> {
    Ok(data_dir()?.join("model.json"))
}

#[cfg(not(target_arch = "wasm32"))]
pub fn load() -> Result<Option<Document>, String> {
    *AUTOSAVE_TOKEN.lock().unwrap() = None;
    let file = path()?;
    match std::fs::read_to_string(file) {
        Ok(json) => restore_autosave(
            &json,
            std::fs::read_to_string(data_dir()?.join("model-selection.json"))
                .ok()
                .as_deref(),
            std::fs::read_to_string(data_dir()?.join("model-edits.json"))
                .ok()
                .as_deref(),
        )
        .map(Some),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}
#[cfg(not(target_arch = "wasm32"))]
pub fn save(doc: &Document) -> Result<(), String> {
    *AUTOSAVE_TOKEN.lock().unwrap() = None;
    let file = path()?;
    std::fs::create_dir_all(file.parent().ok_or(tr!("storage.invalid_storage_path"))?)
        .map_err(|e| e.to_string())?;
    let temporary = file.with_extension("json.tmp");
    let (json, token) = autosave_json(doc)?;
    {
        use std::io::Write;
        let mut out = std::fs::File::create(&temporary).map_err(|e| e.to_string())?;
        out.write_all(json.as_bytes())
            .and_then(|_| out.sync_all())
            .map_err(|e| e.to_string())?;
    }
    std::fs::rename(temporary, file).map_err(|e| e.to_string())?;
    match std::fs::remove_file(data_dir()?.join("model-edits.json")) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.to_string()),
    }
    *AUTOSAVE_TOKEN.lock().unwrap() = Some(token);
    remember_base(doc);
    Ok(())
}
#[cfg(not(target_arch = "wasm32"))]
pub fn save_edits(doc: &Document) -> Result<(), String> {
    let Some(json) = edits_json(doc)? else {
        return save(doc);
    };
    let file = data_dir()?.join("model-edits.json");
    let temporary = file.with_extension("json.tmp");
    {
        use std::io::Write;
        let mut output = std::fs::File::create(&temporary).map_err(|e| e.to_string())?;
        output
            .write_all(json.as_bytes())
            .and_then(|_| output.sync_all())
            .map_err(|e| e.to_string())?;
    }
    std::fs::rename(temporary, file).map_err(|e| e.to_string())?;
    save_selection(doc)
}
#[cfg(not(target_arch = "wasm32"))]
pub fn save_selection(doc: &Document) -> Result<(), String> {
    let Some(json) = selection_json(doc)? else {
        return save(doc);
    };
    let file = data_dir()?.join("model-selection.json");
    let temporary = file.with_extension("json.tmp");
    {
        use std::io::Write;
        let mut output = std::fs::File::create(&temporary).map_err(|e| e.to_string())?;
        output
            .write_all(json.as_bytes())
            .and_then(|_| output.sync_all())
            .map_err(|e| e.to_string())?;
    }
    std::fs::rename(temporary, file).map_err(|e| e.to_string())
}

#[cfg(target_arch = "wasm32")]
#[path = "browser_storage.rs"]
mod browser;
#[cfg(target_arch = "wasm32")]
pub use browser::{
    is_pending, load, poll_save, prepare as prepare_browser_storage, save, save_edits, save_selection,
};
#[cfg(not(target_arch = "wasm32"))]
pub fn is_pending() -> bool {
    false
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[test]
    fn renamed_autosave_preserves_legacy_snapshot_tokens_for_pending_overlays() {
        let mut doc = Document::default();
        doc.model
            .add_edge(benchup_core::DVec3::ZERO, benchup_core::DVec3::X)
            .unwrap();
        let (json, token) = autosave_json(&doc).unwrap();
        let legacy = json.replace("_benchup_autosave", "_ketchup_autosave");
        assert_eq!(source_token(&legacy), token);
        let edge = *doc.model.edges.keys().next().unwrap();
        let overlay = serde_json::to_string(&SavedSelection {
            token,
            selected: BTreeSet::from([edge]),
            context: None,
        })
        .unwrap();
        assert_eq!(
            restore_selection(&legacy, Some(&overlay))
                .unwrap()
                .selection,
            BTreeSet::from([edge])
        );
    }

    #[test]
    fn native_autosave_roundtrip_corruption_and_write_failure_recovery() {
        const CHILD: &str = "BENCHUP_AUTOSAVE_AUDIT_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let directory = tempfile::tempdir().unwrap();
            let result = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "storage::tests::native_autosave_roundtrip_corruption_and_write_failure_recovery", "--nocapture"])
                .env(CHILD, "1")
                .env("BENCHUP_DATA_DIR", directory.path().join("recovery"))
                .output().unwrap();
            assert!(
                result.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&result.stdout),
                String::from_utf8_lossy(&result.stderr)
            );
            return;
        }
        assert!(load().unwrap().is_none());
        let mut doc = Document::default();
        doc.name = "Autosave audit".into();
        doc.model
            .add_edge(benchup_core::DVec3::ZERO, benchup_core::DVec3::X)
            .unwrap();
        save(&doc).unwrap();
        assert_eq!(
            serde_json::to_value(load().unwrap().unwrap()).unwrap(),
            serde_json::to_value(&doc).unwrap()
        );
        let file = path().unwrap();
        std::fs::write(&file, b"corrupt recovery document").unwrap();
        assert!(load().is_err());
        assert_eq!(std::fs::read(&file).unwrap(), b"corrupt recovery document");
        save(&doc).unwrap();
        assert_eq!(load().unwrap().unwrap().name, doc.name);
        // Selection/context save immediately, without rewriting the geometry.
        let source = std::fs::read(&file).unwrap();
        let edge = *doc.model.edges.keys().next().unwrap();
        let group = doc
            .model
            .make_object(&BTreeSet::from([edge]), false, "Group")
            .unwrap();
        save(&doc).unwrap();
        let grouped_source = std::fs::read(&file).unwrap();
        doc.selection = doc.model.objects[&group].entities.clone();
        doc.selection_context = Some(group);
        save_selection(&doc).unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), grouped_source);
        let restored = load().unwrap().unwrap();
        assert_eq!(restored.selection, doc.selection);
        assert_eq!(restored.selection_context, Some(group));
        let overlay = data_dir().unwrap().join("model-selection.json");
        assert!(std::fs::metadata(&overlay).unwrap().len() < 1024);
        // A fresh autosave with reused IDs must ignore the old overlay.
        doc.selection.clear();
        doc.selection_context = None;
        save(&doc).unwrap();
        assert!(load().unwrap().unwrap().selection.is_empty());
        assert_eq!(load().unwrap().unwrap().selection_context, None);
        // Existing unversioned autosaves also support a document-bound overlay.
        std::fs::write(&file, serde_json::to_string(&doc).unwrap()).unwrap();
        load().unwrap();
        doc.selection.insert(group);
        save_selection(&doc).unwrap();
        assert_eq!(load().unwrap().unwrap().selection, doc.selection);
        std::fs::write(&file, source).unwrap();
        assert!(load().unwrap().unwrap().selection.is_empty());
        std::fs::write(&overlay, "corrupt selection").unwrap();
        assert!(load().unwrap().unwrap().selection.is_empty());
        std::fs::remove_file(&overlay).unwrap();
        // A blocked storage directory must report failure and leave the live
        // document available for further edits and an explicit retry.
        std::fs::remove_file(&file).unwrap();
        std::fs::remove_dir(file.parent().unwrap()).unwrap();
        std::fs::write(file.parent().unwrap(), b"blocked").unwrap();
        assert!(save(&doc).is_err());
        doc.model
            .add_edge(benchup_core::DVec3::X, benchup_core::DVec3::Y)
            .unwrap();
        std::fs::remove_file(file.parent().unwrap()).unwrap();
        save(&doc).unwrap();
        assert_eq!(load().unwrap().unwrap().model, doc.model);
    }
}
