//! Tools from the second round of agent testing (BenchUp3D/benchup#42):
//! router profiles, joint settings and trimming, finishes and estimates,
//! boards from an outline with arcs, face operations, layout helpers and
//! queries that spare agents probing the model one entity at a time.
use super::*;
use benchup_core::{
    connections::ConnectionOperation as Operation,
    faces,
    router::{Profile, Settings as RouteSettings},
    surface_treatment::{self, Sheen, Step},
};
use std::f64::consts::FRAC_PI_2;

const MAX_ARC_SEGMENTS: u32 = 96;
const MAX_LISTED: usize = 100;
const AXES: [&str; 3] = ["x", "y", "z"];

// ---------------------------------------------------------------- schemas

fn face_ref() -> Value {
    json!({"oneOf": [{"type": "integer"}, {"type": "string", "enum": ["+x", "-x", "+y", "-y", "+z", "-z", "*"]}]})
}
fn step_schema() -> Value {
    json!({"type": "object", "properties": {
        "enabled": {"type": "boolean"},
        "coats": {"type": "integer", "minimum": 1, "maximum": 100},
        "minutes_per_m2": {"type": "number", "minimum": 0},
        "drying_hours": {"type": "number", "minimum": 0},
        "final_drying_hours": {"type": "number", "minimum": 0},
        "tint_rgb": {"type": "array", "items": {"type": "integer", "minimum": 0, "maximum": 255}, "minItems": 3, "maxItems": 3},
        "strength": {"type": "integer", "minimum": 0, "maximum": 100},
        "sheen": {"type": "string", "enum": ["matte", "satin", "gloss"]}},
        "additionalProperties": false})
}

pub(super) fn definitions() -> Vec<ToolDefinition> {
    let outline_point = json!({"oneOf": [
        {"type": "array", "items": {"type": "number"}, "minItems": 2, "maxItems": 2},
        {"type": "object", "properties": {
            "at": {"type": "array", "items": {"type": "number"}, "minItems": 2, "maxItems": 2},
            "bulge": {"type": "number"}, "sagitta_mm": {"type": "number"},
            "segments": {"type": "integer", "minimum": 2, "maximum": MAX_ARC_SEGMENTS}},
            "required": ["at"], "additionalProperties": false}]});
    let joint_params = json!({"type": "object", "properties": {
        "width_ratio": number(), "height_ratio": number(),
        "fingers": {"type": "integer"}, "dovetails": {"type": "integer"},
        "taper_degrees": number(), "clearance_mm": number(),
        "depth_axis": {"type": "integer", "minimum": 0, "maximum": 2},
        "width_axis": {"type": "integer", "minimum": 0, "maximum": 2},
        "trim": {"type": "string", "enum": ["none", "a", "b"]},
        "trim_axis": {"type": "integer", "minimum": 0, "maximum": 2},
        "keep_end": {"type": "string", "enum": ["lower", "upper"]}},
        "additionalProperties": false});
    vec![
        def("create_profile_board", "Create a shaped board in one call: a closed 2D outline (straight segments and arcs) on a plane, extruded to thickness_mm, grouped, named and optionally given a grain, material and tag. plane xy draws (u, v) = (x, y) and extrudes along +z; xz draws (x, z) and extrudes along +y (into the depth, from the front); yz draws (y, z) and extrudes along +x. origin_mm is where (0, 0) of the outline is; a negative thickness extrudes the other way. Points are [u, v] in mm, or {\"at\": [u, v], \"bulge\": b} or {\"at\": [u, v], \"sagitta_mm\": s} to make the segment from this point to the next an arc. bulge = tan(angle/4) (1 is a half circle); sagitta is how far the arc's middle is from the chord. Positive values bulge to the right of the direction of travel, which is outward on a counter-clockwise outline. Arcs get 12 segments per quarter turn (24 for a half circle) unless segments says otherwise. Returns the board in created[0].",
            json!({"name": text(), "plane": {"type": "string", "enum": ["xy", "xz", "yz"]}, "origin_mm": vec3(),
                "points": {"type": "array", "items": outline_point, "minItems": 3},
                "thickness_mm": number(), "grain_axis": axis_enum(),
                "material_id": {"type": "string", "format": "uuid"}, "tag": tag_name(false)}),
            &["name", "plane", "origin_mm", "points", "thickness_mm"]),
        def("split_face", "Split a face of a part in two along a line or a plane, without enter_object. line_mm is two points on the face; plane cuts the face where the plane crosses it (point_mm on the plane, normal an axis name or a vector). Returns the pieces in faces (id, center_mm, area) so the right one can be pushed or pulled, ordered from the plane's negative side to its positive side (for a line, along normal × line direction): with a plane normal of y, faces[0] is the front piece.",
            json!({"face_id": id(), "line_mm": {"type": "array", "items": vec3(), "minItems": 2, "maxItems": 2},
                "plane": {"type": "object", "properties": {"point_mm": vec3(),
                    "normal": {"oneOf": [axis_enum(), vec3()]}}, "required": ["point_mm", "normal"], "additionalProperties": false}}),
            &["face_id"]),
        def("merge_faces", "Merge flat faces of a board that lie in one plane and share an edge, removing the seam edges between them (erase would remove the faces too). Faces that surround an opening, such as the mouth of a mortise, become one face with a hole, and edges that meet another face's edge partway (T-junctions) are handled. Pass the board's id to merge every such pair, or ids of faces to merge only those. Not available inside components.",
            json!({"id": id(), "ids": ids()}), &[]),
        def("change_thickness", "Change the thickness of boards (their thinnest side) to thickness_mm, keeping the piece's outside size: with anchor auto (default), a board on the outside of the given boards keeps its outer face and the others keep their centre. Boards that go into a resized board with a joint (whichever is a or b) are lengthened or shortened so the joint keeps the same share of the new thickness, and boards that only butt against a resized board's face follow that face (joint_adjustments, butt_adjustments). material_id optionally assigns a material. Boards must be square to the axes. anchor applies to every board of the call: call it twice for different anchors.",
            json!({"ids": ids(), "thickness_mm": number(), "anchor": {"type": "string", "enum": ["auto", "min", "center", "max"]},
                "material_id": {"type": "string", "format": "uuid"}}), &["ids", "thickness_mm"]),
        def("distribute", "Spread objects evenly along an axis between from_mm and to_mm (shelves between a bottom and a top). mode gaps (default) makes the free gaps equal; centers spaces their centres equally. ends gap (default) leaves a gap at each limit too; flush puts the first object at from_mm and the last against to_mm. Objects keep their order along the axis; joints follow them.",
            json!({"ids": ids(), "axis": axis_enum(), "from_mm": number(), "to_mm": number(),
                "mode": {"type": "string", "enum": ["gaps", "centers"]}, "ends": {"type": "string", "enum": ["gap", "flush"]}}),
            &["ids", "axis", "from_mm", "to_mm"]),
        def("set_joint", "Change an existing joint: its kind and any params of add_joint, including trimming (trim a or b, keep_end, trim_axis). Unchanged params keep their values; the orientation is picked again when the kind changes and no axes are given. The joint must still cut.",
            json!({"id": id(), "kind": {"type": "string", "enum": ["mortise_tenon", "half_lap", "tongue_groove", "rabbet", "dovetail", "finger"]},
                "params": joint_params}), &["id"]),
        def("add_profile", "Rout a profile along edges of a board with BenchUp's Router: chamfer, bevel, roundover, cove, rabbet, groove, ogee or bead. Choose edges with between: pairs of faces the edge lies between, each a direction (+x, -y...), a face id from get_object, or * for any face; [[\"+z\", \"-y\"]] is the top front edge and [[\"+z\", \"*\"]] every edge of the top. Or pass edge ids in edges. width_mm is the width (chamfer, bevel, rabbet, groove) or radius (roundover, cove, ogee, bead), default 5; depth_mm (rabbet, groove; default 3); offset_mm (groove setback, bead shoulder; default 2); angle_degrees (bevel, 1-89; default 30). For a rabbet or groove, which of the two faces gets width_mm and which depth_mm depends on the edge: check it (check_model reports a part that should sit in the rabbet as an overlap if it went the wrong way) and pass reverse: true to swap them. Profiles other than chamfer and bevel need square edges. The profile is cut when BenchUp evaluates the model; the call fails if it cannot be cut. get_object lists a board's profiles.",
            json!({"id": id(), "between": {"type": "array", "items": {"type": "array", "items": face_ref(), "minItems": 2, "maxItems": 2}},
                "edges": ids(), "profile": {"type": "string", "enum": ["chamfer", "bevel", "roundover", "cove", "rabbet", "groove", "ogee", "bead"]},
                "width_mm": number(), "depth_mm": number(), "offset_mm": number(), "angle_degrees": number(), "reverse": {"type": "boolean"}}),
            &["id", "profile"]),
        def("remove_profile", "Remove router profiles from a board: the given edges, or all of the board's profiles when edges is omitted.",
            json!({"id": id(), "edges": ids()}), &["id"]),
        def("set_finish", "Set the sanding and finishing of objects (boards; a group sets every board in it), used by get_estimate and BenchUp's Time & Cost Estimates. sanding_grits lists the grits to sand with (e.g. [120, 180, 240]); pre_stain, stain and varnish switch a stage on with its coats, minutes_per_m2, drying hours and look (tint_rgb, strength 0-100, sheen). Omitted values keep their current or default values; the defaults are sanding 16 min/m² per grit; pre_stain 5 min/m², 1 coat, 0.25 h drying; stain 5 min/m², 1 coat, 8 h; varnish 6 min/m² per coat, 3 coats, 6 h between coats and 24 h after the last (final_drying_hours). The area is every face of each board. clear: true goes back to the project defaults.",
            json!({"ids": ids(), "clear": {"type": "boolean"},
                "sanding_grits": {"type": "array", "items": {"type": "integer", "minimum": 24, "maximum": 3000}},
                "pre_stain": step_schema(), "stain": step_schema(), "varnish": step_schema()}), &["ids"]),
        def("get_estimate", "Time estimate for building the boards (all, or those in ids): finishing time per stage from each board's surface area and finish (set_finish), joint cutting by kind and router profiles by length, with BenchUp's default allowances; and the wood by material (area, volume, pieces). Rates and prices edited in the app are not stored in the project, so costs are not included.",
            json!({"ids": ids()}), &[]),
        def("measure_space", "Measure the free space around a point: along each axis, the distance to the nearest face in both directions, what it belongs to, and the free gap between them (e.g. the clear height between two shelves, or the room left to a wall). Walls and floors made of plain faces count too. inside lists boards that contain the point.",
            json!({"point_mm": vec3(), "axes": {"type": "array", "items": axis_enum()}}), &["point_mm"]),
        def("find_surfaces", "List large flat faces that are not part of a closed board, such as the walls and floor of a room drawn as plain faces inside groups, with their direction, plane position, bounds and owner. Largest first. include_solids also lists faces of closed parts (a solid floor or a table top).",
            json!({"min_area_mm2": number(), "include_solids": {"type": "boolean"}, "facing": {"type": "string", "enum": ["+x", "-x", "+y", "-y", "+z", "-z"]},
                "limit": {"type": "integer", "minimum": 1, "maximum": MAX_LISTED}}), &[]),
    ]
}

