use super::*;

// These factor/snap regressions exercise the arbitrary-anchor, center-based
// scale path. Grip-origin behavior has separate interaction coverage below.
fn start_center_scale(h: &mut Harness, pos: Pos2) {
    h.click(pos);
    let center = h.app.doc.model.selection_center(&h.app.selected).unwrap();
    let edit = h.app.modeling.as_mut().unwrap();
    edit.center = center;
    edit.scale_grip = None;
    edit.scale_constraint = None;
}

fn selection(h: &mut Harness) -> u64 {
    let edge = h.app.doc.model.add_edge(-DVec3::ONE, DVec3::ONE).unwrap();
    h.app.set_tool(Tool::Scale);
    h.app.selected.insert(edge);
    let pos = h.at(DVec3::ONE);
    start_center_scale(h, pos);
    h.app.doc.snap_to_geometry = false;
    h.app.doc.snap_to_axes = false;
    edge
}

#[test]
fn scaling_past_the_center_uses_the_factor_magnitude_for_preview_and_commit() {
    let mut h = Harness::new();
    let edge = selection(&mut h);
    let before = h.app.doc.model.clone();
    h.key(Key::ArrowRight);
    let cursor = h.at(DVec3::new(-2., 1., 1.));
    h.frame(
        vec![egui::Event::PointerMoved(cursor)],
        egui::Modifiers::NONE,
    );
    let edit = h.app.modeling.as_ref().unwrap();
    assert!((edit.value - 2.).abs() < 1e-5);
    assert!(edit.error.is_none());
    let vertex = edit.base.edges[&edge].b;
    assert!(
        edit.preview.as_ref().unwrap().vertices[&vertex]
            .position
            .distance(DVec3::new(2., 1., 1.))
            < 1e-5
    );
    let preview = edit.preview.clone().unwrap();
    h.click(cursor);
    assert!(h.app.modeling.is_none());
    assert_eq!(h.app.doc.model, preview);
    h.app.undo(&h.ctx, false);
    assert_eq!(h.app.doc.model, before);
}

#[test]
fn negative_typed_scale_factors_use_their_magnitude_and_zero_still_rejects_collapse() {
    for keys in [
        vec![],
        vec![Key::ArrowRight],
        vec![Key::ArrowRight, Key::ArrowLeft],
    ] {
        for input in ["-0.5", "(1 - 2) / 2"] {
            let mut h = Harness::new();
            let edge = selection(&mut h);
            let before = h.app.doc.model.clone();
            for key in &keys {
                h.key(*key);
            }
            let pointer = h.app.modeling.as_ref().unwrap().pointer;
            h.app.length_input = "-0".into();
            h.app.update_modeling(pointer);
            let edit = h.app.modeling.as_ref().unwrap();
            assert!(edit.error.is_some());
            assert!(edit.preview.is_none());
            assert_eq!(h.app.doc.model, before);

            h.app.length_input = input.into();
            h.app.update_modeling(pointer);
            let edit = h.app.modeling.as_ref().unwrap();
            assert!(edit.error.is_none());
            let mask = edit
                .scale_constraint
                .map_or(DVec3::ONE, |lock| lock.project(DVec3::ONE));
            let vertex = edit.base.edges[&edge].b;
            let expected = DVec3::ONE - mask * 0.5;
            assert!(
                edit.preview.as_ref().unwrap().vertices[&vertex]
                    .position
                    .distance(expected)
                    < 1e-7
            );
            h.key(Key::Enter);
            assert!(h.app.modeling.is_none());
            assert!(
                h.app.doc.model.vertices[&vertex]
                    .position
                    .distance(expected)
                    < 1e-7
            );
        }
    }
}

