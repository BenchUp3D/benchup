use super::*;
use crate::Document;

fn rectangle(x: f64, y: f64, width: f64, height: f64, z: f64) -> [DVec3; 4] {
    [
        DVec3::new(x, y, z),
        DVec3::new(x + width, y, z),
        DVec3::new(x + width, y + height, z),
        DVec3::new(x, y + height, z),
    ]
}

fn draw(model: &mut Model, ring: [DVec3; 4]) -> u64 {
    let before = model.faces.keys().copied().collect::<BTreeSet<_>>();
    for i in 0..4 {
        model.add_edge(ring[i], ring[(i + 1) % 4]).unwrap();
    }
    *model.faces.keys().find(|id| !before.contains(id)).unwrap()
}

fn piece(model: &mut Model, ring: [DVec3; 4], hole: Option<[DVec3; 4]>, depth: f64) -> (u64, u64) {
    let mut stock = Model::default();
    let face = draw(&mut stock, ring);
    if let Some(hole) = hole {
        let fill = draw(&mut stock, hole);
        stock.erase(&BTreeSet::from([fill]));
    }
    if depth != 0. {
        stock.push_pull_along(face, depth * DVec3::Z).unwrap();
    }
    let roots = model.paste_entities(&stock, DVec3::ZERO);
    let board = model.make_object(&roots, false, "Stock").unwrap();
    let face = *model.objects[&board]
        .entities
        .iter()
        .find(|id| model.faces.contains_key(id))
        .unwrap();
    (board, face)
}

fn area(model: &Model, expected: f64) {
    assert!(
        (model.wood_glue_area() - expected).abs() < 1e-9,
        "{}",
        model.wood_glue_area()
    );
}

#[test]
fn wood_glue_contacts_clip_partial_overlaps_and_holes_and_union_repeated_patches() {
    let mut model = Model::default();
    let (a, face_a) = piece(
        &mut model,
        rectangle(0., 0., 2., 2., 0.),
        Some(rectangle(0.75, 0.5, 0.5, 1., 0.)),
        0.,
    );
    let (b, face_b) = piece(&mut model, rectangle(0.5, 0., 2., 2., 0.), None, 0.);
    let (c, _) = piece(&mut model, rectangle(1., 1., 2., 2., 0.), None, 0.);
    assert_eq!(model.glue_contacts(&BTreeSet::from([a, b])), 2);
    area(&model, 2.5);
    let first = model.clone();
    assert_eq!(model.glue_contacts(&BTreeSet::from([face_a, face_b])), 0);
    assert_eq!(model, first);
    // c overlaps the existing patch on a, but only c gains new coverage.
    assert_eq!(model.glue_contacts(&BTreeSet::from([a, c])), 1);
    area(&model, 2.5);
    assert_eq!(model.glue_contacts(&BTreeSet::from([a, c])), 0);
}

#[test]
fn wood_glue_ignores_air_gaps_edge_contacts_and_overlapping_solids() {
    let mut model = Model::default();
    let (a, _) = piece(&mut model, rectangle(0., 0., 1., 1., 0.), None, 0.1);
    let (b, _) = piece(&mut model, rectangle(0., 0., 1., 1., 0.100001), None, 0.1);
    let (c, _) = piece(&mut model, rectangle(1., 0., 1., 1., 0.1), None, 0.1);
    let (d, _) = piece(&mut model, rectangle(0.3, 0.3, 1., 1., 0.), None, 0.1);
    for other in [b, c, d] {
        assert_eq!(model.glue_contacts(&BTreeSet::from([a, other])), 0);
    }
    let (e, _) = piece(&mut model, rectangle(0.5, 0., 1., 1., 0.1), None, 0.1);
    let selected = BTreeSet::from([a, e]);
    model
        .rotate_selection(&selected, DVec3::ZERO, DVec3::new(1., 2., 3.), 42.)
        .unwrap();
    model
        .move_selection(&selected, DVec3::new(19., -38., 57.))
        .unwrap();
    assert_eq!(model.glue_contacts(&selected), 2);
    area(&model, 0.5);
    model.scale_selection(&selected, DVec3::ZERO, 2.).unwrap();
    area(&model, 2.);
}

