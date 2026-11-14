//! Compact scene description sent to the model. Geometry in the document is in
//! metres; everything the model sees or sends is in millimetres.
use benchup_core::{
    DVec3, MaterialId, Model, ObjectKind, StockMaterial, connections::ConnectionOperation, faces,
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub const SCENE_BUDGET: usize = 24_000;
const MAX_FACES: usize = 80;
const MAX_LISTED_SELECTION: usize = 200;
const MAX_LISTED_TAGS: usize = 100;
const MAX_LISTED_LOOSE: usize = 100;
const MAX_LISTED_ANNOTATIONS: usize = 50;

/// Metres to millimetres, rounded to 0.1 mm.
pub fn mm(metres: f64) -> f64 {
    (metres * 10_000.).round() / 10.
}
pub fn from_mm(millimetres: f64) -> f64 {
    millimetres / 1000.
}
pub fn point_mm(p: DVec3) -> Value {
    json!([mm(p.x), mm(p.y), mm(p.z)])
}
fn unit(v: DVec3) -> Value {
    let r = |x: f64| (x * 1000.).round() / 1000.;
    json!([r(v.x), r(v.y), r(v.z)])
}

pub fn kind_of(model: &Model, id: u64) -> &'static str {
    if let Some(object) = model.objects.get(&id) {
        match object.kind {
            ObjectKind::Group => "group",
            ObjectKind::Component { .. } => "component",
        }
    } else if model.faces.contains_key(&id) {
        "face"
    } else if model.edges.contains_key(&id) {
        "edge"
    } else if model.dimensions.contains_key(&id) {
        "dimension"
    } else if model.texts.contains_key(&id) {
        "text"
    } else if model.guides.contains_key(&id) {
        "guide"
    } else if model.connections.contains_key(&id) {
        "joint"
    } else {
        "missing"
    }
}

pub fn bounds_json(model: &Model, ids: &BTreeSet<u64>) -> Option<Value> {
    let (lo, hi) = model.selection_bounds(ids)?;
    Some(json!({"min": point_mm(lo), "max": point_mm(hi), "size": point_mm(hi - lo)}))
}

fn axis_name(v: DVec3) -> Value {
    for (name, axis) in [("x", DVec3::X), ("y", DVec3::Y), ("z", DVec3::Z)] {
        if v.dot(axis).abs() > 0.999 {
            return json!(name);
        }
    }
    unit(v)
}

fn material_json(model: &Model, object: &benchup_core::Object) -> Option<Value> {
    let stock = model.object_stock_material(object)?;
    Some(json!({"id": object.stock_material_id, "name": stock.name}))
}

/// The stock material of the nearest group around `id` that has one.
fn inherited_material(model: &Model, id: u64) -> Option<(u64, Value)> {
    let mut current = model.parent(id);
    while let Some(object) = current.and_then(|c| model.objects.get(&c)) {
        if let Some(material) = material_json(model, object) {
            return Some((object.id, material));
        }
        current = model.parent(object.id);
    }
    None
}

pub fn object_entry(model: &Model, id: u64) -> Option<Value> {
    let object = model.objects.get(&id)?;
    let mut entry = json!({
        "id": id,
        "name": object.name,
        "kind": kind_of(model, id),
    });
    let map = entry.as_object_mut()?;
    if let Some(parent) = model.parent(id) {
        map.insert("parent".into(), json!(parent));
    }
    if let Some(tag) = model.inherited_tag(id) {
        map.insert("tag".into(), json!(tag.name));
    }
    if let Some(bounds) = bounds_json(model, &BTreeSet::from([id])) {
        map.insert("bounds_mm".into(), json!({"min": bounds["min"], "max": bounds["max"]}));
        map.insert("size_mm".into(), bounds["size"].clone());
    }
    if let ObjectKind::Component { definition } = object.kind {
        // Instances of one component share this id.
        map.insert("definition".into(), json!(definition.to_string()[..8]));
    }
    if let Some(material) = material_json(model, object) {
        map.insert("material".into(), material);
    } else if let Some((owner, mut material)) = inherited_material(model, id) {
        material["inherited_from"] = json!(owner);
        map.insert("material".into(), material);
    }
    if let Some(grain) = object.world_grain_axis() {
        map.insert("grain".into(), axis_name(grain));
    }
    if model.is_hidden(id) {
        map.insert("hidden".into(), json!(true));
    }
    if model.is_locked(id) {
        map.insert("locked".into(), json!(true));
    }
    Some(entry)
}

