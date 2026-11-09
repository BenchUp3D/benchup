//! Current identity and read-only compatibility with pre-rename installations.
#[cfg(not(target_arch = "wasm32"))]
pub const APP_ID: &str = "io.github.BenchUp3D.BenchUp";
#[cfg(not(target_arch = "wasm32"))]
pub const LEGACY_APP_ID: &str = "io.github.Ketchup3D.Ketchup";

pub fn preference_string(storage: &dyn eframe::Storage, key: &str) -> Option<String> {
    storage.get_string(key).or_else(|| {
        key.strip_prefix("benchup.")
            .and_then(|suffix| storage.get_string(&format!("ketchup.{suffix}")))
    })
}

pub fn preference_value<T: serde::de::DeserializeOwned>(
    storage: &dyn eframe::Storage,
    key: &str,
) -> Option<T> {
    // An unreadable current value must not resurrect an older preference.
    if storage.get_string(key).is_some() {
        eframe::get_value(storage, key)
    } else {
        key.strip_prefix("benchup.")
            .and_then(|suffix| eframe::get_value(storage, &format!("ketchup.{suffix}")))
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn data_override() -> Option<std::path::PathBuf> {
    std::env::var_os("BENCHUP_DATA_DIR")
        .or_else(|| std::env::var_os("KETCHUP_DATA_DIR"))
        .map(std::path::PathBuf::from)
}

/// Copy once, then atomically publish the complete directory. Keep the old data
/// untouched for rollback, and never overwrite a newer BenchUp workspace.
#[cfg(not(target_arch = "wasm32"))]
pub fn migrate_directory(new: &std::path::Path, old: &std::path::Path) -> std::io::Result<()> {
    if new.exists() || !old.is_dir() {
        return Ok(());
    }
    fn copy(source: &std::path::Path, destination: &std::path::Path) -> std::io::Result<()> {
        for entry in std::fs::read_dir(source)? {
            let entry = entry?;
            let target = destination.join(entry.file_name());
            let kind = entry.file_type()?;
            if kind.is_dir() {
                std::fs::create_dir(&target)?;
                copy(&entry.path(), &target)?;
            } else if kind.is_file() {
                std::fs::copy(entry.path(), target)?;
            }
        }
        Ok(())
    }
    let parent = new
        .parent()
        .ok_or_else(|| std::io::Error::other("Invalid data directory"))?;
    std::fs::create_dir_all(parent)?;
    let staged = tempfile::Builder::new()
        .prefix(".benchup-migration-")
        .tempdir_in(parent)?;
    copy(old, staged.path())?;
    match std::fs::rename(staged.path(), new) {
        Ok(()) => Ok(()),
        Err(_) if new.is_dir() => Ok(()), // Another startup completed migration.
        Err(error) => Err(error),
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn preferences_path() -> Result<std::path::PathBuf, String> {
    if let Some(path) = data_override() {
        return Ok(path.join("preferences.ron"));
    }
    let new = eframe::storage_dir(APP_ID).ok_or("Cannot locate application preferences")?;
    // Linux builds once also used the window title before adopting a desktop ID.
    #[cfg(target_os = "linux")]
    let old_ids = [LEGACY_APP_ID, "KetchUp — 3D workspace"];
    #[cfg(not(target_os = "linux"))]
    let old_ids = ["KetchUp — 3D workspace"];
    for old_id in old_ids {
        if let Some(old) = eframe::storage_dir(old_id) {
            migrate_directory(&new, &old)
                .map_err(|error| format!("Cannot migrate preferences: {error}"))?;
        }
    }
    Ok(new.join("app.ron"))
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[test]
    fn migration_copies_workspace_update_preferences_and_resume_without_overwriting_new_data() {
        let root = tempfile::tempdir().unwrap();
        let old = root.path().join("old");
        let new = root.path().join("new");
        std::fs::create_dir_all(old.join("accounts")).unwrap();
        for name in [
            "model.json",
            "model-edits.json",
            "model-selection.json",
            "updates.json",
            "update-resume.json",
            "app.ron",
            "accounts/session.json",
        ] {
            std::fs::write(old.join(name), name).unwrap();
        }
        migrate_directory(&new, &old).unwrap();
        for name in [
            "model.json",
            "updates.json",
            "update-resume.json",
            "app.ron",
            "accounts/session.json",
        ] {
            assert_eq!(std::fs::read(new.join(name)).unwrap(), name.as_bytes());
            assert!(old.join(name).exists());
        }
        std::fs::write(new.join("model.json"), "new work").unwrap();
        migrate_directory(&new, &old).unwrap();
        assert_eq!(
            std::fs::read_to_string(new.join("model.json")).unwrap(),
            "new work"
        );
    }

    #[test]
    fn renamed_preference_keys_restore_legacy_values_and_current_values_win() {
        #[derive(Default)]
        struct Storage(std::collections::BTreeMap<String, String>);
        impl eframe::Storage for Storage {
            fn get_string(&self, key: &str) -> Option<String> {
                self.0.get(key).cloned()
            }
            fn set_string(&mut self, key: &str, value: String) {
                self.0.insert(key.into(), value);
            }
            fn remove_string(&mut self, key: &str) {
                self.0.remove(key);
            }
            fn flush(&mut self) {}
        }
        let mut storage = Storage::default();
        eframe::set_value(&mut storage, "ketchup.inspector.v1", &42_u32);
        assert_eq!(
            preference_value::<u32>(&storage, "benchup.inspector.v1"),
            Some(42)
        );
        eframe::set_value(&mut storage, "benchup.inspector.v1", &7_u32);
        assert_eq!(
            preference_value::<u32>(&storage, "benchup.inspector.v1"),
            Some(7)
        );
        eframe::Storage::set_string(&mut storage, "benchup.inspector.v1", "invalid".into());
        assert_eq!(
            preference_value::<u32>(&storage, "benchup.inspector.v1"),
            None
        );
        eframe::Storage::remove_string(&mut storage, "benchup.inspector.v1");
        assert_eq!(
            preference_value::<u32>(&storage, "benchup.inspector.v1"),
            Some(42)
        );
    }
}
