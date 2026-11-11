use super::*;

fn block() -> (Harness, u64) {
    let mut h = Harness::new();
    let face = modeling_rectangle(&mut h);
    let cap = h
        .app
        .doc
        .model
        .push_pull_along(face, DVec3::Z * 2.)
        .unwrap();
    h.app.doc.snap_to_geometry = false;
    h.app.doc.snap_to_axes = false;
    h.app
        .doc
        .camera
        .fit(h.app.doc.model.bounds(), dv(h.app.viewport.size()));
    h.key(Key::P);
    (h, cap)
}

fn ctrl_click(h: &mut Harness, at: Pos2) {
    h.pointer_with_modifiers(
        at,
        egui::PointerButton::Primary,
        true,
        egui::Modifiers::CTRL,
    );
    h.pointer_with_modifiers(
        at,
        egui::PointerButton::Primary,
        false,
        egui::Modifiers::CTRL,
    );
}

fn ctrl_key(h: &mut Harness, key: Key) {
    h.frame(
        vec![egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::CTRL,
        }],
        egui::Modifiers::CTRL,
    );
}

fn assert_preserved(before: &Model, after: &Model) {
    for (id, vertex) in &before.vertices {
        assert_eq!(&after.vertices[id], vertex);
    }
    for (id, face) in &before.faces {
        assert_eq!(&after.faces[id], face);
    }
    after.validate().unwrap();
}

#[test]
fn ctrl_toggle_before_push_pull_stays_active_for_typed_commit_and_undo() {
    let (mut h, cap) = block();
    let before = h.app.doc.model.clone();
    let at = h.at(DVec3::new(2., 2., 2.));
    h.frame(vec![], egui::Modifiers::CTRL);
    h.frame(vec![], egui::Modifiers::NONE);
    assert!(h.app.push_pull_new_faces);
    assert_eq!(h.app.doc.model, before);
    h.click(at);
    assert_eq!(h.app.modeling.as_ref().unwrap().face, Some(cap));
    assert!(h.app.modeling.as_ref().unwrap().keep_starting_face);

    modeling_number(&mut h, "1m");
    assert!(h.app.modeling.is_none());
    assert_eq!(h.app.doc.model.faces.len(), 11);
    assert_preserved(&before, &h.app.doc.model);
    assert!(!h.app.selected.contains(&cap));
    assert_eq!(h.app.selected.len(), 1);
    assert_eq!(h.app.doc.history.undo_label(), Some("Push/Pull"));
    let after = h.app.doc.model.clone();
    h.app.undo(&h.ctx, false);
    assert_eq!(h.app.doc.model, before);
    h.app.undo(&h.ctx, true);
    assert_eq!(h.app.doc.model, after);
}

#[test]
fn ctrl_presses_toggle_push_pull_preview_and_releases_leave_it_unchanged() {
    let (mut h, _) = block();
    let before = h.app.doc.model.clone();
    h.click(h.at(DVec3::new(2., 2., 2.)));
    let end = h.at(DVec3::new(2., 2., 3.));
    h.frame(vec![egui::Event::PointerMoved(end)], egui::Modifiers::NONE);
    h.frame(vec![egui::Event::Text("1m".into())], egui::Modifiers::NONE);
    h.frame(vec![], egui::Modifiers::NONE);
    let normal = h.app.modeling.as_ref().unwrap().preview.clone().unwrap();
    assert_eq!(normal.faces.len(), 6);

    for new_faces in [true, false, true, false] {
        h.frame(vec![], egui::Modifiers::CTRL);
        let edit = h.app.modeling.as_ref().unwrap();
        assert_eq!(edit.keep_starting_face, new_faces);
        let preview = edit.preview.as_ref().unwrap();
        assert_eq!(preview.faces.len(), if new_faces { 11 } else { 6 });
        if new_faces {
            assert_preserved(&before, preview);
        } else {
            assert_eq!(preview, &normal);
        }
        let preview = preview.clone();
        assert_eq!(h.app.doc.model, before);
        assert!(!h.app.project.modified);
        // Holding the key and releasing it must not toggle a second time.
        h.frame(vec![], egui::Modifiers::CTRL);
        assert_eq!(
            h.app.modeling.as_ref().unwrap().preview.as_ref().unwrap(),
            &preview
        );
        h.frame(vec![], egui::Modifiers::NONE);
        let edit = h.app.modeling.as_ref().unwrap();
        assert_eq!(edit.keep_starting_face, new_faces);
        assert_eq!(edit.preview.as_ref().unwrap(), &preview);
    }
    h.frame(vec![], egui::Modifiers::CTRL);
    ctrl_key(&mut h, Key::Escape);
    assert!(h.app.modeling.is_none());
    assert_eq!(h.app.doc.model, before);
    assert!(h.app.doc.history.is_empty());
    h.click(h.at(DVec3::new(2., 2., 2.)));
    assert!(h.app.modeling.as_ref().unwrap().keep_starting_face);
}

