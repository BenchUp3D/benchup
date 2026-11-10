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
    // Write a private temp file next to the target, then rename it over the
    // target: the secret is never readable by others and an existing file is
    // never truncated in place.
    let temp = dir.join(format!(".{file_name}.{}.tmp", uuid::Uuid::new_v4().simple()));
    let mut options = std::fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let written = (|| -> std::io::Result<()> {
        let mut file = options.open(&temp)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        file.write_all(value.trim().as_bytes())?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temp, dir.join(file_name))
    })();
    if let Err(error) = written {
        let _ = std::fs::remove_file(&temp);
        return Err(error.to_string());
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn saving_over_a_world_readable_file_leaves_a_private_intact_file() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secret");
        std::fs::write(&path, "old").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        save_in(dir.path(), "secret", "  new-value \n").unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(load_in(dir.path(), "secret").as_deref(), Some("new-value"));
        let leftovers: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
        assert_eq!(leftovers.len(), 1, "no temp file is left behind");
    }

    #[test]
    fn a_new_file_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        save_in(&dir.path().join("nested"), "k", "v").unwrap();
        assert_eq!(load_in(&dir.path().join("nested"), "k").as_deref(), Some("v"));
    }
}
