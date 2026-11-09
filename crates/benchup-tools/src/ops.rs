//! Tools an agent may call, and the executor that applies them to a `Model`.
//! Lengths are millimetres on the wire and metres in the document; conversion
//! happens only in `point`/`length` below. Every failure is reported as
//! structured data so the model can correct itself within the same turn.
use crate::scene::{self, from_mm};
use benchup_core::{
    Axis, DVec3, MaterialId, Model, StockMaterial,
    connections::{Connection, ConnectionOperation},
    inference::AxisConstraint,
    joinery::{JointKind, JointSettings},
};
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};

/// Per call; `entities` enforces it for every tool that takes an id list.
const MAX_IDS: usize = 500;
const MAX_POINTS: usize = 1000;
const MAX_TEXT_CHARS: usize = 500;
/// Input positions and sizes: |coordinate| <= 100 m.
const MAX_COORDINATE_MM: f64 = 1.0e5;
/// Results may add up (move after move), but must stay finite and within 1 km.
const MAX_RESULT_M: f64 = 1000.;
/// Longest tag name core accepts (`Model::add_tag` counts bytes).
const MAX_TAG_BYTES: usize = 128;
const MIN_SCALE: f64 = 1.0e-3;
const MAX_SCALE: f64 = 1.0e3;

/// State shared by the tool calls of one turn.
#[derive(Default)]
pub struct ToolEnv {
    /// Mutating tools refuse to run (shared project without edit access).
    pub read_only: bool,
    /// Current selection; the `select` tool rewrites it.
    pub selection: BTreeSet<u64>,
    /// The designer turned off "Include selection": `get_scene` and
    /// `get_selection` must not reveal `selection`.
    pub hide_selection: bool,
    /// Purchasing catalog the model may assign.
    pub catalog: BTreeMap<MaterialId, StockMaterial>,
    /// Model as it was before the first successful mutating call.
    pub before: Option<Model>,
    /// Shared project: the edit lease this user holds. Mutating tools refuse
    /// entities outside it. `None` means no restriction (not a shared project).
    pub lease: Option<BTreeSet<u64>>,
    /// Entities this turn created (or released from a group it dissolved);
    /// they are not held by anyone else, so they are exempt from the lease.
    pub created: BTreeSet<u64>,
}

pub struct ToolOutcome {
    /// JSON text of this is sent back to the model.
    pub result: Value,
    pub ok: bool,
    /// The scene (not just the editing scope or selection) changed.
    pub mutated: bool,
    pub created: Vec<u64>,
    pub modified: Vec<u64>,
    pub erased: Vec<u64>,
    pub joints: Vec<u64>,
}

struct Fail {
    error: String,
    hint: Option<String>,
}
impl From<&str> for Fail {
    fn from(error: &str) -> Self {
        Self { error: error.into(), hint: None }
    }
}
impl From<String> for Fail {
    fn from(error: String) -> Self {
        Self { error, hint: None }
    }
}
fn fail(error: impl Into<String>, hint: impl Into<String>) -> Fail {
    Fail { error: error.into(), hint: Some(hint.into()) }
}

#[derive(Default)]
struct Done {
    created: Vec<u64>,
    modified: Vec<u64>,
    erased: Vec<u64>,
    joints: Vec<u64>,
    extra: Map<String, Value>,
}
type Res = Result<Done, Fail>;

fn modified(ids: &BTreeSet<u64>) -> Done {
    Done { modified: ids.iter().copied().collect(), ..Default::default() }
}

/// (name, mutating)
const TOOLS: &[(&str, bool)] = &[
    ("get_scene", false),
    ("find_objects", false),
    ("get_object", false),
    ("get_selection", false),
    ("list_stock_materials", false),
    ("create_board", true),
    ("create_copy", true),
    ("draw_polyline", true),
    ("move", true),
    ("rotate", true),
    ("scale", true),
    ("resize_board", true),
    ("push_pull", true),
    ("rename", true),
    ("set_material", true),
    ("set_grain", true),
    ("group", true),
    ("ungroup", true),
    ("tag", true),
    ("paint", true),
    ("erase", true),
    ("add_joint", true),
    ("add_dimension", true),
    ("add_text", true),
    ("select", false),
    ("enter_object", false),
    ("exit_to_root", false),
];

pub fn is_mutating(name: &str) -> bool {
    TOOLS.iter().any(|(n, mutating)| *n == name && *mutating)
}
pub fn is_known(name: &str) -> bool {
    TOOLS.iter().any(|(n, _)| *n == name)
}

// ---------------------------------------------------------------- schemas

fn vec3() -> Value {
    json!({"type": "array", "items": {"type": "number"}, "minItems": 3, "maxItems": 3})
}
fn ids() -> Value {
    json!({"type": "array", "items": {"type": "integer"}, "minItems": 1})
}
fn id() -> Value {
    json!({"type": "integer"})
}
fn text() -> Value {
    json!({"type": "string"})
}
fn number() -> Value {
    json!({"type": "number"})
}
/// A tag name: core rejects names over 128 bytes, so the schema says so.
fn tag_name(nullable: bool) -> Value {
    let kind = if nullable { json!(["string", "null"]) } else { json!("string") };
    json!({"type": kind, "maxLength": MAX_TAG_BYTES,
        "description": format!("At most {MAX_TAG_BYTES} bytes (UTF-8); not \"Untagged\".")})
}
fn axis_enum() -> Value {
    json!({"type": "string", "enum": ["x", "y", "z"]})
}
fn def(name: &str, description: &str, properties: Value, required: &[&str]) -> ToolDefinition {
    ToolDefinition {
        name: name.into(),
        description: description.into(),
        input_schema: json!({
            "type": "object",
            "properties": properties,
            "required": required,
            "additionalProperties": false,
        }),
    }
}

