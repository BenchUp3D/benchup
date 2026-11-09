//! The API key is remembered per server on this device only: a private file
//! natively, `localStorage` on the web. It never enters eframe storage,
//! preferences, or project files.
use super::client::normalize_base;
use crate::app::credentials as secrets;

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
fn key_file_name(base: &str) -> String {
    format!("assistant-{:016x}.key", base_hash(base))
}
#[cfg(target_arch = "wasm32")]
fn key_file_name(_base: &str) -> String {
    String::new()
}
#[cfg(test)]
fn key_file(dir: &std::path::Path, base: &str) -> std::path::PathBuf {
    dir.join(key_file_name(base))
}
fn storage_name(base: &str) -> String {
    format!("benchup.assistant.key.{}", normalize_base(base))
}

#[cfg(test)]
fn save_in(dir: &std::path::Path, base: &str, key: &str) -> Result<(), String> {
    secrets::save_in(dir, &key_file_name(base), key)
}
#[cfg(test)]
fn load_in(dir: &std::path::Path, base: &str) -> Option<String> {
    secrets::load_in(dir, &key_file_name(base))
}
#[cfg(test)]
fn forget_in(dir: &std::path::Path, base: &str) -> Result<(), String> {
    secrets::forget_in(dir, &key_file_name(base))
}

pub(crate) fn load(base: &str) -> Option<String> {
    secrets::load(&key_file_name(base), &storage_name(base))
}
pub(crate) fn save(base: &str, key: &str) -> Result<(), String> {
    secrets::save(&key_file_name(base), &storage_name(base), key)
}
pub(crate) fn forget(base: &str) -> Result<(), String> {
    secrets::forget(&key_file_name(base), &storage_name(base))
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
