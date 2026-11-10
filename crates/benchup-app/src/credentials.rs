//! Secrets that stay on this device: the assistant API key and the agent
//! pairing token. A private (0600) file in the data directory natively,
//! `localStorage` on the web. They never enter eframe storage, preferences, or
//! project files. Callers name the slot with a file name and a storage key.
// Each add-on uses a different subset (the assistant also forgets keys).
#![cfg_attr(not(all(feature = "assistant", feature = "agent-link")), allow(dead_code))]
#[cfg(target_arch = "wasm32")]
use benchup_core::tr;

#[cfg(any(not(target_arch = "wasm32"), test))]
pub(crate) fn save_in(dir: &std::path::Path, file_name: &str, value: &str) -> Result<(), String> {
    use std::io::Write;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let mut options = std::fs::OpenOptions::new();
    options.create(true).write(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(dir.join(file_name)).map_err(|e| e.to_string())?;
    // `mode` only applies when the file is created; tighten an existing one too.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?;
    }
    file.write_all(value.trim().as_bytes()).map_err(|e| e.to_string())
}
#[cfg(any(not(target_arch = "wasm32"), test))]
pub(crate) fn load_in(dir: &std::path::Path, file_name: &str) -> Option<String> {
    let value = std::fs::read_to_string(dir.join(file_name)).ok()?;
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}
#[cfg(any(not(target_arch = "wasm32"), test))]
pub(crate) fn forget_in(dir: &std::path::Path, file_name: &str) -> Result<(), String> {
    match std::fs::remove_file(dir.join(file_name)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn load(file_name: &str, _storage_key: &str) -> Option<String> {
    load_in(&crate::storage::data_dir().ok()?, file_name)
}
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn save(file_name: &str, _storage_key: &str, value: &str) -> Result<(), String> {
    save_in(&crate::storage::data_dir()?, file_name, value)
}
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn forget(file_name: &str, _storage_key: &str) -> Result<(), String> {
    forget_in(&crate::storage::data_dir()?, file_name)
}

#[cfg(target_arch = "wasm32")]
fn local_storage() -> Result<web_sys::Storage, String> {
    web_sys::window()
        .ok_or(tr!("assistant.error.no_browser"))?
        .local_storage()
        .map_err(|_| tr!("assistant.error.storage_unavailable"))?
        .ok_or(tr!("assistant.error.storage_unavailable"))
        .map_err(Into::into)
}
#[cfg(target_arch = "wasm32")]
pub(crate) fn load(_file_name: &str, storage_key: &str) -> Option<String> {
    let value = local_storage().ok()?.get_item(storage_key).ok()??;
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}
#[cfg(target_arch = "wasm32")]
pub(crate) fn save(_file_name: &str, storage_key: &str, value: &str) -> Result<(), String> {
    local_storage()?
        .set_item(storage_key, value.trim())
        .map_err(|_| tr!("assistant.error.cannot_save_key").to_owned())
}
#[cfg(target_arch = "wasm32")]
pub(crate) fn forget(_file_name: &str, storage_key: &str) -> Result<(), String> {
    local_storage()?
        .remove_item(storage_key)
        .map_err(|_| tr!("assistant.error.cannot_save_key").to_owned())
}
