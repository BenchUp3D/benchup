use super::*;

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