#[test]
fn scale_axis_and_plane_locks_constrain_pointer_preview_typed_commit_and_history() {
    for keys in [
        vec![],
        vec![Key::ArrowRight],
        vec![Key::ArrowLeft],
        vec![Key::ArrowUp],
        vec![Key::ArrowRight, Key::ArrowLeft],
        vec![Key::ArrowRight, Key::ArrowUp],
        vec![Key::ArrowLeft, Key::ArrowUp],
    ] {
        let mut h = Harness::new();
        // Parallel projection makes a 1.5x world radius a 1.5x screen radius.
        h.app.doc.camera.projection = benchup_core::Projection::Parallel;
        let edge = selection(&mut h);
        let before = h.app.doc.model.clone();
        for key in keys {
            h.key(key);
        }
        let mask = h
            .app
            .axis_constraint()
            .map_or(DVec3::ONE, |c| c.project(DVec3::ONE));
        let target = DVec3::ONE + mask * 0.5;
        h.frame(
            vec![egui::Event::PointerMoved(h.at(target))],
            egui::Modifiers::NONE,
        );
        let edit = h.app.modeling.as_ref().unwrap();
        assert!(
            (edit.value - 1.5).abs() < 1e-5,
            "{:?}: {}",
            edit.scale_constraint,
            edit.value
        );
        let vertex = before.edges[&edge].b;
        assert!(
            edit.preview.as_ref().unwrap().vertices[&vertex]
                .position
                .distance(target)
                < 1e-5
        );

        h.app.length_input = "2".into();
        h.key(Key::Enter);
        assert!(h.app.modeling.is_none());
        assert!(
            h.app.doc.model.vertices[&vertex]
                .position
                .distance(DVec3::ONE + mask)
                < 1e-7
        );
        let after = h.app.doc.model.clone();
        h.app.undo(&h.ctx, false);
        assert_eq!(h.app.doc.model, before);
        h.app.undo(&h.ctx, true);
        assert_eq!(h.app.doc.model, after);
    }
}

#[test]
fn scale_lock_changes_recompute_preview_at_the_last_pointer() {
    let mut h = Harness::new();
    selection(&mut h);
    let cursor = h.at(DVec3::new(2., 1., 1.));
    h.frame(
        vec![egui::Event::PointerMoved(cursor)],
        egui::Modifiers::NONE,
    );
    h.key(Key::ArrowRight);
    let edit = h.app.modeling.as_ref().unwrap();
    assert_eq!(edit.scale_constraint, Some(AxisConstraint::Axis(Axis::X)));
    assert!((edit.value - 2.).abs() < 1e-5);
    h.key(Key::ArrowLeft);
    assert_eq!(
        h.app.modeling.as_ref().unwrap().scale_constraint,
        Some(AxisConstraint::Plane(Axis::Z))
    );
    h.key(Key::ArrowDown);
    let edit = h.app.modeling.as_ref().unwrap();
    assert_eq!(edit.scale_constraint, None);
    let vertex = edit.base.edges.values().next().unwrap().b;
    let p = edit.preview.as_ref().unwrap().vertices[&vertex].position;
    assert!((p.x - p.y).abs() < 1e-7 && (p.y - p.z).abs() < 1e-7);
}

#[test]
fn uniform_pointer_scale_can_shrink_a_raised_reference_toward_the_center() {
    let mut h = Harness::new();
    h.app.doc.camera.projection = benchup_core::Projection::Parallel;
    let edge = selection(&mut h);
    let target = DVec3::ONE * 0.1;
    h.frame(
        vec![egui::Event::PointerMoved(h.at(target))],
        egui::Modifiers::NONE,
    );
    let edit = h.app.modeling.as_ref().unwrap();
    assert!((edit.value - 0.1).abs() < 1e-5);
    let vertex = edit.base.edges[&edge].b;
    assert!(
        edit.preview.as_ref().unwrap().vertices[&vertex]
            .position
            .distance(target)
            < 1e-5
    );
}

#[test]
fn unlocked_scale_responds_to_every_pointer_direction_without_a_fixed_reference_axis() {
    for projection in [
        benchup_core::Projection::Perspective,
        benchup_core::Projection::Parallel,
    ] {
        let mut h = Harness::new();
        let face = modeling_rectangle(&mut h);
        h.app.set_tool(Tool::Scale);
        h.app.selected.insert(face);
        h.app.doc.camera.projection = projection;
        h.app.doc.snap_to_geometry = false;
        h.app.doc.snap_to_axes = false;
        let center = h.at(DVec3::new(2., 2., 0.));
        let start = h.at(DVec3::new(4., 4., 0.));
        start_center_scale(&mut h, start);
        let radius = start.distance(center) * 1.25;
        for direction in [
            egui::vec2(1., 0.),
            egui::vec2(0., 1.),
            egui::vec2(-1., 0.),
            egui::vec2(0., -1.),
        ] {
            h.frame(
                vec![egui::Event::PointerMoved(center + direction * radius)],
                egui::Modifiers::NONE,
            );
            let edit = h.app.modeling.as_ref().unwrap();
            assert!(edit.scale_constraint.is_none());
            assert!(edit.error.is_none());
            assert!(
                (edit.value - 1.25).abs() < 1e-5,
                "{projection:?}, {direction:?}: {}",
                edit.value
            );
            let preview = edit.preview.as_ref().unwrap();
            assert!((preview.face_area(&preview.faces[&face]) - 25.).abs() < 1e-4);
            let target = h
                .app
                .doc
                .camera
                .project(edit.snap.unwrap().point, dv(h.app.viewport.size()))
                .unwrap();
            let cursor = dv(center + direction * radius - h.app.viewport.min);
            assert!(target.distance(cursor) < 1e-5);
        }
    }
}

