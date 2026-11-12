use super::*;
use benchup_core::tr;

#[test]
fn skp_conversion_writes_the_same_document_as_import_then_save() {
    let directory = tempfile::tempdir().unwrap();
    for (fixture, bytes) in [
        ("placed-window", include_bytes!("../tests/fixtures/placed-window.skp").as_slice()),
        ("modern-triangle", include_bytes!("../tests/fixtures/modern-triangle.skp").as_slice()),
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
