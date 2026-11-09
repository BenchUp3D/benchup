use super::*;
use super::super::client::{FunctionCall, ToolCall};
use serde_json::{Value, json};

pub(crate) fn call(name: &str, args: Value) -> ToolCall {
    ToolCall::new("t1", name, &args)
}
pub(crate) fn run(model: &mut Model, env: &mut ToolEnv, name: &str, args: Value) -> ToolOutcome {
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
pub(crate) fn board(model: &mut Model, env: &mut ToolEnv, name: &str, origin: [f64; 3], size: [f64; 3]) -> u64 {
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
    let outcome = execute(&mut model, &ToolCall { id: "x".into(), kind: "function".into(), function: FunctionCall { name: "move".into(), arguments: "{not json".into() } }, &mut env);
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
    // Parts are now inside the group: editing one needs its scope open.
    let inner = grouped.result["members"][0]["id"].as_u64().unwrap();
    let message = err(&mut model, &mut env, "rename", json!({"id": inner, "name": "Inner"}));
    assert!(message.contains("not editable"), "{message}");
    ok(&mut model, &mut env, "enter_object", json!({"id": group}));
    assert_eq!(model.edit_context, Some(group));
    ok(&mut model, &mut env, "rename", json!({"id": inner, "name": "Inner"}));
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
    assert_eq!(scene["joints"][0]["kind"], "MortiseTenon");
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
        let function = &definition["function"];
        let name = function["name"].as_str().unwrap();
        assert!(is_known(name), "{name}");
        assert!(!function["description"].as_str().unwrap().is_empty());
        let parameters = &function["parameters"];
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

#[test]
fn fenced_json_fallback_is_parsed() {
    let fenced = "I will add the board.\n```json\n{\"name\": \"create_board\", \"arguments\": {\"name\": \"A\", \"origin_mm\": [0,0,0], \"size_mm\": [1,1,1]}}\n```\nDone.";
    let call = fallback_tool_call(fenced).unwrap();
    assert_eq!(call.function.name, "create_board");
    let arguments: Value = serde_json::from_str(&call.function.arguments).unwrap();
    assert_eq!(arguments["name"], "A");
    // Wrapped function form, string arguments, upper-case fence label.
    let wrapped = "```JSON\n{\"function\": {\"name\": \"get_scene\", \"arguments\": \"{}\"}}\n```";
    assert_eq!(fallback_tool_call(wrapped).unwrap().function.name, "get_scene");
    assert_eq!(fallback_tool_call("{\"tool\":\"exit_to_root\"}").unwrap().function.name, "exit_to_root");
    assert!(fallback_tool_call("```json\n{\"name\": \"launch_missiles\", \"arguments\": {}}\n```").is_none());
    assert!(fallback_tool_call("Just text, no tool.").is_none());
    assert!(fallback_tool_call("```json\nnot json\n```").is_none());
    assert!(fallback_tool_call("```rust\nfn main() {}\n```").is_none());
}
