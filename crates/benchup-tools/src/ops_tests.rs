use super::*;
use serde_json::{Value, json};

pub fn call(name: &str, args: Value) -> Call {
    Call::new("t1", name, args)
}
pub fn run(model: &mut Model, env: &mut ToolEnv, name: &str, args: Value) -> ToolOutcome {
    execute(model, &call(name, args), env)
}
fn ok(model: &mut Model, env: &mut ToolEnv, name: &str, args: Value) -> ToolOutcome {
    let outcome = run(model, env, name, args);
    assert!(outcome.ok, "{name} failed: {}", outcome.result);
    outcome
}
fn err(model: &mut Model, env: &mut ToolEnv, name: &str, args: Value) -> String {
    let before = model.clone();
    let outcome = run(model, env, name, args);
    assert!(!outcome.ok, "{name} unexpectedly succeeded: {}", outcome.result);
    assert_eq!(outcome.result["ok"], false);
    assert!(!outcome.mutated);
    assert_eq!(*model, before, "{name} must leave the model untouched on error");
    outcome.result["error"].as_str().unwrap().to_owned()
}
pub fn board(model: &mut Model, env: &mut ToolEnv, name: &str, origin: [f64; 3], size: [f64; 3]) -> u64 {
    ok(model, env, "create_board", json!({"name": name, "origin_mm": origin, "size_mm": size})).created[0]
}
fn bounds(model: &Model, id: u64) -> (DVec3, DVec3) {
    model.selection_bounds(&BTreeSet::from([id])).unwrap()
}
fn near(a: DVec3, b: [f64; 3]) {
    assert!(a.distance(DVec3::from(b)) < 1e-6, "{a:?} != {b:?}");
}

#[test]
fn create_board_converts_millimetres_to_metres() {
    let (mut model, mut env) = (Model::default(), ToolEnv::default());
    let outcome = ok(&mut model, &mut env, "create_board", json!({
        "name": "Top", "origin_mm": [100, 200, 300], "size_mm": [600, 400, 19]}));
    let id = outcome.created[0];
    assert_eq!(model.objects[&id].name, "Top");
    let (lo, hi) = bounds(&model, id);
    near(lo, [0.1, 0.2, 0.3]);
    near(hi, [0.7, 0.6, 0.319]);
    assert_eq!(outcome.result["bounds_mm"]["size"], json!([600.0, 400.0, 19.0]));
    assert_eq!(outcome.result["created"], json!([id]));
    assert!(outcome.mutated);
    assert!(env.before.is_some(), "first mutation snapshots the model");
    // It is a closed solid: six faces.
    let faces = model.expand_entities(&BTreeSet::from([id])).iter().filter(|f| model.faces.contains_key(f)).count();
    assert_eq!(faces, 6);
}

#[test]
fn create_board_applies_material_grain_and_tag() {
    let (mut model, mut env) = (Model::default(), ToolEnv::default());
    let material = benchup_core::MaterialId::new_v4();
    env.catalog.insert(material, StockMaterial { name: "Oak 19".into(), dimensions: [190, 890, 24400] });
    let id = ok(&mut model, &mut env, "create_board", json!({
        "name": "Leg", "origin_mm": [0, 0, 0], "size_mm": [40, 40, 700],
        "material_id": material.to_string(), "grain_axis": "z", "tag": "Legs"})).created[0];
    assert_eq!(model.objects[&id].stock_material_id, Some(material));
    assert!(model.stock_materials.contains_key(&material));
    assert!(model.objects[&id].grain_axis().unwrap().z.abs() > 0.99);
    assert_eq!(model.inherited_tag(id).unwrap().name, "Legs");
    let again = ok(&mut model, &mut env, "create_board", json!({
        "name": "Leg 2", "origin_mm": [100, 0, 0], "size_mm": [40, 40, 700], "tag": "legs"})).created[0];
    assert_eq!(model.tags.len(), 1, "tag names match case-insensitively");
    assert_eq!(model.inherited_tag(again).unwrap().name, "Legs");
    err(&mut model, &mut env, "create_board", json!({
        "name": "X", "origin_mm": [0, 0, 0], "size_mm": [1, 1, 1], "material_id": benchup_core::MaterialId::new_v4().to_string()}));
}

#[test]
fn argument_and_id_errors_are_structured_and_harmless() {
    let (mut model, mut env) = (Model::default(), ToolEnv::default());
    let leg = board(&mut model, &mut env, "Leg", [0., 0., 0.], [40., 40., 700.]);
    assert!(err(&mut model, &mut env, "nope", json!({})).contains("unknown tool"));
    assert!(err(&mut model, &mut env, "move", json!({"ids": [leg]})).contains("invalid arguments"));
    assert!(err(&mut model, &mut env, "move", json!({"ids": [leg], "delta_mm": [1, 2, 3], "extra": 1})).contains("invalid arguments"));
    assert!(err(&mut model, &mut env, "move", json!({"ids": "a", "delta_mm": [1, 2, 3]})).contains("invalid arguments"));
    assert!(err(&mut model, &mut env, "move", json!({"ids": [9999], "delta_mm": [1, 2, 3]})).contains("unknown id"));
    assert!(err(&mut model, &mut env, "move", json!({"ids": [], "delta_mm": [1, 2, 3]})).contains("empty"));
    assert!(err(&mut model, &mut env, "move", json!({"ids": [leg], "delta_mm": [1e12, 0, 0]})).contains("out of range"));
    assert!(err(&mut model, &mut env, "create_board", json!({"name": "", "origin_mm": [0,0,0], "size_mm": [1,1,1]})).contains("name"));
    assert!(err(&mut model, &mut env, "create_board", json!({"name": "a", "origin_mm": [0,0,0], "size_mm": [0,1,1]})).contains("size"));
    assert!(err(&mut model, &mut env, "get_object", json!({"id": 424242})).contains("unknown id"));
    // Malformed text never panics.
    let outcome = execute(&mut model, &Call::from_text("x", "move", "{not json"), &mut env);
    assert!(!outcome.ok);
    let hint = run(&mut model, &mut env, "move", json!({"ids": [9999], "delta_mm": [1, 2, 3]}));
    assert!(hint.result["hint"].is_string());
}

#[test]
fn locked_hidden_and_read_only_are_refused() {
    let (mut model, mut env) = (Model::default(), ToolEnv::default());
    let a = board(&mut model, &mut env, "A", [0., 0., 0.], [100., 100., 100.]);
    let b = board(&mut model, &mut env, "B", [500., 0., 0.], [100., 100., 100.]);
    model.locked.insert(a);
    assert!(err(&mut model, &mut env, "move", json!({"ids": [a], "delta_mm": [1, 0, 0]})).contains("locked"));
    assert!(err(&mut model, &mut env, "rename", json!({"id": a, "name": "Z"})).contains("locked"));
    assert!(err(&mut model, &mut env, "erase", json!({"ids": [a]})).contains("locked"));
    assert!(err(&mut model, &mut env, "add_joint", json!({"a": a, "b": b, "kind": "half_lap"})).contains("locked"));
    // Locked entities can still be read.
    ok(&mut model, &mut env, "get_object", json!({"id": a}));
    model.hidden.insert(b);
    assert!(err(&mut model, &mut env, "move", json!({"ids": [b], "delta_mm": [1, 0, 0]})).contains("hidden"));
    model.hidden.clear();
    env.read_only = true;
    assert_eq!(err(&mut model, &mut env, "move", json!({"ids": [b], "delta_mm": [1, 0, 0]})), "read-only");
    assert_eq!(err(&mut model, &mut env, "create_board", json!({"name":"x","origin_mm":[0,0,0],"size_mm":[1,1,1]})), "read-only");
    // Reads still work when read-only.
    ok(&mut model, &mut env, "get_scene", json!({}));
}

#[test]
fn read_tools_report_scene_objects_and_faces() {
    let (mut model, mut env) = (Model::default(), ToolEnv::default());
    let top = board(&mut model, &mut env, "Table top", [0., 0., 700.], [600., 400., 20.]);
    let leg = board(&mut model, &mut env, "Left leg", [0., 0., 0.], [40., 40., 700.]);
    env.selection = BTreeSet::from([leg]);
    let scene = ok(&mut model, &mut env, "get_scene", json!({})).result;
    assert_eq!(scene["units"], "mm");
    assert_eq!(scene["objects"].as_array().unwrap().len(), 2);
    assert_eq!(scene["selection"], json!([leg]));
    assert_eq!(scene["objects"][0]["id"], leg, "selected objects come first");
    let found = ok(&mut model, &mut env, "find_objects", json!({"query": "LEG"})).result;
    assert_eq!(found["total"], 1);
    assert_eq!(found["objects"][0]["id"], leg);
    assert_eq!(ok(&mut model, &mut env, "find_objects", json!({"query": "group"})).result["total"], 2);
    let object = ok(&mut model, &mut env, "get_object", json!({"id": top})).result;
    assert_eq!(object["size_mm"], json!([600.0, 400.0, 20.0]));
    let faces = object["faces"].as_array().unwrap();
    assert_eq!(faces.len(), 6);
    let facings: BTreeSet<&str> = faces.iter().filter_map(|f| f["facing"].as_str()).collect();
    assert_eq!(facings, BTreeSet::from(["+x", "-x", "+y", "-y", "+z", "-z"]));
    let top_face = faces.iter().find(|f| f["facing"] == "+z").unwrap();
    assert_eq!(top_face["center_mm"][2], 720.0);
    assert_eq!(top_face["area_mm2"], 240_000.0);
    let selection = ok(&mut model, &mut env, "get_selection", json!({})).result;
    assert_eq!(selection["selection"], json!([leg]));
    let materials = ok(&mut model, &mut env, "list_stock_materials", json!({})).result;
    assert_eq!(materials["materials"], json!([]));
}

