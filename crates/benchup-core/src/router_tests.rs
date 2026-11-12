use super::*;
use crate::Document;
use std::collections::BTreeSet;

#[test]
fn cove_split_box_cleans_generated_vertices_and_retains_editable_stock_endpoints() {
    // Geometry from clean-vertices.bup: a drawn box with a split top face and
    // a Cove route. Its apparent aligned vertices were CSG preview artifacts.
    let mut stock: Model =
        serde_json::from_str(include_str!("../tests/fixtures/cove-split-box.json")).unwrap();
    stock.validate().unwrap();
    let before = stock.clone();
    assert_eq!(
        stock.clean_collinear_vertices(&stock.root_entities()),
        Ok(0)
    );
    assert_eq!(stock, before);

    let evaluated = stock.evaluate_joinery();
    assert!(evaluated.errors.is_empty(), "{:?}", evaluated.errors);
    evaluated.model.validate().unwrap();
    let appearance = &evaluated.model.routing_appearance;
    assert_eq!(appearance.endpoint_controls().count(), stock.edges.len());
    for edge in stock.edges.values() {
        let (a, b) = stock.points(edge);
        let control = appearance.endpoint_control(edge.id).unwrap();
        assert_eq!(control.points, [a, b]);
        assert_eq!(control.parent, None);
    }
    let mut cleaned = evaluated.model.clone();
    assert_eq!(
        cleaned.clean_collinear_vertices(&cleaned.root_entities()),
        Ok(0),
        "straight CSG subdivisions must be removed during evaluation"
    );
    assert_eq!(
        stock, before,
        "preview cleanup must not change saved geometry or routes"
    );
    let source_edge = *stock.routes.keys().next().unwrap();
    let routed_edge = *appearance.edges.first().unwrap();
    assert!(
        evaluated.model.solid_volume(routed_edge).unwrap()
            < stock.solid_volume(source_edge).unwrap()
    );

    let copy = evaluated
        .model
        .copy_entities(&evaluated.model.root_entities());
    for (source, control) in &appearance.stock_edge_controls {
        if appearance.by_source.contains_key(source) {
            assert_eq!(
                copy.routing_appearance.endpoint_control(*source),
                Some(control)
            );
        }
    }
}


fn board() -> (Model, u64, [u64; 2]) {
    let mut model = Model::default();
    let corners = [
        DVec3::ZERO,
        DVec3::new(0.3, 0., 0.),
        DVec3::new(0.3, 0.1, 0.),
        DVec3::new(0., 0.1, 0.),
    ];
    for i in 0..4 {
        model.add_edge(corners[i], corners[(i + 1) % 4]).unwrap();
    }
    let face = *model.faces.keys().next().unwrap();
    let normal = crate::faces::plane(&model, &model.faces[&face].vertices)
        .unwrap()
        .normal;
    model.push_pull(face, 0.04 / normal.z).unwrap();
    let body = model
        .make_object(&model.root_entities(), false, "Board")
        .unwrap();
    let pick = |p: DVec3, q: DVec3| {
        model
            .edges
            .values()
            .find(|edge| {
                let (a, b) = model.points(edge);
                (a.distance(p) + b.distance(q)).min(a.distance(q) + b.distance(p)) < 1e-8
            })
            .unwrap()
            .id
    };
    let edges = [pick(corners[0], corners[1]), pick(corners[1], corners[2])];
    (model, body, edges)
}