#[test]
fn ctrl_push_pull_commits_new_faces_with_the_pointer() {
    let (mut h, _) = block();
    let before = h.app.doc.model.clone();
    h.click(h.at(DVec3::new(2., 2., 2.)));
    let end = h.at(DVec3::new(2., 2., 3.));
    h.frame(vec![egui::Event::PointerMoved(end)], egui::Modifiers::CTRL);
    let preview = h.app.modeling.as_ref().unwrap().preview.clone().unwrap();
    assert_eq!(preview.faces.len(), 11);
    h.frame(vec![], egui::Modifiers::NONE);
    h.click(end);
    assert!(h.app.modeling.is_none());
    assert_eq!(h.app.doc.model, preview);
    assert_preserved(&before, &h.app.doc.model);
}

#[test]
fn second_ctrl_press_restores_normal_push_pull_with_axis_lock() {
    let (mut h, cap) = block();
    let before = h.app.doc.model.clone();
    let at = h.at(DVec3::new(2., 2., 2.));
    ctrl_click(&mut h, at);
    ctrl_key(&mut h, Key::ArrowUp);
    assert_eq!(h.app.single_axis_lock(), Some(Axis::Z));
    h.frame(vec![egui::Event::Text("1m".into())], egui::Modifiers::CTRL);
    h.frame(vec![], egui::Modifiers::CTRL);
    assert_eq!(
        h.app
            .modeling
            .as_ref()
            .unwrap()
            .preview
            .as_ref()
            .unwrap()
            .faces
            .len(),
        11
    );
    h.frame(vec![], egui::Modifiers::NONE);
    assert!(h.app.modeling.as_ref().unwrap().keep_starting_face);
    h.frame(vec![], egui::Modifiers::CTRL);
    assert!(!h.app.modeling.as_ref().unwrap().keep_starting_face);
    h.frame(vec![], egui::Modifiers::NONE);
    h.key(Key::Enter);
    assert!(h.app.modeling.is_none());
    assert_eq!(h.app.doc.model.faces.len(), 6);
    assert_eq!(h.app.selected, BTreeSet::from([cap]));
    assert_ne!(h.app.doc.model, before);
    h.app.doc.model.validate().unwrap();
}

#[test]
fn physical_ctrl_keys_toggle_once_and_shortcuts_leave_the_mode_unchanged() {
    let (mut h, _) = block();
    let before = h.app.doc.model.clone();
    for key in [Key::ControlLeft, Key::ControlRight] {
        ctrl_key(&mut h, key);
        let mode = h.app.push_pull_new_faces;
        ctrl_key(&mut h, key);
        assert_eq!(h.app.push_pull_new_faces, mode);
        h.frame(vec![], egui::Modifiers::NONE);
        assert_eq!(h.app.push_pull_new_faces, mode);
    }
    assert!(!h.app.push_pull_new_faces);
    ctrl_key(&mut h, Key::A);
    assert!(!h.app.push_pull_new_faces);
    h.frame(vec![], egui::Modifiers::NONE);
    assert!(!h.app.push_pull_new_faces);
    assert_eq!(h.app.doc.model, before);
    assert!(!h.app.project.modified);
}

#[test]
fn ctrl_tap_between_frames_toggles_once() {
    let (mut h, _) = block();
    for mode in [true, false] {
        h.frame(
            vec![
                egui::Event::ModifiersChanged(egui::Modifiers::CTRL),
                egui::Event::ModifiersChanged(egui::Modifiers::NONE),
            ],
            egui::Modifiers::NONE,
        );
        assert_eq!(h.app.push_pull_new_faces, mode);
        h.frame(vec![], egui::Modifiers::NONE);
        assert_eq!(h.app.push_pull_new_faces, mode);
    }
}

#[test]
fn ctrl_pressed_in_a_dialog_does_not_toggle_when_the_dialog_closes() {
    let (mut h, _) = block();
    h.app.help = true;
    h.frame(vec![], egui::Modifiers::CTRL);
    assert!(!h.app.push_pull_new_faces);
    h.app.help = false;
    h.frame(vec![], egui::Modifiers::CTRL);
    assert!(!h.app.push_pull_new_faces);
    h.frame(vec![], egui::Modifiers::NONE);
    h.frame(vec![], egui::Modifiers::CTRL);
    assert!(h.app.push_pull_new_faces);
}