#[test]
fn transform_tools_move_rotate_scale_and_resize() {
    let (mut model, mut env) = (Model::default(), ToolEnv::default());
    let id = board(&mut model, &mut env, "Rail", [0., 0., 0.], [100., 50., 20.]);
    let outcome = ok(&mut model, &mut env, "move", json!({"ids": [id], "delta_mm": [10, 20, 30]}));
    assert_eq!(outcome.modified, [id]);
    near(bounds(&model, id).0, [0.01, 0.02, 0.03]);
    ok(&mut model, &mut env, "rotate", json!({"ids": [id], "pivot_mm": [10, 20, 30], "axis": "z", "degrees": 90}));
    let (lo, hi) = bounds(&model, id);
    near(hi - lo, [0.05, 0.1, 0.02]);
    ok(&mut model, &mut env, "scale", json!({"ids": [id], "center_mm": [10, 20, 30], "factor": 2}));
    let (lo, hi) = bounds(&model, id);
    near(hi - lo, [0.1, 0.2, 0.04]);
    near(lo, [-0.09, 0.02, 0.03]);
    let outcome = ok(&mut model, &mut env, "resize_board", json!({"id": id, "size_mm": [100, 200, 60]}));
    let (lo2, hi2) = bounds(&model, id);
    near(hi2 - lo2, [0.1, 0.2, 0.06]);
    near(lo2, lo.to_array());
    assert_eq!(outcome.result["bounds_mm"]["size"], json!([100.0, 200.0, 60.0]));
    assert!(err(&mut model, &mut env, "scale", json!({"ids": [id], "center_mm": [0,0,0], "factor": -1})).len() > 3);
    assert!(err(&mut model, &mut env, "resize_board", json!({"id": id, "size_mm": [0, 10, 10]})).contains("size_mm"));
}

#[test]
fn push_pull_extrudes_a_face_of_a_board() {
    let (mut model, mut env) = (Model::default(), ToolEnv::default());
    let id = board(&mut model, &mut env, "Block", [0., 0., 0.], [100., 100., 100.]);
    let object = ok(&mut model, &mut env, "get_object", json!({"id": id})).result;
    let top = object["faces"].as_array().unwrap().iter().find(|f| f["facing"] == "+z").unwrap()["id"].as_u64().unwrap();
    let outcome = ok(&mut model, &mut env, "push_pull", json!({"face_id": top, "distance_mm": 50}));
    assert_eq!(outcome.modified, [id]);
    near(bounds(&model, id).1, [0.1, 0.1, 0.15]);
    assert_eq!(model.edit_context, None, "scope is restored");
    assert!(err(&mut model, &mut env, "push_pull", json!({"face_id": id, "distance_mm": 5})).contains("not a face"));
    assert!(err(&mut model, &mut env, "push_pull", json!({"face_id": top, "distance_mm": 0})).len() > 3);
}

#[test]
fn naming_material_grain_tag_paint_and_erase() {
    let (mut model, mut env) = (Model::default(), ToolEnv::default());
    let a = board(&mut model, &mut env, "A", [0., 0., 0.], [100., 100., 20.]);
    let b = board(&mut model, &mut env, "B", [200., 0., 0.], [100., 100., 20.]);
    ok(&mut model, &mut env, "rename", json!({"id": a, "name": "  Shelf "}));
    assert_eq!(model.objects[&a].name, "Shelf");
    assert!(err(&mut model, &mut env, "rename", json!({"id": a, "name": "  "})).contains("name"));
    let material = benchup_core::MaterialId::new_v4();
    env.catalog.insert(material, StockMaterial { name: "Pine".into(), dimensions: [190, 890, 24400] });
    ok(&mut model, &mut env, "set_material", json!({"ids": [a, b], "material_id": material.to_string()}));
    assert_eq!(model.objects[&b].stock_material_id, Some(material));
    let listed = ok(&mut model, &mut env, "list_stock_materials", json!({})).result;
    assert_eq!(listed["materials"][0]["thickness_mm"], 19.0);
    ok(&mut model, &mut env, "set_material", json!({"ids": [a], "material_id": null}));
    assert_eq!(model.objects[&a].stock_material_id, None);
    ok(&mut model, &mut env, "set_grain", json!({"id": a, "axis": "x"}));
    assert!(model.objects[&a].grain_axis().is_some());
    ok(&mut model, &mut env, "set_grain", json!({"id": a, "axis": null}));
    assert!(model.objects[&a].grain_axis().is_none());
    let tagged = ok(&mut model, &mut env, "tag", json!({"ids": [a, b], "tag": "Shelves"}));
    assert!(tagged.result["tag_id"].is_u64());
    assert_eq!(model.inherited_tag(b).unwrap().name, "Shelves");
    ok(&mut model, &mut env, "tag", json!({"ids": [a], "tag": null}));
    assert!(model.inherited_tag(a).is_none());
    ok(&mut model, &mut env, "paint", json!({"ids": [a], "rgb": [200, 10, 10]}));
    assert!(!model.face_colors.is_empty());
    assert!(err(&mut model, &mut env, "paint", json!({"ids": [a], "rgb": [300, 0, 0]})).contains("invalid arguments"));
    let erased = ok(&mut model, &mut env, "erase", json!({"ids": [b]}));
    assert_eq!(erased.erased, [b]);
    assert!(!model.objects.contains_key(&b));
    assert_eq!(erased.result["erased"], json!([b]));
}

#[test]
fn copy_group_ungroup_and_selection_scope() {
    let (mut model, mut env) = (Model::default(), ToolEnv::default());
    let a = board(&mut model, &mut env, "A", [0., 0., 0.], [100., 100., 20.]);
    let b = board(&mut model, &mut env, "B", [200., 0., 0.], [100., 100., 20.]);
    let copy = ok(&mut model, &mut env, "create_copy", json!({"id": a, "offset_mm": [0, 500, 0]}));
    assert_eq!(copy.created.len(), 1);
    near(bounds(&model, copy.created[0]).0, [0., 0.5, 0.]);
    let grouped = ok(&mut model, &mut env, "group", json!({"ids": [a, b], "name": "Pair", "component": false}));
    let group = grouped.created[0];
    assert_eq!(model.objects[&group].name, "Pair");
    assert_eq!(grouped.result["members"].as_array().unwrap().len(), 2);
    // Parts are now inside the group: editing tools open it for the call.
    let inner = grouped.result["members"][0]["id"].as_u64().unwrap();
    ok(&mut model, &mut env, "rename", json!({"id": inner, "name": "Inner"}));
    assert_eq!(model.edit_context, None, "the group closes again after the call");
    // Ids in different groups are not opened together.
    let outside = board(&mut model, &mut env, "Outside", [0., 900., 0.], [10., 10., 10.]);
    let message = err(&mut model, &mut env, "move", json!({"ids": [inner, outside], "delta_mm": [1, 0, 0]}));
    assert!(message.contains("not editable"), "{message}");
    ok(&mut model, &mut env, "enter_object", json!({"id": group}));
    assert_eq!(model.edit_context, Some(group));
    assert!(err(&mut model, &mut env, "enter_object", json!({"id": 99999})).contains("cannot open"));
    ok(&mut model, &mut env, "exit_to_root", json!({}));
    assert_eq!(model.edit_context, None);
    let freed = ok(&mut model, &mut env, "ungroup", json!({"ids": [group]}));
    assert!(!model.objects.contains_key(&group));
    assert_eq!(freed.erased, [group]);
    assert_eq!(model.objects.values().filter(|o| o.name == "Inner").count(), 1);
    let any = *model.objects.keys().next().unwrap();
    let selected = ok(&mut model, &mut env, "select", json!({"ids": [any]}));
    assert!(!selected.mutated);
    assert_eq!(env.selection, BTreeSet::from([any]));
}

