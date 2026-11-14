//! Tools an agent may call, and the executor that applies them to a `Model`.
//! Lengths are millimetres on the wire and metres in the document; conversion
//! happens only in `point`/`length` below. Every failure is reported as
//! structured data so the model can correct itself within the same turn.
use crate::scene::{self, from_mm};
use benchup_core::{
    Axis, DVec3, MaterialId, Model, StockMaterial,
    connections::{Connection, ConnectionOperation},
    inference::AxisConstraint,
    joinery::{JointKind, JointSettings, KeepEnd, TrimBoard},
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
    ("check_model", false),
    ("get_cut_list", false),
    ("get_guide", false),
    ("get_estimate", false),
    ("measure_space", false),
    ("find_surfaces", false),
    ("create_stock_material", true),
    ("create_board", true),
    ("create_copy", true),
    ("create_profile_board", true),
    ("draw_polyline", true),
    ("split_face", true),
    ("merge_faces", true),
    ("move", true),
    ("move_to", true),
    ("rotate", true),
    ("scale", true),
    ("resize_board", true),
    ("change_thickness", true),
    ("distribute", true),
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
    ("remove_joint", true),
    ("flip_joint", true),
    ("set_joint", true),
    ("add_profile", true),
    ("remove_profile", true),
    ("set_finish", true),
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
fn anchor_enum() -> Value {
    json!({"type": "string", "enum": ["min", "center", "max"]})
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
    let mut list = vec![
        def("get_scene", "Return the scene summary (objects, selection, stock, joints, dimensions, texts). Lengths are in mm. Filters keep it small on big projects: ids lists only those objects and what is inside them; depth limits nesting (0 = top level only, or only the given ids); fields picks object fields (id is always included); sections picks the parts of the summary.",
            json!({"ids": ids(), "depth": {"type": "integer", "minimum": 0, "maximum": 64},
                "fields": {"type": "array", "items": {"type": "string", "enum": scene::OBJECT_FIELDS}},
                "sections": {"type": "array", "items": {"type": "string", "enum": scene::SECTIONS}}}), &[]),
        def("find_objects", "Search objects (groups and components) when the scene summary was truncated. query matches part of a name, tag or material name, a kind (\"group\" or \"component\") or an exact id; omit it or pass \"\" to list every object. Dimensions and texts are not objects.",
            json!({"query": text(), "limit": {"type": "integer", "minimum": 1, "maximum": 100}}), &[]),
        def("get_object", "Details of one entity, including its faces (id, normal, facing, center_mm, area_mm2) for push_pull. Face normals point out of the part; a positive push_pull distance moves a face outward.",
            json!({"id": id()}), &["id"]),
        def("get_selection", "Entities currently selected in the editor.", json!({}), &[]),
        def("list_stock_materials", "Stock materials that can be assigned to boards (ids for material_id). A new project has none: add them with create_stock_material.", json!({}), &[]),
        def("check_model", "Look for problems before building: boards passing through others or overlapping without a joint, joints that cannot be cut, look reversed or lost a board, thickness that does not match the material, open solids, loose geometry, boards without material. passed is true when there are no errors or warnings (ok only says the check ran). Run it after a change. With only_new, it reports only the problems that the earlier calls of this batch (or session turn) introduced, so a dry-run batch ending with check_model{only_new:true} ignores problems that were already there.", json!({"only_new": {"type": "boolean"}}), &[]),
        def("get_cut_list", "Cut list: boards grouped by material and blank size (length along the grain x width x thickness, mm) with quantities, and area totals per material.", json!({}), &[]),
        def("get_guide", "Conventions, workflow and woodworking recipes for building furniture with these tools. Read it before the first change.", json!({}), &[]),
        def("create_stock_material", "Add a stock material (sheet or board) to this project and return its id for material_id. Sizes in mm; length runs along the grain.",
            json!({"name": text(), "thickness_mm": number(), "width_mm": number(), "length_mm": number()}),
            &["name", "thickness_mm", "width_mm", "length_mm"]),
        def("create_board", "Create one rectangular board as a named group. origin_mm is the minimum corner; size_mm is [x, y, z] extents. Prefer this over drawing. Returns the board id in created[0].",
            json!({"name": text(), "origin_mm": vec3(), "size_mm": vec3(),
                "material_id": {"type": "string", "format": "uuid", "description": "id from list_stock_materials or create_stock_material"},
                "grain_axis": axis_enum(), "tag": tag_name(false)}),
            &["name", "origin_mm", "size_mm"]),
        def("create_copy", "Copy an object or group, shifted by offset_mm; a copy of a component is a linked instance. Returns the copy in created[0]. Joints inside a copied object come along; joints with boards outside it do not.",
            json!({"id": id(), "offset_mm": vec3()}), &["id", "offset_mm"]),
        def("draw_polyline", "Draw connected edges through points in the current scope; a closed flat loop makes a face (face_id in the result). Drawn on a board's face (inside it, via enter_object), a line splits that face. To make a shaped board, draw its closed outline, push_pull the face to the thickness, then group created ids.",
            json!({"points_mm": {"type": "array", "items": vec3(), "minItems": 2}, "close": {"type": "boolean"}}), &["points_mm"]),
        def("move", "Translate objects by delta_mm. Joints follow the boards.", json!({"ids": ids(), "delta_mm": vec3()}), &["ids", "delta_mm"]),
        def("move_to", "Move an object so the minimum corner of its bounding box lands on origin_mm.",
            json!({"id": id(), "origin_mm": vec3()}), &["id", "origin_mm"]),
        def("rotate", "Rotate objects about an axis through pivot_mm. Positive degrees follow the right-hand rule: looking from the positive end of the axis, counter-clockwise (about x, +y turns towards +z; about z, +x turns towards +y).",
            json!({"ids": ids(), "pivot_mm": vec3(), "axis": axis_enum(), "degrees": number()}), &["ids", "pivot_mm", "axis", "degrees"]),
        def("scale", "Uniformly scale objects about center_mm.",
            json!({"ids": ids(), "center_mm": vec3(), "factor": {"type": "number", "minimum": MIN_SCALE, "maximum": MAX_SCALE}}), &["ids", "center_mm", "factor"]),
        def("resize_board", "Resize a board to size_mm (world-axis extents). anchor says which side stays put on each axis: min (default), center or max; one value for all axes or [x, y, z]. Joints follow the board.",
            json!({"id": id(), "size_mm": vec3(), "anchor": {"oneOf": [anchor_enum(),
                {"type": "array", "items": anchor_enum(), "minItems": 3, "maxItems": 3}]}}), &["id", "size_mm"]),
        def("push_pull", "Extrude a face outward by distance_mm (see get_object for face ids). Negative cuts into the part; cutting by the full thickness removes that material. Works on faces inside groups without enter_object. Returns the moved face as face_id and everything it made in created.",
            json!({"face_id": id(), "distance_mm": number()}), &["face_id", "distance_mm"]),
        def("rename", "Rename an object.", json!({"id": id(), "name": text()}), &["id", "name"]),
        def("set_material", "Assign a stock material to objects, or null to clear.",
            json!({"ids": ids(), "material_id": {"type": ["string", "null"], "format": "uuid",
                "description": "id from list_stock_materials or create_stock_material"}}), &["ids", "material_id"]),
        def("set_grain", "Set the grain direction of objects along a world axis, or null to clear (\"id\" for one object is accepted too).",
            json!({"ids": ids(), "axis": {"type": ["string", "null"], "enum": ["x", "y", "z", null]}}), &["ids", "axis"]),
        def("group", "Group entities into one object, or make that object a component. Several ids become ONE group or component; for linked copies, make one component and use create_copy. Faces bring their edges along. Member ids change: the group is created[0] and its objects are listed in members.",
            json!({"ids": ids(), "name": text(), "component": {"type": "boolean"}}), &["ids", "name", "component"]),
        def("ungroup", "Dissolve groups/components one level. The released objects keep their ids and are listed in members.", json!({"ids": ids()}), &["ids"]),
        def("tag", "Assign a tag by name (created if missing), or null to clear.",
            json!({"ids": ids(), "tag": tag_name(true)}), &["ids", "tag"]),
        def("paint", "Paint objects or faces with an RGB color.",
            json!({"ids": ids(), "rgb": {"type": "array", "items": {"type": "integer", "minimum": 0, "maximum": 255}, "minItems": 3, "maxItems": 3}}), &["ids", "rgb"]),
        def("erase", "Delete entities (objects, faces, edges, dimensions, texts). Joints that use an erased board are removed with it; use remove_joint to delete a joint alone.", json!({"ids": ids()}), &["ids"]),
        def("add_joint", "Add a wood joint between two boards that already overlap where they meet; the overlap is the joint's depth. a carries the tenon, tongue, tails, fingers or lap: the board that goes into the other (shelf, apron, drawer side). b receives the mortise, groove or sockets (side, leg, drawer front). params.depth_axis is the direction a goes into b; width_axis is the axis across which the tongue, tenon or fingers are spread (width_ratio of it); the third axis is the height (height_ratio, mortise_tenon only). Axes are 0=x, 1=y, 2=z (a's own axes if it is rotated). Omit them and the first orientation that can be cut is chosen. Params by kind: mortise_tenon uses width_ratio and height_ratio; tongue_groove and rabbet use width_ratio; dovetail uses dovetails, width_ratio (share of the width taken by tails) and taper_degrees; finger uses fingers; all use clearance_mm. Ranges: ratios 0.1-0.9 (0.5), fingers 2-16 (4), dovetails 1-16 (1), taper_degrees 0-30 (8), clearance_mm 0-5 (0.1). The joint must be cuttable or the call fails; joints on one board must not cross, touch or end in the same plane. Both boards must be at the same group level. Trimming: trim \"a\" or \"b\" cuts that board's excess flush with the other board's outer face (keep_end lower or upper says which end of the board stays, along trim_axis, default its longest axis). Returns joint_id and the orientation used; get_object on a joint shows its settings, geometry and supported orientations. set_joint changes an existing joint.",
            json!({"a": id(), "b": id(), "kind": {"type": "string", "enum": joint_kinds},
                "params": {"type": "object", "properties": {
                    "width_ratio": number(), "height_ratio": number(),
                    "fingers": {"type": "integer"}, "dovetails": {"type": "integer"},
                    "taper_degrees": number(), "clearance_mm": number(),
                    "depth_axis": {"type": "integer", "minimum": 0, "maximum": 2},
                    "width_axis": {"type": "integer", "minimum": 0, "maximum": 2},
                    "trim": {"type": "string", "enum": ["none", "a", "b"]},
                    "trim_axis": {"type": "integer", "minimum": 0, "maximum": 2},
                    "keep_end": {"type": "string", "enum": ["lower", "upper"]}},
                    "additionalProperties": false}}),
            &["a", "b", "kind"]),
        def("remove_joint", "Delete wood joints by id (from the joints list in get_scene); the boards stay.",
            json!({"ids": ids()}), &["ids"]),
        def("flip_joint", "Swap a joint's a and b (the board that carries the tenon or tongue and the one that receives it), keeping its kind and settings and re-picking the orientation. Use it when check_model reports joint_roles_reversed.",
            json!({"ids": ids()}), &["ids"]),
        def("add_dimension", "Add a dimension between two points. It measures the straight distance between them, or only along axis (x, y or z) when given. A point exactly on a vertex, edge or face attaches to it and follows it when the part moves; there is no snapping to nearby points. offset_mm moves the dimension line away from the measured points, perpendicular to them (e.g. [0, -150, 0] draws it 150 mm in front). Returns length_mm and whether each end attached.",
            json!({"start_mm": vec3(), "end_mm": vec3(), "offset_mm": vec3(), "axis": axis_enum()}), &["start_mm", "end_mm", "offset_mm"]),
        def("add_text", "Add a text note anchored at point_mm; offset_mm places the label away from the anchor with a leader line ([0, 0, 0] puts it on the point).",
            json!({"point_mm": vec3(), "offset_mm": vec3(), "text": text()}), &["point_mm", "offset_mm", "text"]),
        def("select", "Select entities in the editor so the designer sees them.", json!({"ids": ids()}), &["ids"]),
        def("enter_object", "Open a group/component so that creating tools (create_board, draw_polyline, add_dimension...) put new parts inside it. Editing tools reach parts inside groups by themselves when all the ids share one group. Nested groups open from the outside in. The group stays open until exit_to_root or the end of the session (with the benchup CLI, one command or --batch).", json!({"id": id()}), &["id"]),
        def("exit_to_root", "Leave any open group and edit the top level again.", json!({}), &[]),
    ];
    list.extend(more::definitions());
    list
}

// ---------------------------------------------------------------- arguments

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Anchor {
    Min,
    Center,
    Max,
}
#[derive(Deserialize)]
#[serde(untagged)]
enum AnchorArg {
    All(Anchor),
    PerAxis([Anchor; 3]),
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MoveToArgs {
    id: u64,
    origin_mm: V3,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StockArgs {
    name: String,
    thickness_mm: f64,
    width_mm: f64,
    length_mm: f64,
}

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
    #[serde(default)]
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
    /// `"id": 5` is accepted too: agents guess the singular for one id.
    #[serde(alias = "id", deserialize_with = "one_or_many")]
    ids: Vec<u64>,
}
fn one_or_many<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Vec<u64>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(u64),
        Many(Vec<u64>),
    }
    Ok(match OneOrMany::deserialize(deserializer)? {
        OneOrMany::One(id) => vec![id],
        OneOrMany::Many(ids) => ids,
    })
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
    anchor: Option<AnchorArg>,
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
    /// `"id": 5` is accepted too, like the other tools that take several objects.
    #[serde(alias = "id", deserialize_with = "one_or_many")]
    ids: Vec<u64>,
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
    trim: Option<TrimArg>,
    trim_axis: Option<usize>,
    keep_end: Option<KeepEndArg>,
}
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum TrimArg {
    None,
    A,
    B,
}
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum KeepEndArg {
    Lower,
    Upper,
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
    axis: Option<AxisArg>,
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

/// How to reach `id` from the current editing scope: the objects to open, from
/// the outside in, after leaving the current scope if `id` is not inside it.
fn unreachable_hint(model: &Model, id: u64) -> String {
    let mut chain = Vec::new();
    let mut current = model.parent(id).filter(|o| model.objects.contains_key(o));
    while let Some(object) = current {
        if Some(object) == model.edit_context || chain.len() > 256 {
            break;
        }
        chain.push(object);
        current = model.parent(object).filter(|o| model.objects.contains_key(o));
    }
    let Some(&parent) = chain.first() else {
        return "Call exit_to_root, then retry.".into();
    };
    chain.reverse();
    let leave = if model.edit_context.is_some() && current != model.edit_context { "exit_to_root, then " } else { "" };
    let steps: Vec<String> = chain.iter().map(|o| format!("enter_object{{id:{o}}}")).collect();
    format!(
        "{id} is inside object {parent}: call {leave}{} first, in the same session \
         (with the benchup CLI, in the same --batch).",
        steps.join(", then ")
    )
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
    let faces: Vec<u64> = created.iter().copied().filter(|id| model.faces.contains_key(id)).collect();
    let mut done = Done { created, ..Default::default() };
    // A closed loop makes a face; name it so push_pull can follow in a batch.
    if let [face] = faces[..] {
        done.extra.insert("face_id".into(), json!(face));
    }
    done.extra.insert("face_ids".into(), json!(faces));
    // Where each new face is, so a split face's pieces can be told apart.
    let detail: Vec<Value> = faces.iter().filter_map(|f| scene::face_entry(model, *f)).collect();
    done.extra.insert("faces".into(), Value::Array(detail));
    Ok(done)
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
    let target = point(a.size_mm)?;
    if target.min_element() < 1.0e-4 {
        return Err(fail("size_mm out of range", "Each extent must be at least 0.1 mm."));
    }
    let anchors = match a.anchor {
        None => [Anchor::Min; 3],
        Some(AnchorArg::All(anchor)) => [anchor; 3],
        Some(AnchorArg::PerAxis(anchors)) => anchors,
    };
    resize_object(model, id, target, anchors)?;
    Ok(modified(&BTreeSet::from([id])))
}
/// Stretch an object to `target` (world extents), keeping the `anchors` side of each axis.
fn resize_object(model: &mut Model, id: u64, target: DVec3, anchors: [Anchor; 3]) -> Result<(), Fail> {
    let set = BTreeSet::from([id]);
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
    let grown = target - current;
    let shift = DVec3::from_array(std::array::from_fn(|i| match anchors[i] {
        Anchor::Min => 0.,
        Anchor::Center => -grown[i] / 2.,
        Anchor::Max => -grown[i],
    }));
    if shift.length() > 1.0e-12 {
        model.move_selection(&set, shift)?;
    }
    Ok(())
}
fn move_to(model: &mut Model, a: MoveToArgs) -> Res {
    let id = one_object(model, a.id)?;
    let set = BTreeSet::from([id]);
    let (lo, _) = model.selection_bounds(&set).ok_or("the object has no geometry")?;
    let delta = point(a.origin_mm)? - lo;
    if delta.length() > 1.0e-12 {
        model.move_selection(&set, delta)?;
    }
    Ok(modified(&set))
}
fn create_stock_material(model: &mut Model, env: &mut ToolEnv, a: StockArgs) -> Res {
    let name = checked_name(&a.name)?;
    let mut tenths = [0_u64; 3];
    for (slot, (value, what)) in tenths.iter_mut().zip([(a.thickness_mm, "thickness_mm"), (a.width_mm, "width_mm"), (a.length_mm, "length_mm")]) {
        let value = finite(value, what)?;
        if !(0.1..=100_000.).contains(&value) {
            return Err(fail(format!("{what} out of range"), "Use a size between 0.1 mm and 100 m."));
        }
        *slot = (value * 10.).round() as u64;
    }
    if let Some((id, _)) = model.stock_materials.iter().chain(env.catalog.iter()).find(|(_, m)| m.name == name && m.dimensions == tenths) {
        let mut done = Done::default();
        done.extra.insert("material_id".into(), json!(id));
        done.extra.insert("note".into(), json!("A material with this name and size already exists; reusing it."));
        return Ok(done);
    }
    let id = MaterialId::new_v4();
    let material = StockMaterial { name, dimensions: tenths };
    model.stock_materials.insert(id, material.clone());
    env.catalog.insert(id, material);
    let mut done = Done::default();
    done.extra.insert("material_id".into(), json!(id));
    Ok(done)
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
    let existing: BTreeSet<u64> = model.faces.keys().chain(model.edges.keys()).copied().collect();
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
    done.created = model.faces.keys().chain(model.edges.keys()).copied().filter(|id| !existing.contains(id)).collect();
    if let Some(owner) = owner.filter(|o| model.objects.contains_key(o)) {
        done.modified.push(owner);
    }
    if model.faces.contains_key(&new_face) {
        done.extra.insert("new_face_id".into(), json!(new_face));
        done.extra.insert("face_id".into(), json!(new_face));
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
    let set = objects(model, &a.ids, true)?;
    let direction = a.axis.map(|axis| axis.axis().vector());
    for id in &set {
        model.set_grain_direction(*id, direction)?;
    }
    Ok(modified(&set))
}
fn group(model: &mut Model, a: GroupArgs) -> Res {
    let mut set = entities(model, &a.ids, true)?;
    // A face without its edges would leave them loose and duplicate them in the group.
    let rings: Vec<Vec<u64>> = set
        .iter()
        .filter_map(|id| model.faces.get(id))
        .flat_map(|face| std::iter::once(face.vertices.clone()).chain(face.holes.clone()))
        .collect();
    if !rings.is_empty() {
        let mut sides = BTreeSet::new();
        for ring in &rings {
            for (i, a) in ring.iter().enumerate() {
                let b = ring[(i + 1) % ring.len()];
                sides.insert(((*a).min(b), (*a).max(b)));
            }
        }
        set.extend(
            model
                .edges
                .values()
                .filter(|e| sides.contains(&(e.a.min(e.b), e.a.max(e.b))))
                .map(|e| e.id)
                .filter(|id| model.entity_in_context(*id) == Some(*id) && !model.is_locked(*id)),
        );
    }
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
    let note = if done.extra["members"].as_array().is_some_and(Vec::is_empty) {
        "Made a new part from the faces and edges: it is created[0]. check_model tells whether it is a closed solid."
    } else {
        "Member ids may have changed; use the ids in members."
    };
    done.extra.insert("note".into(), json!(note));
    Ok(done)
}
fn ungroup(model: &mut Model, a: IdsArg) -> Res {
    let set = objects(model, &a.ids, true)?;
    let released = model.dissolve_objects(&set, false);
    let mut done = Done {
        modified: released.into_iter().filter(|id| model.contains_entity(id)).collect(),
        erased: set.iter().copied().filter(|id| !model.contains_entity(id)).collect(),
        ..Default::default()
    };
    // Ungrouping keeps the members' ids (grouping does not).
    done.extra.insert("members".into(), Value::Array(done.modified.iter().filter_map(|id| scene::object_entry(model, *id)).collect()));
    Ok(done)
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
    if let Some(joint) = a.ids.iter().find(|id| model.connections.contains_key(id)) {
        return Err(fail(
            format!("{joint} is a joint, not an entity"),
            "Use remove_joint to delete a joint. Erasing a board also removes its joints.",
        ));
    }
    let set = entities(model, &a.ids, true)?;
    let intact: Vec<u64> = model
        .connections
        .iter()
        .filter(|(_, c)| c.members.iter().all(|m| model.contains_entity(m)))
        .map(|(id, _)| *id)
        .collect();
    model.erase(&set);
    let erased: Vec<u64> = set.iter().copied().filter(|id| !model.contains_entity(id)).collect();
    if erased.is_empty() {
        return Err("nothing was erased".into());
    }
    // The core keeps joints whose boards are gone (BenchUp3D/editor#36);
    // remove the ones this call left without a board.
    let orphaned: Vec<u64> = intact
        .into_iter()
        .filter(|id| model.connections[id].members.iter().any(|m| !model.contains_entity(m)))
        .collect();
    for id in &orphaned {
        model.connections.remove(id);
    }
    let mut done = Done { erased, ..Default::default() };
    if !orphaned.is_empty() {
        done.extra.insert("removed_joints".into(), json!(orphaned));
    }
    Ok(done)
}
fn flip_joint(model: &mut Model, env: &ToolEnv, a: IdsArg) -> Res {
    if a.ids.is_empty() || a.ids.len() > MAX_IDS {
        return Err(fail("bad ids", format!("Pass between 1 and {MAX_IDS} joint ids.")));
    }
    let mut boards = BTreeSet::new();
    let mut orientations = Vec::new();
    for id in &a.ids {
        let Some(connection) = model.connections.get(id).cloned() else {
            return Err(fail(format!("{id} is not a joint"), "Use ids from the joints list in get_scene."));
        };
        let ConnectionOperation::Joinery(mut settings) = connection.operation else {
            return Err(fail(format!("{id} is not a wood joint"), "Only wood joints can be flipped."));
        };
        let members = [connection.members[1], connection.members[0]];
        for board in members {
            if !model.contains_entity(&board) {
                return Err(fail(format!("board {board} of joint {id} was erased"), "Remove the joint with remove_joint."));
            }
            if model.is_locked(board) {
                return Err(fail(format!("board {board} of joint {id} is locked"), "Locked entities cannot be changed; tell the designer to unlock it."));
            }
            if !leased(model, env, board) {
                return Err(fail("not-leased", "select it first so you hold the edit lease"));
            }
            boards.insert(board);
        }
        let supported = model.supported_joinery_axes(members, &settings);
        if let Some((d, w)) = preferred_axes(model, members, &settings).filter(|axes| supported.contains(axes)).or(supported.first().copied()) {
            (settings.depth_axis, settings.width_axis) = (d, w);
        }
        orientations.push(json!({"joint": id, "a": members[0], "b": members[1], "depth_axis": settings.depth_axis, "width_axis": settings.width_axis}));
        // The entry face belonged to the old first board.
        model.connections.insert(*id, Connection { members, entry_face: None, operation: ConnectionOperation::Joinery(settings) });
    }
    let errors = scene::joint_errors(model);
    if let Some((id, error)) = a.ids.iter().find_map(|id| errors.get(id).map(|e| (id, e))) {
        return Err(fail(format!("joint {id} cannot be cut once flipped: {error}"), "Check the overlap, or remove the joint and add it again with other params."));
    }
    let mut done = modified(&boards);
    done.extra.insert("flipped".into(), json!(orientations));
    Ok(done)
}
fn remove_joint(model: &mut Model, env: &ToolEnv, a: IdsArg) -> Res {
    if a.ids.is_empty() || a.ids.len() > MAX_IDS {
        return Err(fail("bad ids", format!("Pass between 1 and {MAX_IDS} joint ids.")));
    }
    let mut boards = BTreeSet::new();
    for id in &a.ids {
        let Some(connection) = model.connections.get(id) else {
            return Err(fail(format!("{id} is not a joint"), "Use ids from the joints list in get_scene."));
        };
        if !matches!(connection.operation, ConnectionOperation::Joinery(_)) {
            return Err(fail(format!("{id} is not a wood joint"), "Only wood joints can be removed with this tool."));
        }
        // A joint whose board was erased has nothing left to protect.
        for board in connection.members.iter().filter(|b| model.contains_entity(b)) {
            if model.is_locked(*board) {
                return Err(fail(format!("board {board} of joint {id} is locked"), "Locked entities cannot be changed; tell the designer to unlock it."));
            }
            if !leased(model, env, *board) {
                return Err(fail("not-leased", "select it first so you hold the edit lease"));
            }
            boards.insert(*board);
        }
    }
    for id in &a.ids {
        model.connections.remove(id);
    }
    let mut done = modified(&boards);
    done.extra.insert("removed_joints".into(), json!(a.ids));
    Ok(done)
}
/// The orientation a woodworker would pick, in the first board's axes. Depth
/// is the axis along which `a` goes into `b` (where the overlap is shortest
/// compared with `a`). Tongues, tenons and rabbets are spread across `a`'s
/// thickness; dovetails, fingers, laps and pockets across its broad side.
fn preferred_axes(model: &Model, members: [u64; 2], settings: &JointSettings) -> Option<(usize, usize)> {
    let overlap = model.joinery_options(members, settings).0?;
    let (lo, hi) = model.joinery_frame(members[0]).ok()?.bounds(model, members[0])?;
    let (board, shared) = (hi - lo, overlap.bounds.1 - overlap.bounds.0);
    let depth = (0..3).min_by(|x, y| (shared[*x] / board[*x]).total_cmp(&(shared[*y] / board[*y])))?;
    let (p, q) = ((depth + 1) % 3, (depth + 2) % 3);
    let (thin, broad) = if board[p] <= board[q] { (p, q) } else { (q, p) };
    let width = match settings.kind {
        JointKind::TongueGroove | JointKind::MortiseTenon | JointKind::Rabbet => thin,
        JointKind::Dovetail | JointKind::Finger | JointKind::HalfLap
        | JointKind::Biscuit | JointKind::Domino | JointKind::Groove | JointKind::LooseTongue => broad,
    };
    Some((depth, width))
}
/// Apply the optional joint settings an agent passed; true when it chose the axes.
fn apply_joint_params(settings: &mut JointSettings, p: JointParams) -> Result<bool, Fail> {
    let ratio = |value: Option<f64>, what: &str| -> Result<Option<f64>, Fail> {
        value
            .map(|v| {
                (v.is_finite() && (0.1..=0.9).contains(&v))
                    .then_some(v)
                    .ok_or_else(|| Fail::from(format!("{what} must be between 0.1 and 0.9")))
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
        if !(2..=16).contains(&v) {
            return Err("fingers must be between 2 and 16".into());
        }
        settings.fingers = v;
    }
    if let Some(v) = p.dovetails {
        if !(1..=16).contains(&v) {
            return Err("dovetails must be between 1 and 16".into());
        }
        settings.dovetails = v;
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
    if let Some(trim) = p.trim {
        settings.trim = match trim {
            TrimArg::None => TrimBoard::None,
            TrimArg::A => TrimBoard::First,
            TrimArg::B => TrimBoard::Second,
        };
    }
    if let Some(axis) = p.trim_axis {
        if axis > 2 {
            return Err("trim_axis is 0, 1 or 2".into());
        }
        settings.trim_axis = Some(axis);
    }
    if let Some(end) = p.keep_end {
        settings.keep_end = match end {
            KeepEndArg::Lower => KeepEnd::Lower,
            KeepEndArg::Upper => KeepEnd::Upper,
        };
    }
    Ok(p.depth_axis.is_some() || p.width_axis.is_some())
}
struct JointPair {
    a: u64,
    b: u64,
}
fn add_joint(model: &mut Model, a: JointArgs) -> Res {
    if a.a == a.b {
        return Err(fail("a and b must differ", "A joint connects two different boards."));
    }
    if model.contains_entity(&a.a) && model.contains_entity(&a.b) && model.parent(a.a) != model.parent(a.b) {
        return Err(fail(
            format!("{} and {} are in different groups", a.a, a.b),
            "A joint connects boards at the same level: a board inside a group or component instance cannot be joined \
             to one outside it. Join them before grouping (grouping keeps joints), or keep them as separate boards.",
        ));
    }
    let set = objects(model, &[a.a, a.b], true)?;
    for id in [a.a, a.b] {
        if let Err(error) = model.joinery_frame(id) {
            let hint = if matches!(model.objects[&id].kind, benchup_core::ObjectKind::Component { .. }) {
                "It is a component instance. The board inside it (get_object lists it) can only be joined to boards inside the same instance, \
                 not to boards outside it; make the joints before making the component."
            } else {
                "Join the boards themselves; get_object on a group lists its children."
            };
            return Err(fail(format!("{id} is not a single board: {error}"), hint));
        }
    }
    let mut settings = JointSettings { kind: a.kind.kind(), ..Default::default() };
    let chosen_axes = match a.params {
        Some(p) => apply_joint_params(&mut settings, p)?,
        None => false,
    };
    let mut done = commit_joint(model, [a.a, a.b], settings, chosen_axes, None)?;
    done.modified = set.into_iter().collect();
    Ok(done)
}

/// Pick the orientation, then add the joint (or replace joint `replace`) and
/// make sure it cuts; the error names the joints it collides with.
fn commit_joint(model: &mut Model, members: [u64; 2], mut settings: JointSettings, chosen_axes: bool, replace: Option<u64>) -> Res {
    let a = JointPair { a: members[0], b: members[1] };
    let orientations = |model: &Model, s: &JointSettings| -> String {
        let list: Vec<String> = model.supported_joinery_axes(members, s).iter().map(|(d, w)| format!("{{depth_axis:{d}, width_axis:{w}}}")).collect();
        if list.is_empty() { "none".into() } else { list.join(", ") }
    };
    if settings.depth_axis == settings.width_axis {
        return Err(fail("depth_axis and width_axis must differ", format!("Orientations that fit these boards: {}.", orientations(model, &settings))));
    }
    if model.joinery_options([a.a, a.b], &settings).0.is_none() {
        return Err(fail(
            "these boards do not overlap",
            "Make a enter b where they meet (e.g. a shelf 9 mm into a side); the overlap is the joint's depth.",
        ));
    }
    let supported = model.supported_joinery_axes([a.a, a.b], &settings);
    if !chosen_axes {
        if let Some(axes) = preferred_axes(model, members, &settings).filter(|axes| supported.contains(axes)) {
            (settings.depth_axis, settings.width_axis) = axes;
        }
    }
    if !supported.contains(&(settings.depth_axis, settings.width_axis)) {
        match supported.first() {
            Some((d, w)) if !chosen_axes => (settings.depth_axis, settings.width_axis) = (*d, *w),
            Some(_) => {
                return Err(fail(
                    "this orientation cannot cut these boards",
                    format!("Orientations that fit: {}.", orientations(model, &settings)),
                ));
            }
            None => {
                return Err(fail(
                    "no orientation of this joint fits these boards",
                    "Check the overlap and the params (a smaller clearance, fewer dovetails), or choose another kind.",
                ));
            }
        }
    }
    let connection = Connection { members: [a.a, a.b], entry_face: None, operation: ConnectionOperation::Joinery(settings.clone()) };
    let joint = match replace {
        Some(id) => {
            model.connections.insert(id, connection);
            id
        }
        None => model.add_connection(connection)?,
    };
    if let Some(error) = scene::joint_errors(model).remove(&joint) {
        // Only joints whose shared region touches this one's can be the cause.
        let region = |pair: [u64; 2]| -> Option<(DVec3, DVec3)> {
            let (a, b) = (model.selection_bounds(&BTreeSet::from([pair[0]]))?, model.selection_bounds(&BTreeSet::from([pair[1]]))?);
            Some((a.0.max(b.0), a.1.min(b.1)))
        };
        let near = |x: (DVec3, DVec3), y: (DVec3, DVec3)| (y.0 - x.1).max_element() <= 2.0e-3 && (x.0 - y.1).max_element() <= 2.0e-3;
        let mine = region(members);
        let others: Vec<String> = model
            .connections
            .iter()
            .filter(|(id, c)| **id != joint && c.members.iter().any(|m| members.contains(m)))
            .filter(|(_, c)| matches!((mine, region(c.members)), (Some(x), Some(y)) if near(x, y)))
            .map(|(id, c)| format!("{id} ({} between {} and {})", scene::joint_kind(c), c.members[0], c.members[1]))
            .collect();
        let shaped = members.iter().any(|m| model.expand_entities(&BTreeSet::from([*m])).iter().filter(|e| model.faces.contains_key(e)).count() > 6);
        let hint = if others.is_empty() && shaped {
            "No other joint touches this one; the board's own geometry is the problem: a shaped board with many small faces, \
             or vertices very close to the joint's planes. Change the number of segments of a curve (try 24 or 48), or move the joint a few millimetres."
                .to_owned()
        } else if others.is_empty() && (error.contains("closed solid") || error.contains("precision")) {
            // Nothing touches this joint and the boards are plain boxes: a
            // precision limit of the cutter, which small changes get round.
            let mut hint = "No other joint touches this one and the boards are plain boxes, so this is a precision limit of BenchUp's \
                 joint cutter, not a clash. A small change usually gets round it: height_ratio or width_ratio a little different \
                 (0.6 → 0.65), or one board moved by 0.5 to 1 mm. Try it in a dry run."
                .to_owned();
            let earlier: Vec<String> = model
                .connections
                .iter()
                .filter(|(id, c)| **id != joint && c.members.contains(&members[0]) && matches!(c.operation, ConnectionOperation::Joinery(_)))
                .map(|(id, _)| id.to_string())
                .collect();
            if !earlier.is_empty() {
                hint.push_str(&format!(
                    " Board {} already has joint {} elsewhere; adding this joint first and that one after also gets round it.",
                    members[0],
                    earlier.join(", ")
                ));
            }
            hint
        } else if others.is_empty() {
            format!("Orientations that fit: {}. Check the overlap and params.", orientations(model, &settings))
        } else {
            format!(
                "These boards already have joints {}. Joints on one board must not cross, touch or end in the same plane: \
                 keep 1 to 2 mm of wood between them (for example, stop a shelf short of the back).",
                others.join(", ")
            )
        };
        return Err(fail(format!("the joint cannot be cut: {error}"), hint));
    }
    let mut done = modified(&BTreeSet::from(members));
    done.joints.push(joint);
    done.extra.insert("joint_id".into(), json!(joint));
    done.extra.insert("a".into(), json!(a.a));
    done.extra.insert("b".into(), json!(a.b));
    done.extra.insert("orientation".into(), json!({"depth_axis": settings.depth_axis, "width_axis": settings.width_axis}));
    Ok(done)
}
fn add_dimension(model: &mut Model, a: DimensionArgs) -> Res {
    let axis = a.axis.as_ref().map(AxisArg::axis);
    let id = model.add_dimension_with_axis(point(a.start_mm)?, point(a.end_mm)?, point(a.offset_mm)?, axis)?;
    let mut done = Done { created: vec![id], ..Default::default() };
    if let Some(dimension) = model.dimensions.get(&id) {
        done.extra.insert("length_mm".into(), json!(scene::mm(dimension.length(model))));
        done.extra.insert("start_attached".into(), json!(!dimension.start.weights.is_empty()));
        done.extra.insert("end_attached".into(), json!(!dimension.end.weights.is_empty()));
    }
    Ok(done)
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
            let filter = more::scene_filter(model, parse(arguments)?)?;
            let shared = if env.hide_selection { BTreeSet::new() } else { env.selection.clone() };
            read(scene::summary_with(model, &shared, &env.catalog, scene::SCENE_BUDGET, &filter))
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
            let materials = scene::stock_list(model, &env.catalog);
            let mut out = json!({"materials": materials});
            if out["materials"].as_array().is_some_and(Vec::is_empty) {
                out["note"] = json!("No stock materials yet; add one with create_stock_material.");
            }
            read(out)
        }
        "check_model" => {
            let a: more::CheckArgs = parse(arguments)?;
            if a.only_new.unwrap_or(false) {
                return read(crate::review::check_new(model, env.before.as_ref()));
            }
            read(crate::review::check(model))
        }
        "get_cut_list" => {
            parse::<Empty>(arguments)?;
            read(crate::review::cut_list(model))
        }
        "get_guide" => {
            parse::<Empty>(arguments)?;
            read(json!({"guide": crate::GUIDE}))
        }
        "create_stock_material" => create_stock_material(model, env, parse(arguments)?),
        "move_to" => move_to(model, parse(arguments)?),
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
        "remove_joint" => remove_joint(model, env, parse(arguments)?),
        "flip_joint" => flip_joint(model, env, parse(arguments)?),
        "add_dimension" => add_dimension(model, parse(arguments)?),
        "add_text" => add_text(model, parse(arguments)?),
        "get_estimate" => more::get_estimate(model, parse(arguments)?),
        "measure_space" => more::measure_space(model, parse(arguments)?),
        "find_surfaces" => more::find_surfaces(model, parse(arguments)?),
        "create_profile_board" => more::create_profile_board(model, env, parse(arguments)?),
        "split_face" => more::split_face(model, parse(arguments)?),
        "merge_faces" => more::merge_faces(model, parse(arguments)?),
        "change_thickness" => more::change_thickness(model, env, parse(arguments)?),
        "distribute" => more::distribute(model, parse(arguments)?),
        "set_joint" => more::set_joint(model, env, parse(arguments)?),
        "add_profile" => more::add_profile(model, parse(arguments)?),
        "remove_profile" => more::remove_profile(model, parse(arguments)?),
        "set_finish" => more::set_finish(model, parse(arguments)?),
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
            // Shared project: entering a group the designer does not hold would
            // let later creating tools place geometry inside it.
            if model.contains_entity(&a.id) && !leased(model, env, a.id) {
                return Err(fail("not-leased", "select it first so you hold the edit lease"));
            }
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
const CREATING: &[&str] = &["create_board", "create_copy", "create_profile_board", "draw_polyline", "add_dimension", "add_text"];
/// Editing tools that open the group around their targets by themselves.
const AUTO_SCOPE: &[&str] = &[
    "move", "move_to", "rotate", "scale", "resize_board", "rename", "set_material", "set_grain",
    "group", "ungroup", "tag", "paint", "erase", "add_joint", "create_copy", "change_thickness", "distribute",
    "add_profile", "remove_profile", "set_finish", "merge_faces",
];

/// The group to open so that all `ids` are editable, if they share one that is
/// reachable from the current scope through unlocked, visible groups.
fn auto_scope(model: &Model, ids: &[u64]) -> Option<u64> {
    if ids.is_empty() || !ids.iter().all(|id| model.contains_entity(id)) {
        return None;
    }
    if ids.iter().all(|id| model.entity_in_context(*id) == Some(*id)) {
        return None;
    }
    let owner = model.parent(ids[0]).filter(|o| model.objects.contains_key(o))?;
    if ids.iter().any(|id| model.parent(*id) != Some(owner)) {
        return None;
    }
    let top = model.entity_in_context(owner)?;
    let mut current = Some(owner);
    for _ in 0..256 {
        let object = current?;
        if model.is_hidden(object) || model.is_locked(object) {
            return None;
        }
        if object == top {
            return Some(owner);
        }
        current = model.parent(object);
    }
    None
}

/// Existing entities a mutating call names, read from the raw arguments.
fn target_ids(name: &str, arguments: &str) -> Vec<u64> {
    if CREATING.contains(&name) {
        return Vec::new();
    }
    named_ids(arguments)
}

/// Every id the raw arguments name (ids, id, face_id, a, b).
fn named_ids(arguments: &str) -> Vec<u64> {
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

/// Safety net for shared projects: after a mutating call, every entity that
/// existed before and was changed or removed must have been leased (or created
/// this turn). Ownership is read from the pre-call model. Loose root geometry
/// has no owner, so it is never covered.
fn lease_respected(before: &Model, after: &Model, env: &ToolEnv) -> bool {
    let Some(lease) = &env.lease else { return true };
    let mut parent: BTreeMap<u64, u64> = BTreeMap::new();
    for object in before.objects.values() {
        for child in &object.entities {
            parent.insert(*child, object.id);
        }
    }
    let covered = |id: u64| -> bool {
        let mut current = Some(id);
        for _ in 0..256 {
            let Some(c) = current else { break };
            if lease.contains(&c) || env.created.contains(&c) {
                return true;
            }
            current = parent.get(&c).copied();
        }
        false
    };
    fn changed<T: PartialEq>(old: &BTreeMap<u64, T>, new: &BTreeMap<u64, T>) -> Vec<u64> {
        old.iter().filter(|(id, value)| new.get(id) != Some(*value)).map(|(id, _)| *id).collect()
    }
    let entity_ids = changed(&before.objects, &after.objects)
        .into_iter()
        .chain(changed(&before.faces, &after.faces))
        .chain(changed(&before.edges, &after.edges))
        .chain(changed(&before.dimensions, &after.dimensions))
        .chain(changed(&before.texts, &after.texts))
        .chain(changed(&before.guides, &after.guides))
        .chain(changed(&before.face_colors, &after.face_colors))
        .chain(changed(&before.entity_tags, &after.entity_tags));
    if !entity_ids.into_iter().all(&covered) {
        return false;
    }
    // Hidden / locked flags on existing entities.
    let flags = before.hidden.symmetric_difference(&after.hidden).chain(before.locked.symmetric_difference(&after.locked));
    if !flags.filter(|id| before.contains_entity(id)).all(|id| covered(*id)) {
        return false;
    }
    // A vertex is covered only when everything that uses it is.
    let moved: BTreeSet<u64> = changed(&before.vertices, &after.vertices).into_iter().collect();
    if !moved.is_empty() {
        let mut users: BTreeMap<u64, Vec<u64>> = BTreeMap::new();
        for edge in before.edges.values() {
            users.entry(edge.a).or_default().push(edge.id);
            users.entry(edge.b).or_default().push(edge.id);
        }
        for face in before.faces.values() {
            for v in face.vertices.iter().chain(face.holes.iter().flatten()) {
                users.entry(*v).or_default().push(face.id);
            }
        }
        for v in moved {
            match users.get(&v) {
                Some(list) if list.iter().all(|u| covered(*u)) => {}
                _ => return false,
            }
        }
    }
    changed(&before.connections, &after.connections)
        .into_iter()
        .all(|id| before.connections[&id].members.iter().all(|m| covered(*m)))
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
    // Creating tools write into the open group; it must be one the designer holds.
    if mutating
        && env.lease.is_some()
        && CREATING.contains(&name)
        && model.edit_context.is_some_and(|context| !leased(model, env, context))
    {
        return error_outcome("not-leased".into(), Some("select it first so you hold the edit lease".into()));
    }
    let backup = mutating.then(|| model.clone());
    // `catch_unwind` cannot help on wasm32, where a panic aborts. Every tool
    // therefore validates its arguments (counts, ranges, lengths) before it
    // touches the model, and the result is range-checked below.
    // Editing parts inside a group: open that group for this call only, as the
    // designer would, when every id shares it and the way there is editable.
    let saved_context = model.edit_context;
    let opened = mutating && AUTO_SCOPE.contains(&name) && {
        let owner = auto_scope(model, &named_ids(arguments));
        if let Some(owner) = owner {
            model.edit_context = Some(owner);
        }
        owner.is_some()
    };
    let attempt = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(model, env, name, arguments)));
    if opened {
        model.edit_context = saved_context;
    }
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
        Ok(_) if backup.as_ref().is_some_and(|b| env.lease.is_some() && !lease_respected(b, model, env)) => {
            *model = backup.unwrap_or_default();
            error_outcome("not-leased".into(), Some("select it first so you hold the edit lease".into()))
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

#[path = "ops_more.rs"]
mod more;

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "ops_more_tests.rs"]
mod more_tests;
