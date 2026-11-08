use super::*;
use ketchup_core::DVec3;

#[test]
fn ketchup_files_round_trip_complete_projects_and_replace_existing_files() {
    let directory = tempfile::tempdir().unwrap();
    for (index, extension) in PROJECT_EXTENSIONS.iter().enumerate() {
        let path = directory.path().join(format!("house.{extension}"));
        let mut doc = Document {
            name: "House".into(),
            snap_to_axes: false,
            ..Default::default()
        };
        doc.model.add_edge(DVec3::ZERO, DVec3::X).unwrap();
        doc.camera.yaw += 0.4;
        let json = serde_json::to_vec(&doc).unwrap();
        write(&path, &json).unwrap();
        let restored = decode(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(restored.name, doc.name);
        assert_eq!(restored.model, doc.model);
        assert_eq!(restored.snap_to_axes, doc.snap_to_axes);
        assert_eq!(restored.camera.yaw, doc.camera.yaw);
        doc.name = "Renamed house".into();
        write(&path, &serde_json::to_vec(&doc).unwrap()).unwrap();
        assert_eq!(
            decode(&std::fs::read(&path).unwrap()).unwrap().name,
            doc.name
        );
        assert_eq!(
            std::fs::read_dir(directory.path()).unwrap().count(),
            index + 1
        );
    }
}

#[test]
fn compressed_and_plain_projects_preserve_the_document_and_undo_redo_history() {
    let mut doc = Document {
        name: "House – étage 🏠".into(),
        snap_to_axes: false,
        snap_to_geometry: false,
        ..Default::default()
    };
    for (y, label) in [(0., "First line"), (2., "Second line")] {
        let before = doc.model.clone();
        let edge = doc
            .model
            .add_edge(DVec3::Y * y, DVec3::Y * y + DVec3::X)
            .unwrap();
        doc.history.record(label, before, &doc.model);
        if y == 0. {
            doc.selection.insert(edge);
        }
    }
    assert!(doc.history.undo(&mut doc.model));
    doc.camera.yaw += 0.4;
    doc.viewport_cameras.insert(1, doc.camera.clone());
    doc.display.grid = false;
    let json = serde_json::to_string(&doc).unwrap();
    let compressed = encode(&json).unwrap();
    assert!(compressed.starts_with(&[0x1f, 0x8b, 0x08]));
    assert!(compressed.len() < json.len());
    for bytes in [json.as_bytes(), compressed.as_slice()] {
        let mut restored = decode(bytes).unwrap();
        assert_eq!(restored, doc);
        assert_eq!(restored.history.undo_label(), Some("First line"));
        assert_eq!(restored.history.redo_label(), Some("Second line"));
        assert!(restored.history.undo(&mut restored.model));
        assert!(restored.model.edges.is_empty());
        assert!(restored.history.redo(&mut restored.model));
        assert!(restored.history.redo(&mut restored.model));
        assert_eq!(restored.model.edges.len(), 2);
    }
}

#[test]
fn corrupt_or_incomplete_compressed_projects_are_rejected() {
    let compressed = encode(&serde_json::to_string(&Document::default()).unwrap()).unwrap();
    for length in [2, 10, compressed.len() / 2, compressed.len() - 1] {
        assert!(
            decode(&compressed[..length]).is_err(),
            "Truncated at {length}"
        );
    }
    let mut corrupt = compressed.clone();
    let checksum = corrupt.len() - 8;
    corrupt[checksum] ^= 1;
    assert!(decode(&corrupt).is_err());
    let mut trailing = compressed;
    trailing.extend_from_slice(b"unexpected trailing data");
    assert!(decode(&trailing).is_err());
    assert!(decode(&encode("not json").unwrap()).is_err());
}

#[test]
fn invalid_project_files_are_rejected() {
    assert!(decode(b"not json").is_err());
    assert!(decode(&[0xff]).is_err());
    let mut json = serde_json::to_value(Document::default()).unwrap();
    json["version"] = 999.into();
    assert!(decode(&serde_json::to_vec(&json).unwrap()).is_err());
    assert!(decode(&encode(&json.to_string()).unwrap()).is_err());
    json["version"] = 2.into();
    json["camera"]["distance"] = (-1).into();
    assert!(decode(&serde_json::to_vec(&json).unwrap()).is_err());
    assert!(decode(&encode(&json.to_string()).unwrap()).is_err());
}

#[test]
fn failed_save_keeps_existing_file_intact() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("house.ketchup");
    std::fs::write(&path, b"original").unwrap();
    assert!(write(&path.join("missing/project.ketchup"), b"new").is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"original");
}