#[test]
fn drawing_and_annotation_tools() {
    let (mut model, mut env) = (Model::default(), ToolEnv::default());
    let outcome = ok(&mut model, &mut env, "draw_polyline", json!({
        "points_mm": [[0, 0, 0], [100, 0, 0], [100, 100, 0], [0, 100, 0]], "close": true}));
    assert!(outcome.created.len() >= 5, "four edges and a face: {:?}", outcome.created);
    assert_eq!(model.faces.len(), 1);
    assert!(err(&mut model, &mut env, "draw_polyline", json!({"points_mm": [[0, 0, 0]], "close": false})).contains("at least 2"));
    let dim = ok(&mut model, &mut env, "add_dimension", json!({"start_mm": [0, 0, 0], "end_mm": [100, 0, 0], "offset_mm": [0, -50, 0]}));
    assert!(model.dimensions.contains_key(&dim.created[0]));
    err(&mut model, &mut env, "add_dimension", json!({"start_mm": [0, 0, 0], "end_mm": [0, 0, 0], "offset_mm": [0, -50, 0]}));
    let text = ok(&mut model, &mut env, "add_text", json!({"point_mm": [0, 0, 0], "offset_mm": [0, 0, 100], "text": "Front"}));
    assert_eq!(model.texts[&text.created[0]].text, "Front");
    err(&mut model, &mut env, "add_text", json!({"point_mm": [0, 0, 0], "offset_mm": [0, 0, 100], "text": ""}));
}

#[test]
fn joints_connect_two_boards() {
    let (mut model, mut env) = (Model::default(), ToolEnv::default());
    let leg = board(&mut model, &mut env, "Leg", [0., 0., 0.], [40., 40., 700.]);
    let apron = board(&mut model, &mut env, "Apron", [20., 0., 500.], [300., 40., 80.]);
    let outcome = ok(&mut model, &mut env, "add_joint", json!({
        "a": apron, "b": leg, "kind": "mortise_tenon", "params": {"width_ratio": 0.6, "clearance_mm": 0.2}}));
    assert_eq!(outcome.joints.len(), 1);
    assert_eq!(model.connections.len(), 1);
    match &model.connections[&outcome.joints[0]].operation {
        benchup_core::connections::ConnectionOperation::Joinery(s) => {
            assert_eq!(s.kind, benchup_core::joinery::JointKind::MortiseTenon);
            assert_eq!(s.width_ratio, 0.6);
            assert!((s.clearance - 0.0002).abs() < 1e-12);
        }
        other => panic!("{other:?}"),
    }
    let scene = ok(&mut model, &mut env, "get_scene", json!({})).result;
    assert_eq!(scene["joints"][0]["kind"], "mortise_tenon");
    // A second joint between the same boards is refused by the core.
    err(&mut model, &mut env, "add_joint", json!({"a": leg, "b": apron, "kind": "half_lap"}));
    err(&mut model, &mut env, "add_joint", json!({"a": leg, "b": leg, "kind": "half_lap"}));
    err(&mut model, &mut env, "add_joint", json!({"a": leg, "b": apron, "kind": "bogus"}));
    err(&mut model, &mut env, "add_joint", json!({"a": leg, "b": apron, "kind": "finger", "params": {"width_ratio": 3}}));
}

#[test]
fn failed_calls_do_not_set_the_turn_snapshot() {
    let (mut model, mut env) = (Model::default(), ToolEnv::default());
    err(&mut model, &mut env, "move", json!({"ids": [1], "delta_mm": [1, 0, 0]}));
    assert!(env.before.is_none());
    let id = board(&mut model, &mut env, "A", [0., 0., 0.], [10., 10., 10.]);
    let first = env.before.clone().unwrap();
    assert!(first.objects.is_empty());
    ok(&mut model, &mut env, "move", json!({"ids": [id], "delta_mm": [1, 0, 0]}));
    assert_eq!(env.before.as_ref().unwrap(), &first, "snapshot stays at the first change");
}

#[test]
fn tool_definitions_are_strict_and_match_the_executor() {
    let definitions = definitions();
    assert_eq!(definitions.len(), TOOLS.len());
    for definition in &definitions {
        let name = definition.name.as_str();
        assert!(is_known(name), "{name}");
        assert!(!definition.description.is_empty());
        let parameters = &definition.input_schema;
        assert_eq!(parameters["type"], "object");
        assert_eq!(parameters["additionalProperties"], false, "{name}");
        let properties = parameters["properties"].as_object().unwrap();
        for required in parameters["required"].as_array().unwrap() {
            assert!(properties.contains_key(required.as_str().unwrap()), "{name}.{required}");
        }
        // Every tool accepts the empty object or reports a clean argument error.
        let outcome = run(&mut Model::default(), &mut ToolEnv::default(), name, json!({"__unexpected": 1}));
        assert!(!outcome.ok, "{name} must reject unknown fields");
    }
}

fn first_face(model: &Model, object: u64) -> u64 {
    model.expand_entities(&BTreeSet::from([object])).into_iter().find(|f| model.faces.contains_key(f)).unwrap()
}

#[test]
fn shared_lease_refuses_entities_outside_it() {
    let (mut model, mut env) = (Model::default(), ToolEnv::default());
    let mine = board(&mut model, &mut env, "Mine", [0., 0., 0.], [100., 100., 100.]);
    let theirs = board(&mut model, &mut env, "Theirs", [500., 0., 0.], [100., 100., 100.]);
    // Not shared: no restriction.
    ok(&mut model, &mut env, "move", json!({"ids": [theirs], "delta_mm": [1, 0, 0]}));
    env.created.clear();
    env.lease = Some(BTreeSet::from([mine]));
    let refused = run(&mut model, &mut env, "move", json!({"ids": [mine, theirs], "delta_mm": [1, 0, 0]}));
    assert!(!refused.ok && !refused.mutated);
    assert_eq!(refused.result["error"], "not-leased");
    assert_eq!(refused.result["hint"], "select it first so you hold the edit lease");
    let face = first_face(&model, theirs);
    let attempts = [
        ("rename", json!({"id": theirs, "name": "X"})),
        ("erase", json!({"ids": [theirs]})),
        ("tag", json!({"ids": [theirs], "tag": "t"})),
        ("add_joint", json!({"a": mine, "b": theirs, "kind": "half_lap"})),
        ("push_pull", json!({"face_id": face, "distance_mm": 5})),
        ("resize_board", json!({"id": theirs, "size_mm": [10, 10, 10]})),
    ];
    for (name, args) in attempts {
        assert_eq!(err(&mut model, &mut env, name, args), "not-leased", "{name}");
    }
    // Leased entities, their faces, and reads remain available.
    ok(&mut model, &mut env, "move", json!({"ids": [mine], "delta_mm": [1, 0, 0]}));
    let own_face = first_face(&model, mine);
    ok(&mut model, &mut env, "push_pull", json!({"face_id": own_face, "distance_mm": 5}));
    ok(&mut model, &mut env, "get_object", json!({"id": theirs}));
    // New entities are free to create, and then to edit within the same turn.
    let fresh = board(&mut model, &mut env, "Fresh", [900., 0., 0.], [50., 50., 50.]);
    ok(&mut model, &mut env, "move", json!({"ids": [fresh], "delta_mm": [1, 0, 0]}));
    let copy = ok(&mut model, &mut env, "create_copy", json!({"id": theirs, "offset_mm": [0, 500, 0]})).created[0];
    ok(&mut model, &mut env, "rename", json!({"id": copy, "name": "Copy"}));
    let group = ok(&mut model, &mut env, "group", json!({"ids": [fresh, copy], "name": "G", "component": false})).created[0];
    ok(&mut model, &mut env, "rename", json!({"id": group, "name": "G2"}));
    assert_eq!(model.objects[&theirs].name, "Theirs");
}

#[test]
fn model_output_is_bounded() {
    let (mut model, mut env) = (Model::default(), ToolEnv::default());
    let a = board(&mut model, &mut env, "A", [0., 0., 0.], [2000., 100., 100.]);
    let points: Vec<[f64; 3]> = (0..1001).map(|i| [i as f64, 0., 0.]).collect();
    assert!(err(&mut model, &mut env, "draw_polyline", json!({"points_mm": points})).contains("too many points"));
    let ids: Vec<u64> = vec![a; 501];
    assert!(err(&mut model, &mut env, "move", json!({"ids": ids, "delta_mm": [1, 0, 0]})).contains("too many ids"));
    // Scale factor bounds.
    for factor in [0.0, 0.0009, 1001.0, -2.0] {
        let error = err(&mut model, &mut env, "scale", json!({"ids": [a], "center_mm": [0, 0, 0], "factor": factor}));
        assert!(error.contains("factor"), "{factor}: {error}");
    }
    ok(&mut model, &mut env, "scale", json!({"ids": [a], "center_mm": [0, 0, 0], "factor": 1.5}));
    // In range but the result leaves the sane absolute range: restored.
    assert_eq!(
        err(&mut model, &mut env, "scale", json!({"ids": [a], "center_mm": [0, 0, 0], "factor": 1000.0})),
        "result out of range"
    );
    for _ in 0..9 {
        ok(&mut model, &mut env, "move", json!({"ids": [a], "delta_mm": [100_000, 0, 0]}));
    }
    assert_eq!(err(&mut model, &mut env, "move", json!({"ids": [a], "delta_mm": [100_000, 0, 0]})), "result out of range");
    // Inputs: |coordinate| <= 100 m, size >= 0.1 mm, strings <= 500 chars.
    assert!(err(&mut model, &mut env, "create_board", json!({"name": "x", "origin_mm": [100_001, 0, 0], "size_mm": [1, 1, 1]})).contains("out of range"));
    assert!(err(&mut model, &mut env, "create_board", json!({"name": "x", "origin_mm": [0, 0, 0], "size_mm": [0.05, 1, 1]})).contains("out of range"));
    let long = "t".repeat(501);
    assert!(err(&mut model, &mut env, "add_text", json!({"point_mm": [0, 0, 0], "offset_mm": [0, 0, 0], "text": long})).contains("too long"));
    assert!(err(&mut model, &mut env, "rename", json!({"id": a, "name": long})).contains("name"));
    assert!(err(&mut model, &mut env, "tag", json!({"ids": [a], "tag": long})).contains("too long"));
}

