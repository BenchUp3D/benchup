use ketchup_core::Document;
use ketchup_core::tr;

use std::{
    collections::BTreeSet,
    sync::{LazyLock, Mutex},
};
static AUTOSAVE_TOKEN: LazyLock<Mutex<Option<String>>> = LazyLock::new(|| Mutex::new(None));
#[derive(serde::Serialize, serde::Deserialize)]
struct SavedSelection {
    token: String,
    selected: BTreeSet<u64>,
    context: Option<u64>,
}
pub(crate) fn invalidate_selection_source() {
    *AUTOSAVE_TOKEN.lock().unwrap() = None;
}
fn source_token(json: &str) -> String {
    if let Some((_, tail)) = json.rsplit_once(",\"_ketchup_autosave\":\"")
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
    let token = ketchup_core::MaterialId::new_v4().to_string();
    json.pop();
    json.push_str(",\"_ketchup_autosave\":\"");
    json.push_str(&token);
    json.push_str("\"}");
    Ok((json, token))
}
fn restore_selection(json: &str, overlay: Option<&str>) -> Result<Document, String> {
    let mut doc = Document::from_json(json)?;
    let token = source_token(json);
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
    // An override makes automated/manual testing independent of personal models.
    if let Some(path) = std::env::var_os("KETCHUP_DATA_DIR") {
        return Ok(std::path::PathBuf::from(path));
    }
    directories::ProjectDirs::from("", "Ketchup", "Ketchup")
        .map(|d| d.data_local_dir().to_path_buf())
        .ok_or_else(|| tr!("storage.cannot_locate_the_application_data_directory").into())
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
        Ok(json) => restore_selection(
            &json,
            std::fs::read_to_string(data_dir()?.join("model-selection.json"))
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
    *AUTOSAVE_TOKEN.lock().unwrap() = Some(token);
    Ok(())
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
fn browser_storage() -> Result<web_sys::Storage, String> {
    web_sys::window()
        .ok_or(tr!("storage.no_browser_window"))?
        .local_storage()
        .map_err(|_| tr!("storage.local_storage_is_blocked").to_string())?
        .ok_or_else(|| tr!("storage.local_storage_is_unavailable").into())
}
#[cfg(target_arch = "wasm32")]
pub fn load() -> Result<Option<Document>, String> {
    *AUTOSAVE_TOKEN.lock().unwrap() = None;
    let storage = browser_storage()?;
    let json = storage
        .get_item("ketchup.document.v1")
        .map_err(|_| tr!("storage.cannot_read_local_storage").to_string())?;
    json.map(|json| {
        restore_selection(
            &json,
            storage
                .get_item("ketchup.selection.v1")
                .ok()
                .flatten()
                .as_deref(),
        )
    })
    .transpose()
}
#[cfg(target_arch = "wasm32")]
pub fn save(doc: &Document) -> Result<(), String> {
    *AUTOSAVE_TOKEN.lock().unwrap() = None;
    let (json, token) = autosave_json(doc)?;
    browser_storage()?
        .set_item("ketchup.document.v1", &json)
        .map_err(|_| tr!("storage.local_storage_is_blocked_or_full").to_string())?;
    *AUTOSAVE_TOKEN.lock().unwrap() = Some(token);
    Ok(())
}
#[cfg(target_arch = "wasm32")]
pub fn save_selection(doc: &Document) -> Result<(), String> {
    let Some(json) = selection_json(doc)? else {
        return save(doc);
    };
    browser_storage()?
        .set_item("ketchup.selection.v1", &json)
        .map_err(|_| tr!("storage.local_storage_is_blocked_or_full").to_string())
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[test]
    fn native_autosave_roundtrip_corruption_and_write_failure_recovery() {
        const CHILD: &str = "KETCHUP_AUTOSAVE_AUDIT_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let directory = tempfile::tempdir().unwrap();
            let result = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "storage::tests::native_autosave_roundtrip_corruption_and_write_failure_recovery", "--nocapture"])
                .env(CHILD, "1")
                .env("KETCHUP_DATA_DIR", directory.path().join("recovery"))
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
            .add_edge(ketchup_core::DVec3::ZERO, ketchup_core::DVec3::X)
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
            .add_edge(ketchup_core::DVec3::X, ketchup_core::DVec3::Y)
            .unwrap();
        std::fs::remove_file(file.parent().unwrap()).unwrap();
        save(&doc).unwrap();
        assert_eq!(load().unwrap().unwrap().model, doc.model);
    }
}