#[test]
fn unlocked_scale_from_center_does_not_discard_motion_perpendicular_to_a_world_axis() {
    let mut h = Harness::new();
    let face = modeling_rectangle(&mut h);
    h.app.set_tool(Tool::Scale);
    h.app.selected.insert(face);
    h.app.doc.snap_to_geometry = false;
    h.app.doc.snap_to_axes = false;
    let start = h.at(DVec3::new(2., 2., 0.));
    h.click(start);
    let y = (h.at(DVec3::new(2., 4., 0.)) - start).normalized();
    for movement in [egui::vec2(30., 0.), egui::vec2(0., 30.)] {
        h.frame(
            vec![egui::Event::PointerMoved(start + movement)],
            egui::Modifiers::NONE,
        );
        let edit = h.app.modeling.as_ref().unwrap();
        assert!(edit.scale_constraint.is_none());
        assert!(edit.error.is_none());
        assert!(
            (edit.value - 1.).abs() > 1e-3,
            "{movement:?}: {}",
            edit.value
        );
    }
    let perpendicular = egui::vec2(-y.y, y.x) * 30.;
    let mut responds_to_perpendicular_motion = false;
    for movement in [perpendicular, -perpendicular] {
        h.frame(
            vec![egui::Event::PointerMoved(start + movement)],
            egui::Modifiers::NONE,
        );
        responds_to_perpendicular_motion |=
            (h.app.modeling.as_ref().unwrap().value - 1.).abs() > 1e-3;
    }
    assert!(responds_to_perpendicular_motion);
    h.frame(
        vec![egui::Event::PointerMoved(start)],
        egui::Modifiers::NONE,
    );
    assert!((h.app.modeling.as_ref().unwrap().value - 1.).abs() < 1e-5);
}

#[test]
fn unlocked_scale_keeps_exact_geometry_targets_when_grid_snapping_is_enabled() {
    let mut h = Harness::new();
    let edge = h.app.doc.model.add_edge(-DVec3::ONE, DVec3::ONE).unwrap();
    let target = DVec3::ONE * 2.;
    h.app.doc.model.add_edge(target, target + DVec3::X).unwrap();
    h.app.set_tool(Tool::Scale);
    h.app.selected.insert(edge);
    h.app.doc.snap_to_axes = false;
    h.app.snap_to_grid = true;
    let pos = h.at(DVec3::ONE);
    start_center_scale(&mut h, pos);
    h.frame(
        vec![egui::Event::PointerMoved(h.at(target) + egui::vec2(2., 0.))],
        egui::Modifiers::NONE,
    );
    let edit = h.app.modeling.as_ref().unwrap();
    assert!(edit.scale_constraint.is_none());
    assert_eq!(edit.snap.unwrap().kind, Inference::Endpoint);
    assert!((edit.value - 2.).abs() < 1e-7);
    let vertex = edit.base.edges[&edge].b;
    assert!(
        edit.preview.as_ref().unwrap().vertices[&vertex]
            .position
            .distance(target)
            < 1e-7
    );
}

