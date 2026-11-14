//! Tests for the tools in `ops_more.rs` (BenchUp3D/benchup#42).
use super::*;
use benchup_core::router::Profile;
use serde_json::{Value, json};

fn run(model: &mut Model, env: &mut ToolEnv, name: &str, args: Value) -> ToolOutcome {
    execute(model, &Call::new("t", name, args), env)
}
fn ok(model: &mut Model, env: &mut ToolEnv, name: &str, args: Value) -> Value {
    let outcome = run(model, env, name, args);
    assert!(outcome.ok, "{name} failed: {}", outcome.result);
    outcome.result
}
fn err(model: &mut Model, env: &mut ToolEnv, name: &str, args: Value) -> Value {
    let before = model.clone();
    let outcome = run(model, env, name, args);
    assert!(!outcome.ok, "{name} unexpectedly succeeded: {}", outcome.result);
    assert_eq!(*model, before, "{name} must leave the model untouched on error");
    outcome.result
}
fn board(model: &mut Model, env: &mut ToolEnv, name: &str, origin: [f64; 3], size: [f64; 3]) -> u64 {
    ok(model, env, "create_board", json!({"name": name, "origin_mm": origin, "size_mm": size}))["created"][0].as_u64().unwrap()
}
fn bounds_mm(model: &Model, id: u64) -> ([f64; 3], [f64; 3]) {
    let (lo, hi) = model.selection_bounds(&BTreeSet::from([id])).unwrap();
    let r = |v: DVec3| [scene::mm(v.x), scene::mm(v.y), scene::mm(v.z)];
    (r(lo), r(hi))
}
/// Two sides 18 mm thick and a shelf housed 9 mm into each, with joints.
fn carcass(model: &mut Model, env: &mut ToolEnv) -> (u64, u64, u64, u64, u64) {
    let left = board(model, env, "Left", [0., 0., 0.], [18., 300., 600.]);
    let right = board(model, env, "Right", [482., 0., 0.], [18., 300., 600.]);
    let shelf = board(model, env, "Shelf", [9., 0., 300.], [482., 290., 18.]);
    let j1 = ok(model, env, "add_joint", json!({"a": shelf, "b": left, "kind": "tongue_groove"}))["joint_id"].as_u64().unwrap();
    let j2 = ok(model, env, "add_joint", json!({"a": shelf, "b": right, "kind": "tongue_groove"}))["joint_id"].as_u64().unwrap();
    (left, right, shelf, j1, j2)
}

#[test]
fn every_new_tool_is_listed_with_a_schema() {
    let names: Vec<String> = definitions().into_iter().map(|d| d.name).collect();
    for (name, _) in TOOLS {
        assert!(names.iter().any(|n| n == name), "{name} has no definition");
    }
    assert_eq!(names.len(), TOOLS.len(), "every definition is a known tool");
}

#[test]
fn get_scene_filters_by_ids_depth_fields_and_sections() {
    let (mut model, mut env) = (Model::default(), ToolEnv::default());
    let a = board(&mut model, &mut env, "A", [0., 0., 0.], [10., 10., 10.]);
    let b = board(&mut model, &mut env, "B", [20., 0., 0.], [10., 10., 10.]);
    let group = ok(&mut model, &mut env, "group", json!({"ids": [a, b], "name": "Pair", "component": false}))["created"][0].as_u64().unwrap();
    let c = board(&mut model, &mut env, "C", [40., 0., 0.], [10., 10., 10.]);

    let top = ok(&mut model, &mut env, "get_scene", json!({"depth": 0, "fields": ["name"], "sections": ["objects"]}));
    let listed: Vec<u64> = top["objects"].as_array().unwrap().iter().map(|o| o["id"].as_u64().unwrap()).collect();
    assert_eq!(listed.iter().copied().collect::<BTreeSet<_>>(), BTreeSet::from([group, c]));
    assert_eq!(top["objects"][0].as_object().unwrap().len(), 2, "only id and name: {}", top["objects"][0]);
    assert!(top.get("joints").is_none() && top.get("dimensions").is_none() && top.get("stock_catalog").is_none());

    let inside = ok(&mut model, &mut env, "get_scene", json!({"ids": [group], "depth": 1, "sections": ["objects"]}));
    assert_eq!(inside["objects"].as_array().unwrap().len(), 3, "the group and its two boards");
    assert_eq!(inside["listed_count"], 3);
    let only = ok(&mut model, &mut env, "get_scene", json!({"ids": [group], "depth": 0}));
    assert_eq!(only["objects"].as_array().unwrap().len(), 1);

    let bad = err(&mut model, &mut env, "get_scene", json!({"fields": ["colour"]}));
    assert!(bad["hint"].as_str().unwrap().contains("size_mm"));
    // No filter: as before.
    let all = ok(&mut model, &mut env, "get_scene", json!({}));
    assert_eq!(all["objects"].as_array().unwrap().len(), 4);
    assert!(all.get("joints").is_some());
}