#[test]
fn profiles_produce_valid_solids_and_preserve_stock() {
    for profile in Profile::ALL
        .into_iter()
        .filter(|profile| *profile != Profile::None)
    {
        let (mut model, board, [edge, _]) = board();
        let volume = model.solid_volume(board).unwrap();
        let stock = model.clone();
        model.routes.insert(
            edge,
            Settings {
                profile,
                ..Default::default()
            },
        );
        let evaluated = model.evaluate_joinery();
        assert!(
            evaluated.errors.is_empty(),
            "{profile:?}: {:?}",
            evaluated.errors
        );
        evaluated.model.validate().unwrap();
        let machined = evaluated.model.solid_volume(board).unwrap();
        assert!(
            machined < volume && machined > 0.,
            "{profile:?}: {machined}"
        );
        assert_eq!(stock.vertices, model.vertices);
        assert_eq!(stock.faces, model.faces);
        let removed = volume - machined;
        let expected = match profile {
            Profile::Chamfer => Some(0.3 * 0.005_f64.powi(2) / 2.),
            Profile::Bevel => Some(0.3 * 0.005_f64.powi(2) * 30_f64.to_radians().tan() / 2.),
            Profile::Rabbet | Profile::Groove => Some(0.3 * 0.005 * 0.003),
            _ => None,
        };
        if let Some(expected) = expected {
            assert!(
                (removed - expected).abs() < 1e-10,
                "{profile:?}: {removed} != {expected}"
            );
        }
    }
}

#[test]
fn adjacent_edges_evaluate_together_and_routes_round_trip_with_undo() {
    let (model, board, edges) = board();
    let mut doc = Document {
        model,
        ..Default::default()
    };
    let before = doc.model.clone();
    let original = doc.model.solid_volume(board).unwrap();
    doc.model.routes.insert(edges[0], Settings::default());
    let one = doc
        .model
        .evaluate_joinery()
        .model
        .solid_volume(board)
        .unwrap();
    doc.model.routes.insert(
        edges[1],
        Settings {
            profile: Profile::Cove,
            ..Default::default()
        },
    );
    let mut cache = crate::joinery::EvaluationCache::default();
    let evaluated = cache.evaluate(&doc.model);
    assert!(evaluated.errors.is_empty(), "{:?}", evaluated.errors);
    evaluated.model.validate().unwrap();
    let two = evaluated.model.solid_volume(board).unwrap();
    assert!(two < one && one < original);
    doc.history.record("Route edges", before, &doc.model);
    let mut loaded = Document::from_json(&serde_json::to_string(&doc).unwrap()).unwrap();
    assert_eq!(loaded.model.routes, doc.model.routes);
    assert!(loaded.history.undo(&mut loaded.model));
    assert!(loaded.model.routes.is_empty());
    assert!(loaded.history.redo(&mut loaded.model));
    assert_eq!(loaded.model.routes, doc.model.routes);

    let copy = loaded.model.copy_entities(&BTreeSet::from([board]));
    let mut pasted = Model::default();
    pasted.paste_entities(&copy, DVec3::X);
    assert_eq!(pasted.routes.len(), 2);
    assert!(pasted.evaluate_joinery().errors.is_empty());
    loaded.model.erase(&BTreeSet::from([board]));
    assert!(loaded.model.routes.is_empty());
    loaded.model.validate().unwrap();
}

#[test]
fn edits_recompute_and_invalid_profiles_leave_stock_intact() {
    let (mut model, board, [edge, _]) = board();
    let settings = Settings {
        profile: Profile::Roundover,
        width: 0.05,
        ..Default::default()
    };
    model.routes.insert(edge, settings);
    let invalid = model.evaluate_joinery();
    assert_eq!(invalid.errors.len(), 1);
    assert_eq!(invalid.errors[0].0, edge);
    assert_eq!(invalid.model.vertices, model.vertices);

    for vertex in model.vertices.values_mut() {
        vertex.position.z *= 2.;
    }
    let valid = model.evaluate_joinery();
    assert!(valid.errors.is_empty(), "{:?}", valid.errors);
    assert!(valid.model.solid_volume(board).unwrap() < model.solid_volume(board).unwrap());
    assert_eq!(model.routes[&edge], settings);
}