#[test]
fn push_pull_requires_the_whole_owner_chain_to_be_editable() {
    let (mut model, mut env) = (Model::default(), ToolEnv::default());
    let a = board(&mut model, &mut env, "A", [0., 0., 0.], [100., 100., 100.]);
    let b = board(&mut model, &mut env, "B", [500., 0., 0.], [100., 100., 100.]);
    let outer = ok(&mut model, &mut env, "group", json!({"ids": [a], "name": "Outer", "component": false})).created[0];
    let inner = *model.objects[&outer].entities.iter().next().unwrap();
    let face = first_face(&model, inner);
    // A locked ancestor, however far up, blocks the edit.
    model.locked.insert(outer);
    assert!(err(&mut model, &mut env, "push_pull", json!({"face_id": face, "distance_mm": 5})).contains("locked"));
    model.locked.clear();
    ok(&mut model, &mut env, "push_pull", json!({"face_id": face, "distance_mm": 5}));
    // A face outside the open editing scope cannot be reached through its owner.
    assert!(model.enter_object(outer));
    let other = first_face(&model, b);
    assert!(err(&mut model, &mut env, "push_pull", json!({"face_id": other, "distance_mm": 5})).contains("scope"));
    ok(&mut model, &mut env, "push_pull", json!({"face_id": face, "distance_mm": 5}));
}

#[test]
fn hidden_selection_is_not_revealed_by_get_scene_or_get_selection() {
    let (mut model, mut env) = (Model::default(), ToolEnv::default());
    let a = board(&mut model, &mut env, "A", [0., 0., 0.], [100., 100., 20.]);
    env.selection = BTreeSet::from([a]);
    let shown = ok(&mut model, &mut env, "get_selection", json!({})).result;
    assert_eq!(shown["selection"], json!([a]));
    assert_eq!(ok(&mut model, &mut env, "get_scene", json!({})).result["selection"], json!([a]));
    env.hide_selection = true;
    let hidden = ok(&mut model, &mut env, "get_selection", json!({})).result;
    assert_eq!(hidden, json!({"ok": true, "selection": [], "note": "The designer has turned off sharing their selection."}));
    let scene = ok(&mut model, &mut env, "get_scene", json!({})).result;
    assert_eq!(scene["selection"], json!([]));
    assert!(scene.get("selection_total").is_none_or(|n| n == 0));
}

#[test]
fn tag_names_follow_the_core_limit_in_the_tool_and_the_schema() {
    let (mut model, mut env) = (Model::default(), ToolEnv::default());
    let a = board(&mut model, &mut env, "A", [0., 0., 0.], [100., 100., 20.]);
    ok(&mut model, &mut env, "tag", json!({"ids": [a], "tag": "x".repeat(128)}));
    for long in ["x".repeat(129), "é".repeat(65)] {
        assert!(err(&mut model, &mut env, "tag", json!({"ids": [a], "tag": long})).contains("too long"));
    }
    let board_with_long_tag = err(&mut model, &mut env, "create_board", json!({
        "name": "B", "origin_mm": [0, 0, 0], "size_mm": [1, 1, 1], "tag": "y".repeat(129)}));
    assert!(board_with_long_tag.contains("too long"));
    let definitions = definitions();
    let property = |tool: &str, name: &str| {
        definitions.iter().find(|d| d.name == tool).unwrap().input_schema["properties"][name].clone()
    };
    assert_eq!(property("tag", "tag")["maxLength"], 128);
    assert_eq!(property("create_board", "tag")["maxLength"], 128);
}

#[test]
fn the_scale_schema_states_the_range_the_executor_enforces() {
    let definitions = definitions();
    let scale = definitions.iter().find(|d| d.name == "scale").unwrap();
    let factor = &scale.input_schema["properties"]["factor"];
    assert_eq!(factor["minimum"], MIN_SCALE);
    assert_eq!(factor["maximum"], MAX_SCALE);
    assert!(factor.get("exclusiveMinimum").is_none());
    let (mut model, mut env) = (Model::default(), ToolEnv::default());
    let a = board(&mut model, &mut env, "A", [0., 0., 0.], [100., 100., 20.]);
    ok(&mut model, &mut env, "scale", json!({"ids": [a], "center_mm": [0, 0, 0], "factor": MIN_SCALE}));
    assert!(err(&mut model, &mut env, "scale", json!({"ids": [a], "center_mm": [0, 0, 0], "factor": MAX_SCALE * 2.})).contains("out of range"));
}

#[test]
fn select_is_limited_to_the_lease_in_shared_projects() {
    let (mut model, mut env) = (Model::default(), ToolEnv::default());
    let mine = board(&mut model, &mut env, "Mine", [0., 0., 0.], [100., 100., 100.]);
    let theirs = board(&mut model, &mut env, "Theirs", [500., 0., 0.], [100., 100., 100.]);
    // Not shared: unchanged behaviour.
    ok(&mut model, &mut env, "select", json!({"ids": [theirs]}));
    env.created.clear();
    env.lease = Some(BTreeSet::from([mine]));
    env.selection.clear();
    let message = err(&mut model, &mut env, "select", json!({"ids": [mine, theirs]}));
    assert!(message.contains("lease"), "{message}");
    assert!(env.selection.is_empty(), "a refused select changes nothing");
    ok(&mut model, &mut env, "select", json!({"ids": [mine]}));
    assert_eq!(env.selection, BTreeSet::from([mine]));
    let fresh = board(&mut model, &mut env, "Fresh", [900., 0., 0.], [50., 50., 50.]);
    ok(&mut model, &mut env, "select", json!({"ids": [fresh]}));
}

#[test]
fn openai_and_mcp_adapters_keep_the_wire_shapes() {
    let (openai, mcp, neutral) = (openai_tools(), mcp_tools(), definitions());
    assert_eq!(openai.len(), neutral.len());
    assert_eq!(mcp.len(), neutral.len());
    for (i, d) in neutral.iter().enumerate() {
        assert_eq!(openai[i]["type"], "function");
        assert_eq!(openai[i]["function"]["name"], d.name.as_str());
        assert_eq!(openai[i]["function"]["description"], d.description.as_str());
        assert_eq!(openai[i]["function"]["parameters"], d.input_schema);
        assert_eq!(openai[i].as_object().unwrap().len(), 2);
        assert_eq!(openai[i]["function"].as_object().unwrap().len(), 3);
        assert_eq!(mcp[i]["name"], d.name.as_str());
        assert_eq!(mcp[i]["inputSchema"], d.input_schema);
        assert_eq!(mcp[i].as_object().unwrap().len(), 3);
    }
}

#[test]
fn text_arguments_behave_like_the_app_did() {
    let (mut model, mut env) = (Model::default(), ToolEnv::default());
    // Empty text means no arguments.
    assert!(execute(&mut model, &Call::from_text("1", "get_selection", "  "), &mut env).ok);
    // Valid JSON text equals a JSON value.
    let a = Call::from_text("1", "get_object", "{\"id\": 5}");
    assert_eq!(a.arguments, json!({"id": 5}));
    // Not JSON: structured error, no panic, model untouched.
    let bad = execute(&mut model, &Call::from_text("1", "get_object", "{oops"), &mut env);
    assert!(!bad.ok);
    assert!(bad.result["error"].as_str().unwrap().starts_with("invalid arguments: "));
    assert!(bad.result["hint"].is_string());
}

#[test]
fn lease_blocks_entering_and_creating_inside_foreign_groups() {
    let (mut model, mut env) = (Model::default(), ToolEnv::default());
    let mine = board(&mut model, &mut env, "Mine", [0., 0., 0.], [100., 100., 100.]);
    let theirs = board(&mut model, &mut env, "Theirs", [500., 0., 0.], [100., 100., 100.]);
    let theirs_group = ok(&mut model, &mut env, "group", json!({"ids": [theirs], "name": "TG", "component": false})).created[0];
    let my_group = ok(&mut model, &mut env, "group", json!({"ids": [mine], "name": "MG", "component": false})).created[0];
    env.created.clear();
    env.lease = Some(BTreeSet::from([my_group]));

    assert_eq!(err(&mut model, &mut env, "enter_object", json!({"id": theirs_group})), "not-leased");
    assert_eq!(model.edit_context, None);

    // Scoped inside a group someone else holds: nothing may be created.
    model.edit_context = Some(theirs_group);
    for (name, args) in [
        ("create_board", json!({"name": "X", "origin_mm": [0, 0, 0], "size_mm": [10, 10, 10]})),
        ("draw_polyline", json!({"points_mm": [[0, 0, 0], [10, 0, 0]]})),
        ("add_text", json!({"text": "hi", "position_mm": [0, 0, 0]})),
    ] {
        assert_eq!(err(&mut model, &mut env, name, args), "not-leased", "{name}");
    }
    model.edit_context = None;

    // The leased group can be entered and created in.
    ok(&mut model, &mut env, "enter_object", json!({"id": my_group}));
    ok(&mut model, &mut env, "create_board", json!({"name": "In", "origin_mm": [0, 0, 0], "size_mm": [10, 10, 10]}));
    ok(&mut model, &mut env, "exit_to_root", json!({}));
    // Root creation still works.
    board(&mut model, &mut env, "Fresh", [900., 0., 0.], [50., 50., 50.]);
}

