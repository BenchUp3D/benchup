use super::*;

fn inspector_image(model: &mut Model, offset: DVec3) -> (u64, u64) {
    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgba8(1, 1)
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    let group = model
        .add_reference_image(
            [DVec3::ZERO, DVec3::X, DVec3::X + DVec3::Y, DVec3::Y].map(|p| p + offset),
            benchup_core::ReferenceImage {
                name: "Plan.png".into(),
                asset: benchup_core::reference_images::ImageAssetId::new_v4(),
                vertices: [0; 4],
                settings: Default::default(),
                placement_offset: DVec3::ZERO,
            },
            benchup_core::ImageAsset {
                png: png.into_inner().into(),
                pixels: [1, 1],
            },
        )
        .unwrap();
    let face = model.objects[&group]
        .entities
        .iter()
        .find(|id| model.reference_images.contains_key(id))
        .copied()
        .unwrap();
    (group, face)
}

#[test]
fn reference_image_inspector_preserves_direct_parent_and_ambiguous_assembly_rules() {
    let mut h = Harness::new();
    let edge = h.app.doc.model.add_edge(DVec3::ZERO, DVec3::Z).unwrap();
    h.app.selected = BTreeSet::from([edge]);
    assert_eq!(h.app.selected_reference_image(), None);
    let (group, face) = inspector_image(&mut h.app.doc.model, DVec3::X * 2.);
    for id in [face, group] {
        h.app.selected = BTreeSet::from([id]);
        assert_eq!(h.app.selected_reference_image(), Some(face));
    }
    h.app.selected = BTreeSet::from([edge, group]);
    assert_eq!(h.app.selected_reference_image(), None);
    let outer = h
        .app
        .doc
        .model
        .make_object(&BTreeSet::from([group]), false, "Assembly")
        .unwrap();
    h.app.selected = BTreeSet::from([outer]);
    assert_eq!(
        h.app.selected_reference_image(),
        None,
        "A nested image is not the selected assembly's image"
    );
    let nested = *h.app.doc.model.reference_images.keys().next().unwrap();
    let direct_parent = h.app.doc.model.parent(nested).unwrap();
    h.app.selected = BTreeSet::from([direct_parent]);
    assert_eq!(h.app.selected_reference_image(), Some(nested));
    let (other_group, other_face) = inspector_image(&mut h.app.doc.model, DVec3::X * 4.);
    // One direct image plus another nested image must remain ambiguous.
    h.app
        .doc
        .model
        .objects
        .get_mut(&other_group)
        .unwrap()
        .entities
        .insert(outer);
    h.app.selected = BTreeSet::from([other_group]);
    assert_eq!(h.app.selected_reference_image(), None);
    h.app.selected = BTreeSet::from([other_face]);
    assert_eq!(h.app.selected_reference_image(), Some(other_face));
    h.app.doc.model.reference_images.clear();
    assert_eq!(
        h.app.selected_reference_image(),
        None,
        "Document changes cannot retain an old image result"
    );
}

#[test]
fn image_rectangle_uses_geometry_snapping_and_axis_locked_precision() {
    let mut h = Harness::new();
    let anchor = DVec3::new(0.31, 0.27, 0.);
    h.app.doc.model.add_edge(anchor, anchor + DVec3::X).unwrap();
    h.app.set_tool(Tool::Image);
    h.key(Key::ArrowUp);
    h.click(h.at(anchor) + egui::vec2(2., -1.));
    assert!(
        h.app
            .drawing
            .as_ref()
            .unwrap()
            .origin
            .abs_diff_eq(anchor, 1e-7)
    );
    assert_eq!(h.app.drawing.as_ref().unwrap().normal, DVec3::Z);
    h.frame(
        vec![egui::Event::PointerMoved(
            h.at(anchor + DVec3::new(2., 1., 0.)),
        )],
        egui::Modifiers::NONE,
    );
    h.app.length_input = "2m,1m".into();
    let points = h.app.precise_drawing_points().unwrap();
    assert!((points[0].distance(points[1]) - 2.).abs() < 1e-7);
    assert!((points[1].distance(points[2]) - 1.).abs() < 1e-7);
    assert!(points.iter().all(|p| (p.z - anchor.z).abs() < 1e-7));
    h.click(h.at(anchor + DVec3::new(2., 1., 0.)));
    assert!(h.app.pending_image_rectangle().is_some());
    assert_eq!(
        h.app.doc.model.edges.len(),
        1,
        "picker does not create provisional geometry"
    );
}
#[test]
fn image_drag_finishes_rectangle_and_cancelled_picker_leaves_no_model_edit() {
    let mut h = Harness::new();
    h.app.set_tool(Tool::Image);
    h.app.doc.snap_to_geometry = false;
    h.app.doc.snap_to_axes = false;
    h.key(Key::ArrowUp);
    let a = h.at(DVec3::ZERO);
    let b = h.at(DVec3::new(2., 1., 0.));
    h.pointer(a, egui::PointerButton::Primary, true, false);
    h.frame(vec![egui::Event::PointerMoved(b)], egui::Modifiers::NONE);
    h.pointer(b, egui::PointerButton::Primary, false, false);
    let points = h.app.pending_image_rectangle().unwrap();
    assert!(points[0].abs_diff_eq(DVec3::ZERO, 1e-6));
    assert!((points[0].distance(points[1]) - 2.).abs() < 1e-6);
    h.frame(vec![], egui::Modifiers::NONE);
    assert!(h.app.doc.model.reference_images.is_empty());
    assert!(h.app.doc.model.faces.is_empty());
    assert!(h.app.doc.history.is_empty());
    assert!(h.app.drawing.is_none());
}

#[test]
fn image_rectangles_follow_each_pair_of_axis_locks() {
    for (first, second, normal) in [
        (Key::ArrowRight, Key::ArrowLeft, Axis::Z),
        (Key::ArrowRight, Key::ArrowUp, Axis::Y),
        (Key::ArrowLeft, Key::ArrowUp, Axis::X),
    ] {
        let mut h = Harness::new();
        h.app.set_tool(Tool::Image);
        h.app.doc.snap_to_geometry = false;
        h.app.doc.snap_to_axes = false;
        h.key(first);
        h.key(second);
        h.click(h.at(DVec3::ZERO));
        let drawing = h.app.drawing.as_ref().unwrap();
        assert_eq!(drawing.normal, normal.vector());
        let target = drawing.origin + drawing.u * 2. + drawing.v;
        h.frame(
            vec![egui::Event::PointerMoved(h.at(target))],
            egui::Modifiers::NONE,
        );
        let points = h.app.drawing_points();
        assert_eq!(points.len(), 5);
        assert!(points.iter().all(|p| p.dot(normal.vector()).abs() < 1e-6));
        assert!((points[0].distance(points[1]) - 2.).abs() < 1e-6);
        assert!((points[0].distance(points[3]) - 1.).abs() < 1e-6);
        h.key(Key::Escape);
        assert!(h.app.drawing.is_none());
        assert!(h.app.doc.model.reference_images.is_empty());
    }
}