#[test]
fn component_stock_updates_preserve_each_instances_routing() {
    let (mut model, original, [edge, _]) = board();
    let definition = uuid::Uuid::new_v4();
    let piece = model.objects.get_mut(&original).unwrap();
    piece.kind = ObjectKind::Component { definition };
    piece.definition_entities = piece.entities.iter().map(|&id| (id, id)).collect();
    let settings = Settings {
        profile: Profile::Bevel,
        reverse: true,
        ..Default::default()
    };
    model.routes.insert(edge, settings);
    let copied = model.copy_entities(&BTreeSet::from([original]));
    let pasted = model.paste_entities(&copied, DVec3::X);
    let peer = *pasted.iter().next().unwrap();
    let peer_edge = model.objects[&peer].definition_entities[&edge];
    assert_eq!(model.routes[&peer_edge], settings);
    model.routes.get_mut(&peer_edge).unwrap().profile = Profile::Rabbet;

    assert!(model.enter_object(original));
    for vertex in model.selection_vertices(&BTreeSet::from([original])) {
        model.vertices.get_mut(&vertex).unwrap().position.z *= 2.;
    }
    model.synchronize_edit_context().unwrap();
    let peer_edge = model.objects[&peer].definition_entities[&edge];
    assert_eq!(model.routes[&edge], settings);
    assert_eq!(model.routes[&peer_edge].profile, Profile::Rabbet);
    model.edit_context = None;
    let result = model.evaluate_joinery();
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    result.model.validate().unwrap();

    let copied = model.copy_entities(&BTreeSet::from([peer]));
    let third = *model
        .paste_entities(&copied, DVec3::X)
        .iter()
        .next()
        .unwrap();
    let third_edge = model.objects[&third].definition_entities[&edge];
    assert_eq!(model.routes[&third_edge].profile, Profile::Rabbet);
}

#[test]
fn bevel_and_chamfer_follow_angled_stock_and_round_bits_require_square_edges() {
    let (mut model, board, [edge, _]) = board();
    for vertex in model.vertices.values_mut() {
        vertex.position.y += vertex.position.z * 0.4;
        vertex.position = glam::DQuat::from_rotation_x(0.3) * vertex.position + DVec3::splat(1.);
    }
    let volume = model.solid_volume(board).unwrap();
    for profile in [Profile::Chamfer, Profile::Bevel, Profile::Roundover] {
        model.routes.insert(
            edge,
            Settings {
                profile,
                ..Default::default()
            },
        );
        let result = model.evaluate_joinery();
        if profile == Profile::Roundover {
            assert_eq!(result.errors.len(), 1);
            assert_eq!(result.errors[0].0, edge);
        } else {
            assert!(result.errors.is_empty(), "{:?}", result.errors);
            assert!(result.model.solid_volume(board).unwrap() < volume);
        }
    }
}

#[test]
fn all_profiles_route_ungrouped_solids_without_changing_unrelated_geometry() {
    for profile in Profile::ALL
        .into_iter()
        .filter(|profile| *profile != Profile::None)
    {
        let (mut model, group, [edge, _]) = board();
        model.dissolve_objects(&BTreeSet::from([group]), true);
        let volume = model.solid_volume(edge).unwrap();
        let tag = model.add_tag("Loose stock").unwrap();
        model.assign_tag(&model.root_entities(), Some(tag)).unwrap();
        model.tags.get_mut(&tag).unwrap().visible = false;
        let line = model.add_edge(DVec3::splat(3.), DVec3::splat(4.)).unwrap();
        let corners = [
            DVec3::new(5., 0., 0.),
            DVec3::new(6., 0., 0.),
            DVec3::new(6., 1., 0.),
            DVec3::new(5., 1., 0.),
        ];
        for i in 0..4 {
            model.add_edge(corners[i], corners[(i + 1) % 4]).unwrap();
        }
        let open_face = model
            .faces
            .values()
            .find(|face| model.vertices[&face.vertices[0]].position.x >= 5.)
            .unwrap()
            .clone();
        model.routes.insert(
            edge,
            Settings {
                profile,
                ..Default::default()
            },
        );
        let stock = model.clone();
        let result = crate::joinery::EvaluationCache::default().evaluate(&model);
        assert!(result.errors.is_empty(), "{profile:?}: {:?}", result.errors);
        result.model.validate().unwrap();
        assert!(
            result.model.objects.is_empty(),
            "routing must leave loose geometry ungrouped"
        );
        assert_eq!(result.model.edges[&line], model.edges[&line]);
        assert_eq!(result.model.faces[&open_face.id], open_face);
        let machined_edge = *result
            .model
            .edges
            .keys()
            .find(|id| !stock.edges.contains_key(id))
            .unwrap();
        assert!(result.model.is_hidden(machined_edge));
        assert_eq!(result.model.entity_tags[&machined_edge], tag);
        let machined = result.model.solid_volume(machined_edge).unwrap();
        assert!(machined > 0. && machined < volume, "{profile:?}");
        assert_eq!(model, stock);
    }
}