#[test]
fn lease_rolls_back_calls_that_alter_foreign_loose_geometry() {
    let (mut model, mut env) = (Model::default(), ToolEnv::default());
    ok(&mut model, &mut env, "draw_polyline", json!({"points_mm": [[0, 0, 0], [1000, 0, 0]]}));
    let mine = board(&mut model, &mut env, "Mine", [0., 500., 0.], [100., 100., 100.]);
    env.created.clear();
    let edges_before = model.edges.len();

    // Without a lease the crossing line splits the loose edge.
    let mut free = model.clone();
    ok(&mut free, &mut ToolEnv::default(), "draw_polyline", json!({"points_mm": [[500, 0, 0], [500, 300, 0]]}));
    assert!(free.edges.len() > edges_before + 1, "crossing should split the loose edge");

    env.lease = Some(BTreeSet::from([mine]));
    assert_eq!(
        err(&mut model, &mut env, "draw_polyline", json!({"points_mm": [[500, 0, 0], [500, 300, 0]]})),
        "not-leased"
    );
    assert_eq!(model.edges.len(), edges_before);
    // Geometry that touches nothing existing is fine.
    ok(&mut model, &mut env, "draw_polyline", json!({"points_mm": [[0, 5000, 0], [100, 5000, 0]]}));
}

/// A side and a shelf entering it by 9 mm, joined with a tongue and groove.
fn joined_pair(model: &mut Model, env: &mut ToolEnv) -> (u64, u64, u64) {
    let side = board(model, env, "Side", [0., 0., 0.], [18., 300., 900.]);
    let shelf = board(model, env, "Shelf", [9., 0., 400.], [500., 300., 18.]);
    let joint = ok(model, env, "add_joint", json!({"a": side, "b": shelf, "kind": "tongue_groove"})).joints[0];
    (side, shelf, joint)
}

#[test]
fn erasing_a_board_removes_its_joints() {
    let (mut model, mut env) = (Model::default(), ToolEnv::default());
    let (side, shelf, joint) = joined_pair(&mut model, &mut env);
    let other = board(&mut model, &mut env, "Other", [0., 500., 0.], [18., 300., 900.]);
    let second = board(&mut model, &mut env, "Second", [9., 500., 400.], [500., 300., 18.]);
    let kept = ok(&mut model, &mut env, "add_joint", json!({"a": other, "b": second, "kind": "tongue_groove"})).joints[0];
    let outcome = ok(&mut model, &mut env, "erase", json!({"ids": [shelf]}));
    assert_eq!(outcome.result["removed_joints"], json!([joint]));
    assert!(!model.connections.contains_key(&joint));
    assert!(model.connections.contains_key(&kept), "joints of other boards stay");
    assert!(model.contains_entity(&side));
    // Erasing a group removes the joints of the boards inside it.
    let group = ok(&mut model, &mut env, "group", json!({"ids": [other, second], "name": "Pair", "component": false})).created[0];
    assert_eq!(model.connections.len(), 1);
    let outcome = ok(&mut model, &mut env, "erase", json!({"ids": [group]}));
    assert_eq!(outcome.result["removed_joints"].as_array().unwrap().len(), 1);
    assert!(model.connections.is_empty());
}

#[test]
fn erasing_a_component_board_reports_removed_peer_joints() {
    let (mut model, mut env) = (Model::default(), ToolEnv::default());
    let (side, shelf, _) = joined_pair(&mut model, &mut env);
    let first = model.make_object(&BTreeSet::from([side, shelf]), true, "Assembly").unwrap();
    let copy = model.copy_entities(&BTreeSet::from([first]));
    model.paste_entities(&copy, DVec3::Z * 2.);
    assert!(model.enter_object(first));
    let shelf = model.objects[&first].entities.iter().copied()
        .find(|id| model.objects.get(id).is_some_and(|object| object.name == "Shelf"))
        .unwrap();
    let expected: Vec<_> = model.connections.keys().copied().collect();
    assert_eq!(expected.len(), 2);
    let outcome = ok(&mut model, &mut env, "erase", json!({"ids": [shelf]}));
    assert_eq!(outcome.result["removed_joints"], json!(expected));
    assert!(model.connections.is_empty());
}

#[test]
fn joints_are_removed_with_remove_joint_not_erase() {
    let (mut model, mut env) = (Model::default(), ToolEnv::default());
    let (side, shelf, joint) = joined_pair(&mut model, &mut env);
    let message = err(&mut model, &mut env, "erase", json!({"ids": [joint]}));
    assert!(message.contains("is a joint"), "{message}");
    let hint = run(&mut model, &mut env, "erase", json!({"ids": [joint]})).result["hint"].clone();
    assert!(hint.as_str().unwrap().contains("remove_joint"));
    assert!(err(&mut model, &mut env, "remove_joint", json!({"ids": [side]})).contains("not a joint"));
    let outcome = ok(&mut model, &mut env, "remove_joint", json!({"ids": [joint]}));
    assert!(outcome.mutated, "removing a joint is an undoable change");
    assert_eq!(outcome.result["removed_joints"], json!([joint]));
    assert!(model.connections.is_empty());
    assert!(model.contains_entity(&side) && model.contains_entity(&shelf), "the boards stay");
}

#[test]
fn the_scene_flags_joints_whose_board_is_gone() {
    let (mut model, mut env) = (Model::default(), ToolEnv::default());
    let (_, shelf, joint) = joined_pair(&mut model, &mut env);
    // Projects saved before the core removed joints with their boards
    // (BenchUp3D/editor#36) can still contain such a joint.
    let saved = model.connections[&joint].clone();
    assert_eq!(model.erase(&BTreeSet::from([shelf])), vec![joint], "the core removes the joint with its board");
    assert!(model.connections.is_empty());
    model.connections.insert(joint, saved);
    let scene = ok(&mut model, &mut env, "get_scene", json!({})).result;
    assert_eq!(scene["joints"][0]["missing_boards"], json!([shelf]));
    // Such a joint can still be removed.
    ok(&mut model, &mut env, "remove_joint", json!({"ids": [joint]}));
    assert!(model.connections.is_empty());
}

#[test]
fn scope_errors_name_every_object_to_open() {
    let (mut model, mut env) = (Model::default(), ToolEnv::default());
    let a = board(&mut model, &mut env, "A", [0., 0., 0.], [100., 100., 20.]);
    let b = board(&mut model, &mut env, "B", [200., 0., 0.], [100., 100., 20.]);
    let inner = ok(&mut model, &mut env, "group", json!({"ids": [a, b], "name": "Inner", "component": false})).created[0];
    let other = board(&mut model, &mut env, "C", [0., 300., 0.], [100., 100., 20.]);
    let outer = ok(&mut model, &mut env, "group", json!({"ids": [inner, other], "name": "Outer", "component": false})).created[0];
    let inner = model.objects[&outer].entities.iter().copied().find(|id| model.objects.get(id).is_some_and(|o| o.name == "Inner")).unwrap();
    let part = *model.objects[&inner].entities.iter().find(|id| model.objects.contains_key(id)).unwrap();
    // Mixed with a top-level board, the nested part is not opened automatically.
    let loose = board(&mut model, &mut env, "Loose", [0., 900., 0.], [10., 10., 10.]);
    let hint = |model: &mut Model, env: &mut ToolEnv| {
        run(model, env, "move", json!({"ids": [part, loose], "delta_mm": [1, 0, 0]})).result["hint"].as_str().unwrap().to_owned()
    };
    let from_root = hint(&mut model, &mut env);
    assert!(from_root.contains(&format!("enter_object{{id:{outer}}}, then enter_object{{id:{inner}}}")), "{from_root}");
    ok(&mut model, &mut env, "enter_object", json!({"id": outer}));
    let from_outer = hint(&mut model, &mut env);
    assert!(from_outer.contains(&format!("call enter_object{{id:{inner}}} first")), "{from_outer}");
    assert!(!from_outer.contains("exit_to_root"), "{from_outer}");
    // From an unrelated group, leave it first.
    ok(&mut model, &mut env, "exit_to_root", json!({}));
    let lone = board(&mut model, &mut env, "D", [0., 600., 0.], [100., 100., 20.]);
    let side_group = ok(&mut model, &mut env, "group", json!({"ids": [lone], "name": "Side", "component": false})).created[0];
    ok(&mut model, &mut env, "enter_object", json!({"id": side_group}));
    assert!(hint(&mut model, &mut env).contains("exit_to_root, then enter_object"));
}