#[test]
fn save_to_opened_path_writes_without_showing_a_picker() {
    let directory = tempfile::tempdir().unwrap();
    for extension in PROJECT_EXTENSIONS {
        let path = directory.path().join(format!("saved.{extension}"));
        let mut doc = Document::default();
        let before = doc.model.clone();
        doc.model.add_edge(DVec3::ZERO, DVec3::X).unwrap();
        doc.history.record("Line", before, &doc.model);
        // Saving over a plain JSON project upgrades its encoding atomically.
        std::fs::write(&path, serde_json::to_vec(&doc).unwrap()).unwrap();
        let pending = save(
            &eframe::egui::Context::default(),
            Some(path.clone()),
            doc.name.clone(),
            serde_json::to_string(&doc).unwrap(),
        );
        match pending
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap()
            .unwrap()
        {
            Outcome::Saved(Some(destination)) => assert_eq!(destination, path),
            outcome => panic!("Unexpected save result: {outcome:?}"),
        }
        let saved = std::fs::read(&path).unwrap();
        assert!(saved.starts_with(&[0x1f, 0x8b, 0x08]));
        assert_eq!(decode(&saved).unwrap(), doc);
        let mut json = String::new();
        MultiGzDecoder::new(saved.as_slice())
            .read_to_string(&mut json)
            .unwrap();
        assert_eq!(json, serde_json::to_string(&doc).unwrap());
    }
}

#[test]
fn project_file_names_and_extensions_are_portable() {
    assert_eq!(file_name(" My house "), "My house.k3d");
    assert_eq!(file_name("House.k3d"), "House.k3d");
    assert_eq!(file_name("House.K3D"), "House.K3D");
    assert_eq!(file_name("House.ketchup"), "House.ketchup");
    assert_eq!(file_name(" House.KETCHUP. "), "House.KETCHUP");
    assert_eq!(file_name("../floor:1"), "_floor_1.k3d");
    assert_eq!(file_name(" . "), "Untitled model.k3d");
    assert_eq!(file_name("House.json"), "House.json.k3d");
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("house");
    assert_eq!(
        project_path(path.clone()).unwrap(),
        path.with_extension("k3d")
    );
    assert_eq!(
        project_path(path.with_extension("json")).unwrap(),
        directory.path().join("house.json.k3d")
    );
    for extension in ["k3d", "K3D", "ketchup", "KETCHUP"] {
        let explicit = path.with_extension(extension);
        // The picker has already confirmed this exact destination.
        std::fs::write(&explicit, b"original").unwrap();
        assert_eq!(project_path(explicit.clone()).unwrap(), explicit);
    }
    assert!(
        project_path(path).is_err(),
        "Appending an extension must not bypass overwrite confirmation"
    );
    assert_eq!(
        std::fs::read(directory.path().join("house.k3d")).unwrap(),
        b"original"
    );
}

#[test]
fn opening_either_extension_uses_the_file_stem_and_defaults_save_as_to_k3d() {
    let directory = tempfile::tempdir().unwrap();
    let doc = Document::default();
    let json = serde_json::to_string(&doc).unwrap();
    let compressed = encode(&json).unwrap();
    for extension in ["k3d", "K3D", "ketchup", "KETCHUP"] {
        let path = directory.path().join(format!("My house.{extension}"));
        for bytes in [json.as_bytes(), compressed.as_slice()] {
            write(&path, bytes).unwrap();
            let Outcome::Opened(mut restored, Some(opened_path)) =
                read_project(path.clone()).unwrap()
            else {
                panic!("Expected an opened project");
            };
            assert_eq!(opened_path, path);
            set_document_name(&mut restored, &opened_path);
            assert_eq!(restored.name, "My house");
            assert_eq!(file_name(&restored.name), "My house.k3d");
        }
    }
}