#[test]
fn loose_routes_round_trip_copy_paste_and_undo_and_isolate_invalid_bodies() {
    let (mut model, group, [first, second]) = board();
    model.dissolve_objects(&BTreeSet::from([group]), true);
    let mut doc = Document {
        model,
        ..Default::default()
    };
    let before = doc.model.clone();
    doc.model.routes.insert(first, Settings::default());
    doc.model.routes.insert(
        second,
        Settings {
            profile: Profile::Cove,
            ..Default::default()
        },
    );
    doc.history.record("Route loose edges", before, &doc.model);
    let mut loaded = Document::from_json(&serde_json::to_string(&doc).unwrap()).unwrap();
    assert_eq!(loaded.model.routes, doc.model.routes);
    assert!(loaded.history.undo(&mut loaded.model));
    assert!(loaded.model.routes.is_empty());
    assert!(loaded.history.redo(&mut loaded.model));
    let copy = loaded.model.copy_entities(&loaded.model.root_entities());
    assert_eq!(copy.routes, loaded.model.routes);
    let pasted = loaded.model.paste_entities(&copy, DVec3::X);
    assert_eq!(loaded.model.routes.len(), 4);
    assert!(loaded.model.evaluate_joinery().errors.is_empty());
    // An invalid cut on the original body cannot suspend the independent copy.
    loaded.model.routes.get_mut(&first).unwrap().width = 10.;
    let result = loaded.model.evaluate_joinery();
    assert!(result.errors.iter().any(|(id, _)| *id == first));
    assert!(result.errors.iter().all(|(id, _)| !pasted.contains(id)));
    assert!(result.model.objects.is_empty());
    result.model.validate().unwrap();
    loaded.model.erase(&pasted);
    assert_eq!(loaded.model.routes.len(), 2);
}

#[test]
fn applied_routes_have_an_orange_smooth_finish_without_changing_saved_stock() {
    for grouped in [false, true] {
        for profile in [
            Profile::Roundover,
            Profile::Cove,
            Profile::Ogee,
            Profile::Rabbet,
        ] {
            let (mut model, group, [edge, _]) = board();
            if !grouped {
                model.dissolve_objects(&BTreeSet::from([group]), true);
            }
            model.routes.insert(
                edge,
                Settings {
                    profile,
                    ..Default::default()
                },
            );
            let original_colors = model.face_colors.clone();
            let result = model.evaluate_joinery();
            assert!(result.errors.is_empty(), "{profile:?}: {:?}", result.errors);
            let appearance = &result.model.routing_appearance;
            assert!(!appearance.faces.is_empty());
            assert!(!appearance.edges.is_empty());
            for face in &appearance.faces {
                assert_eq!(result.model.face_colors[face], CUT_COLOR);
            }
            if profile == Profile::Rabbet {
                assert!(
                    appearance.smooth_edges.is_empty(),
                    "keep rebate shoulders sharp"
                );
            } else {
                assert!(!appearance.smooth_edges.is_empty(), "{profile:?}");
                assert!(
                    appearance
                        .normals
                        .values()
                        .all(|normal| (normal.length() - 1.).abs() < 1e-9)
                );
            }
            assert!(model.routing_appearance.faces.is_empty());
            assert_eq!(model.face_colors, original_colors);
            let json = serde_json::to_value(&model).unwrap();
            assert!(json.get("routing_appearance").is_none());
            let loaded: Model = serde_json::from_value(json).unwrap();
            assert_eq!(
                loaded.evaluate_joinery().model.routing_appearance,
                result.model.routing_appearance
            );
            result.model.validate().unwrap();
        }
    }
}