pub fn stock_list(
    model: &Model,
    catalog: &BTreeMap<MaterialId, StockMaterial>,
) -> Vec<Value> {
    let mut all: BTreeMap<MaterialId, &StockMaterial> = model.stock_materials.iter().map(|(k, v)| (*k, v)).collect();
    all.extend(catalog.iter().map(|(k, v)| (*k, v)));
    all.into_iter()
        .map(|(id, stock)| {
            let [t, w, l] = stock.dimensions.map(|tenths| tenths as f64 / 10.);
            json!({"id": id, "name": stock.name, "thickness_mm": t, "width_mm": w, "length_mm": l})
        })
        .collect()
}

/// Joints the core cannot cut, keyed by joint id, with its reason.
pub fn joint_errors(model: &Model) -> BTreeMap<u64, String> {
    if !model.connections.values().any(|c| matches!(c.operation, ConnectionOperation::Joinery(_))) {
        return BTreeMap::new();
    }
    model.evaluate_joinery().errors.into_iter().collect()
}

/// A joint in full: which board carries which part, its settings and whether it cuts.
pub fn joint_detail(model: &Model, id: u64) -> Option<Value> {
    let mut joint = joint_json(model, id)?;
    let connection = &model.connections[&id];
    if let ConnectionOperation::Joinery(s) = &connection.operation {
        let axes = ["x", "y", "z"];
        joint["roles"] = json!({
            "a": "carries the tenon, tongue, tails, fingers or lap (the board that goes into the other)",
            "b": "receives the mortise, groove, sockets or lap",
        });
        use benchup_core::joinery::JointKind;
        let mut params = json!({
            "depth_axis": s.depth_axis, "width_axis": s.width_axis,
            "depth_axis_name": axes[s.depth_axis.min(2)], "width_axis_name": axes[s.width_axis.min(2)],
            "clearance_mm": mm(s.clearance),
        });
        // Only the settings this kind of joint uses.
        match s.kind {
            JointKind::MortiseTenon => {
                params["width_ratio"] = json!(s.width_ratio);
                params["height_ratio"] = json!(s.height_ratio);
            }
            JointKind::TongueGroove | JointKind::Rabbet => params["width_ratio"] = json!(s.width_ratio),
            JointKind::Dovetail => {
                params["dovetails"] = json!(s.dovetails);
                params["width_ratio"] = json!(s.width_ratio);
                params["taper_degrees"] = json!(s.taper_degrees);
            }
            JointKind::Finger => params["fingers"] = json!(s.fingers),
            JointKind::Biscuit | JointKind::Domino | JointKind::Groove | JointKind::LooseTongue => {
                params["pocket_thickness_mm"] = json!(mm(s.pocket_thickness));
                params["pocket_depth_mm"] = json!(mm(s.pocket_depth));
                if matches!(s.kind, JointKind::Biscuit | JointKind::Domino) {
                    params["pocket_width_mm"] = json!(mm(s.pocket_width));
                    params["pocket_count"] = json!(s.pocket_count);
                }
            }
            JointKind::HalfLap => {}
        }
        joint["params"] = params;
        let supported: Vec<Value> = model
            .supported_joinery_axes(connection.members, s)
            .into_iter()
            .map(|(d, w)| json!({"depth_axis": d, "width_axis": w}))
            .collect();
        joint["supported_orientations"] = json!(supported);
        if let Some(overlap) = model.joinery_options(connection.members, s).0 {
            joint["overlap_mm"] = point_mm(overlap.bounds.1 - overlap.bounds.0);
        }
    }
    match joint_errors(model).get(&id) {
        Some(error) => joint["status"] = json!(error),
        None => joint["status"] = json!("ok"),
    }
    Some(joint)
}

