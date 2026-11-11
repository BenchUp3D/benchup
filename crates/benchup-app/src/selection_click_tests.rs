use super::*;

fn body_fixture() -> (Harness, u64, BTreeSet<u64>, BTreeSet<u64>) {
    let mut h = Harness::new();
    let face = properties_outline(
        &mut h.app.doc.model,
        &[
            DVec3::ZERO,
            DVec3::X * 4.,
            DVec3::new(4., 4., 0.),
            DVec3::Y * 4.,
        ],
    );
    let cap = h.app.doc.model.push_pull(face, 2.).unwrap();
    let body = h.app.doc.model.selectable_entities();
    let mut perimeter = BTreeSet::from([cap]);
    perimeter.extend(
        h.app
            .doc
            .model
            .edges
            .values()
            .filter(|edge| {
                let (a, b) = h.app.doc.model.points(edge);
                (a.z - 2.).abs() < 1e-6 && (b.z - 2.).abs() < 1e-6
            })
            .map(|edge| edge.id),
    );
    assert_eq!(perimeter.len(), 5);
    properties_outline(
        &mut h.app.doc.model,
        &[
            DVec3::new(6., 0., 0.),
            DVec3::new(8., 0., 0.),
            DVec3::new(8., 4., 0.),
            DVec3::new(6., 4., 0.),
        ],
    );
    h.app
        .doc
        .camera
        .set_standard_view(benchup_core::StandardView::Top);
    h.app.fit(&h.ctx);
    h.frame(vec![], egui::Modifiers::NONE);
    (h, cap, perimeter, body)
}

#[test]
fn repeated_face_clicks_select_face_perimeter_then_only_its_connected_body() {
    let (mut h, cap, perimeter, body) = body_fixture();
    let point = h.at(DVec3::new(2., 2., 2.));
    let model = h.app.doc.model.clone();
    h.click(point);
    assert_eq!(h.app.selected, BTreeSet::from([cap]));
    h.click(point);
    assert_eq!(h.app.selected, perimeter);
    h.click(point);
    assert_eq!(h.app.selected, body);
    assert_eq!(h.app.doc.model, model);
    assert!(h.app.doc.history.is_empty());
    // A new click after the multi-click timeout starts with the face again.
    h.time += 1.;
    h.click(point);
    assert_eq!(h.app.selected, BTreeSet::from([cap]));
}

#[test]
fn triple_clicking_an_edge_selects_connected_faces_and_edges() {
    let (mut h, _, _, body) = body_fixture();
    let point = h.at(DVec3::new(2., 0., 2.));
    h.click(point);
    assert_eq!(h.app.selected.len(), 1);
    assert!(
        h.app
            .doc
            .model
            .edges
            .contains_key(h.app.selected.first().unwrap())
    );
    h.click(point);
    h.click(point);
    assert_eq!(h.app.selected, body);
}

#[test]
fn shift_multi_click_toggles_each_extent_against_the_original_selection() {
    for initially_selected in [false, true] {
        let (mut h, cap, perimeter, body) = body_fixture();
        let other = *h
            .app
            .doc
            .model
            .faces
            .keys()
            .find(|id| !body.contains(id))
            .unwrap();
        h.app.selected = BTreeSet::from([other]);
        if initially_selected {
            h.app.selected.extend(&body);
        }
        let before = h.app.selected.clone();
        let point = h.at(DVec3::new(2., 2., 2.));
        for extent in [BTreeSet::from([cap]), perimeter, body] {
            h.pointer(point, egui::PointerButton::Primary, true, true);
            h.pointer(point, egui::PointerButton::Primary, false, true);
            assert_eq!(
                h.app.selected,
                before.symmetric_difference(&extent).copied().collect()
            );
        }
    }
}

#[test]
fn double_click_still_opens_groups_and_selection_stays_in_their_editing_scope() {
    let (mut h, _, _, _) = body_fixture();
    // Include the disconnected face in the same group: triple-click must still
    // select only connected geometry, rather than every entity in that group.
    let all = h.app.doc.model.selectable_entities();
    let group = h
        .app
        .doc
        .model
        .make_object(&all, false, "Assembly")
        .unwrap();
    // Grouping remaps geometry IDs.
    let cap = h
        .app
        .doc
        .model
        .faces
        .values()
        .find(|face| {
            face.vertices
                .iter()
                .all(|id| (h.app.doc.model.vertices[id].position.z - 2.).abs() < 1e-6)
        })
        .unwrap()
        .id;
    let mut perimeter = BTreeSet::from([cap]);
    perimeter.extend(
        h.app
            .doc
            .model
            .edges
            .values()
            .filter(|edge| {
                let (a, b) = h.app.doc.model.points(edge);
                (a.z - 2.).abs() < 1e-6 && (b.z - 2.).abs() < 1e-6
            })
            .map(|edge| edge.id),
    );
    h.frame(vec![], egui::Modifiers::NONE);
    let point = h.at(DVec3::new(2., 2., 2.));
    h.click(point);
    assert_eq!(h.app.selected, BTreeSet::from([group]));
    h.click(point);
    assert_eq!(h.app.doc.model.edit_context, Some(group));
    assert!(h.app.selected.is_empty());
    let body = h.app.doc.model.solid_entities(cap).unwrap();
    for extent in [BTreeSet::from([cap]), perimeter, body] {
        h.click(point);
        assert_eq!(h.app.selected, extent);
        assert!(!h.app.selected.contains(&group));
    }
}