#[test]
fn routed_primitives_resolve_to_stock_and_selection_resolves_back_to_visible_geometry() {
    for grouped in [false, true] {
        let (mut model, group, [edge, _]) = board();
        if !grouped {
            model.dissolve_objects(&BTreeSet::from([group]), true);
        }
        model.routes.insert(
            edge,
            Settings {
                profile: Profile::Roundover,
                ..Default::default()
            },
        );
        let result = model.evaluate_joinery();
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        let appearance = &result.model.routing_appearance;
        for &derived in result.model.edges.keys().chain(result.model.faces.keys()) {
            assert!(model.contains_entity(&appearance.sources[&derived]));
        }
        for (&derived, &source) in &appearance.sources {
            let rendered = appearance.render_entities(&BTreeSet::from([source]));
            assert!(rendered.contains(&derived));
            assert!(appearance.by_source[&source].contains(&derived));
        }
        for id in &appearance.faces {
            assert_eq!(
                appearance.sources[id], edge,
                "cut surfaces edit the original routed edge"
            );
        }
        for &source in model.faces.keys() {
            assert!(
                appearance.by_source.contains_key(&source),
                "retained stock faces remain selectable"
            );
        }
    }
}

#[test]
fn none_profile_leaves_stock_unchanged() {
    let (mut model, _, [edge, _]) = board();
    let stock = model.clone();
    model.routes.insert(
        edge,
        Settings {
            profile: Profile::None,
            ..Default::default()
        },
    );
    let result = model.evaluate_joinery();
    assert!(result.errors.is_empty());
    assert_eq!(result.model.vertices, stock.vertices);
    assert_eq!(result.model.edges, stock.edges);
    assert_eq!(result.model.faces, stock.faces);
    assert!(result.model.routing_appearance.faces.is_empty());
}

fn surface_path(grouped: bool, split_face: bool) -> (Model, Option<u64>, u64) {
    let (mut model, group, _) = board();
    if grouped {
        assert!(model.enter_object(group));
    } else {
        model.dissolve_objects(&BTreeSet::from([group]), true);
    }
    let (a, b) = if split_face {
        (DVec3::new(0., 0.05, 0.04), DVec3::new(0.3, 0.05, 0.04))
    } else {
        (DVec3::new(0.05, 0.05, 0.04), DVec3::new(0.25, 0.05, 0.04))
    };
    model.draw_polyline(&[a, b], false).unwrap();
    let edge = model
        .edges
        .values()
        .find(|edge| {
            let (p, q) = model.points(edge);
            p.distance(a) + q.distance(b) < 1e-8 || p.distance(b) + q.distance(a) < 1e-8
        })
        .unwrap()
        .id;
    assert!(model.route_on_surface(edge));
    model.edit_context = None;
    (model, grouped.then_some(group), edge)
}

#[test]
fn drawn_open_paths_and_coplanar_face_seams_accept_every_router_profile() {
    for grouped in [false, true] {
        for split_face in [false, true] {
            for profile in Profile::ALL.into_iter().filter(|p| *p != Profile::None) {
                let (mut model, group, edge) = surface_path(grouped, split_face);
                let stock = model.clone();
                let length = model.edge_length(&model.edges[&edge]);
                model.routes.insert(
                    edge,
                    Settings {
                        profile,
                        ..Default::default()
                    },
                );
                let result = model.evaluate_joinery();
                assert!(
                    result.errors.is_empty(),
                    "{grouped}/{split_face}/{profile:?}: {:?}",
                    result.errors
                );
                result.model.validate().unwrap();
                let body = group.unwrap_or_else(|| {
                    *result.model.routing_appearance.edges.iter().next().unwrap()
                });
                let removed = 0.3 * 0.1 * 0.04 - result.model.solid_volume(body).unwrap();
                assert!(removed > 0., "{profile:?}");
                if matches!(profile, Profile::Groove | Profile::Rabbet) {
                    assert!((removed - length * 0.005 * 0.003).abs() < 1e-9, "{removed}");
                }
                assert_eq!(model.vertices, stock.vertices);
                assert_eq!(model.faces, stock.faces);
                assert!(
                    result.model.routing_appearance.faces.iter().all(|id| result
                        .model
                        .routing_appearance
                        .sources[id]
                        == edge)
                );
            }
        }
    }
}

