//! Compact scene description sent to the model. Geometry in the document is in
//! metres; everything the model sees or sends is in millimetres.
use benchup_core::{
    DVec3, MaterialId, Model, ObjectKind, StockMaterial, connections::ConnectionOperation, faces,
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) const SCENE_BUDGET: usize = 24_000;
const MAX_FACES: usize = 80;
const MAX_LISTED_SELECTION: usize = 200;
const MAX_LISTED_TAGS: usize = 100;

/// Metres to millimetres, rounded to 0.1 mm.
pub(crate) fn mm(metres: f64) -> f64 {
    (metres * 10_000.).round() / 10.
}
pub(crate) fn from_mm(millimetres: f64) -> f64 {
    millimetres / 1000.
}
pub(crate) fn point_mm(p: DVec3) -> Value {
    json!([mm(p.x), mm(p.y), mm(p.z)])
}
fn unit(v: DVec3) -> Value {
    let r = |x: f64| (x * 1000.).round() / 1000.;
    json!([r(v.x), r(v.y), r(v.z)])
}

pub(crate) fn kind_of(model: &Model, id: u64) -> &'static str {
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

pub(crate) fn bounds_json(model: &Model, ids: &BTreeSet<u64>) -> Option<Value> {
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

pub(crate) fn object_entry(model: &Model, id: u64) -> Option<Value> {
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
    if let Some(material) = material_json(model, object) {
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

pub(crate) fn stock_list(
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

fn joint_json(model: &Model, id: u64) -> Option<Value> {
    let connection = model.connections.get(&id)?;
    let kind = match &connection.operation {
        ConnectionOperation::Joinery(settings) => json!(format!("{:?}", settings.kind)),
        ConnectionOperation::Fasteners(_) => json!("Fasteners"),
    };
    Some(json!({"id": id, "a": connection.members[0], "b": connection.members[1], "kind": kind}))
}

/// Everything the model needs to plan an edit, within `budget` characters.
/// Selected objects and those in the current editing scope are listed first.
pub(crate) fn summary(
    model: &Model,
    selection: &BTreeSet<u64>,
    catalog: &BTreeMap<MaterialId, StockMaterial>,
    budget: usize,
) -> Value {
    let stock = stock_list(model, catalog);
    let joints: Vec<_> = model.connections.keys().take(100).filter_map(|id| joint_json(model, *id)).collect();
    let mut out = json!({
        "units": "mm",
        "up_axis": "z",
        "edit_path": model.edit_path(),
        "selection": selection.iter().take(MAX_LISTED_SELECTION).collect::<Vec<_>>(),
        "tags": model.tags.values().take(MAX_LISTED_TAGS).map(|t| json!({"id": t.id, "name": t.name, "visible": t.visible})).collect::<Vec<_>>(),
        "stock_catalog": stock.iter().take(40).collect::<Vec<_>>(),
        "joints": joints,
        "loose_geometry": {
            "edges": model.root_entities().iter().filter(|id| model.edges.contains_key(id)).count(),
            "faces": model.root_entities().iter().filter(|id| model.faces.contains_key(id)).count(),
        },
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
/// Report and extrude along the outward direction: away from the owning
/// part's center. Returns the factor (1 or -1) that maps the stored normal to
/// the outward one.
pub(crate) fn outward_sign(model: &Model, face_id: u64) -> f64 {
    let outward = || -> Option<f64> {
        let face = model.faces.get(&face_id)?;
        let normal = faces::plane(model, &face.vertices)?.normal;
        let owner = model.parent(face_id)?;
        let center = model.selection_center(&BTreeSet::from([owner]))?;
        let toward_face = model.face_center(face)? - center;
        Some(if toward_face.dot(normal) < -1e-9 { -1. } else { 1. })
    };
    outward().unwrap_or(1.)
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
    if let Some(center) = model.face_center(face) {
        entry["center_mm"] = point_mm(center);
    }
    if let Some(facing) = facing {
        entry["facing"] = json!(facing);
    }
    Some(entry)
}

/// One entity in detail. Objects list their faces so `push_pull` can target one.
pub(crate) fn describe(model: &Model, id: u64) -> Option<Value> {
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
    if let Some(joint) = joint_json(model, id) {
        return Some(joint);
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

/// Details for entities the prompt names, keyed by id, within `budget`
/// characters; the rest are counted under `omitted` (use get_object for them).
pub(crate) fn references(model: &Model, ids: &BTreeSet<u64>, budget: usize) -> Value {
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