#[test]
fn check_model_only_new_reports_problems_added_by_the_batch() {
    let (mut model, mut env) = (Model::default(), ToolEnv::default());
    // An existing problem: two boards overlapping without a joint.
    board(&mut model, &mut env, "A", [0., 0., 0.], [100., 100., 18.]);
    board(&mut model, &mut env, "B", [50., 0., 0.], [100., 100., 18.]);
    env.before = None; // a new batch starts
    let none_yet = ok(&mut model, &mut env, "check_model", json!({"only_new": true}));
    assert_eq!(none_yet["issues"], json!([]));
    assert_eq!(none_yet["passed"], true);
    board(&mut model, &mut env, "C", [500., 0., 0.], [100., 100., 18.]);
    let clean = ok(&mut model, &mut env, "check_model", json!({"only_new": true}));
    let kinds: Vec<&str> = clean["issues"].as_array().unwrap().iter().map(|i| i["kind"].as_str().unwrap()).collect();
    assert!(!kinds.contains(&"overlap_without_joint"), "{clean}");
    assert!(clean["existing_issues"].as_u64().unwrap() >= 1);
    assert_eq!(clean["passed"], true);
    board(&mut model, &mut env, "D", [550., 0., 0.], [100., 100., 18.]);
    let new = ok(&mut model, &mut env, "check_model", json!({"only_new": true}));
    assert_eq!(new["passed"], false);
    assert_eq!(new["warnings"], 1, "{new}");
    // Without only_new both overlaps are reported.
    assert_eq!(ok(&mut model, &mut env, "check_model", json!({}))["warnings"], 2);
}

#[test]
fn check_model_only_new_does_not_hide_copied_overlaps_after_regrouping() {
    let (mut model, mut env) = (Model::default(), ToolEnv::default());
    let a = board(&mut model, &mut env, "A", [0., 0., 0.], [100., 100., 18.]);
    let b = board(&mut model, &mut env, "B", [50., 0., 0.], [100., 100., 18.]);
    env.before = None;
    for id in [a, b] {
        ok(&mut model, &mut env, "create_copy", json!({"id": id, "offset_mm": [1000, 0, 0]}));
    }
    for regroup in [false, true] {
        if regroup {
            ok(&mut model, &mut env, "group", json!({"ids": [a, b], "name": "Original pair", "component": false}));
        }
        let check = ok(&mut model, &mut env, "check_model", json!({"only_new": true}));
        assert_eq!(check["passed"], false, "{check}");
        assert_eq!(check["warnings"], 1, "the copied overlap is new even though its message matches: {check}");
        assert_eq!(check["existing_by_kind"]["overlap_without_joint"], 1, "the original overlap survives regrouping: {check}");
    }
}

#[test]
fn distribute_makes_equal_gaps_or_centres() {
    let (mut model, mut env) = (Model::default(), ToolEnv::default());
    let shelves: Vec<u64> = (0..3).map(|i| board(&mut model, &mut env, "Shelf", [0., 0., 10. * i as f64], [500., 300., 18.])).collect();
    let result = ok(&mut model, &mut env, "distribute", json!({"ids": shelves, "axis": "z", "from_mm": 18, "to_mm": 882}));
    // (864 - 3 * 18) / 4 = 202.5
    assert_eq!(result["gap_mm"], 202.5);
    assert_eq!(bounds_mm(&model, shelves[0]).0[2], 220.5);
    assert_eq!(bounds_mm(&model, shelves[2]).1[2], 882. - 202.5);
    ok(&mut model, &mut env, "distribute", json!({"ids": shelves, "axis": "z", "from_mm": 0, "to_mm": 900, "ends": "flush"}));
    assert_eq!(bounds_mm(&model, shelves[0]).0[2], 0.);
    assert_eq!(bounds_mm(&model, shelves[2]).1[2], 900.);
    ok(&mut model, &mut env, "distribute", json!({"ids": shelves, "axis": "z", "from_mm": 0, "to_mm": 800, "mode": "centers"}));
    assert_eq!(bounds_mm(&model, shelves[1]).0[2], 400. - 9.);
    let too_tight = err(&mut model, &mut env, "distribute", json!({"ids": shelves, "axis": "z", "from_mm": 0, "to_mm": 40}));
    assert!(too_tight["error"].as_str().unwrap().contains("do not fit"));
}