#[test]
fn wood_glue_saves_and_undoes_and_follows_face_edits() {
    let mut doc = Document::default();
    let face = draw(&mut doc.model, rectangle(0., 0., 2., 1., 0.));
    let before = doc.model.clone();
    assert!(doc.model.glue_face(face));
    assert!(!doc.model.glue_face(face));
    doc.history.record("Apply wood glue", before, &doc.model);
    let json = serde_json::to_string(&doc).unwrap();
    let mut doc = Document::from_json(&json).unwrap();
    area(&doc.model, 2.);
    assert!(doc.history.undo(&mut doc.model));
    area(&doc.model, 0.);
    assert!(doc.history.redo(&mut doc.model));
    doc.model
        .add_edge(DVec3::new(1., 0., 0.), DVec3::new(1., 1., 0.))
        .unwrap();
    assert_eq!(doc.model.wood_glue.len(), 2);
    area(&doc.model, 2.);
    let half = *doc.model.wood_glue.keys().next().unwrap();
    let cap = doc.model.push_pull_along(half, DVec3::Z * 0.2).unwrap();
    assert!(!doc.model.wood_glue.contains_key(&half));
    assert!(doc.model.wood_glue.contains_key(&cap));
    area(&doc.model, 2.);
    doc.model.erase(&BTreeSet::from([cap]));
    area(&doc.model, 1.);
    doc.model.validate().unwrap();
}

#[test]
fn wood_glue_mesh_partitions_bare_and_coated_without_a_depth_overlay() {
    let mut model = Model::default();
    let (a, face) = piece(&mut model, rectangle(0., 0., 2., 1., 0.), None, 0.);
    let (b, _) = piece(&mut model, rectangle(1., 0., 2., 1., 0.), None, 0.);
    model.glue_contacts(&BTreeSet::from([a, b]));
    let (bare, coated) = model.glue_mesh(face, &model.face_triangles(&model.faces[&face]));
    let area = |triangles: &[[DVec3; 3]]| {
        triangles
            .iter()
            .map(|[a, b, c]| (*b - *a).cross(*c - *a).length() / 2.)
            .sum::<f64>()
    };
    assert!((area(&bare) - 1.).abs() < 1e-9);
    assert!((area(&coated) - 1.).abs() < 1e-9);
    assert!(bare.iter().flatten().all(|p| p.x <= 1. && p.z == 0.));
    assert!(coated.iter().flatten().all(|p| p.x >= 1. && p.z == 0.));
}

#[test]
fn wood_glue_survives_boolean_replacement_only_where_the_original_surface_remains() {
    let mut model = Model::default();
    let (a, face) = piece(&mut model, rectangle(0., 0., 2., 1., 0.), None, 0.);
    let (b, _) = piece(&mut model, rectangle(1., 0., 2., 1., 0.), None, 0.);
    assert!(model.glue_face(face));
    model
        .boolean_solids(a, b, crate::SolidOperation::Intersect)
        .unwrap();
    area(&model, 1.);
    model.validate().unwrap();
}

#[test]
fn wood_glue_area_unions_matching_coverage_but_keeps_unmatched_and_separate_surfaces() {
    let mut model = Model::default();
    let (_, a) = piece(&mut model, rectangle(0., 0., 2., 1., 0.), None, 0.);
    let (_, b) = piece(&mut model, rectangle(1., 0., 2., 1., 0.), None, 0.);
    assert!(model.glue_face(a));
    assert!(model.glue_face(b));
    area(&model, 3.);
    let (_, c) = piece(&mut model, rectangle(0., 0., 2., 1., 0.001), None, 0.);
    assert!(model.glue_face(c));
    area(&model, 5.);
    let (_, d) = piece(&mut model, rectangle(0., 0., 2., 1., 0.), None, 0.);
    assert!(model.glue_face(d));
    area(&model, 5.);
}