/// Stable scene names for all joint kinds (mortise_tenon, tongue_groove, ...).
pub fn joint_kind(connection: &benchup_core::connections::Connection) -> &'static str {
    use benchup_core::joinery::JointKind;
    match &connection.operation {
        ConnectionOperation::Joinery(settings) => match settings.kind {
            JointKind::MortiseTenon => "mortise_tenon",
            JointKind::HalfLap => "half_lap",
            JointKind::TongueGroove => "tongue_groove",
            JointKind::Rabbet => "rabbet",
            JointKind::Dovetail => "dovetail",
            JointKind::Finger => "finger",
            JointKind::Biscuit => "biscuit",
            JointKind::Domino => "domino",
            JointKind::Groove => "groove",
            JointKind::LooseTongue => "loose_tongue",
        },
        ConnectionOperation::Fasteners(_) => "fasteners",
    }
}

fn joint_json(model: &Model, id: u64) -> Option<Value> {
    let connection = model.connections.get(&id)?;
    let kind = json!(joint_kind(connection));
    let mut joint = json!({"id": id, "a": connection.members[0], "b": connection.members[1], "kind": kind});
    let missing: Vec<u64> = connection.members.iter().copied().filter(|m| !model.contains_entity(m)).collect();
    if !missing.is_empty() {
        // Left behind when a board was erased (BenchUp3D/editor#36).
        joint["missing_boards"] = json!(missing);
    }
    Some(joint)
}

/// Everything the model needs to plan an edit, within `budget` characters.
/// Selected objects and those in the current editing scope are listed first.
pub fn summary(
    model: &Model,
    selection: &BTreeSet<u64>,
    catalog: &BTreeMap<MaterialId, StockMaterial>,
    budget: usize,
) -> Value {
    let stock = stock_list(model, catalog);
    let errors = joint_errors(model);
    let joints: Vec<_> = model
        .connections
        .keys()
        .take(100)
        .filter_map(|id| {
            let mut joint = joint_json(model, *id)?;
            if let Some(error) = errors.get(id) {
                joint["status"] = json!(error);
            }
            Some(joint)
        })
        .collect();
    let root = model.root_entities();
    let loose = |pick: &dyn Fn(&u64) -> bool| -> Vec<u64> { root.iter().copied().filter(|id| pick(id)).collect() };
    let (loose_edges, loose_faces) = (loose(&|id| model.edges.contains_key(id)), loose(&|id| model.faces.contains_key(id)));
    let mut out = json!({
        "units": "mm",
        "up_axis": "z",
        "axes": {"x": "width, left to right", "y": "depth; the front of a piece faces -y", "z": "height, up"},
        "edit_path": model.edit_path(),
        "selection": selection.iter().take(MAX_LISTED_SELECTION).collect::<Vec<_>>(),
        "tags": model.tags.values().take(MAX_LISTED_TAGS).map(|t| json!({"id": t.id, "name": t.name, "visible": t.visible})).collect::<Vec<_>>(),
        "stock_catalog": stock.iter().take(40).collect::<Vec<_>>(),
        "joints": joints,
        "loose_geometry": {
            "edges": loose_edges.len(),
            "faces": loose_faces.len(),
            "edge_ids": loose_edges.iter().take(MAX_LISTED_LOOSE).collect::<Vec<_>>(),
            "face_ids": loose_faces.iter().take(MAX_LISTED_LOOSE).collect::<Vec<_>>(),
        },
        "dimensions": model.dimensions.keys().take(MAX_LISTED_ANNOTATIONS).filter_map(|id| annotation_json(model, *id)).collect::<Vec<_>>(),
        "texts": model.texts.keys().take(MAX_LISTED_ANNOTATIONS).filter_map(|id| annotation_json(model, *id)).collect::<Vec<_>>(),
        "object_count": model.objects.len(),
    });
    let in_scope: BTreeSet<u64> = model.selectable_entities();
    let mut order: Vec<u64> = Vec::with_capacity(model.objects.len());
    order.extend(selection.iter().filter(|id| model.objects.contains_key(id)));
    order.extend(model.objects.keys().filter(|id| in_scope.contains(id) && !selection.contains(id)));
    order.extend(model.objects.keys().filter(|id| !in_scope.contains(id) && !selection.contains(id)));
    let mut used = out.to_string().len() + 160;
    let mut objects = Vec::new();
    for id in &order {
        let Some(entry) = object_entry(model, *id) else { continue };
        let cost = entry.to_string().len() + 1;
        if used + cost > budget {
            break;
        }
        used += cost;
        objects.push(entry);
    }
    if selection.len() > MAX_LISTED_SELECTION {
        out["selection_total"] = json!(selection.len());
    }
    let omitted = order.len() - objects.len();
    out["objects"] = Value::Array(objects);
    if omitted > 0 {
        out["omitted"] = json!(omitted);
        out["hint"] = json!("Scene truncated: use find_objects{query} or get_object{id} to inspect omitted objects.");
    }
    out
}