#[test]
fn add_profile_routes_edges_between_named_faces() {
    let (mut model, mut env) = (Model::default(), ToolEnv::default());
    let top = board(&mut model, &mut env, "Top", [0., 0., 0.], [600., 300., 20.]);
    let result = ok(&mut model, &mut env, "add_profile", json!({"id": top, "between": [["+z", "-y"]], "profile": "chamfer", "width_mm": 4}));
    assert_eq!(result["edges"].as_array().unwrap().len(), 1);
    assert_eq!(result["length_mm"], 600.);
    let edge = result["edges"][0].as_u64().unwrap();
    assert_eq!(model.routes[&edge].profile, Profile::Chamfer);
    assert!((model.routes[&edge].width - 0.004).abs() < 1e-12);
    let detail = ok(&mut model, &mut env, "get_object", json!({"id": top}));
    assert_eq!(detail["profiles"][0]["profile"], "chamfer");
    let between: BTreeSet<String> = detail["profiles"][0]["between"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_owned()).collect();
    assert_eq!(between, BTreeSet::from(["+z".to_owned(), "-y".to_owned()]));
    assert_eq!(ok(&mut model, &mut env, "get_scene", json!({}))["profile_count"], 1);
    // The whole top perimeter, rounded over.
    let all = ok(&mut model, &mut env, "add_profile", json!({"id": top, "between": [["+z", "*"]], "profile": "roundover", "width_mm": 3}));
    assert_eq!(all["edges"].as_array().unwrap().len(), 4);
    let evaluated = model.evaluate_joinery();
    assert!(evaluated.errors.is_empty(), "{:?}", evaluated.errors);
    assert!(evaluated.model.solid_volume(top).unwrap() < model.solid_volume(top).unwrap());
    let check = ok(&mut model, &mut env, "check_model", json!({}));
    assert_eq!(check["profiles"]["total"], 4);
    // Too big to cut: refused, model unchanged.
    let refused = err(&mut model, &mut env, "add_profile", json!({"id": top, "between": [["-z", "+y"]], "profile": "roundover", "width_mm": 50}));
    assert!(refused["error"].as_str().unwrap().contains("cannot be cut"), "{refused}");
    let missing = err(&mut model, &mut env, "add_profile", json!({"id": top, "between": [["+z", "-z"]], "profile": "chamfer"}));
    assert!(missing["error"].as_str().unwrap().contains("no edge"));
    // Remove them all.
    let removed = ok(&mut model, &mut env, "remove_profile", json!({"id": top}));
    assert_eq!(removed["removed_profiles"].as_array().unwrap().len(), 4);
    assert!(model.routes.is_empty());
    err(&mut model, &mut env, "remove_profile", json!({"id": top}));
}

#[test]
fn joints_can_be_trimmed_and_changed_and_report_their_geometry() {
    let (mut model, mut env) = (Model::default(), ToolEnv::default());
    let (left, _right, shelf, j1, _j2) = carcass(&mut model, &mut env);
    let detail = ok(&mut model, &mut env, "get_object", json!({"id": j1}));
    let geometry = &detail["geometry"];
    assert_eq!(geometry["depth_mm"], 9.);
    assert_eq!(geometry["depth_direction"], "-x", "the shelf goes into the left side towards -x: {geometry}");
    assert_eq!(geometry["region_mm"]["min"], json!([9.0, 0.0, 300.0]));
    assert_eq!(geometry["tongue_thickness_mm"], 9.);
    // The shelf is 290 deep on a 300 deep side: the groove is open at the front only.
    assert_eq!(geometry["cut_in_b"]["y"], "stopped", "{geometry}");
    assert_eq!(geometry["cut_in_b"]["z"], "closed");
    assert_eq!(geometry["through_b"], false);

    let changed = ok(&mut model, &mut env, "set_joint", json!({"id": j1, "params": {"width_ratio": 0.33}}));
    assert_eq!(changed["joint_id"], j1);
    let ConnectionOperation::Joinery(settings) = &model.connections[&j1].operation else { panic!() };
    assert!((settings.width_ratio - 0.33).abs() < 1e-12);
    assert_eq!(settings.kind, JointKind::TongueGroove);
    ok(&mut model, &mut env, "set_joint", json!({"id": j1, "kind": "mortise_tenon"}));
    let ConnectionOperation::Joinery(settings) = &model.connections[&j1].operation else { panic!() };
    assert_eq!(settings.kind, JointKind::MortiseTenon);
    assert_eq!(model.connections[&j1].members, [shelf, left]);
    err(&mut model, &mut env, "set_joint", json!({"id": j1, "params": {"width_ratio": 2}}));
    err(&mut model, &mut env, "set_joint", json!({"id": 999_999}));

    // A rail crossing a leg, trimmed flush with the leg's outer face.
    let (mut model, mut env) = (Model::default(), ToolEnv::default());
    let leg = board(&mut model, &mut env, "Leg", [0., 0., 0.], [50., 50., 700.]);
    let rail = board(&mut model, &mut env, "Rail", [-30., 10., 600.], [400., 22., 80.]);
    let joint = ok(&mut model, &mut env, "add_joint", json!({"a": rail, "b": leg, "kind": "mortise_tenon",
        "params": {"trim": "a", "keep_end": "upper"}}))["joint_id"].as_u64().unwrap();
    let detail = ok(&mut model, &mut env, "get_object", json!({"id": joint}));
    assert_eq!(detail["trim"]["board"], "a");
    assert_eq!(detail["trim"]["keep_end"], "upper");
    let evaluated = model.evaluate_joinery();
    assert!(evaluated.errors.is_empty(), "{:?}", evaluated.errors);
    let (lo, _) = evaluated.model.selection_bounds(&BTreeSet::from([rail])).unwrap();
    assert!(lo.x > -1e-6, "the rail's excess past the leg is trimmed: {lo:?}");
}