/// A tool as offered to an agent, independent of any wire format.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    /// JSON Schema of the arguments (always an object, no extra fields).
    pub input_schema: Value,
}

/// One tool invocation. Arguments are a JSON value (lengths in millimetres).
#[derive(Clone, Debug, PartialEq)]
pub struct Call {
    pub id: String,
    pub name: String,
    pub arguments: Value,
    /// Argument text that was not valid JSON; `execute` reports it exactly as
    /// it reports any other malformed arguments.
    raw: Option<String>,
}
impl Call {
    pub fn new(id: impl Into<String>, name: impl Into<String>, arguments: Value) -> Self {
        Self { id: id.into(), name: name.into(), arguments, raw: None }
    }
    /// Arguments as the text a model produced. Empty text means `{}`; text
    /// that is not JSON is kept and fails with the structured
    /// `invalid arguments: ...` error when executed.
    pub fn from_text(id: impl Into<String>, name: impl Into<String>, arguments: &str) -> Self {
        let text = if arguments.trim().is_empty() { "{}" } else { arguments };
        match serde_json::from_str::<Value>(text) {
            Ok(value) => Self::new(id, name, value),
            Err(_) => Self { raw: Some(arguments.to_owned()), ..Self::new(id, name, Value::Null) },
        }
    }
}

/// OpenAI-compatible `tools` array (`{"type": "function", "function": {..}}`).
pub fn openai_tools() -> Vec<Value> {
    definitions()
        .into_iter()
        .map(|d| json!({"type": "function", "function": {
            "name": d.name, "description": d.description, "parameters": d.input_schema}}))
        .collect()
}

/// MCP `tools/list` entries (`{name, description, inputSchema}`).
pub fn mcp_tools() -> Vec<Value> {
    definitions()
        .into_iter()
        .map(|d| json!({"name": d.name, "description": d.description, "inputSchema": d.input_schema}))
        .collect()
}

pub fn definitions() -> Vec<ToolDefinition> {
    let joint_kinds = json!(["mortise_tenon", "half_lap", "tongue_groove", "rabbet", "dovetail", "finger"]);
    vec![
        def("get_scene", "Return the whole scene summary again (objects, selection, stock, joints). Lengths are in mm.", json!({}), &[]),
        def("find_objects", "Search objects by name, tag, material, kind (group/component) or id. Use when the scene summary was truncated.",
            json!({"query": text(), "limit": {"type": "integer", "minimum": 1, "maximum": 100}}), &["query"]),
        def("get_object", "Details of one entity, including its faces (id, normal, center, area) for push_pull. Face normals point out of the part; a positive push_pull distance moves a face outward.",
            json!({"id": id()}), &["id"]),
        def("get_selection", "Entities currently selected in the editor.", json!({}), &[]),
        def("list_stock_materials", "Stock materials that can be assigned to boards.", json!({}), &[]),
        def("create_board", "Create one rectangular board as a named group. origin_mm is the minimum corner; size_mm is [x, y, z] extents. Prefer this over drawing.",
            json!({"name": text(), "origin_mm": vec3(), "size_mm": vec3(),
                "material_id": {"type": "string", "description": "id from list_stock_materials"},
                "grain_axis": axis_enum(), "tag": tag_name(false)}),
            &["name", "origin_mm", "size_mm"]),
        def("create_copy", "Copy an object or group, shifted by offset_mm. Returns the new ids.",
            json!({"id": id(), "offset_mm": vec3()}), &["id", "offset_mm"]),
        def("draw_polyline", "Draw loose connected edges through points; a closed loop makes a face. Not grouped.",
            json!({"points_mm": {"type": "array", "items": vec3(), "minItems": 2}, "close": {"type": "boolean"}}), &["points_mm"]),
        def("move", "Translate objects by delta_mm.", json!({"ids": ids(), "delta_mm": vec3()}), &["ids", "delta_mm"]),
        def("rotate", "Rotate objects about an axis through pivot_mm.",
            json!({"ids": ids(), "pivot_mm": vec3(), "axis": axis_enum(), "degrees": number()}), &["ids", "pivot_mm", "axis", "degrees"]),
        def("scale", "Uniformly scale objects about center_mm.",
            json!({"ids": ids(), "center_mm": vec3(), "factor": {"type": "number", "minimum": MIN_SCALE, "maximum": MAX_SCALE}}), &["ids", "center_mm", "factor"]),
        def("resize_board", "Resize a board to size_mm (world-axis extents), keeping its minimum corner fixed.",
            json!({"id": id(), "size_mm": vec3()}), &["id", "size_mm"]),
        def("push_pull", "Extrude a face outward by distance_mm (see get_object for face ids). Negative cuts into the part.",
            json!({"face_id": id(), "distance_mm": number()}), &["face_id", "distance_mm"]),
        def("rename", "Rename an object.", json!({"id": id(), "name": text()}), &["id", "name"]),
        def("set_material", "Assign a stock material to objects, or null to clear.",
            json!({"ids": ids(), "material_id": {"type": ["string", "null"]}}), &["ids", "material_id"]),
        def("set_grain", "Set the grain direction of an object along a world axis, or null to clear.",
            json!({"id": id(), "axis": {"type": ["string", "null"], "enum": ["x", "y", "z", null]}}), &["id", "axis"]),
        def("group", "Group objects, or make them a component (linked copies). Member ids change; see the result.",
            json!({"ids": ids(), "name": text(), "component": {"type": "boolean"}}), &["ids", "name", "component"]),
        def("ungroup", "Dissolve groups/components one level.", json!({"ids": ids()}), &["ids"]),
        def("tag", "Assign a tag by name (created if missing), or null to clear.",
            json!({"ids": ids(), "tag": tag_name(true)}), &["ids", "tag"]),
        def("paint", "Paint objects or faces with an RGB color.",
            json!({"ids": ids(), "rgb": {"type": "array", "items": {"type": "integer", "minimum": 0, "maximum": 255}, "minItems": 3, "maxItems": 3}}), &["ids", "rgb"]),
        def("erase", "Delete entities.", json!({"ids": ids()}), &["ids"]),
        def("add_joint", "Add a wood joint between two boards. The boards must already overlap where they meet (the overlap sets the joint depth). Optional params tune the joint (ratios 0-1, clearance in mm).",
            json!({"a": id(), "b": id(), "kind": {"type": "string", "enum": joint_kinds},
                "params": {"type": "object", "properties": {
                    "width_ratio": number(), "height_ratio": number(),
                    "fingers": {"type": "integer"}, "dovetails": {"type": "integer"},
                    "taper_degrees": number(), "clearance_mm": number(),
                    "depth_axis": {"type": "integer", "minimum": 0, "maximum": 2},
                    "width_axis": {"type": "integer", "minimum": 0, "maximum": 2}},
                    "additionalProperties": false}}),
            &["a", "b", "kind"]),
        def("add_dimension", "Add a dimension annotation between two points, drawn offset_mm away.",
            json!({"start_mm": vec3(), "end_mm": vec3(), "offset_mm": vec3()}), &["start_mm", "end_mm", "offset_mm"]),
        def("add_text", "Add a text note anchored at point_mm.",
            json!({"point_mm": vec3(), "offset_mm": vec3(), "text": text()}), &["point_mm", "offset_mm", "text"]),
        def("select", "Select entities in the editor so the designer sees them.", json!({"ids": ids()}), &["ids"]),
        def("enter_object", "Open a group/component for editing its inner parts.", json!({"id": id()}), &["id"]),
        def("exit_to_root", "Leave any open group and edit the top level again.", json!({}), &[]),
    ]
}

