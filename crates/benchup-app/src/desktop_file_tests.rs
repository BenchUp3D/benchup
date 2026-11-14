use super::*;
use benchup_core::tr;

#[test]
fn skp_conversion_writes_the_same_document_as_import_then_save() {
    let directory = tempfile::tempdir().unwrap();
    for (fixture, bytes) in [
        ("placed-window", include_bytes!("../../benchup-io/tests/fixtures/placed-window.skp").as_slice()),
        ("modern-triangle", include_bytes!("../../benchup-io/tests/fixtures/modern-triangle.skp").as_slice()),
    ] {
        let input = directory.path().join(format!("{fixture}.skp"));
        let output = directory.path().join(format!("{fixture}-converted.bup"));
        std::fs::write(&input, bytes).unwrap();
        convert_skp(&input, &output).unwrap();
        let written = std::fs::read(&output).unwrap();
        assert!(written.starts_with(&[0x1f, 0x8b]), "conversion must not embed a preview");
        let mut doc = crate::project_files::decode(&written).unwrap();
        assert_eq!(doc.name, format!("{fixture}-converted"));
        doc.model.validate().unwrap();
        let imported = crate::interchange::import_skp(bytes, fixture).unwrap();
        assert_eq!(doc.model.faces.len(), imported.faces.len());
        assert_eq!(doc.model.objects.len(), imported.objects.len());
        assert_eq!(doc.model.objects.last_key_value().unwrap().1.name, fixture);
        assert!(!doc.selection.is_empty());
        assert_eq!(doc.history.undo_label(), Some(tr!("projects.import_sketchup")));
        assert!(doc.history.undo(&mut doc.model));
        assert_eq!(doc.model, Model::default());
    }
}

#[test]
fn skp_conversion_rejects_bad_or_missing_input_without_writing_output() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("bad.skp");
    let output = directory.path().join("out.bup");
    std::fs::write(&input, b"not a SketchUp file").unwrap();
    assert!(convert_skp(&input, &output).is_err());
    assert!(convert_skp(&directory.path().join("missing.skp"), &output).is_err());
    assert!(!output.exists());
}

#[test]
fn repeated_document_requests_open_once_with_the_latest_last() {
    // Uses a local queue: app tests drain the shared queue on every frame.
    let mut queue = VecDeque::new();
    let first = PathBuf::from("/tmp/launch-first.bup");
    let second = PathBuf::from("/tmp/launch-second.bup");
    push_latest(&mut queue, first.clone());
    push_latest(&mut queue, second.clone());
    push_latest(&mut queue, first.clone());
    assert_eq!(queue, [second, first]);
}

#[test]
fn only_project_paths_are_queued_and_made_absolute() {
    assert_eq!(project_path("https://example.com/share/abc".into()), None);
    assert_eq!(project_path("/tmp/notes.txt".into()), None);
    let absolute = std::env::temp_dir().join("Chair.BUP");
    assert_eq!(project_path(absolute.clone()), Some(absolute));
    let relative = project_path("chair.bup".into()).unwrap();
    assert!(relative.is_absolute());
    assert_eq!(relative, std::env::current_dir().unwrap().join("chair.bup"));
}