#[test]
fn multiple_click_releases_in_one_frame_expand_in_event_order() {
    let (mut h, _, _, body) = body_fixture();
    let point = h.at(DVec3::new(2., 2., 2.));
    let mut events = vec![egui::Event::PointerMoved(point)];
    for _ in 0..3 {
        for pressed in [true, false] {
            events.push(egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            });
        }
    }
    h.frame(events, egui::Modifiers::NONE);
    assert_eq!(h.app.selected, body);
}

#[test]
fn repeated_selection_ignores_hidden_geometry_and_hidden_tags() {
    let (mut h, _, mut perimeter, mut body) = body_fixture();
    let hidden = *perimeter
        .iter()
        .find(|id| h.app.doc.model.edges.contains_key(id))
        .unwrap();
    h.app.doc.model.hidden.insert(hidden);
    let tagged = *body
        .iter()
        .find(|id| **id != hidden && h.app.doc.model.edges.contains_key(id))
        .unwrap();
    let tag = h.app.doc.model.add_tag("Hidden selection fixture").unwrap();
    h.app.doc.model.tags.get_mut(&tag).unwrap().visible = false;
    h.app.doc.model.entity_tags.insert(tagged, tag);
    for id in [hidden, tagged] {
        perimeter.remove(&id);
        body.remove(&id);
    }
    h.frame(vec![], egui::Modifiers::NONE);
    let point = h.at(DVec3::new(2., 2., 2.));
    h.click(point);
    h.click(point);
    assert_eq!(h.app.selected, perimeter);
    h.click(point);
    assert_eq!(h.app.selected, body);
}

#[test]
fn face_double_click_includes_hole_edges_but_not_a_connected_wire() {
    let mut h = Harness::new();
    for (lo, hi) in [(0., 6.), (2., 4.)] {
        properties_outline(
            &mut h.app.doc.model,
            &[
                DVec3::new(lo, lo, 0.),
                DVec3::new(hi, lo, 0.),
                DVec3::new(hi, hi, 0.),
                DVec3::new(lo, hi, 0.),
            ],
        );
    }
    let outer = h
        .app
        .doc
        .model
        .faces
        .values()
        .find(|face| !face.holes.is_empty())
        .unwrap()
        .id;
    let inner = *h
        .app
        .doc
        .model
        .faces
        .keys()
        .find(|id| **id != outer)
        .unwrap();
    h.app.doc.model.erase(&BTreeSet::from([inner]));
    let wire = h
        .app
        .doc
        .model
        .add_edge(DVec3::new(2., 2., 0.), DVec3::new(3., 3., 0.))
        .unwrap();
    let body = h.app.doc.model.selectable_entities();
    assert_eq!(body.len(), 10);
    let perimeter: BTreeSet<_> = body.iter().copied().filter(|id| *id != wire).collect();
    h.app
        .doc
        .camera
        .set_standard_view(benchup_core::StandardView::Top);
    h.app.fit(&h.ctx);
    h.frame(vec![], egui::Modifiers::NONE);
    let point = h.at(DVec3::new(1., 1., 0.));
    h.click(point);
    assert_eq!(h.app.selected, BTreeSet::from([outer]));
    h.click(point);
    assert_eq!(h.app.selected, perimeter);
    h.click(point);
    assert_eq!(h.app.selected, body);
}

#[test]
fn double_click_still_edits_text_annotations() {
    let mut h = Harness::new();
    let id = h
        .app
        .doc
        .model
        .add_text(DVec3::ZERO, DVec3::X * 2., "Beam label")
        .unwrap();
    h.frame(vec![], egui::Modifiers::NONE);
    let point = h.at(DVec3::X * 2.);
    h.click(point);
    assert_eq!(h.app.selected, BTreeSet::from([id]));
    assert!(h.app.text_editor.is_none());
    h.click(point);
    assert_eq!(h.app.text_editor.as_ref().unwrap().text, "Beam label");
}