#[test]
fn scale_object_targets_win_over_grid_and_disabled_object_snapping_is_respected() {
    for geometry in [false, true] {
        let mut h = Harness::new();
        let a = DVec3::new(0.23, 0.37, 0.41);
        let b = a + DVec3::X * 2.;
        let target = b + DVec3::X * 2.13;
        let edge = h.app.doc.model.add_edge(a, b).unwrap();
        h.app.doc.model.add_edge(target, target + DVec3::Y).unwrap();
        h.app.set_tool(Tool::Scale);
        h.app.selected.insert(edge);
        let pos = h.at(b);
        start_center_scale(&mut h, pos);
        h.app.doc.snap_to_geometry = geometry;
        h.app.doc.snap_to_axes = false;
        h.app.snap_to_grid = true;
        h.app.doc.display.grid_auto_size = false;
        h.app.doc.display.grid_spacing = 0.5;
        h.key(Key::ArrowRight);
        let cursor = h.at(target) + egui::vec2(2., 0.);
        h.frame(
            vec![egui::Event::PointerMoved(cursor)],
            egui::Modifiers::NONE,
        );
        let edit = h.app.modeling.as_ref().unwrap();
        let snap = edit.snap.unwrap();
        if geometry {
            assert_eq!(snap.kind, Inference::Endpoint);
            assert!(snap.point.distance(target) < 1e-7);
        } else {
            assert!((snap.point.x / 0.5 - (snap.point.x / 0.5).round()).abs() < 1e-7);
            assert!(snap.point.distance(target) > 0.01);
        }
        assert!((snap.point.y - b.y).abs() < 1e-7);
        assert!((snap.point.z - b.z).abs() < 1e-7);
        let vertex = edit.base.edges[&edge].b;
        assert!(
            edit.preview.as_ref().unwrap().vertices[&vertex]
                .position
                .distance(snap.point)
                < 1e-7
        );
        let preview = edit.preview.clone().unwrap();
        h.click(cursor);
        assert_eq!(h.app.doc.model, preview);
    }
}

#[test]
fn scale_from_center_has_a_usable_pointer_reference_and_typed_factors_override_snapping() {
    let mut h = Harness::new();
    let edge = h.app.doc.model.add_edge(-DVec3::X, DVec3::X).unwrap();
    h.app.set_tool(Tool::Scale);
    h.app.selected.insert(edge);
    h.click(h.at(DVec3::ZERO));
    h.app.doc.snap_to_geometry = false;
    h.app.doc.snap_to_axes = false;
    h.key(Key::ArrowRight);
    h.frame(
        vec![egui::Event::PointerMoved(h.at(DVec3::X))],
        egui::Modifiers::NONE,
    );
    assert!((h.app.modeling.as_ref().unwrap().value - 2.).abs() < 1e-5);
    h.app.snap_to_grid = true;
    h.app.length_input = "0.37".into();
    h.key(Key::Enter);
    let (a, b) = h.app.doc.model.points(&h.app.doc.model.edges[&edge]);
    assert!(a.distance(-DVec3::X * 0.37) < 1e-7);
    assert!(b.distance(DVec3::X * 0.37) < 1e-7);
}

#[test]
fn rectangle_scale_near_center_stays_proportional_for_three_pixel_movements() {
    for geometry in [false, true] {
        for distance in [18., 180.] {
            for keys in [
                vec![],
                vec![Key::ArrowRight],
                vec![Key::ArrowRight, Key::ArrowLeft],
            ] {
                for offset in [egui::vec2(0., 0.), egui::vec2(1., -1.), egui::vec2(5., 2.)] {
                    let mut h = Harness::new();
                    let face = modeling_rectangle(&mut h);
                    h.app.set_tool(Tool::Scale);
                    h.app.selected.insert(face);
                    h.app.doc.camera.distance = distance;
                    h.app.doc.snap_to_geometry = geometry;
                    h.app.doc.snap_to_axes = false;
                    for key in &keys {
                        h.key(*key);
                    }
                    let start = h.at(DVec3::new(2., 2., 0.)) + offset;
                    start_center_scale(&mut h, start);
                    let edit = h.app.modeling.as_ref().unwrap();
                    assert!(
                        (edit.value - 1.).abs() < 1e-5,
                        "initial factor: {}",
                        edit.value
                    );
                    for delta in [
                        egui::vec2(3., 0.),
                        egui::vec2(-3., 0.),
                        egui::vec2(0., 3.),
                        egui::vec2(0., -3.),
                    ] {
                        h.frame(
                            vec![egui::Event::PointerMoved(start + delta)],
                            egui::Modifiers::NONE,
                        );
                        let edit = h.app.modeling.as_ref().unwrap();
                        assert!(
                            (0.85..1.15).contains(&edit.value),
                            "geometry={geometry}, distance={distance}, keys={keys:?}, offset={offset:?}, delta={delta:?}: {}",
                            edit.value,
                        );
                        assert!(edit.preview.is_some());
                    }
                }
            }
        }
    }
}