#[test]
fn detached_surface_paths_share_stock_keep_other_paths_and_round_trip_with_undo() {
    let (mut model, _, first) = surface_path(false, false);
    let second = model
        .add_edge(DVec3::new(0.05, 0.075, 0.04), DVec3::new(0.25, 0.075, 0.04))
        .unwrap();
    let stock = model.clone();
    let mut doc = Document {
        model,
        ..Default::default()
    };
    doc.model.routes.insert(
        first,
        Settings {
            profile: Profile::Groove,
            ..Default::default()
        },
    );
    let one = doc.model.evaluate_joinery();
    assert!(one.errors.is_empty(), "{:?}", one.errors);
    assert!(one.model.edges.contains_key(&second));
    doc.model.routes.insert(
        second,
        Settings {
            profile: Profile::Cove,
            ..Default::default()
        },
    );
    let two = doc.model.evaluate_joinery();
    assert!(two.errors.is_empty(), "{:?}", two.errors);
    two.model.validate().unwrap();
    assert!(two.model.objects.is_empty());
    doc.history.record("Route surface paths", stock, &doc.model);
    let mut loaded = Document::from_json(&serde_json::to_string(&doc).unwrap()).unwrap();
    assert!(loaded.model.evaluate_joinery().errors.is_empty());
    assert!(loaded.history.undo(&mut loaded.model));
    assert!(loaded.model.routes.is_empty());
    assert!(loaded.history.redo(&mut loaded.model));
    assert_eq!(loaded.model.routes.len(), 2);
}

#[test]
fn floating_or_overwide_surface_paths_do_not_cut_unrelated_stock() {
    for (height, width) in [(0.05, 0.005), (0.04, 0.2)] {
        let (mut model, group, _) = board();
        model.dissolve_objects(&BTreeSet::from([group]), true);
        let edge = model
            .add_edge(
                DVec3::new(0.05, 0.05, height),
                DVec3::new(0.25, 0.05, height),
            )
            .unwrap();
        model.routes.insert(
            edge,
            Settings {
                profile: Profile::Groove,
                width,
                ..Default::default()
            },
        );
        let result = model.evaluate_joinery();
        assert_eq!(result.errors.len(), 1);
        assert_eq!(result.errors[0].0, edge);
        assert_eq!(result.model.vertices, model.vertices);
        assert_eq!(result.model.faces, model.faces);
    }
}

#[test]
fn surface_grooves_follow_rotated_stock_and_reversed_path_direction() {
    for reverse in [false, true] {
        let (mut model, _, edge) = surface_path(false, false);
        let rotation = glam::DQuat::from_rotation_y(0.63) * glam::DQuat::from_rotation_x(0.31);
        for vertex in model.vertices.values_mut() {
            vertex.position = rotation * vertex.position + DVec3::new(3., -2., 1.);
        }
        if reverse {
            let edge = model.edges.get_mut(&edge).unwrap();
            std::mem::swap(&mut edge.a, &mut edge.b);
        }
        model.routes.insert(
            edge,
            Settings {
                profile: Profile::Groove,
                ..Default::default()
            },
        );
        let result = model.evaluate_joinery();
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        let body = *result.model.routing_appearance.edges.iter().next().unwrap();
        let volume = result.model.solid_volume(body).unwrap();
        assert!((volume - (0.3 * 0.1 * 0.04 - 0.2 * 0.005 * 0.003)).abs() < 1e-9);
        result.model.validate().unwrap();
    }
}