#[test]
fn create_profile_board_extrudes_an_outline_with_arcs() {
    let (mut model, mut env) = (Model::default(), ToolEnv::default());
    // A 400 x 300 top with a half-round front edge... drawn counter-clockwise
    // from +z, the front (-y) arc bulging outward.
    let result = ok(&mut model, &mut env, "create_profile_board", json!({
        "name": "Top", "plane": "xy", "origin_mm": [0, 0, 700], "thickness_mm": 20, "grain_axis": "x",
        "points": [{"at": [0, 0], "sagitta_mm": 50}, [400, 0], [400, 300], [0, 300]]}));
    let top = result["created"][0].as_u64().unwrap();
    assert_eq!(result["arcs"], 1);
    let (lo, hi) = bounds_mm(&model, top);
    assert_eq!(lo, [0., -50., 700.], "the arc bulges 50 mm to the front");
    assert_eq!(hi, [400., 300., 720.]);
    assert!(model.solid_volume(top).is_ok());
    assert_eq!(model.objects[&top].name, "Top");
    // The arc's sagitta from a bulge: a half circle of radius 200 on the back.
    let half = ok(&mut model, &mut env, "create_profile_board", json!({
        "name": "Half", "plane": "xy", "origin_mm": [0, 1000, 0], "thickness_mm": -18,
        "points": [[0, 0], [400, 0], {"at": [400, 100], "bulge": 1}, [0, 100]]}));
    let id = half["created"][0].as_u64().unwrap();
    let (lo, hi) = bounds_mm(&model, id);
    assert_eq!(lo[2], -18., "a negative thickness extrudes downward");
    assert_eq!(hi[2], 0.);
    assert!((hi[1] - 1300.).abs() < 0.2, "a half circle on the 400 mm chord reaches 200 mm further: {hi:?}");
    // A side panel in the yz plane.
    let side = ok(&mut model, &mut env, "create_profile_board", json!({
        "name": "Side", "plane": "yz", "origin_mm": [500, 0, 0], "thickness_mm": 18,
        "points": [[0, 0], [300, 0], [300, 800], [0, 800]]}))["created"][0].as_u64().unwrap();
    assert_eq!(bounds_mm(&model, side), ([500., 0., 0.], [518., 300., 800.]));
    // Joints work with the shaped board.
    let check = ok(&mut model, &mut env, "check_model", json!({}));
    assert!(check["issues"].as_array().unwrap().iter().all(|i| i["kind"] != "open_solid"), "{check}");
    err(&mut model, &mut env, "create_profile_board", json!({
        "name": "Flat", "plane": "xy", "origin_mm": [0, 0, 0], "thickness_mm": 18, "points": [[0, 0], [100, 0], [200, 0]]}));
}

#[test]
fn split_face_by_plane_then_push_one_piece_and_merge_back() {
    let (mut model, mut env) = (Model::default(), ToolEnv::default());
    let top = board(&mut model, &mut env, "Top", [0., 0., 0.], [600., 300., 20.]);
    let faces = ok(&mut model, &mut env, "get_object", json!({"id": top}))["faces"].clone();
    let front = faces.as_array().unwrap().iter().find(|f| f["facing"] == "-y").unwrap()["id"].as_u64().unwrap();
    let split = ok(&mut model, &mut env, "split_face", json!({"face_id": front, "plane": {"point_mm": [200, 0, 0], "normal": "x"}}));
    let pieces = split["faces"].as_array().unwrap();
    assert_eq!(pieces.len(), 2, "{split}");
    let areas: BTreeSet<u64> = pieces.iter().map(|f| f["area_mm2"].as_f64().unwrap() as u64).collect();
    assert_eq!(areas, BTreeSet::from([200 * 20, 400 * 20]));
    let small = pieces.iter().find(|f| f["area_mm2"] == 4000.0).unwrap()["id"].as_u64().unwrap();
    ok(&mut model, &mut env, "push_pull", json!({"face_id": small, "distance_mm": 30}));
    assert_eq!(bounds_mm(&model, top).0[1], -30.);
    assert!(model.solid_volume(top).is_ok());

    // Another board: split its top, then merge the seam away.
    let panel = board(&mut model, &mut env, "Panel", [1000., 0., 0.], [400., 200., 18.]);
    let faces = ok(&mut model, &mut env, "get_object", json!({"id": panel}))["faces"].clone();
    let up = faces.as_array().unwrap().iter().find(|f| f["facing"] == "+z").unwrap()["id"].as_u64().unwrap();
    let count = |model: &Model| model.expand_entities(&BTreeSet::from([panel])).iter().filter(|e| model.faces.contains_key(e)).count();
    ok(&mut model, &mut env, "split_face", json!({"face_id": up, "line_mm": [[1100, 0, 18], [1100, 200, 18]]}));
    assert_eq!(count(&model), 7);
    let merged = ok(&mut model, &mut env, "merge_faces", json!({"id": panel}));
    assert!(!merged["merged"].as_array().unwrap().is_empty());
    assert_eq!(count(&model), 6, "the seam is gone and the faces are kept");
    assert!(model.solid_volume(panel).is_ok());
    let volume = model.solid_volume(panel).unwrap();
    assert!((volume - 0.4 * 0.2 * 0.018).abs() < 1e-9);
    model.validate().unwrap();
    err(&mut model, &mut env, "merge_faces", json!({"id": panel}));
    err(&mut model, &mut env, "split_face", json!({"face_id": up, "plane": {"point_mm": [5000, 0, 0], "normal": "x"}}));
}