// ---------------------------------------------------------------- arguments

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum AxisArg {
    X,
    Y,
    Z,
}
impl AxisArg {
    fn axis(&self) -> Axis {
        match self {
            Self::X => Axis::X,
            Self::Y => Axis::Y,
            Self::Z => Axis::Z,
        }
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum JointArg {
    MortiseTenon,
    HalfLap,
    TongueGroove,
    Rabbet,
    Dovetail,
    Finger,
}
impl JointArg {
    fn kind(&self) -> JointKind {
        match self {
            Self::MortiseTenon => JointKind::MortiseTenon,
            Self::HalfLap => JointKind::HalfLap,
            Self::TongueGroove => JointKind::TongueGroove,
            Self::Rabbet => JointKind::Rabbet,
            Self::Dovetail => JointKind::Dovetail,
            Self::Finger => JointKind::Finger,
        }
    }
}

type V3 = [f64; 3];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FindArgs {
    query: String,
    limit: Option<usize>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IdArg {
    id: u64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IdsArg {
    ids: Vec<u64>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateBoardArgs {
    name: String,
    origin_mm: V3,
    size_mm: V3,
    material_id: Option<MaterialId>,
    grain_axis: Option<AxisArg>,
    tag: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CopyArgs {
    id: u64,
    offset_mm: V3,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PolylineArgs {
    points_mm: Vec<V3>,
    close: Option<bool>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MoveArgs {
    ids: Vec<u64>,
    delta_mm: V3,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RotateArgs {
    ids: Vec<u64>,
    pivot_mm: V3,
    axis: AxisArg,
    degrees: f64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScaleArgs {
    ids: Vec<u64>,
    center_mm: V3,
    factor: f64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResizeArgs {
    id: u64,
    size_mm: V3,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PushPullArgs {
    face_id: u64,
    distance_mm: f64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RenameArgs {
    id: u64,
    name: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MaterialArgs {
    ids: Vec<u64>,
    material_id: Option<MaterialId>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GrainArgs {
    id: u64,
    axis: Option<AxisArg>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GroupArgs {
    ids: Vec<u64>,
    name: String,
    component: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TagArgs {
    ids: Vec<u64>,
    tag: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PaintArgs {
    ids: Vec<u64>,
    rgb: [u8; 3],
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct JointParams {
    width_ratio: Option<f64>,
    height_ratio: Option<f64>,
    fingers: Option<u32>,
    dovetails: Option<u32>,
    taper_degrees: Option<f64>,
    clearance_mm: Option<f64>,
    depth_axis: Option<usize>,
    width_axis: Option<usize>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct JointArgs {
    a: u64,
    b: u64,
    kind: JointArg,
    params: Option<JointParams>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DimensionArgs {
    start_mm: V3,
    end_mm: V3,
    offset_mm: V3,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TextArgs {
    point_mm: V3,
    offset_mm: V3,
    text: String,
}

fn parse<T: DeserializeOwned>(arguments: &str) -> Result<T, Fail> {
    let text = if arguments.trim().is_empty() { "{}" } else { arguments };
    serde_json::from_str(text).map_err(|e| {
        fail(
            format!("invalid arguments: {e}"),
            "Send a JSON object that matches the tool schema exactly: required fields, no extra fields, lengths in mm.",
        )
    })
}
/// Millimetres on the wire to a metre vector in the document.
fn point(v: V3) -> Result<DVec3, Fail> {
    if v.iter().any(|x| !x.is_finite() || x.abs() > MAX_COORDINATE_MM) {
        return Err(fail("coordinate out of range", "Use finite values within 100 m (100000 mm) of the origin, in millimetres."));
    }
    Ok(DVec3::new(from_mm(v[0]), from_mm(v[1]), from_mm(v[2])))
}
fn finite(value: f64, what: &str) -> Result<f64, Fail> {
    value.is_finite().then_some(value).ok_or_else(|| Fail::from(format!("{what} must be a finite number")))
}

// ---------------------------------------------------------------- id checks

fn unreachable_hint(model: &Model, id: u64) -> String {
    match model.parent(id) {
        Some(parent) if model.objects.contains_key(&parent) => {
            format!("{id} is inside object {parent}: call enter_object{{id:{parent}}} first, or exit_to_root to edit the top level.")
        }
        _ => "Call exit_to_root, then retry.".into(),
    }
}

/// Resolve ids that must exist and be visible; mutating use also needs them
/// unlocked and selectable in the current editing scope.
fn entities(model: &Model, ids: &[u64], mutating: bool) -> Result<BTreeSet<u64>, Fail> {
    if ids.is_empty() {
        return Err(fail("ids must not be empty", "Pass at least one entity id from get_scene."));
    }
    if ids.len() > MAX_IDS {
        return Err(fail("too many ids", format!("Pass at most {MAX_IDS} ids per call.")));
    }
    for id in ids {
        if !model.contains_entity(id) {
            return Err(fail(format!("unknown id {id}"), "Call get_scene or find_objects to see valid ids."));
        }
        if model.is_hidden(*id) {
            return Err(fail(format!("entity {id} is hidden"), "Hidden entities cannot be used."));
        }
        if mutating {
            if model.is_locked(*id) {
                return Err(fail(format!("entity {id} is locked"), "Locked entities cannot be changed; tell the designer to unlock it."));
            }
            if model.entity_in_context(*id) != Some(*id) {
                return Err(fail(format!("entity {id} is not editable in the current scope"), unreachable_hint(model, *id)));
            }
        }
    }
    Ok(ids.iter().copied().collect())
}
fn objects(model: &Model, ids: &[u64], mutating: bool) -> Result<BTreeSet<u64>, Fail> {
    let set = entities(model, ids, mutating)?;
    if let Some(id) = set.iter().find(|id| !model.objects.contains_key(id)) {
        return Err(fail(format!("{id} is not a group or component"), "Use ids of objects listed under objects in get_scene."));
    }
    Ok(set)
}
fn one_object(model: &Model, id: u64) -> Result<u64, Fail> {
    objects(model, &[id], true).map(|_| id)
}

fn find_material(model: &Model, env: &ToolEnv, id: MaterialId) -> Result<StockMaterial, Fail> {
    env.catalog
        .get(&id)
        .or_else(|| model.stock_materials.get(&id))
        .cloned()
        .ok_or_else(|| fail(format!("unknown material_id {id}"), "Call list_stock_materials for valid ids."))
}
fn assign_material(model: &mut Model, env: &ToolEnv, object: u64, material: Option<MaterialId>) -> Result<(), Fail> {
    let definition = material.map(|id| find_material(model, env, id)).transpose()?;
    if let (Some(id), Some(definition)) = (material, definition) {
        model.stock_materials.insert(id, definition);
    }
    let target = model.objects.get_mut(&object).ok_or("missing object")?;
    target.stock_material_id = material;
    target.stock_material = None;
    Ok(())
}
fn tag_named(model: &mut Model, name: &str) -> Result<u64, Fail> {
    let name = name.trim();
    if name.len() > MAX_TAG_BYTES {
        return Err(fail("tag name too long", format!("Use at most {MAX_TAG_BYTES} bytes (UTF-8).")));
    }
    let folded = name.to_lowercase();
    if let Some(tag) = model.tags.values().find(|t| t.name.to_lowercase() == folded) {
        return Ok(tag.id);
    }
    Ok(model.add_tag(name)?)
}
fn checked_name(name: &str) -> Result<String, Fail> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 128 {
        return Err(fail("name must be 1-128 characters", "Choose a short descriptive name."));
    }
    Ok(name.to_owned())
}

// ---------------------------------------------------------------- tools

fn read(value: Value) -> Res {
    let mut done = Done::default();
    if let Value::Object(map) = value {
        done.extra = map;
    }
    Ok(done)
}

fn create_board(model: &mut Model, env: &ToolEnv, a: CreateBoardArgs) -> Res {
    let name = checked_name(&a.name)?;
    let origin = point(a.origin_mm)?;
    let size = point(a.size_mm)?;
    if size.min_element() < 1.0e-4 || size.max_element() > 100. {
        return Err(fail("size_mm out of range", "Each extent must be between 0.1 mm and 100000 mm."));
    }
    let material = a.material_id;
    if let Some(id) = material {
        find_material(model, env, id)?;
    }
    let mut drawing = Model::default();
    let corners = [
        origin,
        origin + DVec3::X * size.x,
        origin + DVec3::new(size.x, size.y, 0.),
        origin + DVec3::Y * size.y,
        origin,
    ];
    drawing.draw_polyline(&corners, false)?;
    let face = drawing.faces.keys().next().copied().ok_or("the base rectangle did not form a face")?;
    drawing.push_pull_along(face, DVec3::Z * size.z)?;
    let roots = model.paste_entities(&drawing, DVec3::ZERO);
    let object = model
        .make_object(&roots, false, &name)
        .ok_or_else(|| fail("the board could not be created", "The current editing scope may be locked; try exit_to_root."))?;
    if material.is_some() {
        assign_material(model, env, object, material)?;
    }
    if let Some(axis) = a.grain_axis {
        model.set_grain_direction(object, Some(axis.axis().vector()))?;
    }
    if let Some(tag) = a.tag.as_deref().filter(|t| !t.trim().is_empty()) {
        let tag = tag_named(model, tag)?;
        model.assign_tag(&BTreeSet::from([object]), Some(tag))?;
    }
    Ok(Done { created: vec![object], ..Default::default() })
}

fn create_copy(model: &mut Model, a: CopyArgs) -> Res {
    let source = entities(model, &[a.id], false)?;
    let offset = point(a.offset_mm)?;
    let copy = model.copy_entities(&source);
    let before: BTreeSet<u64> = model.root_entities();
    let pasted = model.paste_entities(&copy, offset);
    let created: Vec<u64> = if pasted.is_empty() { Vec::new() } else { pasted.difference(&before).copied().collect() };
    if created.is_empty() {
        return Err("nothing was copied".into());
    }
    Ok(Done { created, ..Default::default() })
}

fn draw_polyline(model: &mut Model, a: PolylineArgs) -> Res {
    if a.points_mm.len() < 2 {
        return Err(fail("points_mm needs at least 2 points", "Give the corners in order."));
    }
    if a.points_mm.len() > MAX_POINTS {
        return Err(fail("too many points", format!("Pass at most {MAX_POINTS} points per call.")));
    }
    let mut points = a.points_mm.into_iter().map(point).collect::<Result<Vec<_>, _>>()?;
    if a.close.unwrap_or(false)
        && let Some(first) = points.first().copied()
    {
        points.push(first);
    }
    let known = |m: &Model| -> BTreeSet<u64> { m.edges.keys().chain(m.faces.keys()).copied().collect() };
    let before = known(model);
    model.draw_polyline(&points, false)?;
    let created: Vec<u64> = known(model).difference(&before).copied().collect();
    Ok(Done { created, ..Default::default() })
}

fn move_objects(model: &mut Model, a: MoveArgs) -> Res {
    let set = entities(model, &a.ids, true)?;
    model.move_selection(&set, point(a.delta_mm)?)?;
    Ok(modified(&set))
}
fn rotate(model: &mut Model, a: RotateArgs) -> Res {
    let set = entities(model, &a.ids, true)?;
    let degrees = finite(a.degrees, "degrees")?;
    model.rotate_selection(&set, point(a.pivot_mm)?, a.axis.axis().vector(), degrees)?;
    Ok(modified(&set))
}
fn scale(model: &mut Model, a: ScaleArgs) -> Res {
    let set = entities(model, &a.ids, true)?;
    let factor = finite(a.factor, "factor")?;
    if !(MIN_SCALE..=MAX_SCALE).contains(&factor) {
        return Err(fail("factor out of range", format!("Use a factor between {MIN_SCALE} and {MAX_SCALE}; scale in several steps if needed.")));
    }
    model.scale_selection(&set, point(a.center_mm)?, factor)?;
    Ok(modified(&set))
}
fn resize_board(model: &mut Model, a: ResizeArgs) -> Res {
    let id = one_object(model, a.id)?;
    let set = BTreeSet::from([id]);
    let target = point(a.size_mm)?;
    if target.min_element() < 1.0e-4 {
        return Err(fail("size_mm out of range", "Each extent must be at least 0.1 mm."));
    }
    let (lo, hi) = model.selection_bounds(&set).ok_or("the object has no geometry")?;
    let current = hi - lo;
    for axis in [Axis::X, Axis::Y, Axis::Z] {
        let (old, new) = (current.dot(axis.vector()), target.dot(axis.vector()));
        if (new - old).abs() < 1.0e-9 {
            continue;
        }
        if old < 1.0e-9 {
            return Err("the object is flat along that axis and cannot be resized".into());
        }
        model.scale_selection_constrained(&set, lo, new / old, Some(AxisConstraint::Axis(axis)))?;
    }
    Ok(modified(&set))
}
fn push_pull(model: &mut Model, a: PushPullArgs) -> Res {
    let face = a.face_id;
    if !model.faces.contains_key(&face) {
        return Err(fail(format!("{face} is not a face"), "Use a face id from get_object."));
    }
    if model.is_hidden(face) || model.is_locked(face) {
        return Err(fail(format!("face {face} is hidden or locked"), "Pick another face."));
    }
    // The edit scope is switched to the face's owner below, so check here, with
    // the same rules as the other tools, that the owner chain may be edited.
    let top = model.entity_in_context(face).ok_or_else(|| {
        fail(format!("face {face} is not editable in the current scope"), unreachable_hint(model, face))
    })?;
    entities(model, &[top], true)?;
    let mut ancestor = model.parent(face);
    while let Some(owner) = ancestor {
        if model.is_hidden(owner) || model.is_locked(owner) {
            return Err(fail(format!("face {face} is inside hidden or locked object {owner}"), "Locked entities cannot be changed; tell the designer to unlock it."));
        }
        ancestor = model.parent(owner);
    }
    let distance = from_mm(finite(a.distance_mm, "distance_mm")?) * scene::outward_sign(model, face);
    let owner = model.parent(face);
    let saved = model.edit_context;
    // Faces inside an object are edited through that object's scope.
    let result = match owner {
        Some(owner) if saved != Some(owner) => {
            model.edit_context = Some(owner);
            let result = model.push_pull(face, distance);
            model.edit_context = saved;
            result
        }
        _ => model.push_pull(face, distance),
    };
    let new_face = result?;
    let mut done = Done::default();
    if let Some(owner) = owner.filter(|o| model.objects.contains_key(o)) {
        done.modified.push(owner);
    }
    if model.faces.contains_key(&new_face) {
        done.extra.insert("new_face_id".into(), json!(new_face));
    }
    Ok(done)
}
fn rename(model: &mut Model, a: RenameArgs) -> Res {
    let id = one_object(model, a.id)?;
    let name = checked_name(&a.name)?;
    model.objects.get_mut(&id).ok_or("missing object")?.name = name;
    Ok(modified(&BTreeSet::from([id])))
}
fn set_material(model: &mut Model, env: &ToolEnv, a: MaterialArgs) -> Res {
    let set = objects(model, &a.ids, true)?;
    if let Some(id) = a.material_id {
        find_material(model, env, id)?;
    }
    for id in &set {
        assign_material(model, env, *id, a.material_id)?;
    }
    Ok(modified(&set))
}
fn set_grain(model: &mut Model, a: GrainArgs) -> Res {
    let id = one_object(model, a.id)?;
    model.set_grain_direction(id, a.axis.map(|axis| axis.axis().vector()))?;
    Ok(modified(&BTreeSet::from([id])))
}
fn group(model: &mut Model, a: GroupArgs) -> Res {
    let set = entities(model, &a.ids, true)?;
    let name = checked_name(&a.name)?;
    let group = model
        .make_object(&set, a.component, &name)
        .ok_or_else(|| fail("the selection could not be grouped", "Locked or empty selections cannot be grouped."))?;
    let members: BTreeSet<u64> = model.objects[&group].entities.clone();
    let mut done = Done {
        created: vec![group],
        erased: set.iter().copied().filter(|id| !model.contains_entity(id)).collect(),
        ..Default::default()
    };
    done.extra.insert(
        "members".into(),
        Value::Array(members.iter().filter_map(|id| scene::object_entry(model, *id)).collect()),
    );
    done.extra.insert("note".into(), json!("Member ids may have changed; use the ids in members."));
    Ok(done)
}
fn ungroup(model: &mut Model, a: IdsArg) -> Res {
    let set = objects(model, &a.ids, true)?;
    let released = model.dissolve_objects(&set, false);
    Ok(Done {
        modified: released.into_iter().filter(|id| model.contains_entity(id)).collect(),
        erased: set.iter().copied().filter(|id| !model.contains_entity(id)).collect(),
        ..Default::default()
    })
}
fn tag(model: &mut Model, a: TagArgs) -> Res {
    let set = entities(model, &a.ids, true)?;
    let tag = match a.tag.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
        Some(name) => Some(tag_named(model, name)?),
        None => None,
    };
    model.assign_tag(&set, tag)?;
    let mut done = modified(&set);
    done.extra.insert("tag_id".into(), json!(tag));
    Ok(done)
}
fn paint(model: &mut Model, a: PaintArgs) -> Res {
    let set = entities(model, &a.ids, true)?;
    for id in &set {
        if model.objects.contains_key(id) {
            model.paint_object(*id, a.rgb)?;
        } else if model.faces.contains_key(id) {
            model.paint_face(*id, a.rgb)?;
        } else {
            return Err(fail(format!("{id} cannot be painted"), "Paint objects or faces."));
        }
    }
    Ok(modified(&set))
}
fn erase(model: &mut Model, a: IdsArg) -> Res {
    let set = entities(model, &a.ids, true)?;
    model.erase(&set);
    let erased: Vec<u64> = set.iter().copied().filter(|id| !model.contains_entity(id)).collect();
    if erased.is_empty() {
        return Err("nothing was erased".into());
    }
    Ok(Done { erased, ..Default::default() })
}
fn add_joint(model: &mut Model, a: JointArgs) -> Res {
    if a.a == a.b {
        return Err(fail("a and b must differ", "A joint connects two different boards."));
    }
    let set = objects(model, &[a.a, a.b], true)?;
    let mut settings = JointSettings { kind: a.kind.kind(), ..Default::default() };
    if let Some(p) = a.params {
        let ratio = |value: Option<f64>, what: &str| -> Result<Option<f64>, Fail> {
            value
                .map(|v| {
                    (v.is_finite() && v > 0. && v <= 1.)
                        .then_some(v)
                        .ok_or_else(|| Fail::from(format!("{what} must be between 0 and 1")))
                })
                .transpose()
        };
        if let Some(v) = ratio(p.width_ratio, "width_ratio")? {
            settings.width_ratio = v;
        }
        if let Some(v) = ratio(p.height_ratio, "height_ratio")? {
            settings.height_ratio = v;
        }
        if let Some(v) = p.fingers {
            settings.fingers = v.clamp(1, 64);
        }
        if let Some(v) = p.dovetails {
            settings.dovetails = v.clamp(1, 64);
        }
        if let Some(v) = p.taper_degrees {
            settings.taper_degrees = finite(v, "taper_degrees")?.clamp(0., 30.);
        }
        if let Some(v) = p.clearance_mm {
            settings.clearance = from_mm(finite(v, "clearance_mm")?.clamp(0., 5.));
        }
        for (axis, slot) in [(p.depth_axis, &mut settings.depth_axis), (p.width_axis, &mut settings.width_axis)] {
            if let Some(axis) = axis {
                if axis > 2 {
                    return Err("axis indexes are 0, 1 or 2".into());
                }
                *slot = axis;
            }
        }
    }
    let joint = model.add_connection(Connection {
        members: [a.a, a.b],
        entry_face: None,
        operation: ConnectionOperation::Joinery(settings),
    })?;
    let mut done = modified(&set);
    done.joints.push(joint);
    done.extra.insert("joint_id".into(), json!(joint));
    Ok(done)
}
fn add_dimension(model: &mut Model, a: DimensionArgs) -> Res {
    let id = model.add_dimension(point(a.start_mm)?, point(a.end_mm)?, point(a.offset_mm)?)?;
    Ok(Done { created: vec![id], ..Default::default() })
}
fn add_text(model: &mut Model, a: TextArgs) -> Res {
    if a.text.chars().count() > MAX_TEXT_CHARS {
        return Err(fail("text too long", format!("Use at most {MAX_TEXT_CHARS} characters.")));
    }
    let id = model.add_text(point(a.point_mm)?, point(a.offset_mm)?, &a.text)?;
    Ok(Done { created: vec![id], ..Default::default() })
}

fn find_objects(model: &Model, a: FindArgs) -> Res {
    let query = a.query.trim().to_lowercase();
    let limit = a.limit.unwrap_or(40).clamp(1, 100);
    let matches = |id: u64| -> bool {
        let Some(object) = model.objects.get(&id) else { return false };
        query.is_empty()
            || id.to_string() == query
            || object.name.to_lowercase().contains(&query)
            || scene::kind_of(model, id) == query
            || model.inherited_tag(id).is_some_and(|t| t.name.to_lowercase().contains(&query))
            || model
                .object_stock_material(object)
                .is_some_and(|m| m.name.to_lowercase().contains(&query))
    };
    let found: Vec<u64> = model.objects.keys().copied().filter(|id| matches(*id)).collect();
    read(json!({
        "total": found.len(),
        "objects": found.iter().take(limit).filter_map(|id| scene::object_entry(model, *id)).collect::<Vec<_>>(),
    }))
}

fn run(model: &mut Model, env: &mut ToolEnv, name: &str, arguments: &str) -> Res {
    match name {
        "get_scene" => {
            parse::<Empty>(arguments)?;
            let shared = if env.hide_selection { BTreeSet::new() } else { env.selection.clone() };
            read(scene::summary(model, &shared, &env.catalog, scene::SCENE_BUDGET))
        }
        "find_objects" => find_objects(model, parse(arguments)?),
        "get_object" => {
            let a: IdArg = parse(arguments)?;
            read(scene::describe(model, a.id).ok_or_else(|| fail(format!("unknown id {}", a.id), "Call get_scene or find_objects."))?)
        }
        "get_selection" => {
            parse::<Empty>(arguments)?;
            if env.hide_selection {
                return read(json!({"selection": [], "note": "The designer has turned off sharing their selection."}));
            }
            let live: Vec<u64> = env.selection.iter().copied().filter(|id| model.contains_entity(id)).collect();
            read(json!({
                "selection": live,
                "objects": live.iter().filter_map(|id| scene::object_entry(model, *id)).collect::<Vec<_>>(),
            }))
        }
        "list_stock_materials" => {
            parse::<Empty>(arguments)?;
            read(json!({"materials": scene::stock_list(model, &env.catalog)}))
        }
        "create_board" => create_board(model, env, parse(arguments)?),
        "create_copy" => create_copy(model, parse(arguments)?),
        "draw_polyline" => draw_polyline(model, parse(arguments)?),
        "move" => move_objects(model, parse(arguments)?),
        "rotate" => rotate(model, parse(arguments)?),
        "scale" => scale(model, parse(arguments)?),
        "resize_board" => resize_board(model, parse(arguments)?),
        "push_pull" => push_pull(model, parse(arguments)?),
        "rename" => rename(model, parse(arguments)?),
        "set_material" => set_material(model, env, parse(arguments)?),
        "set_grain" => set_grain(model, parse(arguments)?),
        "group" => group(model, parse(arguments)?),
        "ungroup" => ungroup(model, parse(arguments)?),
        "tag" => tag(model, parse(arguments)?),
        "paint" => paint(model, parse(arguments)?),
        "erase" => erase(model, parse(arguments)?),
        "add_joint" => add_joint(model, parse(arguments)?),
        "add_dimension" => add_dimension(model, parse(arguments)?),
        "add_text" => add_text(model, parse(arguments)?),
        "select" => {
            let a: IdsArg = parse(arguments)?;
            let chosen = entities(model, &a.ids, false)?;
            // Shared project: only what this designer may edit can be selected.
            if let Some(id) = chosen.iter().find(|id| !leased(model, env, **id)) {
                return Err(fail(
                    format!("entity {id} is not in your edit lease"),
                    "Only entities you hold the edit lease on (or created this turn) can be selected.",
                ));
            }
            env.selection = chosen;
            read(json!({"selection": env.selection}))
        }
        "enter_object" => {
            let a: IdArg = parse(arguments)?;
            if !model.enter_object(a.id) {
                return Err(fail(
                    format!("cannot open {}", a.id),
                    "It must be an unlocked, visible group or component in the current scope.",
                ));
            }
            read(json!({"edit_path": model.edit_path()}))
        }
        "exit_to_root" => {
            parse::<Empty>(arguments)?;
            model.edit_context = None;
            read(json!({"edit_path": model.edit_path()}))
        }
        _ => Err(Fail::from("unknown tool")),
    }
}

fn error_outcome(error: String, hint: Option<String>) -> ToolOutcome {
    let mut result = json!({"ok": false, "error": error});
    if let Some(hint) = hint {
        result["hint"] = json!(hint);
    }
    ToolOutcome {
        result,
        ok: false,
        mutated: false,
        created: Vec::new(),
        modified: Vec::new(),
        erased: Vec::new(),
        joints: Vec::new(),
    }
}

/// Tools that only add new entities: nothing existing is touched, so there is
/// nothing to lease. (`create_copy` reads its source without changing it.)
const CREATING: &[&str] = &["create_board", "create_copy", "draw_polyline", "add_dimension", "add_text"];

/// Existing entities a mutating call names, read from the raw arguments.
fn target_ids(name: &str, arguments: &str) -> Vec<u64> {
    if CREATING.contains(&name) {
        return Vec::new();
    }
    let Ok(Value::Object(map)) = serde_json::from_str::<Value>(arguments) else { return Vec::new() };
    let mut ids: Vec<u64> = map.get("ids").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_u64).collect();
    ids.extend(["id", "face_id", "a", "b"].iter().filter_map(|key| map.get(*key)?.as_u64()));
    ids
}

/// An entity is leased when it, or an object containing it, is in the lease
/// (or was created by this turn). Always true outside shared projects.
fn leased(model: &Model, env: &ToolEnv, id: u64) -> bool {
    let Some(lease) = &env.lease else { return true };
    let mut current = Some(id);
    for _ in 0..256 {
        let Some(c) = current else { break };
        if lease.contains(&c) || env.created.contains(&c) {
            return true;
        }
        current = model.parent(c);
    }
    false
}

fn out_of_range(model: &Model, done: &Done) -> bool {
    let touched: BTreeSet<u64> = done.created.iter().chain(&done.modified).copied().filter(|id| model.contains_entity(id)).collect();
    model
        .selection_bounds(&touched)
        .is_some_and(|(lo, hi)| [lo, hi].iter().any(|p| p.to_array().iter().any(|v| !v.is_finite() || v.abs() > MAX_RESULT_M)))
}

/// Apply one tool call. A failed or panicking mutating call leaves the model
/// exactly as it was; a successful one records the turn's `before` snapshot.
pub fn execute(model: &mut Model, call: &Call, env: &mut ToolEnv) -> ToolOutcome {
    let name = call.name.as_str();
    if !is_known(name) {
        let names: Vec<_> = TOOLS.iter().map(|(n, _)| *n).collect();
        return error_outcome(format!("unknown tool {name}"), Some(format!("Available tools: {}", names.join(", "))));
    }
    let mutating = is_mutating(name);
    if mutating && env.read_only {
        return error_outcome(
            "read-only".into(),
            Some("This project is read-only for the designer; describe what you would change instead.".into()),
        );
    }
    let text = call.raw.clone().unwrap_or_else(|| call.arguments.to_string());
    let arguments = text.as_str();
    if mutating
        && target_ids(name, arguments).into_iter().any(|id| model.contains_entity(&id) && !leased(model, env, id))
    {
        return error_outcome("not-leased".into(), Some("select it first so you hold the edit lease".into()));
    }
    let backup = mutating.then(|| model.clone());
    // `catch_unwind` cannot help on wasm32, where a panic aborts. Every tool
    // therefore validates its arguments (counts, ranges, lengths) before it
    // touches the model, and the result is range-checked below.
    let attempt = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(model, env, name, arguments)));
    let result = match attempt {
        Ok(result) => result,
        Err(_) => Err(Fail::from("internal error while applying the tool")),
    };
    match result {
        Err(failure) => {
            if let Some(backup) = backup {
                *model = backup;
            }
            error_outcome(failure.error, failure.hint)
        }
        Ok(done) if backup.is_some() && out_of_range(model, &done) => {
            *model = backup.unwrap_or_default();
            error_outcome(
                "result out of range".into(),
                Some("The result would be non-finite or beyond 1 km from the origin; use smaller values.".into()),
            )
        }
        Ok(done) => {
            env.created.extend(&done.created);
            if name == "ungroup" {
                // Released parts have no leased container any more.
                env.created.extend(&done.modified);
            }
            let mut mutated = false;
            if let Some(backup) = backup {
                mutated = !backup.same_scene(model);
                if mutated && env.before.is_none() {
                    env.before = Some(backup);
                }
            }
            let mut result = Value::Object(done.extra);
            result["ok"] = json!(true);
            if mutating {
                let live = |ids: &[u64]| -> Vec<u64> { ids.iter().copied().filter(|id| model.contains_entity(id)).collect() };
                let (created, changed) = (live(&done.created), live(&done.modified));
                let touched: BTreeSet<u64> = created.iter().chain(&changed).copied().collect();
                result["created"] = json!(created);
                result["modified"] = json!(changed);
                if !done.erased.is_empty() {
                    result["erased"] = json!(done.erased);
                }
                if let Some(bounds) = scene::bounds_json(model, &touched) {
                    result["bounds_mm"] = bounds;
                }
                let summaries: Vec<Value> = touched.iter().take(20).filter_map(|id| scene::object_entry(model, *id)).collect();
                if !summaries.is_empty() {
                    result["objects"] = Value::Array(summaries);
                }
            }
            ToolOutcome {
                result,
                ok: true,
                mutated,
                created: done.created,
                modified: done.modified,
                erased: done.erased,
                joints: done.joints,
            }
        }
    }
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