#[test]
fn find_objects_lists_everything_without_a_query() {
    let (mut model, mut env) = (Model::default(), ToolEnv::default());
    board(&mut model, &mut env, "Side", [0., 0., 0.], [18., 300., 900.]);
    board(&mut model, &mut env, "Shelf", [18., 0., 400.], [500., 300., 18.]);
    assert_eq!(ok(&mut model, &mut env, "find_objects", json!({})).result["total"], 2);
    assert_eq!(ok(&mut model, &mut env, "find_objects", json!({"query": "shel"})).result["total"], 1);
    assert_eq!(ok(&mut model, &mut env, "find_objects", json!({"query": "group"})).result["total"], 2);
}

fn axes_of(outcome: &ToolOutcome) -> (u64, u64) {
    let o = &outcome.result["orientation"];
    (o["depth_axis"].as_u64().unwrap(), o["width_axis"].as_u64().unwrap())
}

#[test]
fn joints_pick_the_woodworking_orientation_and_report_status() {
    let (mut model, mut env) = (Model::default(), ToolEnv::default());
    let side = board(&mut model, &mut env, "Side", [0., 0., 0.], [18., 300., 900.]);
    let shelf = board(&mut model, &mut env, "Shelf", [9., 0., 400.], [500., 300., 18.]);
    // The tongue runs across the shelf's thickness, along its whole depth.
    let joint = ok(&mut model, &mut env, "add_joint", json!({"a": shelf, "b": side, "kind": "tongue_groove"}));
    assert_eq!(axes_of(&joint), (0, 2));
    let detail = ok(&mut model, &mut env, "get_object", json!({"id": joint.joints[0]})).result;
    assert_eq!(detail["status"], "ok");
    assert_eq!(detail["params"]["width_axis_name"], "z");
    assert!(detail["roles"]["a"].as_str().unwrap().contains("tenon"));
    assert!(!detail["supported_orientations"].as_array().unwrap().is_empty());
    // A drawer side enters the front along y; its tails spread over its height.
    let front = board(&mut model, &mut env, "Front", [1000., 0., 0.], [460., 20., 216.]);
    let drawer_side = board(&mut model, &mut env, "Drawer side", [1002., 6., 8.], [12., 420., 200.]);
    let dovetail = ok(&mut model, &mut env, "add_joint", json!({"a": drawer_side, "b": front, "kind": "dovetail", "params": {"dovetails": 4}}));
    assert_eq!(axes_of(&dovetail), (1, 2));
    // Clear errors instead of silent clamping or a later failure.
    let leg = board(&mut model, &mut env, "Leg", [2000., 0., 0.], [70., 70., 720.]);
    let apron = board(&mut model, &mut env, "Apron", [2040., 24., 620.], [600., 22., 100.]);
    assert!(err(&mut model, &mut env, "add_joint", json!({"a": apron, "b": leg, "kind": "mortise_tenon", "params": {"width_ratio": 0.95}})).contains("0.1 and 0.9"));
    assert!(err(&mut model, &mut env, "add_joint", json!({"a": apron, "b": leg, "kind": "finger", "params": {"fingers": 40}})).contains("between 2 and 16"));
    assert!(err(&mut model, &mut env, "add_joint", json!({"a": apron, "b": leg, "kind": "mortise_tenon", "params": {"depth_axis": 1, "width_axis": 1}})).contains("must differ"));
    let apart = board(&mut model, &mut env, "Apart", [5000., 0., 0.], [100., 100., 100.]);
    assert!(err(&mut model, &mut env, "add_joint", json!({"a": apart, "b": leg, "kind": "half_lap"})).contains("do not overlap"));
}

#[test]
fn stock_materials_can_be_created_and_assigned() {
    let (mut model, mut env) = (Model::default(), ToolEnv::default());
    let listed = ok(&mut model, &mut env, "list_stock_materials", json!({})).result;
    assert!(listed["note"].as_str().unwrap().contains("create_stock_material"));
    let created = ok(&mut model, &mut env, "create_stock_material", json!({"name": "Oak 19", "thickness_mm": 19, "width_mm": 1220, "length_mm": 2440}));
    assert!(created.mutated, "a new material is saved with the project");
    let id = created.result["material_id"].as_str().unwrap().to_owned();
    assert_eq!(model.stock_materials.values().next().unwrap().dimensions, [190, 12200, 24400]);
    let again = ok(&mut model, &mut env, "create_stock_material", json!({"name": "Oak 19", "thickness_mm": 19, "width_mm": 1220, "length_mm": 2440}));
    assert_eq!(again.result["material_id"], id.as_str(), "the same material is reused");
    let shelf = board(&mut model, &mut env, "Shelf", [0., 0., 0.], [500., 300., 19.]);
    ok(&mut model, &mut env, "set_material", json!({"ids": [shelf], "material_id": id}));
    assert!(model.objects[&shelf].stock_material_id.is_some());
    let cut = ok(&mut model, &mut env, "get_cut_list", json!({})).result;
    assert_eq!(cut["pieces"][0]["material"], "Oak 19");
    assert_eq!(cut["pieces"][0]["length_mm"], 500.0);
    assert_eq!(cut["materials"][0]["minimum_stock_pieces"], 1);
}

#[test]
fn shaped_boards_from_an_outline_report_outward_faces() {
    let (mut model, mut env) = (Model::default(), ToolEnv::default());
    // An L-shaped part: concave, so "away from the centre" is not outward everywhere.
    let drawn = ok(&mut model, &mut env, "draw_polyline", json!({
        "points_mm": [[0, 0, 0], [400, 0, 0], [400, 100, 0], [100, 100, 0], [100, 400, 0], [0, 400, 0]], "close": true}));
    let face = drawn.result["face_id"].as_u64().unwrap();
    let pushed = ok(&mut model, &mut env, "push_pull", json!({"face_id": face, "distance_mm": 18}));
    assert!(!pushed.created.is_empty(), "push_pull lists what it created");
    let mut ids: Vec<u64> = drawn.created.clone();
    ids.extend(pushed.created.iter().copied().filter(|id| model.contains_entity(id)));
    let part = ok(&mut model, &mut env, "group", json!({"ids": ids, "name": "L", "component": false})).created[0];
    let scene = ok(&mut model, &mut env, "get_scene", json!({})).result;
    assert_eq!(scene["loose_geometry"]["edges"], 0, "faces bring their edges into the group");
    let detail = ok(&mut model, &mut env, "get_object", json!({"id": part})).result;
    // The inner corner faces point into the notch, not away from the part's centre.
    let facing = |x: f64, y: f64| -> String {
        detail["faces"].as_array().unwrap().iter()
            .find(|f| (f["center_mm"][0].as_f64().unwrap() - x).abs() < 1. && (f["center_mm"][1].as_f64().unwrap() - y).abs() < 1.)
            .map(|f| f["facing"].as_str().unwrap().to_owned()).unwrap()
    };
    assert_eq!(facing(250., 100.), "+y");
    assert_eq!(facing(100., 250.), "+x");
    assert_eq!(facing(200., 0.), "-y");
    assert!(detail["faces"].as_array().unwrap().iter().all(|f| f["center_mm"].is_array()), "concave faces get a center too");
    let check = ok(&mut model, &mut env, "check_model", json!({})).result;
    assert!(check["issues"].as_array().unwrap().iter().all(|i| i["kind"] != "open_solid"), "{check}");
}

#[test]
fn resize_anchor_and_move_to() {
    let (mut model, mut env) = (Model::default(), ToolEnv::default());
    let divider = board(&mut model, &mut env, "Divider", [491., 0., 0.], [18., 300., 1800.]);
    ok(&mut model, &mut env, "resize_board", json!({"id": divider, "size_mm": [19, 300, 1800], "anchor": "center"}));
    near(bounds(&model, divider).0, [0.4905, 0., 0.]);
    ok(&mut model, &mut env, "resize_board", json!({"id": divider, "size_mm": [19, 294, 1800], "anchor": ["min", "max", "min"]}));
    near(bounds(&model, divider).0, [0.4905, 0.006, 0.]);
    ok(&mut model, &mut env, "move_to", json!({"id": divider, "origin_mm": [0, 0, 100]}));
    near(bounds(&model, divider).0, [0., 0., 0.1]);
}

#[test]
fn check_model_finds_unjoined_overlaps() {
    let (mut model, mut env) = (Model::default(), ToolEnv::default());
    let side = board(&mut model, &mut env, "Side", [0., 0., 0.], [18., 300., 900.]);
    let shelf = board(&mut model, &mut env, "Shelf", [9., 0., 400.], [500., 300., 18.]);
    let check = ok(&mut model, &mut env, "check_model", json!({})).result;
    assert_eq!(check["warnings"], 1);
    assert_eq!(check["issues"][0]["kind"], "overlap_without_joint");
    ok(&mut model, &mut env, "add_joint", json!({"a": shelf, "b": side, "kind": "tongue_groove"}));
    let check = ok(&mut model, &mut env, "check_model", json!({})).result;
    assert_eq!(check["warnings"], 0, "{check}");
    assert_eq!(check["errors"], 0, "{check}");
}

