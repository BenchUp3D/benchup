//! The API key is remembered per server on this device only: a private file
//! natively, `localStorage` on the web. It never enters eframe storage,
//! preferences, or project files.
use super::client::normalize_base;
#[cfg(target_arch = "wasm32")]
use benchup_core::tr;

/// FNV-1a keeps file names stable and free of URL characters.
#[cfg(any(not(target_arch = "wasm32"), test))]
fn base_hash(base: &str) -> u64 {
    normalize_base(base)
        .as_bytes()
        .iter()
        .fold(0xcbf29ce484222325u64, |hash, b| {
            (hash ^ *b as u64).wrapping_mul(0x100000001b3)
        })
}
#[cfg(any(not(target_arch = "wasm32"), test))]
fn key_file(dir: &std::path::Path, base: &str) -> std::path::PathBuf {
    dir.join(format!("assistant-{:016x}.key", base_hash(base)))
}
#[cfg(any(target_arch = "wasm32", test))]
fn storage_name(base: &str) -> String {
    format!("benchup.assistant.key.{}", normalize_base(base))
}

#[cfg(any(not(target_arch = "wasm32"), test))]
fn save_in(dir: &std::path::Path, base: &str, key: &str) -> Result<(), String> {
    use std::io::Write;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let mut options = std::fs::OpenOptions::new();
    options.create(true).write(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let path = key_file(dir, base);
    let mut file = options.open(&path).map_err(|e| e.to_string())?;
    // `mode` only applies when the file is created; tighten an existing one too.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?;
    }
    file.write_all(key.trim().as_bytes()).map_err(|e| e.to_string())
}
#[cfg(any(not(target_arch = "wasm32"), test))]
fn load_in(dir: &std::path::Path, base: &str) -> Option<String> {
    let key = std::fs::read_to_string(key_file(dir, base)).ok()?;
    let key = key.trim();
    (!key.is_empty()).then(|| key.to_owned())
}
#[cfg(any(not(target_arch = "wasm32"), test))]
fn forget_in(dir: &std::path::Path, base: &str) -> Result<(), String> {
    match std::fs::remove_file(key_file(dir, base)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn load(base: &str) -> Option<String> {
    load_in(&crate::storage::data_dir().ok()?, base)
}
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn save(base: &str, key: &str) -> Result<(), String> {
    save_in(&crate::storage::data_dir()?, base, key)
}
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn forget(base: &str) -> Result<(), String> {
    forget_in(&crate::storage::data_dir()?, base)
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
pub(crate) fn load(base: &str) -> Option<String> {
    let key = local_storage().ok()?.get_item(&storage_name(base)).ok()??;
    let key = key.trim();
    (!key.is_empty()).then(|| key.to_owned())
}
#[cfg(target_arch = "wasm32")]
pub(crate) fn save(base: &str, key: &str) -> Result<(), String> {
    local_storage()?
        .set_item(&storage_name(base), key.trim())
        .map_err(|_| tr!("assistant.error.cannot_save_key").to_owned())
}
#[cfg(target_arch = "wasm32")]
pub(crate) fn forget(base: &str) -> Result<(), String> {
    local_storage()?
        .remove_item(&storage_name(base))
        .map_err(|_| tr!("assistant.error.cannot_save_key").to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_round_trip_per_server_in_a_private_file() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = ("https://a.example/v1", "https://b.example/v1");
        assert_eq!(load_in(dir.path(), a), None);
        save_in(dir.path(), a, "  sk-secret \n").unwrap();
        assert_eq!(load_in(dir.path(), a).as_deref(), Some("sk-secret"));
        // A trailing slash is the same server; another server has no key.
        assert_eq!(load_in(dir.path(), "https://a.example/v1/").as_deref(), Some("sk-secret"));
        assert_eq!(load_in(dir.path(), b), None);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(key_file(dir.path(), a)).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        save_in(dir.path(), a, "sk-new").unwrap();
        assert_eq!(load_in(dir.path(), a).as_deref(), Some("sk-new"));
        forget_in(dir.path(), a).unwrap();
        assert_eq!(load_in(dir.path(), a), None);
        forget_in(dir.path(), a).unwrap();
        assert_eq!(storage_name("https://a.example/v1/"), "benchup.assistant.key.https://a.example/v1");
    }

    #[cfg(unix)]
    #[test]
    fn an_existing_world_readable_key_file_is_tightened() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let base = "https://a.example/v1";
        let path = key_file(dir.path(), base);
        std::fs::write(&path, "old").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        save_in(dir.path(), base, "sk-new").unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(load_in(dir.path(), base).as_deref(), Some("sk-new"));
    }
}