#[test]
fn near_center_scale_uses_the_selection_extent_for_grid_and_exact_object_targets() {
    let mut h = Harness::new();
    let face = modeling_rectangle(&mut h);
    let center = DVec3::new(2., 2., 0.);
    let target = center + DVec3::X * 4.;
    h.app.doc.model.add_edge(target, target + DVec3::Z).unwrap();
    h.app.set_tool(Tool::Scale);
    h.app.selected.insert(face);
    h.app.doc.snap_to_axes = false;
    h.click(h.at(center) + egui::vec2(1., 0.));
    h.key(Key::ArrowRight);
    h.app.snap_to_grid = true;
    h.app.doc.display.grid_auto_size = false;
    h.app.doc.display.grid_spacing = 0.25;
    h.frame(
        vec![egui::Event::PointerMoved(h.at(target) + egui::vec2(2., 0.))],
        egui::Modifiers::NONE,
    );
    let edit = h.app.modeling.as_ref().unwrap();
    assert_eq!(edit.snap.unwrap().kind, Inference::Endpoint);
    assert!((edit.value - 3.).abs() < 1e-7);
}

#[test]
fn yellow_scale_grips_keep_opposite_origin_fixed_and_commit_one_undo_step() {
    for point in [DVec3::X, DVec3::new(1., 1., 0.), DVec3::ONE] {
        let mut h = Harness::new();
        let edge = h.app.doc.model.add_edge(-DVec3::ONE, DVec3::ONE).unwrap();
        h.app.selected.insert(edge);
        h.app.set_tool(Tool::Scale);
        h.app.doc.camera.projection = benchup_core::Projection::Parallel;
        let before = h.app.doc.model.clone();
        let pos = h.at(point);
        h.click(pos);
        let edit = h.app.modeling.as_ref().unwrap();
        assert_eq!(edit.anchor, point);
        assert_eq!(edit.center, -point);
        assert!(edit.scale_grip.is_some());
        let expected_constraint = if point == DVec3::X {
            Some(AxisConstraint::Axis(Axis::X))
        } else if point.z == 0. {
            Some(AxisConstraint::Plane(Axis::Z))
        } else {
            None
        };
        assert_eq!(edit.scale_constraint, expected_constraint);
        h.app.length_input = "2".into();
        h.app.update_modeling(pos);
        let edit = h.app.modeling.as_ref().unwrap();
        let mask = expected_constraint.map_or(DVec3::ONE, |c| c.project(DVec3::ONE));
        let vertex = before.edges[&edge].b;
        let expected = -point + (DVec3::ONE + point) * (DVec3::ONE + mask);
        assert!(
            edit.preview.as_ref().unwrap().vertices[&vertex]
                .position
                .distance(expected)
                < 1e-7
        );
        h.key(Key::Enter);
        assert_eq!(h.app.doc.history.position(), 1);
        assert!(
            h.app.doc.model.vertices[&vertex]
                .position
                .distance(expected)
                < 1e-7
        );
        h.app.undo(&h.ctx, false);
        assert_eq!(h.app.doc.model, before);
    }
}

#[test]
fn scale_grip_cancel_preserves_document_and_locked_selection_has_no_grips() {
    let mut h = Harness::new();
    let edge = h.app.doc.model.add_edge(-DVec3::ONE, DVec3::ONE).unwrap();
    h.app.selected.insert(edge);
    h.app.set_tool(Tool::Scale);
    h.frame(vec![], egui::Modifiers::NONE);
    let before = h.app.doc.clone();
    h.click(h.at(DVec3::ONE));
    h.app.length_input = "2".into();
    h.key(Key::Escape);
    assert!(h.app.modeling.is_none());
    assert_eq!(h.app.doc, before);
    h.app.doc.model.locked.insert(edge);
    assert!(h.app.scale_grip_at(h.at(DVec3::ONE), &h.ctx).is_none());
}