#[test]
fn the_scene_lists_annotations_axes_and_component_definitions() {
    let (mut model, mut env) = (Model::default(), ToolEnv::default());
    let a = board(&mut model, &mut env, "Shelf", [0., 0., 0.], [500., 300., 18.]);
    let component = ok(&mut model, &mut env, "group", json!({"ids": [a], "name": "Shelf", "component": true})).created[0];
    let copy = ok(&mut model, &mut env, "create_copy", json!({"id": component, "offset_mm": [0, 0, 300]})).created[0];
    ok(&mut model, &mut env, "add_dimension", json!({"start_mm": [0, 0, 0], "end_mm": [500, 0, 0], "offset_mm": [0, -100, 0]}));
    ok(&mut model, &mut env, "add_text", json!({"point_mm": [0, 0, 18], "offset_mm": [0, 0, 0], "text": "Bookcase"}));
    let scene = ok(&mut model, &mut env, "get_scene", json!({})).result;
    assert!(scene["axes"]["y"].as_str().unwrap().contains("-y"));
    assert_eq!(scene["dimensions"][0]["length_mm"], 500.0);
    assert_eq!(scene["texts"][0]["text"], "Bookcase");
    let definition = |id: u64| scene["objects"].as_array().unwrap().iter().find(|o| o["id"] == id).unwrap()["definition"].clone();
    assert!(definition(component).is_string());
    assert_eq!(definition(component), definition(copy));
    let guide = ok(&mut model, &mut env, "get_guide", json!({})).result;
    assert!(guide["guide"].as_str().unwrap().contains("front of a piece faces -Y"));
}

#[test]
fn check_model_flags_reversed_roles_and_trusts_real_geometry() {
    let (mut model, mut env) = (Model::default(), ToolEnv::default());
    let side = board(&mut model, &mut env, "Side", [0., 0., 0.], [18., 300., 900.]);
    let shelf = board(&mut model, &mut env, "Shelf", [9., 0., 400.], [500., 300., 18.]);
    // Side as a: the side would carry the tongue.
    ok(&mut model, &mut env, "add_joint", json!({"a": side, "b": shelf, "kind": "tongue_groove"}));
    let check = ok(&mut model, &mut env, "check_model", json!({})).result;
    assert_eq!(check["issues"][0]["kind"], "joint_roles_reversed", "{check}");
    // A notched board around a post: the boxes overlap, the solids do not.
    let (mut model, mut env) = (Model::default(), ToolEnv::default());
    board(&mut model, &mut env, "Post", [0., 0., 0.], [50., 50., 800.]);
    let outline = ok(&mut model, &mut env, "draw_polyline", json!({
        "points_mm": [[50, 0, 700], [400, 0, 700], [400, 300, 700], [0, 300, 700], [0, 50, 700], [50, 50, 700]], "close": true}));
    let face = outline.result["face_id"].as_u64().unwrap();
    let pushed = ok(&mut model, &mut env, "push_pull", json!({"face_id": face, "distance_mm": 30}));
    assert_eq!(pushed.result["face_id"], pushed.result["new_face_id"]);
    let mut ids = outline.created.clone();
    ids.extend(pushed.created.iter().copied().filter(|id| model.contains_entity(id)));
    ok(&mut model, &mut env, "group", json!({"ids": ids, "name": "Top", "component": false}));
    let check = ok(&mut model, &mut env, "check_model", json!({})).result;
    assert_eq!(check["warnings"], 0, "{check}");
    assert_eq!(check["model_bounds_mm"]["size"], json!([400.0, 300.0, 800.0]));
}

#[test]
fn joints_explain_level_and_assembly_mistakes() {
    let (mut model, mut env) = (Model::default(), ToolEnv::default());
    let side = board(&mut model, &mut env, "Side", [0., 0., 0.], [18., 300., 900.]);
    let shelf = board(&mut model, &mut env, "Shelf", [9., 0., 400.], [500., 300., 18.]);
    let module = ok(&mut model, &mut env, "group", json!({"ids": [shelf], "name": "Module", "component": false})).created[0];
    let inner = model.objects[&module].entities.iter().copied().find(|id| model.objects.contains_key(id)).unwrap();
    assert!(err(&mut model, &mut env, "add_joint", json!({"a": inner, "b": side, "kind": "tongue_groove"})).contains("different groups"));
    assert!(err(&mut model, &mut env, "add_joint", json!({"a": module, "b": side, "kind": "tongue_groove"})).contains("not a single board"));
}

#[test]
fn cut_list_uses_materials_set_on_groups() {
    let (mut model, mut env) = (Model::default(), ToolEnv::default());
    let shelf = board(&mut model, &mut env, "Shelf", [0., 0., 0.], [500., 300., 18.]);
    let unassigned = ok(&mut model, &mut env, "get_cut_list", json!({})).result;
    assert!(unassigned["materials"][0]["note"].as_str().unwrap().contains("create_stock_material"));
    let id = ok(&mut model, &mut env, "create_stock_material", json!({"name": "Birch 18", "thickness_mm": 18, "width_mm": 1220, "length_mm": 2440})).result["material_id"].clone();
    let group = ok(&mut model, &mut env, "group", json!({"ids": [shelf], "name": "Unit", "component": false})).created[0];
    ok(&mut model, &mut env, "set_material", json!({"ids": [group], "material_id": id}));
    let cut = ok(&mut model, &mut env, "get_cut_list", json!({})).result;
    assert_eq!(cut["pieces"][0]["material"], "Birch 18");
    let check = ok(&mut model, &mut env, "check_model", json!({})).result;
    assert!(check["issues"].as_array().unwrap().iter().all(|i| i["kind"] != "no_material"), "{check}");
}

#[test]
fn same_named_materials_are_told_apart_by_assignment() {
    let (mut model, mut env) = (Model::default(), ToolEnv::default());
    let material = |model: &mut Model, env: &mut ToolEnv, thickness: u64, width: u64| {
        ok(model, env, "create_stock_material", json!({"name": "Plywood", "thickness_mm": thickness, "width_mm": width, "length_mm": 2440}))
            .result["material_id"].clone()
    };
    let thick = material(&mut model, &mut env, 18, 1220);
    let thin = material(&mut model, &mut env, 12, 610);
    assert_ne!(thick, thin);
    let top = board(&mut model, &mut env, "Top", [0., 0., 500.], [600., 300., 18.]);
    let back = board(&mut model, &mut env, "Back", [0., 400., 0.], [600., 12., 480.]);
    ok(&mut model, &mut env, "set_material", json!({"ids": [top], "material_id": thick}));
    ok(&mut model, &mut env, "set_material", json!({"ids": [back], "material_id": thin}));
    let check = ok(&mut model, &mut env, "check_model", json!({})).result;
    assert!(check["issues"].as_array().unwrap().iter().all(|i| i["kind"] != "thickness_mismatch"), "{check}");
    let cut = ok(&mut model, &mut env, "get_cut_list", json!({})).result;
    let materials = cut["materials"].as_array().unwrap();
    assert_eq!(materials.len(), 2, "{cut}");
    let mut stock: Vec<_> = materials.iter().map(|m| (m["material"].clone(), m["stock_mm"]["thickness"].clone(), m["stock_mm"]["width"].clone())).collect();
    stock.sort_by_key(|(_, t, _)| t.as_f64().unwrap() as i64);
    assert_eq!(stock, [(json!("Plywood"), json!(12.0), json!(610.0)), (json!("Plywood"), json!(18.0), json!(1220.0))]);
}