#[test]
fn drawing_an_unrouted_surface_line_does_not_suspend_existing_routing() {
    fn shell_volume(model: &Model) -> f64 {
        let faces = model.faces.keys().copied().collect();
        let shell = model.copy_entities(&faces);
        shell
            .solid_volume(*shell.edges.keys().next().unwrap())
            .unwrap()
    }
    for grouped in [false, true] {
        let (mut model, group, [edge, _]) = board();
        if !grouped {
            model.dissolve_objects(&BTreeSet::from([group]), true);
        }
        model.routes.insert(edge, Settings::default());
        let before = model.evaluate_joinery();
        assert!(before.errors.is_empty(), "{:?}", before.errors);
        let volume = shell_volume(&before.model);
        model.edit_context = grouped.then_some(group);
        let face_count = model.faces.len();
        let path = model
            .add_edge(DVec3::new(0.05, 0.06, 0.04), DVec3::new(0.25, 0.06, 0.04))
            .unwrap();
        assert_eq!(model.faces.len(), face_count);
        model.edit_context = None;
        let stock = model.clone();
        if grouped {
            assert!(
                model
                    .validate_solid_operand(group, crate::solids::SolidOperation::Union)
                    .is_err(),
                "the ordinary solid tools must retain their stricter stray-edge validation"
            );
        }
        let after = model.evaluate_joinery();
        assert!(after.errors.is_empty(), "{grouped}: {:?}", after.errors);
        after.model.validate().unwrap();
        assert!((shell_volume(&after.model) - volume).abs() < 1e-12);
        assert!(
            after.model.edges.contains_key(&path),
            "the new path must remain selectable"
        );
        assert_eq!(after.model.routing_appearance.sources[&path], path);
        assert_eq!(
            model, stock,
            "evaluation must not erase or edit the source drawing"
        );
        model.routes.insert(
            path,
            Settings {
                profile: Profile::Groove,
                ..Default::default()
            },
        );
        let routed_path = model.evaluate_joinery();
        assert!(routed_path.errors.is_empty(), "{:?}", routed_path.errors);
        routed_path.model.validate().unwrap();
        assert!(shell_volume(&routed_path.model) < volume);
    }
}

#[test]
fn allowing_router_paths_does_not_allow_an_open_stock_shell() {
    let (mut model, group, [edge, _]) = board();
    model.dissolve_objects(&BTreeSet::from([group]), true);
    let missing = model
        .faces
        .values()
        .find(|face| {
            face.vertices
                .iter()
                .all(|id| (model.vertices[id].position.z - 0.04).abs() < 1e-8)
        })
        .unwrap()
        .id;
    model.erase(&BTreeSet::from([missing]));
    model.routes.insert(edge, Settings::default());
    let result = model.evaluate_joinery();
    assert!(!result.errors.is_empty());
    assert_eq!(result.model.vertices, model.vertices);
    assert_eq!(result.model.faces, model.faces);
}

#[test]
fn selecting_a_routed_path_uses_original_controls_and_keeps_the_machined_solid() {
    for grouped in [false, true] {
        for surface in [false, true] {
            let (mut model, group, edge) = if surface {
                surface_path(grouped, false)
            } else {
                let (mut model, group, [edge, _]) = board();
                if !grouped {
                    model.dissolve_objects(&BTreeSet::from([group]), true);
                }
                (model, grouped.then_some(group), edge)
            };
            let (a, b) = model.points(&model.edges[&edge]);
            model.routes.insert(
                edge,
                Settings {
                    profile: Profile::Roundover,
                    ..Default::default()
                },
            );
            let result = model.evaluate_joinery();
            assert!(result.errors.is_empty(), "{:?}", result.errors);
            let appearance = &result.model.routing_appearance;
            assert_eq!(
                appearance.selection_entities(&BTreeSet::from([edge])),
                BTreeSet::from([edge])
            );
            let control = &appearance.edge_controls[&edge];
            assert_eq!(control.points, [a, b]);
            assert_eq!(control.parent, group);
            assert!(!appearance.faces.is_empty());
            assert!(
                appearance
                    .faces
                    .iter()
                    .all(|id| result.model.faces.contains_key(id))
            );
            // The original line is interaction metadata, not a stray solid edge.
            assert!(!result.model.edges.contains_key(&edge));
            result.model.validate().unwrap();
            let copy = result.model.copy_entities(&result.model.root_entities());
            assert_eq!(
                copy.routing_appearance.edge_controls,
                appearance.edge_controls
            );
        }
    }
}