#[test]
fn split_face_rejects_lines_on_other_faces_and_outside_the_selected_face() {
    let (mut model, mut env) = (Model::default(), ToolEnv::default());
    let panel = board(&mut model, &mut env, "Panel", [0., 0., 0.], [400., 200., 18.]);
    let up = ok(&mut model, &mut env, "get_object", json!({"id": panel}))["faces"].as_array().unwrap().iter()
        .find(|f| f["facing"] == "+z").unwrap()["id"].as_u64().unwrap();
    for line in [
        [[100, 0, 0], [100, 200, 0]], // The bottom face, not the requested top.
        [[100, -100, 18], [100, 300, 18]], // Would add edges outside the board.
        [[100, 0, 18], [100, 0, 18]], // No direction.
    ] {
        err(&mut model, &mut env, "split_face", json!({"face_id": up, "line_mm": line}));
    }
    let split = ok(&mut model, &mut env, "split_face", json!({"face_id": up, "line_mm": [[100, 0, 18], [100, 200, 18]]}));
    assert_eq!(split["faces"].as_array().unwrap().len(), 2);
    assert!(split["faces"].as_array().unwrap().iter().all(|f| f["facing"] == "+z"));
    model.validate().unwrap();
}

#[test]
fn split_face_clips_a_line_across_a_concave_outline() {
    let (mut model, mut env) = (Model::default(), ToolEnv::default());
    let panel = ok(&mut model, &mut env, "create_profile_board", json!({
        "name": "U panel", "plane": "xy", "origin_mm": [0, 0, 0], "thickness_mm": 20,
        "points": [[0, 0], [300, 0], [300, 300], [200, 300], [200, 100], [100, 100], [100, 300], [0, 300]]
    }))["created"][0].as_u64().unwrap();
    let up = ok(&mut model, &mut env, "get_object", json!({"id": panel}))["faces"].as_array().unwrap().iter()
        .find(|f| f["facing"] == "+z").unwrap()["id"].as_u64().unwrap();
    let split = ok(&mut model, &mut env, "split_face", json!({"face_id": up, "line_mm": [[0, 200, 20], [300, 200, 20]]}));
    let pieces = split["faces"].as_array().unwrap();
    assert_eq!(pieces.len(), 3, "the two arms are cut separately: {split}");
    assert_eq!(pieces.iter().map(|f| f["area_mm2"].as_f64().unwrap()).sum::<f64>(), 70_000.);
    assert!(!model.edges.values().any(|edge| {
        let (a, b) = model.points(edge);
        ((a + b) * 0.5 - DVec3::new(0.15, 0.2, 0.02)).length() < 1e-9
    }), "the cut must not bridge the empty space between the arms");
    model.validate().unwrap();
}

#[test]
fn merge_faces_respects_face_and_seam_locks_when_given_a_board() {
    let (mut model, mut env) = (Model::default(), ToolEnv::default());
    let panel = board(&mut model, &mut env, "Panel", [0., 0., 0.], [400., 200., 18.]);
    let up = ok(&mut model, &mut env, "get_object", json!({"id": panel}))["faces"].as_array().unwrap().iter()
        .find(|f| f["facing"] == "+z").unwrap()["id"].as_u64().unwrap();
    let split = ok(&mut model, &mut env, "split_face", json!({"face_id": up, "line_mm": [[100, 0, 18], [100, 200, 18]]}));
    let face = split["face_ids"][0].as_u64().unwrap();
    let seam = model.edges.values().find(|edge| {
        let (a, b) = model.points(edge);
        (a.x - 0.1).abs() < 1e-9 && (b.x - 0.1).abs() < 1e-9 && (a.z - 0.018).abs() < 1e-9 && (b.z - 0.018).abs() < 1e-9
    }).unwrap().id;
    for locked in [face, seam] {
        model.locked.insert(locked);
        for args in [json!({"id": panel}), json!({"ids": [panel]}), json!({"ids": split["face_ids"]})] {
            err(&mut model, &mut env, "merge_faces", args);
        }
        assert!(model.contains_entity(&locked));
        model.validate().unwrap();
        model.locked.remove(&locked);
    }
    ok(&mut model, &mut env, "merge_faces", json!({"id": panel}));
    model.validate().unwrap();
}

#[test]
fn change_thickness_keeps_outer_faces_and_joint_shares() {
    let (mut model, mut env) = (Model::default(), ToolEnv::default());
    let (left, right, shelf, _j1, _j2) = carcass(&mut model, &mut env);
    let result = ok(&mut model, &mut env, "change_thickness", json!({"ids": [left, right], "thickness_mm": 16}));
    assert_eq!(bounds_mm(&model, left), ([0., 0., 0.], [16., 300., 600.]), "the left side keeps its outer face");
    assert_eq!(bounds_mm(&model, right), ([484., 0., 0.], [500., 300., 600.]), "the right side keeps its outer face");
    // The shelf now enters each side by half of 16 mm.
    assert_eq!(bounds_mm(&model, shelf).0[0], 8., "{result}");
    assert_eq!(bounds_mm(&model, shelf).1[0], 492.);
    assert_eq!(result["joint_adjustments"].as_array().unwrap().len(), 2);
    let check = ok(&mut model, &mut env, "check_model", json!({}));
    assert_eq!(check["joints"]["failing"], json!([]), "{check}");
    // A shelf on its own keeps its centre.
    ok(&mut model, &mut env, "change_thickness", json!({"ids": [shelf], "thickness_mm": 20, "anchor": "center"}));
    assert_eq!(bounds_mm(&model, shelf).0[2], 299.);
    err(&mut model, &mut env, "change_thickness", json!({"ids": [shelf], "thickness_mm": 0}));
}

