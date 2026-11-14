use super::*;

#[test]
fn arc_snaps_all_three_points_and_commits_as_one_edit() {
    let mut h = Harness::new();
    let start = DVec3::new(1., 1., 0.);
    let end = DVec3::new(5., 1., 0.);
    let through = DVec3::new(3., 3., 0.);
    for point in [start, end, through] {
        h.app
            .doc
            .model
            .draw_polyline(&[point, point + DVec3::Z], false)
            .unwrap();
    }
    h.app.set_tool(Tool::Arc);
    let before = h.app.doc.model.clone();
    for point in [start, end, through] {
        let cursor = h.at(point) + egui::vec2(1., 0.);
        h.frame(
            vec![egui::Event::PointerMoved(cursor)],
            egui::Modifiers::NONE,
        );
        let snap = h.app.snap.unwrap();
        assert_eq!(snap.kind, Inference::Endpoint);
        assert_eq!(snap.point, point);
        assert_eq!(h.app.doc.model, before);
        if point == through {
            let preview = h.app.drawing_points();
            assert_eq!(preview.first(), Some(&start));
            assert_eq!(preview.last(), Some(&end));
            assert!(preview.contains(&through));
            let center = DVec3::new(3., 1., 0.);
            assert!(
                preview
                    .iter()
                    .all(|p| (p.distance(center) - 2.).abs() < 1e-7)
            );
        }
        h.click(cursor);
    }
    assert!(h.app.drawing.is_none());
    assert!(h.app.doc.model.edges.len() > before.edges.len());
    assert!(h.app.doc.model.faces.is_empty());
    h.app.doc.model.validate().unwrap();
    h.app.undo(&h.ctx, false);
    assert_eq!(h.app.doc.model, before);
}

#[test]
fn arc_uses_locked_plane_grid_and_typed_chord_and_height() {
    let mut h = Harness::new();
    h.app.set_tool(Tool::Arc);
    h.app.doc.display.grid_auto_size = false;
    h.app.doc.display.grid_spacing = 0.5;
    h.app.snap_to_grid = true;
    h.app.doc.snap_to_axes = false;
    h.key(Key::ArrowRight);
    h.key(Key::ArrowUp);
    h.click(h.at(DVec3::ZERO));
    let aim = h.at(DVec3::new(3.12, 0., 0.07));
    h.frame(vec![egui::Event::PointerMoved(aim)], egui::Modifiers::NONE);
    assert_eq!(h.app.snap.unwrap().point, DVec3::new(3., 0., 0.));
    h.app.length_input = "4m".into();
    h.key(Key::Enter);
    let end = DVec3::new(4., 0., 0.);
    assert_eq!(h.app.drawing.as_ref().unwrap().arc_end, Some(end));
    assert!(h.app.length_input.is_empty());
    assert!(h.app.doc.model.edges.is_empty());
    h.key(Key::ArrowUp);
    // Removing Z would lock the YZ plane, which would move the second endpoint.
    assert_eq!(h.app.plane_lock(), Some(Axis::Y));
    let aim = h.at(DVec3::new(2., 0., 3.));
    h.frame(vec![egui::Event::PointerMoved(aim)], egui::Modifiers::NONE);
    h.app.length_input = "3m".into();
    let preview = h.app.drawing_points();
    assert!(preview.contains(&DVec3::new(2., 0., 3.)));
    assert!(preview.iter().all(|p| p.y.abs() < 1e-7));
    assert!(preview.iter().any(|p| p.x < 0.), "major arc");
    h.key(Key::Enter);
    assert!(h.app.drawing.is_none());
    assert_eq!(h.app.doc.model.edges.len(), preview.len() - 1);
}

#[test]
fn collinear_arc_stays_pending_and_escape_discards_it() {
    let mut h = Harness::new();
    h.app.set_tool(Tool::Arc);
    h.app.doc.display.grid_auto_size = false;
    h.app.doc.display.grid_spacing = 1.;
    h.app.snap_to_grid = true;
    h.click(h.at(DVec3::ZERO));
    h.click(h.at(DVec3::X * 4.));
    h.click(h.at(DVec3::X * 2.));
    assert!(h.app.drawing.as_ref().unwrap().arc_end.is_some());
    assert!(h.app.doc.model.edges.is_empty());
    h.key(Key::Escape);
    assert!(h.app.drawing.is_none());
    assert!(h.app.doc.model.edges.is_empty());
}

#[test]
fn arc_height_box_measures_the_sagitta_on_the_pointer_side() {
    let mut h = Harness::new();
    h.app.doc.snap_to_geometry = false;
    h.app.set_tool(Tool::Arc);
    h.click(h.at(DVec3::ZERO));
    h.click(h.at(DVec3::new(2., 0., 0.)));
    let drawing = h.app.drawing.as_ref().unwrap();
    let (start, end) = (drawing.origin, drawing.arc_end.unwrap());
    let midpoint = (start + end) * 0.5;
    let across = drawing.normal.cross((end - start).normalize());
    let mut snap = h.app.snap.unwrap();
    // A pointer at the apex gives its own distance from the chord.
    snap.point = midpoint + across * 0.5;
    h.app.snap = Some(snap);
    let (mid, apex, height) = h.app.arc_height_segment().unwrap();
    assert!(mid.distance(midpoint) < 1e-9);
    assert!((height - 0.5).abs() < 1e-9, "{height}");
    assert!(apex.distance(snap.point) < 1e-9);
    // Off-center pointers report the apex of the arc through them.
    snap.point = start + (end - start) * 0.25 + across * 0.3;
    h.app.snap = Some(snap);
    let (_, _, height) = h.app.arc_height_segment().unwrap();
    let sagitta = h.app.arc_points(false).unwrap().iter()
        .map(|p| (*p - start).dot(across))
        .fold(0., f64::max);
    assert!((height - sagitta).abs() < 1e-2, "{height} vs {sagitta}");
    assert!(height > 0.3);
    // A typed height is shown as-is, on the pointer's side.
    h.app.length_input = "10cm".into();
    let (_, apex, height) = h.app.arc_height_segment().unwrap();
    assert!((height - 0.1).abs() < 1e-9);
    assert!((apex - midpoint).dot(across) > 0.);
}