#[test]
fn pull_readout_includes_existing_depth_and_signed_difference() {
    let (mut h, _) = block();
    h.click(h.at(DVec3::new(2., 2., 2.)));
    let edit = h.app.modeling.as_ref().unwrap();
    assert!((edit.pull_depth - 2.).abs() < 1e-7);
    let units = h.app.doc.display.units;
    for (difference, total) in [(1., 3.), (-0.5, 1.5), (0., 2.)] {
        assert_eq!(
            h.app.push_pull_readout(edit, difference),
            format!("Difference: {} · Total: {}", units.format_length(difference), units.format_length(total)),
        );
    }
    assert!(!h.app.project.modified);
}

#[test]
fn pull_snapping_excludes_its_boundary_even_through_connected_geometry() {
    let mut h = Harness::new();
    let face = modeling_rectangle(&mut h);
    // A connected edge must not reintroduce the face's corner as an endpoint.
    h.app.doc.model.add_edge(DVec3::new(4., 4., 0.), DVec3::new(4., 4., 2.)).unwrap();
    let reference = DVec3::new(1., 1., 3.);
    h.app.doc.model.add_edge(reference, reference + DVec3::X).unwrap();
    h.app.doc.snap_to_axes = false;
    h.app.set_tool(Tool::PushPull);
    h.app.begin_modeling(h.at(DVec3::new(1.3, 1.7, 0.)), &h.ctx);
    assert_eq!(h.app.modeling.as_ref().unwrap().face, Some(face));
    for point in [DVec3::new(4., 4., 0.), DVec3::new(2., 4., 0.)] {
        let cursor = h.at(point);
        h.app.update_modeling(cursor);
        let edit = h.app.modeling.as_ref().unwrap();
        assert!(edit.snap.is_none(), "self snap at {point:?}: {:?}", edit.snap);
        let ray = h.app.doc.camera.ray(dv(cursor - h.app.viewport.min), dv(h.app.viewport.size()));
        let expected = (ray.axis(edit.anchor, edit.direction).unwrap() - edit.anchor).dot(edit.direction);
        assert!((edit.value - expected).abs() < 1e-5);
    }
    h.app.update_modeling(h.at(reference));
    let edit = h.app.modeling.as_ref().unwrap();
    assert_eq!(edit.snap.unwrap().kind, Inference::Endpoint);
    assert!(edit.snap.unwrap().point.distance(reference) < 1e-7);
    assert!((edit.value.abs() - 3.).abs() < 1e-7);
}

#[test]
fn invalid_pull_reference_falls_back_to_free_drag_without_a_warning() {
    let (mut h, _) = block();
    let invalid = DVec3::new(6., 2., 0.);
    let valid = DVec3::new(6., 2., 3.);
    for point in [invalid, valid] {
        h.app.doc.model.add_edge(point, point + DVec3::X).unwrap();
    }
    h.app.begin_modeling(h.at(DVec3::new(2., 2., 2.)), &h.ctx);
    let before = h.app.doc.model.clone();
    let edit = h.app.modeling.as_ref().unwrap();
    let invalid_value = (invalid - edit.anchor).dot(edit.direction);
    assert!(BenchUpApp::modeling_result(Tool::PushPull, edit, invalid_value).is_err());
    h.app.doc.snap_to_geometry = true;
    h.app.update_modeling(h.at(invalid));
    let edit = h.app.modeling.as_ref().unwrap();
    assert!(edit.snap.is_none());
    assert!(edit.error.is_none());
    assert!(edit.preview.is_some());
    assert!((edit.value - invalid_value).abs() > 1e-5);
    assert_eq!(h.app.doc.model, before);

    h.app.update_modeling(h.at(valid));
    let edit = h.app.modeling.as_ref().unwrap();
    assert_eq!(edit.snap.unwrap().kind, Inference::Endpoint);
    assert!(edit.preview.is_some());
    assert!(edit.error.is_none());
}

#[test]
fn flat_face_starts_with_zero_pull_depth() {
    let mut h = Harness::new();
    let face = modeling_rectangle(&mut h);
    let plane = benchup_core::faces::plane(&h.app.doc.model, &h.app.doc.model.faces[&face].vertices).unwrap();
    assert!(BenchUpApp::push_pull_depth(&h.app.doc.model, face, plane.origin, plane.normal).abs() < 1e-7);
}