#[test]
fn set_finish_and_get_estimate() {
    let (mut model, mut env) = (Model::default(), ToolEnv::default());
    let (left, right, shelf, _, _) = carcass(&mut model, &mut env);
    let group = ok(&mut model, &mut env, "group", json!({"ids": [left, right, shelf], "name": "Case", "component": false}))["created"][0].as_u64().unwrap();
    let shelf = model.objects.values().find(|o| o.name == "Shelf").unwrap().id;
    let empty = ok(&mut model, &mut env, "get_estimate", json!({}));
    assert_eq!(empty["finished_boards"], 0);
    assert!(empty["hint"].is_string());
    assert_eq!(empty["joinery"][0]["kind"], "tongue_groove");
    assert_eq!(empty["joinery"][0]["count"], 2);
    assert_eq!(empty["joinery"][0]["work_minutes"], 16.);

    let set = ok(&mut model, &mut env, "set_finish", json!({"ids": [group], "sanding_grits": [120, 180],
        "varnish": {"coats": 2, "sheen": "satin"}}));
    assert_eq!(set["objects_set"], 4, "the group and its three boards");
    let detail = ok(&mut model, &mut env, "get_object", json!({"id": shelf}));
    assert_eq!(detail["finish"]["sanding_grits"], json!([120, 180]));
    assert_eq!(detail["finish"]["varnish"]["coats"], 2);
    assert_eq!(detail["finish"]["varnish"]["sheen"], "satin");
    assert_eq!(detail["finish"]["stain"], false);
    let estimate = ok(&mut model, &mut env, "get_estimate", json!({}));
    assert_eq!(estimate["finished_boards"], 3);
    assert!(estimate["total"]["work_minutes"].as_f64().unwrap() > 16.);
    assert!(estimate["finishing"].as_array().unwrap().len() >= 3, "{estimate}");
    let one = ok(&mut model, &mut env, "get_estimate", json!({"ids": [shelf]}));
    assert_eq!(one["boards"], 1);
    ok(&mut model, &mut env, "set_finish", json!({"ids": [group], "clear": true}));
    assert!(model.objects[&shelf].surface_treatment_override().is_none());
    err(&mut model, &mut env, "set_finish", json!({"ids": [shelf]}));
}

#[test]
fn scoped_estimates_only_include_selected_wood_and_keep_inherited_materials() {
    let (mut model, mut env) = (Model::default(), ToolEnv::default());
    let a = board(&mut model, &mut env, "Small", [0., 0., 0.], [100., 100., 20.]);
    let b = board(&mut model, &mut env, "Large", [200., 0., 0.], [1000., 1000., 20.]);
    let group = ok(&mut model, &mut env, "group", json!({"ids": [a, b], "name": "Pair", "component": false}))["created"][0].as_u64().unwrap();
    let small = model.objects.values().find(|o| o.name == "Small").unwrap().id;
    let material = ok(&mut model, &mut env, "create_stock_material", json!({"name": "Plywood", "thickness_mm": 20, "width_mm": 1220, "length_mm": 2440}))["material_id"].clone();
    ok(&mut model, &mut env, "set_material", json!({"ids": [group], "material_id": material}));
    board(&mut model, &mut env, "Unrelated", [2000., 0., 0.], [1000., 1000., 20.]);
    let one = ok(&mut model, &mut env, "get_estimate", json!({"ids": [small]}));
    assert_eq!(one["boards"], 1);
    assert_eq!(one["wood"].as_array().unwrap().len(), 1);
    assert_eq!(one["wood"][0]["material"], "Plywood");
    assert_eq!(one["wood"][0]["pieces"], 1);
    assert_eq!(one["wood"][0]["area_m2"], 0.01);
    assert_eq!(one["wood"][0]["volume_m3"], 0.0002);
    let pair = ok(&mut model, &mut env, "get_estimate", json!({"ids": [group, small]}));
    assert_eq!(pair["boards"], 2);
    assert_eq!(pair["wood"].as_array().unwrap().len(), 1);
    assert_eq!(pair["wood"][0]["pieces"], 2);
    assert_eq!(pair["wood"][0]["area_m2"], 1.01);
    let all = ok(&mut model, &mut env, "get_estimate", json!({}));
    assert_eq!(all["wood"], crate::review::cut_list(&model)["materials"]);
    assert_eq!(all["wood"].as_array().unwrap().iter().map(|m| m["pieces"].as_u64().unwrap()).sum::<u64>(), 3);
}