/// Face normals follow drawing winding, so a closed part can have inward ones.
/// Report and extrude along the outward direction. A ray from inside the face
/// along its stored normal crosses the part's other faces an odd number of
/// times when that normal points into the part. Returns the factor (1 or -1)
/// that maps the stored normal to the outward one. Open shapes and loose faces
/// keep their stored normal.
pub fn outward_sign(model: &Model, face_id: u64) -> f64 {
    let outward = || -> Option<f64> {
        let face = model.faces.get(&face_id)?;
        let normal = faces::plane(model, &face.vertices)?.normal;
        let owner = model.parent(face_id)?;
        // Start inside the face (its largest triangle), not at a concave face's centroid.
        let start = model
            .face_triangles(face)
            .into_iter()
            .max_by(|a, b| triangle_area(a).total_cmp(&triangle_area(b)))
            .map(|[a, b, c]| (a + b + c) / 3.)?;
        // A slight skew keeps the ray off edges and vertices of axis-aligned parts.
        let direction = (normal + DVec3::new(0.000_137, 0.000_291, 0.000_173)).normalize();
        let crossings: usize = model
            .expand_entities(&BTreeSet::from([owner]))
            .iter()
            .filter(|id| **id != face_id)
            .filter_map(|id| model.faces.get(id))
            .map(|other| model.face_triangles(other).iter().filter(|t| ray_hits(start, direction, t)).count())
            .sum();
        Some(if crossings % 2 == 1 { -1. } else { 1. })
    };
    outward().unwrap_or(1.)
}

fn triangle_area([a, b, c]: &[DVec3; 3]) -> f64 {
    (*b - *a).cross(*c - *a).length()
}

/// Möller–Trumbore: does the ray from `origin` along `direction` cross the triangle ahead of it?
fn ray_hits(origin: DVec3, direction: DVec3, [a, b, c]: &[DVec3; 3]) -> bool {
    const EPS: f64 = 1e-12;
    let (e1, e2) = (*b - *a, *c - *a);
    let p = direction.cross(e2);
    let det = e1.dot(p);
    if det.abs() < EPS {
        return false;
    }
    let t0 = origin - *a;
    let u = t0.dot(p) / det;
    if !(0. ..=1.).contains(&u) {
        return false;
    }
    let q = t0.cross(e1);
    let v = direction.dot(q) / det;
    if v < 0. || u + v > 1. {
        return false;
    }
    e2.dot(q) / det > 1e-9
}

/// A face as listed by get_object: id, normal, facing, center and area.
pub fn face_entry(model: &Model, id: u64) -> Option<Value> {
    face_json(model, id)
}