#[test]
fn scale_without_selection_selects_target_before_starting_a_handle_edit() {
    for grouped in [false, true] {
        let mut h = Harness::new();
        let face = modeling_rectangle(&mut h);
        let target = if grouped {
            h.app
                .doc
                .model
                .make_object(&BTreeSet::from([face]), false, "Panel")
                .unwrap()
        } else {
            face
        };
        h.app.selected.clear();
        h.app.set_tool(Tool::Scale);
        let before = h.app.doc.model.clone();
        let history_position = h.app.doc.history.position();
        h.click(h.at(DVec3::new(2., 2., 0.)));
        assert_eq!(h.app.selected, BTreeSet::from([target]));
        assert!(h.app.modeling.is_none(), "first click only selects");
        assert_eq!(h.app.doc.model, before);
        assert_eq!(h.app.doc.history.position(), history_position);
        assert!(!h.app.project.modified);

        let corner = h.at(DVec3::new(4., 4., 0.));
        assert!(h.app.scale_grip_at(corner, &h.ctx).is_some());
        h.click(corner);
        let edit = h
            .app
            .modeling
            .as_ref()
            .expect("second click starts scaling");
        assert!(edit.scale_grip.is_some());
        assert_eq!(edit.entities, BTreeSet::from([target]));
        assert_eq!(edit.center, DVec3::ZERO);
    }
}

#[test]
fn scale_click_on_empty_space_keeps_selection_and_edit_empty() {
    let mut h = Harness::new();
    h.app.set_tool(Tool::Scale);
    let before = h.app.doc.model.clone();
    h.click(h.app.viewport.center());
    assert!(h.app.selected.is_empty());
    assert!(h.app.modeling.is_none());
    assert_eq!(h.app.doc.model, before);
    assert!(h.app.doc.history.is_empty());
    assert!(!h.app.project.modified);
}

#[test]
fn scale_preview_reuses_model_allocations_and_unchanged_factors() {
    let mut h = Harness::new();
    let edge = selection(&mut h);
    assert!(h.app.modeling.as_ref().unwrap().scale_preview.is_some());
    let pos = h.app.modeling.as_ref().unwrap().pointer;
    let before = h.app.doc.model.clone();
    h.app.length_input = "2".into();
    h.app.update_modeling(pos);
    let vertex = before.edges[&edge].b;
    let allocation = &h
        .app
        .modeling
        .as_ref()
        .unwrap()
        .preview
        .as_ref()
        .unwrap()
        .vertices[&vertex] as *const _;
    for factor in ["2", "3", "0.5", "1"] {
        h.app.length_input = factor.into();
        h.app.update_modeling(pos);
        let edit = h.app.modeling.as_ref().unwrap();
        assert!(edit.error.is_none());
        assert_eq!(
            allocation,
            &edit.preview.as_ref().unwrap().vertices[&vertex] as *const _
        );
        assert_eq!(h.app.doc.model, before);
    }
    h.app.length_input = "0".into();
    h.app.update_modeling(pos);
    assert!(h.app.modeling.as_ref().unwrap().preview.is_none());
    h.app.length_input = "2".into();
    h.app.update_modeling(pos);
    assert!(h.app.modeling.as_ref().unwrap().error.is_none());
    let preview = h.app.modeling.as_ref().unwrap().preview.clone().unwrap();
    h.key(Key::Enter);
    assert_eq!(h.app.doc.model, preview);
    h.app.undo(&h.ctx, false);
    assert_eq!(h.app.doc.model, before);
}

#[test]
fn large_object_scale_uses_uniforms_until_commit_and_preserves_undo() {
    let mut h = Harness::new();
    h.app.doc.model = crate::picking::tests::triangle_grid(1024);
    let object = h.app.doc.model.make_object(&h.app.doc.model.root_entities(), false, "Large object").unwrap();
    h.app.selected = BTreeSet::from([object]);
    h.app.fit(&h.ctx);
    h.app.set_tool(Tool::Scale);
    h.frame(vec![], egui::Modifiers::NONE);
    let before = h.app.doc.model.clone();
    let (_, hi) = before.selection_bounds(&h.app.selected).unwrap();
    let pos = h.at(hi);
    h.app.begin_modeling(pos, &h.ctx);
    let mut expected = before.clone();
    for factor in ["1.1", "0.9", "1.2"] {
        h.app.length_input = factor.into();
        h.app.update_modeling(pos);
        let edit = h.app.modeling.as_ref().unwrap();
        assert!(edit.gpu_scale.is_some(), "Large complete objects must use GPU preview");
        assert!(edit.preview.is_none(), "GPU scaling must not rebuild CPU geometry");
        assert_eq!(h.app.doc.model, before);
        expected = before.clone();
        expected.scale_selection_constrained(&edit.entities, edit.center,
            factor.parse().unwrap(), edit.scale_constraint).unwrap();
    }
    h.key(Key::Enter);
    assert_eq!(h.app.doc.model, expected);
    h.app.undo(&h.ctx, false);
    assert_eq!(h.app.doc.model, before);
}