#[test]
fn measure_space_and_find_surfaces_see_walls_made_of_faces() {
    let (mut model, mut env) = (Model::default(), ToolEnv::default());
    let (_, _, shelf, _, _) = carcass(&mut model, &mut env);
    let bottom = board(&mut model, &mut env, "Bottom", [18., 0., 0.], [464., 290., 18.]);
    // A wall behind the piece at y = 400, a plain face inside a group.
    let wall = ok(&mut model, &mut env, "draw_polyline", json!({"points_mm": [[-500, 400, 0], [1000, 400, 0], [1000, 400, 2400], [-500, 400, 2400]], "close": true}));
    let room = ok(&mut model, &mut env, "group", json!({"ids": wall["created"].clone(), "name": "Room", "component": false}))["created"][0].as_u64().unwrap();
    let space = ok(&mut model, &mut env, "measure_space", json!({"point_mm": [250, 150, 100]}));
    let z = &space["axes"]["z"];
    assert_eq!(z["minus"]["object"], bottom);
    assert_eq!(z["plus"]["object"], shelf);
    assert_eq!(z["free_gap_mm"], 300. - 18., "{z}");
    assert_eq!(space["axes"]["y"]["plus"]["name"], "Room");
    assert_eq!(space["axes"]["y"]["plus"]["distance_mm"], 250.);
    assert_eq!(space["axes"]["x"]["free_gap_mm"], 464.);
    let surfaces = ok(&mut model, &mut env, "find_surfaces", json!({}));
    assert_eq!(surfaces["total"], 1, "boards are closed solids and are not listed: {surfaces}");
    assert_eq!(surfaces["surfaces"][0]["object"], room);
    assert_eq!(surfaces["surfaces"][0]["plane"], "y = 400");
    let up = ok(&mut model, &mut env, "find_surfaces", json!({"facing": "+z"}));
    assert_eq!(up["total"], 0);
}

#[test]
fn new_mutating_tools_reach_boards_inside_groups_and_respect_leases() {
    let (mut model, mut env) = (Model::default(), ToolEnv::default());
    let a = board(&mut model, &mut env, "A", [0., 0., 0.], [500., 300., 18.]);
    let b = board(&mut model, &mut env, "B", [0., 0., 100.], [500., 300., 18.]);
    ok(&mut model, &mut env, "group", json!({"ids": [a, b], "name": "Pair", "component": false}));
    let ids: Vec<u64> = model.objects.values().filter(|o| o.name != "Pair").map(|o| o.id).collect();
    ok(&mut model, &mut env, "add_profile", json!({"id": ids[0], "between": [["+z", "-y"]], "profile": "chamfer"}));
    ok(&mut model, &mut env, "distribute", json!({"ids": ids, "axis": "z", "from_mm": 0, "to_mm": 300}));
    assert_eq!(model.edit_context, None, "the group opened for the call is closed again");
    // Shared project: boards outside the lease are refused.
    env.lease = Some(BTreeSet::new());
    env.created.clear();
    let refused = err(&mut model, &mut env, "add_profile", json!({"id": ids[1], "between": [["+z", "-y"]], "profile": "chamfer"}));
    assert_eq!(refused["error"], "not-leased");
}