// ---------------------------------------------------------------- scene filters

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SceneArgs {
    ids: Option<Vec<u64>>,
    depth: Option<usize>,
    fields: Option<Vec<String>>,
    sections: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CheckArgs {
    pub only_new: Option<bool>,
}

fn object_depth(model: &Model, id: u64) -> usize {
    let mut depth = 0;
    let mut current = model.parent(id);
    while let Some(parent) = current.filter(|p| model.objects.contains_key(p)) {
        depth += 1;
        if depth > 256 {
            break;
        }
        current = model.parent(parent);
    }
    depth
}

pub(super) fn scene_filter(model: &Model, a: SceneArgs) -> Result<scene::Filter, Fail> {
    let check = |list: &Option<Vec<String>>, allowed: &[&str], what: &str| -> Result<Option<BTreeSet<String>>, Fail> {
        let Some(list) = list else { return Ok(None) };
        if let Some(bad) = list.iter().find(|f| !allowed.contains(&f.as_str())) {
            return Err(fail(format!("unknown {what} {bad}"), format!("Choose from: {}.", allowed.join(", "))));
        }
        Ok(Some(list.iter().cloned().collect()))
    };
    let fields = check(&a.fields, scene::OBJECT_FIELDS, "field")?;
    // The output names the stock section stock_catalog; accept either name.
    let sections = a.sections.map(|list| list.into_iter().map(|s| if s == "stock_catalog" { "stock".to_owned() } else { s }).collect());
    let sections = check(&sections, scene::SECTIONS, "section")?;
    let objects = match (a.ids, a.depth) {
        (None, None) => None,
        (None, Some(depth)) => Some(model.objects.keys().copied().filter(|id| object_depth(model, *id) <= depth).collect()),
        (Some(ids), depth) => {
            if ids.len() > MAX_IDS {
                return Err(fail("too many ids", format!("Pass at most {MAX_IDS} ids per call.")));
            }
            if let Some(id) = ids.iter().find(|id| !model.objects.contains_key(id)) {
                return Err(fail(format!("{id} is not a group or component"), "Use object ids from get_scene or find_objects."));
            }
            let mut list = Vec::new();
            let mut seen = BTreeSet::new();
            for id in ids {
                let base = object_depth(model, id);
                let inside = model.expand_entities(&BTreeSet::from([id]));
                for object in std::iter::once(id).chain(inside.into_iter().filter(|e| model.objects.contains_key(e) && *e != id)) {
                    let relative = object_depth(model, object).saturating_sub(base);
                    if depth.is_none_or(|d| relative <= d) && seen.insert(object) {
                        list.push(object);
                    }
                }
            }
            Some(list)
        }
    };
    Ok(scene::Filter { objects, fields, sections })
}

// ---------------------------------------------------------------- shaped boards

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum PlaneArg {
    Xy,
    Xz,
    Yz,
}
#[derive(Deserialize)]
#[serde(untagged)]
enum OutlinePoint {
    Plain([f64; 2]),
    Arc(ArcPoint),
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ArcPoint {
    at: [f64; 2],
    bulge: Option<f64>,
    sagitta_mm: Option<f64>,
    segments: Option<u32>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ProfileBoardArgs {
    name: String,
    plane: PlaneArg,
    origin_mm: V3,
    points: Vec<OutlinePoint>,
    thickness_mm: f64,
    grain_axis: Option<AxisArg>,
    material_id: Option<MaterialId>,
    tag: Option<String>,
}

/// Points between `p` and `q` (both excluded) on the arc whose middle is
/// `sagitta` to the right of the chord. Millimetres, in the outline's plane.
fn arc_points(p: [f64; 2], q: [f64; 2], sagitta: f64, segments: Option<u32>) -> Result<Vec<[f64; 2]>, Fail> {
    let (dx, dy) = (q[0] - p[0], q[1] - p[1]);
    let chord = dx.hypot(dy);
    if chord < 1.0e-6 {
        return Err(fail("an arc joins two points at the same place", "Give each arc a start and an end that differ."));
    }
    let half = chord / 2.;
    if sagitta.abs() < 1.0e-9 {
        return Ok(Vec::new());
    }
    // The right of the travel direction (dx, dy) is (dy, -dx).
    let right = [dy / chord, -dx / chord];
    let mid = [(p[0] + q[0]) / 2., (p[1] + q[1]) / 2.];
    let radius = (half * half + sagitta * sagitta) / (2. * sagitta.abs());
    let apex = [mid[0] + right[0] * sagitta, mid[1] + right[1] * sagitta];
    let side = sagitta.signum();
    let center = [apex[0] - right[0] * side * radius, apex[1] - right[1] * side * radius];
    let sweep = 4. * (sagitta / half).atan();
    let count = match segments {
        Some(n) => n,
        // An even count puts a vertex at the arc's middle, where it reaches furthest.
        None => (((sweep.abs() / FRAC_PI_2 * 6.).round() as u32) * 2).max(2),
    };
    if !(2..=MAX_ARC_SEGMENTS).contains(&count) {
        return Err(fail("too many arc segments", format!("Use between 2 and {MAX_ARC_SEGMENTS} segments per arc.")));
    }
    let start = (p[1] - center[1]).atan2(p[0] - center[0]);
    Ok((1..count)
        .map(|k| {
            let angle = start + sweep * f64::from(k) / f64::from(count);
            [center[0] + radius * angle.cos(), center[1] + radius * angle.sin()]
        })
        .collect())
}

/// How an outline segment curves: by bulge (relative to the chord) or sagitta (mm).
#[derive(Clone, Copy)]
enum Curve {
    Bulge(f64),
    Sagitta(f64),
}

/// The closed outline in the plane, arcs expanded, and how many arcs it has.
fn outline(points: &[OutlinePoint]) -> Result<(Vec<[f64; 2]>, usize), Fail> {
    let corners: Vec<([f64; 2], Option<Curve>, Option<u32>)> = points
        .iter()
        .map(|point| match point {
            OutlinePoint::Plain(at) => Ok((*at, None, None)),
            OutlinePoint::Arc(arc) => {
                let curve = match (arc.bulge, arc.sagitta_mm) {
                    (Some(_), Some(_)) => return Err(fail("give either bulge or sagitta_mm", "An arc is set by one of them.")),
                    (Some(bulge), None) => Some(Curve::Bulge(bulge)),
                    (None, Some(sagitta)) => Some(Curve::Sagitta(sagitta)),
                    (None, None) => None,
                };
                Ok((arc.at, curve, arc.segments))
            }
        })
        .collect::<Result<_, Fail>>()?;
    let mut out = Vec::new();
    let mut arcs = 0;
    for (i, (at, curve, segments)) in corners.iter().enumerate() {
        if at.iter().any(|v| !v.is_finite() || v.abs() > MAX_COORDINATE_MM) {
            return Err(fail("coordinate out of range", "Use finite values within 100000 mm."));
        }
        out.push(*at);
        let Some(curve) = curve else { continue };
        let next = corners[(i + 1) % corners.len()].0;
        let half = (next[0] - at[0]).hypot(next[1] - at[1]) / 2.;
        let sagitta = match *curve {
            Curve::Bulge(bulge) => {
                if !bulge.is_finite() || bulge.abs() > 10. {
                    return Err(fail("bulge out of range", "Use a bulge between -10 and 10 (1 is a half circle)."));
                }
                bulge * half
            }
            Curve::Sagitta(sagitta) => finite(sagitta, "sagitta_mm")?,
        };
        let points = arc_points(*at, next, sagitta, *segments)?;
        arcs += usize::from(!points.is_empty());
        out.extend(points);
    }
    Ok((out, arcs))
}

pub(super) fn create_profile_board(model: &mut Model, env: &ToolEnv, a: ProfileBoardArgs) -> Res {
    let name = checked_name(&a.name)?;
    if a.points.len() < 3 {
        return Err(fail("points needs at least 3 corners", "Give the outline's corners in order."));
    }
    if a.points.len() > MAX_POINTS {
        return Err(fail("too many points", format!("Pass at most {MAX_POINTS} points.")));
    }
    let thickness = finite(a.thickness_mm, "thickness_mm")?;
    if !(0.1..=MAX_COORDINATE_MM).contains(&thickness.abs()) {
        return Err(fail("thickness_mm out of range", "Use a thickness between 0.1 mm and 100000 mm (negative extrudes the other way)."));
    }
    if let Some(id) = a.material_id {
        find_material(model, env, id)?;
    }
    let (flat, arcs) = outline(&a.points)?;
    if flat.len() > MAX_POINTS * 4 {
        return Err(fail("the outline has too many points", "Use fewer arcs or fewer segments per arc."));
    }
    // Drop repeated points; reject outlines with no area.
    let mut ring: Vec<[f64; 2]> = Vec::with_capacity(flat.len());
    for p in flat {
        if ring.last().is_none_or(|q: &[f64; 2]| (p[0] - q[0]).hypot(p[1] - q[1]) > 1.0e-4) {
            ring.push(p);
        }
    }
    while ring.len() > 1 && (ring[0][0] - ring[ring.len() - 1][0]).hypot(ring[0][1] - ring[ring.len() - 1][1]) <= 1.0e-4 {
        ring.pop();
    }
    let area: f64 = (0..ring.len()).map(|i| {
        let (p, q) = (ring[i], ring[(i + 1) % ring.len()]);
        p[0] * q[1] - q[0] * p[1]
    }).sum::<f64>() / 2.;
    if ring.len() < 3 || area.abs() < 1.0 {
        return Err(fail("the outline has no area", "Give at least three corners that do not lie on one line."));
    }
    let origin = point(a.origin_mm)?;
    let (u, v, normal) = match a.plane {
        PlaneArg::Xy => (DVec3::X, DVec3::Y, DVec3::Z),
        PlaneArg::Xz => (DVec3::X, DVec3::Z, DVec3::Y),
        PlaneArg::Yz => (DVec3::Y, DVec3::Z, DVec3::X),
    };
    let mut corners: Vec<DVec3> = ring.iter().map(|p| origin + u * from_mm(p[0]) + v * from_mm(p[1])).collect();
    if corners.iter().any(|p| p.abs().max_element() > MAX_RESULT_M) {
        return Err(fail("coordinate out of range", "Keep the outline within 100 m of the origin."));
    }
    corners.push(corners[0]);
    let mut drawing = Model::default();
    drawing.draw_polyline(&corners, false).map_err(|e| fail(format!("the outline could not be drawn: {e}"), "Check that the outline does not cross itself."))?;
    let faces: Vec<u64> = drawing.faces.keys().copied().collect();
    let [face] = faces[..] else {
        return Err(fail("the outline does not make one face", "Check that the outline is closed and does not cross or touch itself."));
    };
    drawing.push_pull_along(face, normal * from_mm(thickness))?;
    let roots = model.paste_entities(&drawing, DVec3::ZERO);
    let object = model
        .make_object(&roots, false, &name)
        .ok_or_else(|| fail("the board could not be created", "The current editing scope may be locked; try exit_to_root."))?;
    if let Err(reason) = model.solid_volume(object) {
        return Err(fail(format!("the outline did not make a closed board: {reason}"), "Check that the outline does not cross itself, and use fewer, longer segments."));
    }
    if a.material_id.is_some() {
        assign_material(model, env, object, a.material_id)?;
    }
    if let Some(axis) = a.grain_axis {
        model.set_grain_direction(object, Some(axis.axis().vector()))?;
    }
    if let Some(tag) = a.tag.as_deref().filter(|t| !t.trim().is_empty()) {
        let tag = tag_named(model, tag)?;
        model.assign_tag(&BTreeSet::from([object]), Some(tag))?;
    }
    let mut done = Done { created: vec![object], ..Default::default() };
    done.extra.insert("outline_points".into(), json!(ring.len()));
    done.extra.insert("arcs".into(), json!(arcs));
    done.extra.insert("area_mm2".into(), json!(area.abs().round()));
    Ok(done)
}

// ---------------------------------------------------------------- faces

/// Checks that a face may be edited and returns the object that owns it.
fn editable_face(model: &Model, face: u64) -> Result<Option<u64>, Fail> {
    if !model.faces.contains_key(&face) {
        return Err(fail(format!("{face} is not a face"), "Use a face id from get_object."));
    }
    if model.is_hidden(face) || model.is_locked(face) {
        return Err(fail(format!("face {face} is hidden or locked"), "Pick another face."));
    }
    let top = model
        .entity_in_context(face)
        .ok_or_else(|| fail(format!("face {face} is not editable in the current scope"), unreachable_hint(model, face)))?;
    entities(model, &[top], true)?;
    let mut ancestor = model.parent(face);
    while let Some(owner) = ancestor {
        if model.is_hidden(owner) || model.is_locked(owner) {
            return Err(fail(format!("face {face} is inside hidden or locked object {owner}"), "Locked entities cannot be changed; tell the designer to unlock it."));
        }
        ancestor = model.parent(owner);
    }
    Ok(model.parent(face).filter(|o| model.objects.contains_key(o)))
}

/// Run `edit` with the face owner's scope open, as the designer would.
fn in_scope<T>(model: &mut Model, owner: Option<u64>, edit: impl FnOnce(&mut Model) -> T) -> T {
    let saved = model.edit_context;
    if owner.is_some() {
        model.edit_context = owner;
    }
    let result = edit(model);
    model.edit_context = saved;
    result
}

#[derive(Deserialize)]
#[serde(untagged)]
enum NormalArg {
    Axis(AxisArg),
    Vector(V3),
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PlaneCut {
    point_mm: V3,
    normal: NormalArg,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SplitArgs {
    face_id: u64,
    line_mm: Option<[V3; 2]>,
    plane: Option<PlaneCut>,
}

/// Segments where a plane crosses a face, inside it.
fn plane_segments(model: &Model, face: u64, origin: DVec3, normal: DVec3) -> Vec<(DVec3, DVec3)> {
    let Some(f) = model.faces.get(&face) else { return Vec::new() };
    let Some(face_plane) = faces::plane(model, &f.vertices) else { return Vec::new() };
    let direction = normal.cross(face_plane.normal);
    if direction.length() < 1.0e-9 {
        return Vec::new();
    }
    let direction = direction.normalize();
    let eps = 1.0e-9;
    let mut hits: Vec<DVec3> = Vec::new();
    for ring in std::iter::once(&f.vertices).chain(&f.holes) {
        for (i, a) in ring.iter().enumerate() {
            let (pa, pb) = (model.vertices[a].position, model.vertices[&ring[(i + 1) % ring.len()]].position);
            let (da, db) = (normal.dot(pa - origin), normal.dot(pb - origin));
            if da.abs() <= eps {
                hits.push(pa);
            } else if (da < -eps && db > eps) || (da > eps && db < -eps) {
                hits.push(pa + (pb - pa) * (da / (da - db)));
            }
        }
    }
    hits.sort_by(|a, b| direction.dot(*a).total_cmp(&direction.dot(*b)));
    hits.dedup_by(|a, b| a.distance(*b) < 1.0e-7);
    hits.windows(2)
        .filter(|pair| pair[0].distance(pair[1]) > 1.0e-6 && model.face_contains(f, (pair[0] + pair[1]) / 2., false))
        .map(|pair| (pair[0], pair[1]))
        .collect()
}

pub(super) fn split_face(model: &mut Model, a: SplitArgs) -> Res {
    let owner = editable_face(model, a.face_id)?;
    let face_normal = scene::outward_normal(model, a.face_id).unwrap_or(DVec3::Z);
    let order_axis;
    let segments = match (a.line_mm, a.plane) {
        (Some([p, q]), None) => {
            let (p, q) = (point(p)?, point(q)?);
            order_axis = face_normal.cross(q - p).normalize_or_zero();
            vec![(p, q)]
        }
        (None, Some(cut)) => {
            let normal = match cut.normal {
                NormalArg::Axis(axis) => axis.axis().vector(),
                NormalArg::Vector(v) => {
                    let v = DVec3::from_array(v);
                    if !v.is_finite() || v.length() < 1.0e-9 {
                        return Err(fail("normal must be a non-zero vector", "Use an axis name or a direction such as [0, 1, 0]."));
                    }
                    v.normalize()
                }
            };
            order_axis = normal;
            let segments = plane_segments(model, a.face_id, point(cut.point_mm)?, normal);
            if segments.is_empty() {
                return Err(fail("the plane does not cross the face", "Check the plane's position with the face's center_mm and area in get_object."));
            }
            segments
        }
        _ => return Err(fail("give either line_mm or plane", "line_mm is two points on the face; plane is {point_mm, normal}.")),
    };
    let before: BTreeSet<u64> = model.faces.keys().chain(model.edges.keys()).copied().collect();
    in_scope(model, owner, |model| -> Result<(), Fail> {
        for (p, q) in &segments {
            model.draw_polyline(&[*p, *q], true)?;
        }
        Ok(())
    })?;
    let created: Vec<u64> = model.faces.keys().chain(model.edges.keys()).copied().filter(|id| !before.contains(id)).collect();
    let new_faces: Vec<u64> = created.iter().copied().filter(|id| model.faces.contains_key(id)).collect();
    if new_faces.is_empty() {
        return Err(fail("the face was not split", "The line must cross the face from edge to edge, lying on it."));
    }
    // The original face keeps its id when it survives as one of the pieces.
    let mut pieces: Vec<u64> = std::iter::once(a.face_id).filter(|f| model.faces.contains_key(f)).chain(new_faces.iter().copied()).collect();
    // From the negative to the positive side of the plane (or of the line,
    // turning from the face's normal towards the line's direction).
    let centre = |f: &u64| model.faces.get(f).and_then(|face| model.face_center(face)).map_or(0., |c| c.dot(order_axis));
    pieces.sort_by(|x, y| centre(x).total_cmp(&centre(y)));
    let mut done = Done { created, ..Default::default() };
    done.modified.extend(owner);
    done.extra.insert("face_ids".into(), json!(pieces));
    done.extra.insert("faces".into(), Value::Array(pieces.iter().filter_map(|f| scene::face_entry(model, *f)).collect()));
    Ok(done)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MergeArgs {
    id: Option<u64>,
    ids: Option<Vec<u64>>,
}

fn ring_edges(ring: &[u64]) -> impl Iterator<Item = (u64, u64)> + '_ {
    (0..ring.len()).map(move |i| (ring[i], ring[(i + 1) % ring.len()]))
}

/// The boundary of two faces that share edges: the outer loop, the loops
/// left inside it (holes, such as the mouth of a mortise the two faces
/// surround), and the vertex pairs of the seam edges removed. `None` when
/// they share no edge or the boundary branches (they touch at a point).
struct Merged {
    outer: Vec<u64>,
    holes: Vec<Vec<u64>>,
    shared: Vec<(u64, u64)>,
}
fn merged_ring(first: &[u64], second: &[u64], position: &dyn Fn(u64) -> DVec3, normal: DVec3) -> Option<Merged> {
    let mut directed: Vec<(u64, u64)> = ring_edges(first).collect();
    let mut shared = Vec::new();
    for (a, b) in ring_edges(second) {
        if let Some(i) = directed.iter().position(|e| *e == (b, a)) {
            directed.remove(i);
            shared.push((a.min(b), a.max(b)));
        } else {
            directed.push((a, b));
        }
    }
    if shared.is_empty() || directed.is_empty() {
        return None;
    }
    let mut next: BTreeMap<u64, u64> = BTreeMap::new();
    for (a, b) in &directed {
        if next.insert(*a, *b).is_some() {
            return None;
        }
    }
    let mut left: BTreeSet<u64> = next.keys().copied().collect();
    let mut loops = Vec::new();
    while let Some(start) = left.pop_first() {
        let mut ring = vec![start];
        let mut current = *next.get(&start)?;
        while current != start {
            if !left.remove(&current) {
                return None;
            }
            ring.push(current);
            current = *next.get(&current)?;
        }
        loops.push(ring);
    }
    let area = |ring: &Vec<u64>| -> f64 {
        (0..ring.len()).map(|i| position(ring[i]).cross(position(ring[(i + 1) % ring.len()])).dot(normal)).sum::<f64>().abs()
    };
    let outer = (0..loops.len()).max_by(|x, y| area(&loops[*x]).total_cmp(&area(&loops[*y])))?;
    let outer = loops.remove(outer);
    Some(Merged { outer, holes: loops, shared })
}

/// `ring` with the vertices of `other` that lie inside its edges added, so
/// that two faces meeting at a T-junction (one edge against two) share
/// edges vertex for vertex.
fn with_t_vertices(ring: &[u64], other: &[u64], position: &dyn Fn(u64) -> DVec3) -> Vec<u64> {
    let mut out = Vec::with_capacity(ring.len());
    for (a, b) in ring_edges(ring) {
        out.push(a);
        let (pa, pb) = (position(a), position(b));
        let delta = pb - pa;
        let length = delta.length_squared();
        if length < 1.0e-18 {
            continue;
        }
        let mut inside: Vec<(f64, u64)> = other
            .iter()
            .copied()
            .filter(|v| *v != a && *v != b && !ring.contains(v))
            .filter_map(|v| {
                let t = (position(v) - pa).dot(delta) / length;
                ((1.0e-9..1. - 1.0e-9).contains(&t) && (pa + delta * t).distance(position(v)) < 1.0e-8).then_some((t, v))
            })
            .collect();
        inside.sort_by(|x, y| x.0.total_cmp(&y.0));
        inside.dedup_by_key(|(_, v)| *v);
        out.extend(inside.into_iter().map(|(_, v)| v));
    }
    out
}

pub(super) fn merge_faces(model: &mut Model, a: MergeArgs) -> Res {
    // One object in ids means that board, as id would.
    let (id, ids) = match (a.id, a.ids) {
        (None, Some(ids)) if ids.len() == 1 && model.objects.contains_key(&ids[0]) => (Some(ids[0]), None),
        other => other,
    };
    let (board, candidates): (u64, BTreeSet<u64>) = match (id, ids) {
        (Some(id), None) => {
            let board = one_object(model, id)?;
            let faces = model.objects[&board].entities.iter().copied().filter(|e| model.faces.contains_key(e)).collect();
            (board, faces)
        }
        (None, Some(ids)) => {
            if ids.len() < 2 || ids.len() > MAX_IDS {
                return Err(fail("pass at least two face ids", "Or pass the board's id to merge all its flat seams."));
            }
            let mut owner = None;
            for face in &ids {
                let parent = editable_face(model, *face)?;
                if owner.is_some() && owner != parent {
                    return Err(fail("the faces belong to different parts", "Merge faces of one board at a time."));
                }
                owner = parent;
            }
            let board = owner.ok_or_else(|| fail("the faces are loose geometry", "Merge faces of a board (group them first)."))?;
            (board, ids.into_iter().collect())
        }
        _ => return Err(fail("give either id or ids", "id is a board; ids are faces of one board.")),
    };
    let mut path = Some(board);
    while let Some(object) = path {
        if matches!(model.objects.get(&object).map(|o| &o.kind), Some(benchup_core::ObjectKind::Component { .. })) {
            return Err(fail("faces inside components cannot be merged by this tool", "Merge the faces in BenchUp, or explode the component first."));
        }
        if model.is_locked(object) {
            return Err(fail(format!("object {object} is locked"), "Locked entities cannot be changed; tell the designer to unlock it."));
        }
        path = model.parent(object);
    }
    let plane_of = |model: &Model, face: u64| model.faces.get(&face).and_then(|f| faces::plane(model, &f.vertices));
    let mut faces_left: BTreeSet<u64> = candidates;
    let mut removed_faces = Vec::new();
    let mut removed_edges = BTreeSet::new();
    let mut merged = Vec::new();
    'search: loop {
        let list: Vec<u64> = faces_left.iter().copied().collect();
        for (i, f1) in list.iter().enumerate() {
            for f2 in &list[i + 1..] {
                let (Some(p1), Some(p2)) = (plane_of(model, *f1), plane_of(model, *f2)) else { continue };
                if p1.normal.dot(p2.normal).abs() < 1. - 1.0e-9 || p1.normal.dot(p2.origin - p1.origin).abs() > 1.0e-7 {
                    continue;
                }
                let (first, mut second) = (model.faces[f1].vertices.clone(), model.faces[f2].vertices.clone());
                if p1.normal.dot(p2.normal) < 0. {
                    second.reverse();
                }
                let position = |v: u64| model.vertices.get(&v).map_or(DVec3::ZERO, |v| v.position);
                let (first_t, second_t) = (with_t_vertices(&first, &second, &position), with_t_vertices(&second, &first, &position));
                let Some(Merged { outer: ring, holes: inner, mut shared }) = merged_ring(&first_t, &second_t, &position, p1.normal) else { continue };
                // Edges that only the faces' old outlines used (an edge split at a T-junction) go too.
                shared.extend(ring_edges(&first).chain(ring_edges(&second)).map(|(a, b)| (a.min(b), a.max(b))));
                shared.sort_unstable();
                shared.dedup();
                let holes: Vec<Vec<u64>> = model.faces[f1].holes.iter().chain(&model.faces[f2].holes).cloned().chain(inner).collect();
                let color = model.face_colors.get(f1).or_else(|| model.face_colors.get(f2)).copied();
                let target = model.faces.get_mut(f1).ok_or("missing face")?;
                target.vertices = ring;
                target.holes = holes;
                if let Some(color) = color {
                    model.face_colors.insert(*f1, color);
                }
                model.faces.remove(f2);
                model.face_colors.remove(f2);
                model.wood_glue.remove(f2);
                model.entity_tags.remove(f2);
                for object in model.objects.values_mut() {
                    object.entities.remove(f2);
                }
                faces_left.remove(f2);
                removed_faces.push(*f2);
                merged.push(json!([f1, f2]));
                // The seam edges no face uses any more.
                for (a, b) in shared {
                    let still_used = model.faces.values().any(|f| {
                        std::iter::once(&f.vertices).chain(&f.holes).any(|r| ring_edges(r).any(|(x, y)| (x.min(y), x.max(y)) == (a, b)))
                    });
                    if still_used {
                        continue;
                    }
                    let edge = model.edges.values().find(|e| (e.a.min(e.b), e.a.max(e.b)) == (a, b)).map(|e| e.id);
                    if let Some(edge) = edge {
                        model.edges.remove(&edge);
                        model.routes.remove(&edge);
                        model.entity_tags.remove(&edge);
                        for object in model.objects.values_mut() {
                            object.entities.remove(&edge);
                        }
                        removed_edges.insert(edge);
                    }
                }
                continue 'search;
            }
        }
        break;
    }
    if merged.is_empty() {
        return Err(fail("no faces to merge", "Only flat faces in one plane that share an edge can be merged."));
    }
    // Vertices no edge uses any more, and straight-run vertices left on the merged outline.
    let used: BTreeSet<u64> = model.edges.values().flat_map(|e| [e.a, e.b]).collect();
    model.vertices.retain(|id, _| used.contains(id));
    let _ = in_scope(model, Some(board), |model| {
        let contents = model.objects[&board].entities.clone();
        model.clean_collinear_vertices(&contents)
    });
    let mut done = modified(&BTreeSet::from([board]));
    done.erased = removed_faces.into_iter().chain(removed_edges).collect();
    done.extra.insert("merged".into(), json!(merged));
    done.extra.insert(
        "faces".into(),
        Value::Array(faces_left.iter().filter(|f| model.faces.contains_key(f)).take(MAX_LISTED).filter_map(|f| scene::face_entry(model, *f)).collect()),
    );
    Ok(done)
}

// ---------------------------------------------------------------- layout

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum SpacingMode {
    Gaps,
    Centers,
}
#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum EndsArg {
    Gap,
    Flush,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DistributeArgs {
    ids: Vec<u64>,
    axis: AxisArg,
    from_mm: f64,
    to_mm: f64,
    mode: Option<SpacingMode>,
    ends: Option<EndsArg>,
}

pub(super) fn distribute(model: &mut Model, a: DistributeArgs) -> Res {
    let set = entities(model, &a.ids, true)?;
    let axis = a.axis.axis().vector();
    let (from, to) = (from_mm(finite(a.from_mm, "from_mm")?), from_mm(finite(a.to_mm, "to_mm")?));
    if from.abs() > MAX_RESULT_M || to.abs() > MAX_RESULT_M {
        return Err(fail("coordinate out of range", "Use limits within 100000 mm."));
    }
    let (from, to) = (from.min(to), from.max(to));
    let mut items: Vec<(u64, f64, f64)> = set
        .iter()
        .map(|id| {
            let (lo, hi) = model.selection_bounds(&BTreeSet::from([*id])).ok_or_else(|| Fail::from(format!("{id} has no geometry")))?;
            Ok((*id, lo.dot(axis), (hi - lo).dot(axis)))
        })
        .collect::<Result<_, Fail>>()?;
    items.sort_by(|x, y| x.1.total_cmp(&y.1).then(x.0.cmp(&y.0)));
    let n = items.len() as f64;
    let total: f64 = items.iter().map(|(_, _, size)| size).sum();
    let length = to - from;
    let flush = matches!(a.ends, Some(EndsArg::Flush));
    if flush && items.len() < 2 {
        return Err(fail("ends flush needs at least two objects", "Use ends gap to centre one object."));
    }
    let mut starts = Vec::with_capacity(items.len());
    let mut gap = None;
    match a.mode.unwrap_or(SpacingMode::Gaps) {
        SpacingMode::Gaps => {
            let g = (length - total) / if flush { n - 1. } else { n + 1. };
            if g < -1.0e-9 {
                return Err(fail(
                    format!("the objects do not fit: they take {} mm of the {} mm between the limits", scene::mm(total), scene::mm(length)),
                    "Widen the limits or use fewer or thinner objects.",
                ));
            }
            let mut cursor = from + if flush { 0. } else { g };
            for (_, _, size) in &items {
                starts.push(cursor);
                cursor += size + g;
            }
            gap = Some(g);
        }
        SpacingMode::Centers => {
            let (first, last) = if flush {
                (from + items[0].2 / 2., to - items[items.len() - 1].2 / 2.)
            } else {
                (from + length / (n + 1.), to - length / (n + 1.))
            };
            let step = if items.len() > 1 { (last - first) / (n - 1.) } else { 0. };
            for (k, (_, _, size)) in items.iter().enumerate() {
                let center = if items.len() > 1 { first + step * k as f64 } else { (from + to) / 2. };
                starts.push(center - size / 2.);
            }
        }
    }
    let mut positions = Vec::new();
    for ((id, lo, size), start) in items.iter().zip(&starts) {
        let delta = start - lo;
        if delta.abs() > 1.0e-12 {
            model.move_selection(&BTreeSet::from([*id]), axis * delta)?;
        }
        positions.push(json!({"id": id, "min_mm": scene::mm(*start), "max_mm": scene::mm(start + size)}));
    }
    let mut done = modified(&set);
    if let Some(g) = gap {
        done.extra.insert("gap_mm".into(), json!(scene::mm(g)));
    }
    done.extra.insert("positions".into(), json!(positions));
    Ok(done)
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum ThicknessAnchor {
    Auto,
    Min,
    Center,
    Max,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ThicknessArgs {
    ids: Vec<u64>,
    thickness_mm: f64,
    anchor: Option<ThicknessAnchor>,
    material_id: Option<MaterialId>,
}

fn bounds_of(model: &Model, id: u64) -> Result<(DVec3, DVec3), Fail> {
    model.selection_bounds(&BTreeSet::from([id])).ok_or_else(|| Fail::from(format!("{id} has no geometry")))
}

pub(super) fn change_thickness(model: &mut Model, env: &ToolEnv, a: ThicknessArgs) -> Res {
    let set = objects(model, &a.ids, true)?;
    let thickness = finite(a.thickness_mm, "thickness_mm")?;
    if !(0.1..=10_000.).contains(&thickness) {
        return Err(fail("thickness_mm out of range", "Use a thickness between 0.1 mm and 10000 mm."));
    }
    let thickness = from_mm(thickness);
    if let Some(id) = a.material_id {
        find_material(model, env, id)?;
    }
    for id in &set {
        let aligned = model.joinery_frame(*id).is_ok_and(|f| f.is_project_aligned());
        if !aligned {
            return Err(fail(format!("{id} is not a board square to the axes"), "Resize rotated boards with resize_board, or rotate them square first."));
        }
    }
    let before: BTreeMap<u64, (DVec3, DVec3)> = set.iter().map(|id| Ok((*id, bounds_of(model, *id)?))).collect::<Result<_, Fail>>()?;
    let outer = before.values().fold((DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY)), |acc, b| (acc.0.min(b.0), acc.1.max(b.1)));
    // Joints into the resized boards, measured before resizing: the board that
    // goes in, the axis, which side it comes from and the overlap.
    struct Entry {
        joint: u64,
        board: u64,
        receiver: u64,
        axis: usize,
        from_high: bool,
        share: f64,
    }
    // Which board goes into which is read from the geometry, not from the
    // joint's a and b, so reversed joints are adjusted too.
    let thin_axis = |b: &(DVec3, DVec3)| {
        let size = b.1 - b.0;
        (0..3).min_by(|x, y| size[*x].total_cmp(&size[*y])).unwrap_or(0)
    };
    let mut entries = Vec::new();
    for (joint, connection) in &model.connections {
        if !matches!(connection.operation, Operation::Joinery(_)) {
            continue;
        }
        for (inner, receiver) in [(connection.members[0], connection.members[1]), (connection.members[1], connection.members[0])] {
            let (Some(rb), Ok(ib)) = (before.get(&receiver), bounds_of(model, inner)) else { continue };
            let axis = thin_axis(rb);
            let (lo, hi) = (ib.0[axis].max(rb.0[axis]), ib.1[axis].min(rb.1[axis]));
            if hi - lo <= 1.0e-9 {
                continue;
            }
            let from_high = ib.1[axis] > rb.1[axis] + 1.0e-9 && ib.0[axis] > rb.0[axis] + 1.0e-9;
            let from_low = ib.0[axis] < rb.0[axis] - 1.0e-9 && ib.1[axis] < rb.1[axis] - 1.0e-9;
            if from_high == from_low {
                continue;
            }
            entries.push(Entry { joint: *joint, board: inner, receiver, axis, from_high, share: (hi - lo) / (rb.1[axis] - rb.0[axis]) });
        }
    }
    // Boards that only butt against a resized board's face, without a joint:
    // they follow that face so they neither overlap it nor leave a gap.
    struct Butt {
        board: u64,
        against: u64,
        axis: usize,
        on_high_side: bool,
    }
    let joined: BTreeSet<[u64; 2]> = model.connections.values().map(|c| [c.members[0].min(c.members[1]), c.members[0].max(c.members[1])]).collect();
    let mut butts = Vec::new();
    for (id, rb) in &before {
        let axis = thin_axis(rb);
        for other in model.objects.keys().copied().filter(|o| !before.contains_key(o) && !joined.contains(&[(*id).min(*o), (*id).max(*o)])) {
            let object = &model.objects[&other];
            if model.is_hidden(other) || object.entities.iter().any(|e| model.objects.contains_key(e)) || model.parent(other) != model.parent(*id) {
                continue;
            }
            let Ok(ob) = bounds_of(model, other) else { continue };
            let across = (0..3).filter(|i| *i != axis).all(|i| ob.1[i].min(rb.1[i]) - ob.0[i].max(rb.0[i]) > 1.0e-6);
            if !across {
                continue;
            }
            if (ob.0[axis] - rb.1[axis]).abs() < 1.0e-7 {
                butts.push(Butt { board: other, against: *id, axis, on_high_side: true });
            } else if (ob.1[axis] - rb.0[axis]).abs() < 1.0e-7 {
                butts.push(Butt { board: other, against: *id, axis, on_high_side: false });
            }
        }
    }
    let mut resized = Vec::new();
    for (id, (lo, hi)) in &before {
        let size = *hi - *lo;
        let axis = (0..3).min_by(|x, y| size[*x].total_cmp(&size[*y])).unwrap_or(0);
        let anchor = match a.anchor.as_ref().unwrap_or(&ThicknessAnchor::Auto) {
            ThicknessAnchor::Min => Anchor::Min,
            ThicknessAnchor::Center => Anchor::Center,
            ThicknessAnchor::Max => Anchor::Max,
            ThicknessAnchor::Auto => {
                let (at_min, at_max) = (lo[axis] <= outer.0[axis] + 1.0e-7, hi[axis] >= outer.1[axis] - 1.0e-7);
                match (at_min, at_max) {
                    (true, false) => Anchor::Min,
                    (false, true) => Anchor::Max,
                    _ => Anchor::Center,
                }
            }
        };
        let mut target = size;
        target[axis] = thickness;
        let mut anchors = [Anchor::Min; 3];
        anchors[axis] = anchor;
        resize_object(model, *id, target, anchors)?;
        if a.material_id.is_some() {
            assign_material(model, env, *id, a.material_id)?;
        }
        let name = match anchor {
            Anchor::Min => "min",
            Anchor::Center => "center",
            Anchor::Max => "max",
        };
        resized.push(json!({"id": id, "axis": AXES[axis], "anchor": name}));
    }
    let mut adjusted = Vec::new();
    let mut touched: BTreeSet<u64> = set.clone();
    for entry in &entries {
        let rb = bounds_of(model, entry.receiver)?;
        let ib = bounds_of(model, entry.board)?;
        let axis = entry.axis;
        let overlap = entry.share * (rb.1[axis] - rb.0[axis]);
        // The end inside the receiver moves; the other end stays.
        let (new_lo, new_hi) = if entry.from_high { (rb.1[axis] - overlap, ib.1[axis]) } else { (ib.0[axis], rb.0[axis] + overlap) };
        if (new_lo - ib.0[axis]).abs() < 1.0e-9 && (new_hi - ib.1[axis]).abs() < 1.0e-9 {
            continue;
        }
        if new_hi - new_lo < 1.0e-4 {
            return Err(fail(format!("board {} would become too short", entry.board), "Check the joints of the resized boards."));
        }
        let mut target = ib.1 - ib.0;
        target[axis] = new_hi - new_lo;
        let mut anchors = [Anchor::Min; 3];
        anchors[axis] = if entry.from_high { Anchor::Max } else { Anchor::Min };
        resize_object(model, entry.board, target, anchors)?;
        touched.insert(entry.board);
        adjusted.push(json!({"joint": entry.joint, "board": entry.board, "axis": AXES[axis],
            "overlap_mm": scene::mm(overlap), "length_mm": scene::mm(new_hi - new_lo)}));
    }
    let mut butted = Vec::new();
    for butt in &butts {
        let rb = bounds_of(model, butt.against)?;
        let ob = bounds_of(model, butt.board)?;
        let axis = butt.axis;
        let (new_lo, new_hi) = if butt.on_high_side { (rb.1[axis], ob.1[axis]) } else { (ob.0[axis], rb.0[axis]) };
        if (new_lo - ob.0[axis]).abs() < 1.0e-9 && (new_hi - ob.1[axis]).abs() < 1.0e-9 {
            continue;
        }
        if new_hi - new_lo < 1.0e-4 {
            return Err(fail(format!("board {} would become too short", butt.board), "Move it away from the resized board first."));
        }
        let mut target = ob.1 - ob.0;
        target[axis] = new_hi - new_lo;
        let mut anchors = [Anchor::Min; 3];
        anchors[axis] = if butt.on_high_side { Anchor::Max } else { Anchor::Min };
        resize_object(model, butt.board, target, anchors)?;
        touched.insert(butt.board);
        butted.push(json!({"board": butt.board, "against": butt.against, "axis": AXES[axis], "length_mm": scene::mm(new_hi - new_lo)}));
    }
    let mut done = modified(&touched);
    done.extra.insert("resized".into(), json!(resized));
    done.extra.insert("joint_adjustments".into(), json!(adjusted));
    if !butted.is_empty() {
        done.extra.insert("butt_adjustments".into(), json!(butted));
    }
    Ok(done)
}

// ---------------------------------------------------------------- joints

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SetJointArgs {
    id: u64,
    kind: Option<JointArg>,
    params: Option<JointParams>,
}

pub(super) fn set_joint(model: &mut Model, env: &ToolEnv, a: SetJointArgs) -> Res {
    let Some(connection) = model.connections.get(&a.id).cloned() else {
        return Err(fail(format!("{} is not a joint", a.id), "Use ids from the joints list in get_scene."));
    };
    let Operation::Joinery(mut settings) = connection.operation else {
        return Err(fail(format!("{} is not a wood joint", a.id), "Only wood joints can be changed."));
    };
    for board in connection.members {
        if !model.contains_entity(&board) {
            return Err(fail(format!("board {board} of joint {} was erased", a.id), "Remove the joint with remove_joint."));
        }
        if model.is_locked(board) {
            return Err(fail(format!("board {board} of joint {} is locked", a.id), "Locked entities cannot be changed; tell the designer to unlock it."));
        }
        if !leased(model, env, board) {
            return Err(fail("not-leased", "select it first so you hold the edit lease"));
        }
    }
    let changed_kind = a.kind.as_ref().is_some_and(|k| k.kind() != settings.kind);
    if let Some(kind) = &a.kind {
        settings.kind = kind.kind();
    }
    let chosen_axes = match a.params {
        Some(p) => apply_joint_params(&mut settings, p)?,
        None => false,
    };
    // Keep the current orientation unless the kind changed and no axes were given.
    commit_joint(model, connection.members, settings, chosen_axes || !changed_kind, Some(a.id)).map_err(|failure| {
        if failure.error == "this orientation cannot cut these boards" && !chosen_axes {
            fail("the joint's current orientation cannot cut it with these settings", failure.hint.unwrap_or_default())
        } else {
            failure
        }
    })
}

// ---------------------------------------------------------------- router

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum ProfileArg {
    Chamfer,
    Bevel,
    Roundover,
    Cove,
    Rabbet,
    Groove,
    Ogee,
    Bead,
}
impl ProfileArg {
    fn profile(self) -> Profile {
        match self {
            Self::Chamfer => Profile::Chamfer,
            Self::Bevel => Profile::Bevel,
            Self::Roundover => Profile::Roundover,
            Self::Cove => Profile::Cove,
            Self::Rabbet => Profile::Rabbet,
            Self::Groove => Profile::Groove,
            Self::Ogee => Profile::Ogee,
            Self::Bead => Profile::Bead,
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AddProfileArgs {
    id: u64,
    #[serde(default)]
    between: Vec<[Value; 2]>,
    #[serde(default)]
    edges: Vec<u64>,
    profile: ProfileArg,
    width_mm: Option<f64>,
    depth_mm: Option<f64>,
    offset_mm: Option<f64>,
    angle_degrees: Option<f64>,
    reverse: Option<bool>,
}

enum FaceSel {
    Id(u64),
    Facing(DVec3),
    Any,
}
fn face_selector(value: &Value) -> Result<FaceSel, Fail> {
    if let Some(id) = value.as_u64() {
        return Ok(FaceSel::Id(id));
    }
    let hint = "Name a face by the direction it faces (+x, -x, +y, -y, +z, -z), by its id from get_object, or * for any face.";
    let text = value.as_str().ok_or_else(|| fail(format!("{value} is not a face"), hint))?;
    if text == "*" {
        return Ok(FaceSel::Any);
    }
    let (sign, axis) = match text.as_bytes() {
        [b'+', axis] => (1., *axis),
        [b'-', axis] => (-1., *axis),
        _ => return Err(fail(format!("{text} is not a face direction"), hint)),
    };
    let axis = match axis {
        b'x' => DVec3::X,
        b'y' => DVec3::Y,
        b'z' => DVec3::Z,
        _ => return Err(fail(format!("{text} is not a face direction"), hint)),
    };
    Ok(FaceSel::Facing(axis * sign))
}

fn board_only(model: &Model, id: u64) -> Result<u64, Fail> {
    let board = one_object(model, id)?;
    if model.objects[&board].entities.iter().any(|e| model.objects.contains_key(e)) {
        return Err(fail(format!("{board} is a group of parts"), "Profile one board: get_object lists the group's children."));
    }
    Ok(board)
}

fn mm_setting(value: Option<f64>, default: f64, what: &str) -> Result<f64, Fail> {
    let Some(value) = value else { return Ok(default) };
    let value = from_mm(finite(value, what)?);
    if !RouteSettings::SIZE.contains(&value) {
        return Err(fail(format!("{what} out of range"), "Use a size between 0.1 mm and 100000 mm."));
    }
    Ok(value)
}

pub(super) fn add_profile(model: &mut Model, a: AddProfileArgs) -> Res {
    let board = board_only(model, a.id)?;
    let adjacency = scene::edge_faces(model, board);
    let mut chosen: BTreeSet<u64> = BTreeSet::new();
    for edge in &a.edges {
        if !adjacency.contains_key(edge) {
            return Err(fail(format!("{edge} is not an edge of board {board}"), "Choose edges with between, e.g. [[\"+z\", \"-y\"]] for the top front edge."));
        }
        chosen.insert(*edge);
    }
    let normals: BTreeMap<u64, Option<DVec3>> = adjacency.values().flatten().map(|f| (*f, scene::outward_normal(model, *f))).collect();
    let matches = |face: u64, selector: &FaceSel| match selector {
        FaceSel::Any => true,
        FaceSel::Id(id) => face == *id,
        FaceSel::Facing(direction) => normals.get(&face).copied().flatten().is_some_and(|n| n.dot(*direction) > 0.999),
    };
    for [x, y] in &a.between {
        let (sx, sy) = (face_selector(x)?, face_selector(y)?);
        let found: Vec<u64> = adjacency
            .iter()
            .filter(|(_, faces)| {
                let [f, g] = faces[..] else { return false };
                (matches(f, &sx) && matches(g, &sy)) || (matches(f, &sy) && matches(g, &sx))
            })
            .map(|(edge, _)| *edge)
            .collect();
        if found.is_empty() {
            return Err(fail(
                format!("no edge of board {board} lies between {x} and {y}"),
                "Faces are named by the direction they face (+x, -y...), by id from get_object, or * for any face. get_object lists the board's faces.",
            ));
        }
        chosen.extend(found);
    }
    if chosen.is_empty() {
        return Err(fail("no edges chosen", "Pass between, e.g. [[\"+z\", \"-y\"]] for the top front edge, or edge ids in edges."));
    }
    if chosen.len() > MAX_IDS {
        return Err(fail("too many edges", format!("Profile at most {MAX_IDS} edges per call.")));
    }
    let defaults = RouteSettings::default();
    let angle = finite(a.angle_degrees.unwrap_or(defaults.angle_degrees), "angle_degrees")?;
    if !RouteSettings::ANGLE.contains(&angle) {
        return Err(fail("angle_degrees out of range", "Use an angle between 1 and 89 degrees."));
    }
    let settings = RouteSettings {
        profile: a.profile.profile(),
        width: mm_setting(a.width_mm, defaults.width, "width_mm")?,
        depth: mm_setting(a.depth_mm, defaults.depth, "depth_mm")?,
        offset: mm_setting(a.offset_mm, defaults.offset, "offset_mm")?,
        angle_degrees: angle,
        reverse: a.reverse.unwrap_or(false),
    };
    for edge in &chosen {
        model.routes.insert(*edge, settings);
    }
    let errors = scene::joint_errors(model);
    if let Some((edge, error)) = chosen.iter().find_map(|e| errors.get(e).map(|error| (*e, error))) {
        return Err(fail(
            format!("the profile cannot be cut on edge {edge}: {error}"),
            "Use a smaller width or depth, a chamfer or bevel on angled edges, or other edges. Profiles must not overlap joints or each other.",
        ));
    }
    let length: f64 = chosen.iter().filter_map(|e| model.edges.get(e)).map(|e| model.edge_length(e)).sum();
    let mut done = modified(&BTreeSet::from([board]));
    done.extra.insert("profile".into(), json!(scene::profile_name(settings.profile)));
    done.extra.insert("edges".into(), json!(chosen));
    done.extra.insert("length_mm".into(), json!(scene::mm(length)));
    Ok(done)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RemoveProfileArgs {
    id: u64,
    edges: Option<Vec<u64>>,
}

pub(super) fn remove_profile(model: &mut Model, a: RemoveProfileArgs) -> Res {
    let board = board_only(model, a.id)?;
    let mine: BTreeSet<u64> = model.objects[&board].entities.iter().copied().filter(|e| model.routes.contains_key(e)).collect();
    let targets: Vec<u64> = match a.edges {
        None => mine.iter().copied().collect(),
        Some(edges) => {
            if let Some(edge) = edges.iter().find(|e| !mine.contains(e)) {
                return Err(fail(format!("edge {edge} of board {board} has no profile"), "get_object on the board lists its profiles."));
            }
            edges
        }
    };
    if targets.is_empty() {
        return Err(fail(format!("board {board} has no profiles"), "get_object on the board lists its profiles."));
    }
    for edge in &targets {
        model.routes.remove(edge);
    }
    let mut done = modified(&BTreeSet::from([board]));
    done.extra.insert("removed_profiles".into(), json!(targets));
    Ok(done)
}

// ---------------------------------------------------------------- finishes

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum SheenArg {
    Matte,
    Satin,
    Gloss,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StepArgs {
    enabled: Option<bool>,
    coats: Option<u32>,
    minutes_per_m2: Option<f64>,
    drying_hours: Option<f64>,
    final_drying_hours: Option<f64>,
    tint_rgb: Option<[u8; 3]>,
    strength: Option<u8>,
    sheen: Option<SheenArg>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FinishArgs {
    ids: Vec<u64>,
    #[serde(default)]
    clear: bool,
    sanding_grits: Option<Vec<u32>>,
    pre_stain: Option<StepArgs>,
    stain: Option<StepArgs>,
    varnish: Option<StepArgs>,
}

fn apply_step(step: &mut Step, a: &StepArgs, what: &str) -> Result<(), Fail> {
    step.enabled = a.enabled.unwrap_or(true);
    if let Some(coats) = a.coats {
        if !(1..=100).contains(&coats) {
            return Err(fail(format!("{what}.coats out of range"), "Use 1 to 100 coats."));
        }
        step.coats = coats;
    }
    for (value, slot, name) in [
        (a.minutes_per_m2, &mut step.minutes_per_m2, "minutes_per_m2"),
        (a.drying_hours, &mut step.drying_hours, "drying_hours"),
    ] {
        if let Some(value) = value {
            if !value.is_finite() || !(0. ..=10_000.).contains(&value) {
                return Err(fail(format!("{what}.{name} out of range"), "Use a value between 0 and 10000."));
            }
            *slot = value;
        }
    }
    if let Some(hours) = a.final_drying_hours {
        if !hours.is_finite() || !(0. ..=10_000.).contains(&hours) {
            return Err(fail(format!("{what}.final_drying_hours out of range"), "Use a value between 0 and 10000."));
        }
        step.final_drying_hours = Some(hours);
    }
    if let Some(tint) = a.tint_rgb {
        step.appearance.tint = Some(tint);
    }
    if let Some(strength) = a.strength {
        step.appearance.strength = Some(strength.min(100));
    }
    if let Some(sheen) = &a.sheen {
        step.appearance.sheen = Some(match sheen {
            SheenArg::Matte => Sheen::Matte,
            SheenArg::Satin => Sheen::Satin,
            SheenArg::Gloss => Sheen::Gloss,
        });
    }
    Ok(())
}

pub(super) fn set_finish(model: &mut Model, a: FinishArgs) -> Res {
    let set = objects(model, &a.ids, true)?;
    if !a.clear && a.sanding_grits.is_none() && a.pre_stain.is_none() && a.stain.is_none() && a.varnish.is_none() {
        return Err(fail("nothing to set", "Pass sanding_grits, pre_stain, stain or varnish, or clear: true."));
    }
    if let Some(grits) = &a.sanding_grits
        && (grits.len() > 20 || grits.iter().any(|g| !(24..=3000).contains(g)))
    {
        return Err(fail("sanding_grits out of range", "List up to 20 grits between 24 and 3000."));
    }
    // A group sets every object inside it; finishing is counted per board.
    let mut targets: BTreeSet<u64> = model.expand_entities(&set).into_iter().filter(|e| model.objects.contains_key(e)).collect();
    targets.extend(&set);
    if let Some(id) = targets.iter().find(|id| model.is_locked(**id)) {
        return Err(fail(format!("entity {id} is locked"), "Locked entities cannot be changed; tell the designer to unlock it."));
    }
    for id in &targets {
        let object = model.objects.get_mut(id).ok_or("missing object")?;
        if a.clear {
            object.surface_treatment = surface_treatment::Settings::default();
            object.surface_treatment_customized = false;
            continue;
        }
        let mut settings = object.surface_treatment.clone();
        if let Some(grits) = &a.sanding_grits {
            let mut all: BTreeSet<u32> = settings.sanding_grits.iter().copied().collect();
            all.extend(grits);
            let old: BTreeMap<u32, Step> = settings.sanding_grits.iter().copied().zip(settings.sanding.iter().cloned()).collect();
            let after: BTreeMap<u32, bool> = settings.sanding_grits.iter().copied().zip(settings.sanding_after_varnish.iter().copied()).collect();
            settings.sanding_grits = all.iter().copied().collect();
            settings.sanding = all
                .iter()
                .map(|grit| Step { enabled: grits.contains(grit), ..old.get(grit).cloned().unwrap_or_default() })
                .collect();
            settings.sanding_after_varnish = all.iter().map(|grit| after.get(grit).copied().unwrap_or(false)).collect();
        }
        for (args, step, what) in [(&a.pre_stain, &mut settings.pre_stain, "pre_stain"), (&a.stain, &mut settings.stain, "stain"), (&a.varnish, &mut settings.varnish, "varnish")] {
            if let Some(args) = args {
                apply_step(step, args, what)?;
            }
        }
        settings.sanitize();
        object.surface_treatment = settings;
        object.surface_treatment_customized = true;
    }
    let mut done = modified(&targets);
    if let Some(first) = set.first().and_then(|id| scene::finish_json(model, *id)) {
        done.extra.insert("finish".into(), first);
    }
    done.extra.insert("objects_set".into(), json!(targets.len()));
    Ok(done)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct EstimateArgs {
    ids: Option<Vec<u64>>,
}

/// BenchUp's default planning allowance per joint, in minutes.
fn joint_minutes(kind: JointKind) -> f64 {
    match kind {
        JointKind::MortiseTenon => 15.,
        JointKind::HalfLap => 8.,
        JointKind::TongueGroove => 8.,
        JointKind::Rabbet => 5.,
        JointKind::Dovetail => 25.,
        JointKind::Finger => 15.,
    }
}
const ROUTER_SETUP_MINUTES: f64 = 5.;
const ROUTER_MINUTES_PER_METRE: f64 = 1.;

fn round1(value: f64) -> f64 {
    (value * 10.).round() / 10.
}

pub(super) fn get_estimate(model: &Model, a: EstimateArgs) -> Res {
    let scope: Option<BTreeSet<u64>> = match &a.ids {
        Some(ids) => Some(model.expand_entities(&objects(model, ids, false)?)),
        None => None,
    };
    let in_scope = |id: u64| scope.as_ref().is_none_or(|s| s.contains(&id));
    // Each face counts once, for the object that holds it.
    let mut areas: BTreeMap<u64, f64> = BTreeMap::new();
    for object in model.objects.values() {
        if model.is_hidden(object.id) {
            continue;
        }
        for face in object.entities.iter().filter(|e| in_scope(**e)).filter_map(|e| model.faces.get(e)) {
            let area = model.face_area(face);
            if area.is_finite() && area > 0. && !model.is_hidden(face.id) {
                *areas.entry(object.id).or_default() += area;
            }
        }
    }
    let defaults = surface_treatment::Settings::default();
    let mut tasks: BTreeMap<String, (f64, f64, usize)> = BTreeMap::new();
    let mut finished = 0;
    for (id, area) in &areas {
        let mut settings = model.objects[id].surface_treatment_override().unwrap_or(&defaults).clone();
        settings.sanitize();
        if settings.any_enabled() {
            finished += 1;
        }
        for (task, duration) in settings.tasks(*area) {
            let entry = tasks.entry(task.label()).or_default();
            entry.0 += duration.work;
            // Boards dry together after each coat; wait for the slowest one.
            entry.1 = entry.1.max(duration.wait);
            entry.2 += 1;
        }
    }
    let finishing: Vec<Value> = tasks
        .iter()
        .map(|(task, (work, wait, count))| json!({"task": task, "work_minutes": round1(*work), "wait_minutes": round1(*wait), "boards": count}))
        .collect();
    let mut joints: BTreeMap<&'static str, (usize, f64)> = BTreeMap::new();
    for connection in model.connections.values() {
        let Operation::Joinery(settings) = &connection.operation else { continue };
        if !connection.members.iter().all(|m| in_scope(*m)) {
            continue;
        }
        let entry = joints.entry(scene::joint_kind(connection)).or_default();
        entry.0 += 1;
        entry.1 += joint_minutes(settings.kind);
    }
    let mut router: BTreeMap<&'static str, (usize, f64)> = BTreeMap::new();
    for (edge, settings) in &model.routes {
        let Some(e) = model.edges.get(edge) else { continue };
        if settings.profile == Profile::None || !in_scope(*edge) {
            continue;
        }
        let entry = router.entry(scene::profile_name(settings.profile)).or_default();
        entry.0 += 1;
        entry.1 += model.edge_length(e);
    }
    let finishing_work: f64 = tasks.values().map(|t| t.0).sum();
    let finishing_wait: f64 = tasks.values().map(|t| t.1).sum();
    let joint_work: f64 = joints.values().map(|j| j.1).sum();
    let router_rows: Vec<Value> = router
        .iter()
        .map(|(profile, (count, metres))| {
            json!({"profile": profile, "edges": count, "length_mm": scene::mm(*metres),
                "work_minutes": round1(ROUTER_SETUP_MINUTES + metres * ROUTER_MINUTES_PER_METRE)})
        })
        .collect();
    let router_work: f64 = router.values().map(|(_, metres)| ROUTER_SETUP_MINUTES + metres * ROUTER_MINUTES_PER_METRE).sum();
    let work = finishing_work + joint_work + router_work;
    let total_area: f64 = areas.values().sum();
    let mut out = json!({
        "boards": areas.len(),
        "finished_boards": finished,
        "surface_area_m2": (total_area * 1000.).round() / 1000.,
        "finishing": finishing,
        "joinery": joints.iter().map(|(kind, (count, minutes))| json!({"kind": kind, "count": count, "work_minutes": round1(*minutes)})).collect::<Vec<_>>(),
        "router": router_rows,
        "total": {"work_minutes": round1(work), "wait_minutes": round1(finishing_wait), "work_hours": round1(work / 60.),
            "elapsed_hours": round1((work + finishing_wait) / 60.),
            "by_category": {"finishing_work_minutes": round1(finishing_work), "finishing_wait_minutes": round1(finishing_wait),
                "joinery_work_minutes": round1(joint_work), "router_work_minutes": round1(router_work)}},
        "wood": crate::review::cut_list(model)["materials"].clone(),
        "note": "BenchUp's default allowances: joints by kind, router 5 min setup per profile plus 1 min per metre, finishing per m² and coat. \
                 The last coat of a stage waits its final drying time. elapsed_hours adds work and drying, as if done one after the other. \
                 Boards without set_finish use the defaults, which sand and finish nothing. Rates and prices edited in the app are not stored in the project.",
    });
    if finished == 0 {
        out["hint"] = json!("No board has a finish yet: set one with set_finish to estimate finishing time.");
    }
    read(out)
}

// ---------------------------------------------------------------- queries

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SpaceArgs {
    point_mm: V3,
    axes: Option<Vec<AxisArg>>,
}

/// Distance along the ray to the triangle, when it is ahead.
fn ray_triangle(origin: DVec3, direction: DVec3, [a, b, c]: &[DVec3; 3]) -> Option<f64> {
    let (e1, e2) = (*b - *a, *c - *a);
    let p = direction.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-14 {
        return None;
    }
    let t0 = origin - *a;
    let u = t0.dot(p) / det;
    if !(-1e-9..=1. + 1e-9).contains(&u) {
        return None;
    }
    let q = t0.cross(e1);
    let v = direction.dot(q) / det;
    if v < -1e-9 || u + v > 1. + 1e-9 {
        return None;
    }
    let t = e2.dot(q) / det;
    (t > 1e-9).then_some(t)
}

fn owner_info(model: &Model, entity: u64) -> Value {
    let Some(owner) = model.parent(entity).filter(|o| model.objects.contains_key(o)) else {
        return json!({"object": null});
    };
    let mut path = Vec::new();
    let mut current = Some(owner);
    while let Some(object) = current.and_then(|c| model.objects.get(&c)) {
        path.push(object.name.clone());
        current = model.parent(object.id);
        if path.len() > 64 {
            break;
        }
    }
    path.reverse();
    json!({"object": owner, "name": model.objects[&owner].name, "path": path})
}

pub(super) fn measure_space(model: &Model, a: SpaceArgs) -> Res {
    let origin = point(a.point_mm)?;
    let axes: Vec<AxisArg> = a.axes.unwrap_or_else(|| vec![AxisArg::X, AxisArg::Y, AxisArg::Z]);
    let triangles: Vec<(u64, Vec<[DVec3; 3]>)> = model
        .faces
        .values()
        .filter(|f| !model.is_hidden(f.id) && model.parent(f.id).is_none_or(|p| !model.is_hidden(p)))
        .map(|f| (f.id, model.face_triangles(f)))
        .collect();
    let cast = |direction: DVec3| -> Option<(u64, f64)> {
        triangles
            .iter()
            .filter_map(|(id, tris)| tris.iter().filter_map(|t| ray_triangle(origin, direction, t)).min_by(f64::total_cmp).map(|t| (*id, t)))
            .min_by(|x, y| x.1.total_cmp(&y.1))
    };
    let mut out = serde_json::Map::new();
    for axis in axes {
        let vector = axis.axis().vector();
        let name = ["x", "y", "z"][match axis {
            AxisArg::X => 0,
            AxisArg::Y => 1,
            AxisArg::Z => 2,
        }];
        let mut entry = json!({});
        let mut gap = Some(0.);
        for (sign, key) in [(-1., "minus"), (1., "plus")] {
            match cast(vector * sign) {
                Some((face, t)) => {
                    let mut hit = owner_info(model, face);
                    hit["face"] = json!(face);
                    hit["distance_mm"] = json!(scene::mm(t));
                    hit["at_mm"] = scene::point_mm(origin + vector * sign * t);
                    entry[key] = hit;
                    gap = gap.map(|g| g + t);
                }
                None => {
                    entry[key] = Value::Null;
                    gap = None;
                }
            }
        }
        entry["free_gap_mm"] = gap.map_or(Value::Null, |g| json!(scene::mm(g)));
        out.insert(name.into(), entry);
    }
    let inside: Vec<Value> = model
        .objects
        .keys()
        .filter(|id| !model.is_hidden(**id) && model.objects[*id].entities.iter().all(|e| !model.objects.contains_key(e)))
        .filter(|id| model.selection_bounds(&BTreeSet::from([**id])).is_some_and(|(lo, hi)| (origin - lo).min_element() > 1e-9 && (hi - origin).min_element() > 1e-9))
        .take(20)
        .map(|id| json!({"id": id, "name": model.objects[id].name}))
        .collect();
    let mut result = json!({"point_mm": scene::point_mm(origin), "axes": Value::Object(out)});
    if result["axes"].as_object().is_some_and(|axes| axes.values().any(|a| a["minus"].is_null() || a["plus"].is_null())) {
        result["null_means"] = json!("nothing in that direction, so the gap along that axis is open (free_gap_mm null)");
    }
    if !inside.is_empty() {
        result["inside"] = json!(inside);
        result["note"] = json!("The point is inside the bounding box of these boards; distances are measured from inside them.");
    }
    read(result)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SurfacesArgs {
    min_area_mm2: Option<f64>,
    facing: Option<String>,
    limit: Option<usize>,
    include_solids: Option<bool>,
}

pub(super) fn find_surfaces(model: &Model, a: SurfacesArgs) -> Res {
    let min_area = finite(a.min_area_mm2.unwrap_or(100_000.), "min_area_mm2")?.max(0.) / 1.0e6;
    let limit = a.limit.unwrap_or(40).clamp(1, MAX_LISTED);
    let wanted = match &a.facing {
        Some(text) => match face_selector(&json!(text))? {
            FaceSel::Facing(direction) => Some(direction),
            _ => return Err(fail("facing must be a direction", "Use +x, -x, +y, -y, +z or -z.")),
        },
        None => None,
    };
    let mut solid: BTreeMap<u64, bool> = BTreeMap::new();
    let mut found: Vec<(f64, Value)> = Vec::new();
    for face in model.faces.values() {
        if model.is_hidden(face.id) {
            continue;
        }
        let area = model.face_area(face);
        if area.is_nan() || area < min_area {
            continue;
        }
        let owner = model.parent(face.id).filter(|o| model.objects.contains_key(o));
        if owner.is_some_and(|o| model.is_hidden(o)) {
            continue;
        }
        let closed = owner.is_some_and(|o| *solid.entry(o).or_insert_with(|| model.solid_volume(o).is_ok()));
        if closed && !a.include_solids.unwrap_or(false) {
            continue;
        }
        let Some(plane) = faces::plane(model, &face.vertices) else { continue };
        // Faces of closed parts point outward; a loose face's normal is only
        // its drawing order, so either side matches a direction.
        let normal = if closed { plane.normal * scene::outward_sign(model, face.id) } else { plane.normal };
        if let Some(direction) = wanted {
            let along = normal.dot(direction);
            if !(along > 0.999 || (!closed && along < -0.999)) {
                continue;
            }
        }
        let mut entry = owner_info(model, face.id);
        entry["id"] = json!(face.id);
        entry["area_mm2"] = json!((area * 1.0e6).round());
        if let Some(facing) = scene::facing_name(normal) {
            entry["facing"] = json!(facing);
        }
        entry["normal"] = json!([round1(normal.x * 100.) / 100., round1(normal.y * 100.) / 100., round1(normal.z * 100.) / 100.]);
        if let Some(axis) = (0..3).find(|i| normal[*i].abs() > 0.999) {
            entry["plane"] = json!(format!("{} = {}", ["x", "y", "z"][axis], scene::mm(plane.origin[axis])));
        }
        if let Some(bounds) = scene::bounds_json(model, &BTreeSet::from([face.id])) {
            entry["bounds_mm"] = json!({"min": bounds["min"], "max": bounds["max"]});
        }
        if let Some(center) = model.face_center(face) {
            entry["center_mm"] = scene::point_mm(center);
        }
        found.push((area, entry));
    }
    found.sort_by(|x, y| y.0.total_cmp(&x.0));
    let total = found.len();
    let mut out = json!({"total": total, "surfaces": found.into_iter().take(limit).map(|(_, e)| e).collect::<Vec<_>>()});
    if total == 0 {
        out["note"] = json!("No open surfaces of that size: everything is a closed board. Lower min_area_mm2 to see smaller faces.");
    }
    read(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn faces_meeting_at_a_t_junction_merge() {
        // Two unit squares side by side; the left one's shared edge is whole,
        // the right one's is split at its middle (vertex 7).
        let points: BTreeMap<u64, DVec3> = [
            (1, [0., 0.]), (2, [1., 0.]), (3, [1., 1.]), (4, [0., 1.]),
            (5, [2., 0.]), (6, [2., 1.]), (7, [1., 0.5]),
        ]
        .into_iter()
        .map(|(id, [x, y])| (id, DVec3::new(x, y, 0.)))
        .collect();
        let position = |v: u64| points[&v];
        let (left, right) = (vec![1, 2, 3, 4], vec![2, 5, 6, 3, 7]);
        assert!(merged_ring(&left, &right, &position, DVec3::Z).is_none(), "without the T-junction vertex they share no edge");
        let (l, r) = (with_t_vertices(&left, &right, &position), with_t_vertices(&right, &left, &position));
        assert_eq!(l, vec![1, 2, 7, 3, 4]);
        let merged = merged_ring(&l, &r, &position, DVec3::Z).unwrap();
        assert_eq!(merged.outer.len(), 6, "{:?}", merged.outer);
        assert!(merged.holes.is_empty());
    }
}