#[test]
fn existing_pocket_joints_can_be_inspected_estimated_and_flipped() {
    for (kind, name, discrete, minutes) in [
        (JointKind::Biscuit, "biscuit", true, 5.),
        (JointKind::Domino, "domino", true, 5.),
        (JointKind::Groove, "groove", false, 5.),
        (JointKind::LooseTongue, "loose_tongue", false, 8.),
    ] {
        let (mut model, mut env) = (Model::default(), ToolEnv::default());
        let a = board(&mut model, &mut env, "A", [0., 0., 0.], [300., 100., 40.]);
        let b = board(&mut model, &mut env, "B", [250., 0., 0.], [300., 100., 40.]);
        let settings = JointSettings {
            kind, pocket_width: 0.035, pocket_thickness: 0.006,
            pocket_depth: 0.012, pocket_count: 2, ..Default::default()
        };
        let joint = model.add_connection(Connection {
            members: [a, b], entry_face: None,
            operation: ConnectionOperation::Joinery(settings.clone()),
        }).unwrap();
        let scene = ok(&mut model, &mut env, "get_scene", json!({})).result;
        assert_eq!(scene["joints"][0]["kind"], name);
        for flipped in [false, true] {
            if flipped {
                ok(&mut model, &mut env, "flip_joint", json!({"ids": [joint]}));
                assert_eq!(model.connections[&joint].members, [b, a]);
                assert_eq!(model.connections[&joint].operation, ConnectionOperation::Joinery(settings.clone()));
            }
            let detail = ok(&mut model, &mut env, "get_object", json!({"id": joint})).result;
            assert_eq!(detail["kind"], name);
            assert_eq!(detail["status"], "ok", "{detail}");
            assert!(detail.get("geometry").is_none(), "pocket cuts must not report traditional profile bounds: {detail}");
            assert_eq!(detail["params"]["width_axis_name"], "y");
            assert_eq!(detail["params"]["pocket_thickness_mm"], 6.0);
            assert_eq!(detail["params"]["pocket_depth_mm"], 12.0);
            if discrete {
                assert_eq!(detail["params"]["pocket_width_mm"], 35.0);
                assert_eq!(detail["params"]["pocket_count"], 2);
            } else {
                assert!(detail["params"].get("pocket_width_mm").is_none());
                assert!(detail["params"].get("pocket_count").is_none());
            }
            assert!(detail["params"].get("width_ratio").is_none());
            let estimate = ok(&mut model, &mut env, "get_estimate", json!({})).result;
            assert_eq!(estimate["joinery"], json!([{"kind": name, "count": 1, "work_minutes": minutes}]));
            assert_eq!(estimate["total"]["by_category"]["joinery_work_minutes"], minutes);
            assert_eq!(estimate["total"]["work_minutes"], minutes);
            let scoped = ok(&mut model, &mut env, "get_estimate", json!({"ids": [a, b]})).result;
            assert_eq!(scoped["joinery"], estimate["joinery"]);
            let one = ok(&mut model, &mut env, "get_estimate", json!({"ids": [a]})).result;
            assert_eq!(one["joinery"], json!([]));
            assert_eq!(one["total"]["by_category"]["joinery_work_minutes"], 0.);
        }
    }
}

#[test]
fn flip_joint_swaps_roles_and_keeps_settings() {
    let (mut model, mut env) = (Model::default(), ToolEnv::default());
    let side = board(&mut model, &mut env, "Side", [0., 0., 0.], [18., 300., 900.]);
    let shelf = board(&mut model, &mut env, "Shelf", [9., 0., 400.], [500., 300., 18.]);
    let joint = ok(&mut model, &mut env, "add_joint", json!({"a": side, "b": shelf, "kind": "tongue_groove", "params": {"width_ratio": 0.4}})).joints[0];
    let flipped = ok(&mut model, &mut env, "flip_joint", json!({"ids": [joint]}));
    assert!(flipped.mutated);
    assert_eq!(model.connections[&joint].members, [shelf, side]);
    let detail = ok(&mut model, &mut env, "get_object", json!({"id": joint})).result;
    assert_eq!(detail["params"]["width_ratio"], 0.4, "settings are kept");
    assert_eq!(detail["params"]["width_axis_name"], "z", "the orientation is re-picked for the new a");
    assert!(detail["params"].get("fingers").is_none(), "only this kind's settings");
    let check = ok(&mut model, &mut env, "check_model", json!({})).result;
    assert_eq!(check["warnings"], 0, "{check}");
    assert_eq!(check["joints"]["ok"], 1);
}

#[test]
fn editing_tools_reach_parts_inside_groups() {
    let (mut model, mut env) = (Model::default(), ToolEnv::default());
    let a = board(&mut model, &mut env, "A", [0., 0., 0.], [100., 100., 20.]);
    let b = board(&mut model, &mut env, "B", [200., 0., 0.], [100., 100., 20.]);
    let group = ok(&mut model, &mut env, "group", json!({"ids": [a, b], "name": "Pair", "component": false})).created[0];
    let inner: Vec<u64> = model.objects[&group].entities.iter().copied().filter(|id| model.objects.contains_key(id)).collect();
    ok(&mut model, &mut env, "move", json!({"ids": [inner[0]], "delta_mm": [0, 0, 50]}));
    near(bounds(&model, inner[0]).0, [bounds(&model, inner[0]).0.x, 0., 0.05]);
    ok(&mut model, &mut env, "resize_board", json!({"id": inner[1], "size_mm": [100, 100, 30]}));
    assert_eq!(model.edit_context, None);
    assert_eq!(model.parent(inner[0]), Some(group), "the part stays in its group");
}

#[test]
fn check_model_reports_boards_passing_through_and_thickness() {
    let (mut model, mut env) = (Model::default(), ToolEnv::default());
    board(&mut model, &mut env, "Divider", [491., 0., 0.], [18., 300., 1800.]);
    let shelf = board(&mut model, &mut env, "Long shelf", [9., 0., 1000.], [1200., 300., 18.]);
    let id = ok(&mut model, &mut env, "create_stock_material", json!({"name": "Board 16", "thickness_mm": 16, "width_mm": 1220, "length_mm": 2440})).result["material_id"].clone();
    ok(&mut model, &mut env, "set_material", json!({"ids": [shelf], "material_id": id}));
    let check = ok(&mut model, &mut env, "check_model", json!({})).result;
    let kinds: Vec<&str> = check["issues"].as_array().unwrap().iter().map(|i| i["kind"].as_str().unwrap()).collect();
    assert!(kinds.contains(&"passes_through"), "{check}");
    assert!(kinds.contains(&"thickness_mismatch"), "{check}");
    assert_eq!(check["errors"], 1);
}

#[test]
fn dimensions_measure_along_an_axis_and_report_attachment() {
    let (mut model, mut env) = (Model::default(), ToolEnv::default());
    board(&mut model, &mut env, "Side", [0., 0., 0.], [18., 300., 900.]);
    let straight = ok(&mut model, &mut env, "add_dimension", json!({"start_mm": [0, 0, 0], "end_mm": [300, 0, 900], "offset_mm": [-100, 0, 0]}));
    assert!(straight.result["length_mm"].as_f64().unwrap() > 900.);
    assert_eq!(straight.result["start_attached"], true, "a corner of the board");
    let along = ok(&mut model, &mut env, "add_dimension", json!({"start_mm": [0, 0, 0], "end_mm": [300, 0, 900], "offset_mm": [-150, 0, 0], "axis": "z"}));
    assert_eq!(along.result["length_mm"], 900.0);
    let free = ok(&mut model, &mut env, "add_dimension", json!({"start_mm": [-50, -50, 0], "end_mm": [-50, -50, 100], "offset_mm": [-10, 0, 0]}));
    assert_eq!(free.result["start_attached"], false, "no snapping to nearby corners");
    let scene = ok(&mut model, &mut env, "get_scene", json!({})).result;
    assert_eq!(scene["dimensions"][1]["axis"], "z");
}

#[test]
fn the_scene_shows_materials_inherited_from_groups() {
    let (mut model, mut env) = (Model::default(), ToolEnv::default());
    let shelf = board(&mut model, &mut env, "Shelf", [0., 0., 0.], [500., 300., 18.]);
    let id = ok(&mut model, &mut env, "create_stock_material", json!({"name": "MDF", "thickness_mm": 18, "width_mm": 1220, "length_mm": 2440})).result["material_id"].clone();
    let group = ok(&mut model, &mut env, "group", json!({"ids": [shelf], "name": "Unit", "component": false})).created[0];
    ok(&mut model, &mut env, "set_material", json!({"ids": [group], "material_id": id}));
    let inner = *model.objects[&group].entities.iter().find(|id| model.objects.contains_key(id)).unwrap();
    let entry = ok(&mut model, &mut env, "get_object", json!({"id": inner})).result;
    assert_eq!(entry["material"]["name"], "MDF");
    assert_eq!(entry["material"]["inherited_from"], group);
}

#[test]
fn small_conveniences_from_the_last_round() {
    let (mut model, mut env) = (Model::default(), ToolEnv::default());
    let side = board(&mut model, &mut env, "Side", [0., 0., 0.], [18., 300., 900.]);
    let shelf = board(&mut model, &mut env, "Shelf", [9., 0., 400.], [500., 300., 18.]);
    let joint = ok(&mut model, &mut env, "add_joint", json!({"a": side, "b": shelf, "kind": "tongue_groove"})).joints[0];
    // A single "id" works where a list of ids is expected.
    ok(&mut model, &mut env, "flip_joint", json!({"id": joint}));
    let check = ok(&mut model, &mut env, "check_model", json!({})).result;
    assert_eq!(check["passed"], true, "{check}");
    // A drawn face comes back with where it is.
    let drawn = ok(&mut model, &mut env, "draw_polyline", json!({"points_mm": [[0, 600, 0], [100, 600, 0], [100, 700, 0], [0, 700, 0]], "close": true}));
    assert_eq!(drawn.result["faces"][0]["center_mm"], json!([50.0, 650.0, 0.0]));
    let check = ok(&mut model, &mut env, "check_model", json!({})).result;
    assert_eq!(check["passed"], true, "loose geometry is only info: {check}");
}