#[test]
fn round_five_fixes() {
    // merge_faces reaches a board inside a group by itself.
    let (mut model, mut env) = (Model::default(), ToolEnv::default());
    let a = board(&mut model, &mut env, "A", [0., 0., 0.], [400., 200., 18.]);
    let b = board(&mut model, &mut env, "B", [0., 0., 100.], [400., 200., 18.]);
    ok(&mut model, &mut env, "group", json!({"ids": [a, b], "name": "Pair", "component": false}));
    let inner = model.objects.values().find(|o| o.name == "A").unwrap().id;
    let up = ok(&mut model, &mut env, "get_object", json!({"id": inner}))["faces"].as_array().unwrap().iter().find(|f| f["facing"] == "+z").unwrap()["id"].as_u64().unwrap();
    let split = ok(&mut model, &mut env, "split_face", json!({"face_id": up, "plane": {"point_mm": [100, 0, 0], "normal": "x"}}));
    let centres: Vec<f64> = split["faces"].as_array().unwrap().iter().map(|f| f["center_mm"][0].as_f64().unwrap()).collect();
    assert!(centres[0] < centres[1], "pieces go from the plane's negative side to its positive side: {centres:?}");
    ok(&mut model, &mut env, "merge_faces", json!({"id": inner}));
    assert_eq!(model.edit_context, None);

    // set_grain takes several ids, or one id.
    let ids: Vec<u64> = model.objects.values().filter(|o| o.name != "Pair").map(|o| o.id).collect();
    ok(&mut model, &mut env, "set_grain", json!({"ids": ids, "axis": "x"}));
    ok(&mut model, &mut env, "set_grain", json!({"id": ids[0], "axis": "y"}));

    // change_thickness follows the geometry when joints are reversed, and
    // boards that only butt against a thickened board follow its face.
    let (mut model, mut env) = (Model::default(), ToolEnv::default());
    let left = board(&mut model, &mut env, "Left", [0., 0., 0.], [18., 300., 600.]);
    let divider = board(&mut model, &mut env, "Divider", [491., 0., 0.], [18., 300., 600.]);
    let shelf = board(&mut model, &mut env, "Shelf", [9., 0., 300.], [482., 300., 18.]);
    // Reversed on purpose: the side is a, the shelf b.
    model.add_connection(benchup_core::connections::Connection {
        members: [left, shelf],
        entry_face: None,
        operation: ConnectionOperation::Joinery(JointSettings { kind: JointKind::TongueGroove, depth_axis: 0, width_axis: 2, ..Default::default() }),
    }).unwrap();
    let right = board(&mut model, &mut env, "Right", [982., 0., 0.], [18., 300., 600.]);
    let result = ok(&mut model, &mut env, "change_thickness", json!({"ids": [left, divider, right], "thickness_mm": 19}));
    assert_eq!(result["joint_adjustments"].as_array().unwrap().len(), 1, "{result}");
    assert_eq!(result["butt_adjustments"].as_array().unwrap().len(), 1, "{result}");
    assert_eq!(bounds_mm(&model, shelf).0[0], 9.5, "the shelf enters half of 19 mm");
    assert_eq!(bounds_mm(&model, shelf).1[0], 490.5, "and stops at the thickened divider");

    // only_new does not report existing issues again after grouping renews ids.
    let (mut model, mut env) = (Model::default(), ToolEnv::default());
    let a = board(&mut model, &mut env, "A", [0., 0., 0.], [100., 100., 18.]);
    let b = board(&mut model, &mut env, "B", [50., 0., 0.], [100., 100., 18.]);
    env.before = None;
    ok(&mut model, &mut env, "group", json!({"ids": [a, b], "name": "G", "component": false}));
    let check = ok(&mut model, &mut env, "check_model", json!({"only_new": true}));
    assert_eq!(check["issues"], json!([]), "{check}");
    assert_eq!(check["existing_by_kind"]["overlap_without_joint"], 1);

    // A mortise closed inside the leg reads closed, even with the rail flush with the leg's face.
    let (mut model, mut env) = (Model::default(), ToolEnv::default());
    let leg = board(&mut model, &mut env, "Leg", [0., 0., 0.], [50., 50., 425.]);
    let rail = board(&mut model, &mut env, "Rail", [0., 0., 300.], [600., 22., 70.]);
    let joint = ok(&mut model, &mut env, "add_joint", json!({"a": rail, "b": leg, "kind": "mortise_tenon", "params": {"width_ratio": 0.33, "height_ratio": 0.75}}))["joint_id"].as_u64().unwrap();
    let geometry = ok(&mut model, &mut env, "get_object", json!({"id": joint}))["geometry"].clone();
    assert_eq!(geometry["cut_in_b"]["y"], "closed", "{geometry}");
    assert_eq!(geometry["cut_in_b"]["z"], "closed", "{geometry}");
    assert_eq!(geometry["through_b"], true);

    // A back in a rabbet routed into the side is not an overlap.
    let (mut model, mut env) = (Model::default(), ToolEnv::default());
    let side = board(&mut model, &mut env, "Side", [0., 0., 0.], [18., 300., 900.]);
    ok(&mut model, &mut env, "add_profile", json!({"id": side, "between": [["+y", "+x"]], "profile": "rabbet", "width_mm": 6, "depth_mm": 9}));
    board(&mut model, &mut env, "Back", [9., 294., 0.], [500., 6., 900.]);
    let check = ok(&mut model, &mut env, "check_model", json!({}));
    assert!(check["issues"].as_array().unwrap().iter().all(|i| i["kind"] != "overlap_without_joint"), "{check}");
}

#[test]
fn merge_faces_leaves_one_face_with_the_mortise_mouth_as_a_hole() {
    // A mortise made by splitting a leg's face and pushing the mouth in, as an agent would.
    let (mut model, mut env) = (Model::default(), ToolEnv::default());
    let leg = board(&mut model, &mut env, "Leg", [0., 0., 0.], [70., 70., 700.]);
    let front = ok(&mut model, &mut env, "get_object", json!({"id": leg}))["faces"].as_array().unwrap().iter()
        .find(|f| f["facing"] == "-y").unwrap()["id"].as_u64().unwrap();
    let split = |model: &mut Model, env: &mut ToolEnv, face: u64, at: [f64; 3], normal: &str, piece: usize| -> u64 {
        ok(model, env, "split_face", json!({"face_id": face, "plane": {"point_mm": at, "normal": normal}}))["faces"][piece]["id"].as_u64().unwrap()
    };
    let upper = split(&mut model, &mut env, front, [0., 0., 550.], "z", 1);
    let band = split(&mut model, &mut env, upper, [0., 0., 630.], "z", 0);
    let right = split(&mut model, &mut env, band, [24., 0., 0.], "x", 1);
    let mouth = split(&mut model, &mut env, right, [46., 0., 0.], "x", 0);
    ok(&mut model, &mut env, "push_pull", json!({"face_id": mouth, "distance_mm": -30}));
    ok(&mut model, &mut env, "merge_faces", json!({"id": leg}));
    let faces = ok(&mut model, &mut env, "get_object", json!({"id": leg}))["faces"].clone();
    let fronts: Vec<f64> = faces.as_array().unwrap().iter().filter(|f| f["facing"] == "-y").map(|f| f["area_mm2"].as_f64().unwrap()).collect();
    assert_eq!(faces.as_array().unwrap().len(), 11, "6 faces of the leg and 5 of the mortise: {faces}");
    assert!(fronts.contains(&(70. * 700. - 22. * 80.)), "one front face around the mouth: {fronts:?}");
    assert!(model.solid_volume(leg).is_ok());
    model.validate().unwrap();
}