fn face_json(model: &Model, id: u64) -> Option<Value> {
    let face = model.faces.get(&id)?;
    let n = faces::plane(model, &face.vertices)?.normal * outward_sign(model, id);
    let facing = [(DVec3::X, "x"), (DVec3::Y, "y"), (DVec3::Z, "z")]
        .into_iter()
        .find(|(axis, _)| n.dot(*axis).abs() > 0.999)
        .map(|(axis, name)| format!("{}{name}", if n.dot(axis) > 0. { '+' } else { '-' }));
    let mut entry = json!({
        "id": id,
        "normal": unit(n),
        "area_mm2": (model.face_area(face) * 1.0e6).round(),
    });
    // The core has no centroid for some concave faces; any interior point will do.
    let center = model.face_center(face).or_else(|| {
        model.face_triangles(face).into_iter().max_by(|a, b| triangle_area(a).total_cmp(&triangle_area(b))).map(|[a, b, c]| (a + b + c) / 3.)
    });
    if let Some(center) = center {
        entry["center_mm"] = point_mm(center);
    }
    entry["facing"] = Value::Null;
    if let Some(facing) = facing {
        entry["facing"] = json!(facing);
    }
    Some(entry)
}

/// One entity in detail. Objects list their faces so `push_pull` can target one.
pub fn describe(model: &Model, id: u64) -> Option<Value> {
    if let Some(mut entry) = object_entry(model, id) {
        let object = &model.objects[&id];
        entry["children"] = json!(object.entities);
        let mut all: Vec<u64> = model
            .expand_entities(&BTreeSet::from([id]))
            .into_iter()
            .filter(|f| model.faces.contains_key(f))
            .collect();
        let total = all.len();
        all.truncate(MAX_FACES);
        entry["faces"] = Value::Array(all.iter().filter_map(|f| face_json(model, *f)).collect());
        if total > MAX_FACES {
            entry["faces_omitted"] = json!(total - MAX_FACES);
        }
        return Some(entry);
    }
    if let Some(face) = face_json(model, id) {
        let mut face = face;
        face["kind"] = json!("face");
        if let Some(parent) = model.parent(id) {
            face["parent"] = json!(parent);
        }
        return Some(face);
    }
    if let Some(joint) = joint_detail(model, id) {
        return Some(joint);
    }
    if let Some(annotation) = annotation_json(model, id) {
        return Some(annotation);
    }
    if model.contains_entity(&id) {
        let mut entry = json!({"id": id, "kind": kind_of(model, id)});
        if let Some(bounds) = bounds_json(model, &BTreeSet::from([id])) {
            entry["bounds_mm"] = json!({"min": bounds["min"], "max": bounds["max"]});
        }
        return Some(entry);
    }
    None
}

/// A dimension (its points and measured length) or a text note (its anchor and text).
pub fn annotation_json(model: &Model, id: u64) -> Option<Value> {
    if let Some(dimension) = model.dimensions.get(&id) {
        let (a, b) = dimension.points(model);
        let mut entry = json!({
            "id": id, "kind": "dimension", "start_mm": point_mm(a), "end_mm": point_mm(b),
            "offset_mm": point_mm(dimension.offset), "length_mm": mm(dimension.length(model)),
        });
        if let Some(axis) = dimension.axis {
            entry["axis"] = json!(axis_name(axis.vector()));
        }
        return Some(entry);
    }
    let text = model.texts.get(&id)?;
    Some(json!({
        "id": id, "kind": "text", "point_mm": point_mm(text.anchor.position(model)),
        "offset_mm": point_mm(text.offset), "text": text.text,
    }))
}

/// Details for entities the prompt names, keyed by id, within `budget`
/// characters; the rest are counted under `omitted` (use get_object for them).
pub fn references(model: &Model, ids: &BTreeSet<u64>, budget: usize) -> Value {
    let mut out = serde_json::Map::new();
    let mut used = 0;
    for (index, id) in ids.iter().enumerate() {
        let entry = describe(model, *id).unwrap_or_else(|| json!({"id": id, "kind": "missing"}));
        used += entry.to_string().len() + 8;
        if used > budget && index > 0 {
            out.insert("omitted".into(), json!(ids.len() - index));
            break;
        }
        out.insert(id.to_string(), entry);
    }
    Value::Object(out)
}
