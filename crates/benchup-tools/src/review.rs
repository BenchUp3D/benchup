//! Read-only reviews an agent cannot do by looking at the model: a cut list and
//! a check for problems such as overlapping boards without a joint.
use crate::scene::{self, mm, point_mm};
use benchup_core::{DVec3, Model, connections::ConnectionOperation, joinery::JointSettings};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

/// Most boards compared pairwise by `check`.
const MAX_CHECKED_BOARDS: usize = 600;
const MAX_LISTED_ISSUES: usize = 60;

/// Objects that hold board geometry only (faces and edges, no nested objects).
fn boards(model: &Model) -> Vec<u64> {
    model
        .objects
        .iter()
        .filter(|(id, object)| {
            !model.is_hidden(**id)
                && !object.entities.is_empty()
                && object.entities.iter().all(|e| model.faces.contains_key(e) || model.edges.contains_key(e))
                && object.entities.iter().any(|e| model.faces.contains_key(e))
        })
        .map(|(id, _)| *id)
        .collect()
}

/// A board's blank in its own axes: thickness, width and length (along the
/// grain when it is set, otherwise the longest side), in metres.
fn blank(model: &Model, id: u64) -> Option<[f64; 3]> {
    let frame = model.joinery_frame(id).ok()?;
    let (lo, hi) = frame.bounds(model, id)?;
    let extent = hi - lo;
    let axes: [DVec3; 3] = [DVec3::X, DVec3::Y, DVec3::Z].map(|a| (frame.world(a) - frame.world(DVec3::ZERO)).normalize_or_zero());
    let mut order = [0, 1, 2];
    order.sort_by(|a, b| extent[*a].total_cmp(&extent[*b]));
    let thickness = order[0];
    let (mut width, mut length) = (order[1], order[2]);
    if let Some(grain) = model.objects[&id].world_grain_axis() {
        if axes[width].dot(grain).abs() > axes[length].dot(grain).abs() {
            std::mem::swap(&mut width, &mut length);
        }
    }
    Some([extent[thickness], extent[width], extent[length]])
}

/// The board's stock material, or the nearest one set on a group around it.
fn material_name(model: &Model, id: u64) -> Option<String> {
    let mut current = Some(id);
    while let Some(object) = current.and_then(|c| model.objects.get(&c)) {
        if let Some(material) = model.object_stock_material(object) {
            return Some(material.name.clone());
        }
        current = model.parent(object.id);
    }
    None
}

fn board_bounds(model: &Model, id: u64) -> Option<(DVec3, DVec3)> {
    model.selection_bounds(&BTreeSet::from([id]))
}

/// Axes along which the shared region spans the whole board.
fn full_axes(board: (DVec3, DVec3), shared: DVec3) -> usize {
    let size = board.1 - board.0;
    (0..3).filter(|i| shared[*i] >= size[*i] - 1.0e-4).count()
}

/// Boards grouped by material and blank size, with totals per material.
pub fn cut_list(model: &Model) -> Value {
    let mut groups: BTreeMap<(String, [i64; 3]), (usize, BTreeSet<String>, Vec<u64>)> = BTreeMap::new();
    let mut skipped = Vec::new();
    for id in boards(model) {
        let Some(size) = blank(model, id) else {
            skipped.push(id);
            continue;
        };
        let material = material_name(model, id).unwrap_or_else(|| "unassigned".into());
        let key = (material, size.map(|m| (m * 10_000.).round() as i64));
        let entry = groups.entry(key).or_default();
        entry.0 += 1;
        entry.1.insert(model.objects[&id].name.clone());
        entry.2.push(id);
    }
    let mut totals: BTreeMap<String, (usize, f64, f64)> = BTreeMap::new();
    let pieces: Vec<Value> = groups
        .iter()
        .map(|((material, tenths), (count, names, ids))| {
            let [t, w, l] = tenths.map(|v| v as f64 / 10.);
            let total = totals.entry(material.clone()).or_default();
            total.0 += count;
            total.1 += l * w / 1.0e6 * *count as f64;
            total.2 += l * w * t / 1.0e9 * *count as f64;
            json!({"material": material, "length_mm": l, "width_mm": w, "thickness_mm": t, "quantity": count,
                "names": names, "ids": ids.iter().take(20).collect::<Vec<_>>()})
        })
        .collect();
    let materials: Vec<Value> = totals
        .iter()
        .map(|(name, (count, area, volume))| {
            let mut entry = json!({"material": name, "pieces": count,
                "area_m2": (area * 1000.).round() / 1000., "volume_m3": (volume * 10_000.).round() / 10_000.});
            let stock = model.stock_materials.values().find(|m| &m.name == name);
            if let Some(stock) = stock.filter(|s| s.dimensions[1] > 0 && s.dimensions[2] > 0) {
                let [_, w, l] = stock.dimensions.map(|v| v as f64 / 10.);
                let stock_area = w * l / 1.0e6;
                entry["stock_mm"] = json!({"width": w, "length": l});
                entry["minimum_stock_pieces"] = json!((area / stock_area).ceil() as u64);
            } else if name == "unassigned" {
                entry["note"] = json!("Assign a stock material with sizes (create_stock_material, set_material) to get minimum_stock_pieces.");
            }
            entry
        })
        .collect();
    let mut out = json!({
        "pieces": pieces,
        "materials": materials,
        "note": "Blank sizes include tenons and tongues (joints are cut from the overlap). \
                 minimum_stock_pieces is an area lower bound with no saw kerf, trimming or nesting; \
                 add a margin, or plan the layout in BenchUp.",
    });
    if !skipped.is_empty() {
        out["skipped"] = json!(skipped);
    }
    out
}

fn overlap(a: (DVec3, DVec3), b: (DVec3, DVec3)) -> Option<DVec3> {
    let size = a.1.min(b.1) - a.0.max(b.0);
    (size.min_element() > 1.0e-6).then_some(size)
}

/// Problems worth fixing before building: overlapping boards without a joint,
/// joints that cannot be cut or lost a board, open solids and loose geometry.
pub fn check(model: &Model) -> Value {
    let mut issues = Vec::new();
    let mut issue = |severity: &str, kind: &str, message: String, ids: Vec<u64>| {
        issues.push(json!({"severity": severity, "kind": kind, "message": message, "ids": ids}));
    };
    let joined: BTreeSet<[u64; 2]> = model
        .connections
        .values()
        .map(|c| [c.members[0].min(c.members[1]), c.members[0].max(c.members[1])])
        .collect();
    let all = boards(model);
    let checked = &all[..all.len().min(MAX_CHECKED_BOARDS)];
    let bounds: Vec<(u64, (DVec3, DVec3), bool)> = checked
        .iter()
        .filter_map(|id| {
            let aligned = model.joinery_frame(*id).is_ok_and(|f| f.is_project_aligned());
            Some((*id, model.selection_bounds(&BTreeSet::from([*id]))?, aligned))
        })
        .collect();
    for (i, (a, box_a, aligned_a)) in bounds.iter().enumerate() {
        for (b, box_b, aligned_b) in &bounds[i + 1..] {
            let Some(size) = overlap(*box_a, *box_b) else { continue };
            if joined.contains(&[(*a).min(*b), (*a).max(*b)]) {
                continue;
            }
            // Bounding boxes overlap; a notch around the other board means the solids do not.
            if model.joinery_options([*a, *b], &JointSettings::default()).0.is_none() {
                continue;
            }
            let (name_a, name_b) = (&model.objects[a].name, &model.objects[b].name);
            let approximate = if *aligned_a && *aligned_b { "" } else { " (measured on bounding boxes; a board is rotated)" };
            issue(
                "warning",
                "overlap_without_joint",
                format!("{name_a} and {name_b} overlap by {:?} mm without a joint{approximate}. Add a joint, or move one so they only touch.",
                    [mm(size.x), mm(size.y), mm(size.z)]),
                vec![*a, *b],
            );
        }
    }
    for (id, error) in scene::joint_errors(model) {
        let members = model.connections.get(&id).map_or(vec![id], |c| vec![id, c.members[0], c.members[1]]);
        issue("error", "joint_not_cut", format!("Joint {id} cannot be cut: {error}"), members);
    }
    for (id, connection) in &model.connections {
        let [a, b] = connection.members;
        if let (ConnectionOperation::Joinery(_), Some(box_a), Some(box_b)) = (&connection.operation, board_bounds(model, a), board_bounds(model, b)) {
            // The board that goes into the other is spanned by the shared region across its section.
            if let Some(shared) = overlap(box_a, box_b) {
                if full_axes(box_b, shared) > full_axes(box_a, shared) {
                    issue(
                        "warning",
                        "joint_roles_reversed",
                        format!("Joint {id} looks reversed: {} goes into {}, so it should be a and {} b (remove_joint, then add_joint).",
                            model.objects[&b].name, model.objects[&a].name, model.objects[&a].name),
                        vec![*id, a, b],
                    );
                }
            }
        }
        let missing: Vec<u64> = connection.members.iter().copied().filter(|m| !model.contains_entity(m)).collect();
        if !missing.is_empty() && matches!(connection.operation, ConnectionOperation::Joinery(_)) {
            issue("error", "joint_without_board", format!("Joint {id} refers to erased boards {missing:?}; remove it with remove_joint."), vec![*id]);
        }
    }
    for id in &all {
        if let Err(reason) = model.solid_volume(*id) {
            issue("warning", "open_solid", format!("{} is not a closed solid: {reason}", model.objects[id].name), vec![*id]);
        }
    }
    let loose: Vec<u64> = model.root_entities().into_iter().filter(|id| model.edges.contains_key(id) || model.faces.contains_key(id)).collect();
    if !loose.is_empty() {
        issue("info", "loose_geometry", format!("{} loose edges and faces at the top level; group them into a part or erase them.", loose.len()),
            loose.into_iter().take(20).collect());
    }
    let unassigned: Vec<u64> = all.iter().copied().filter(|id| material_name(model, *id).is_none()).collect();
    if !unassigned.is_empty() {
        issue("info", "no_material", format!("{} boards have no stock material.", unassigned.len()), unassigned.into_iter().take(20).collect());
    }
    let total = issues.len();
    let errors = issues.iter().filter(|i| i["severity"] == "error").count();
    let warnings = issues.iter().filter(|i| i["severity"] == "warning").count();
    issues.truncate(MAX_LISTED_ISSUES);
    let mut out = json!({"boards": all.len(), "errors": errors, "warnings": warnings, "issues": issues});
    if total > MAX_LISTED_ISSUES {
        out["issues_omitted"] = json!(total - MAX_LISTED_ISSUES);
    }
    if all.len() > MAX_CHECKED_BOARDS {
        out["note"] = json!(format!("Only the first {MAX_CHECKED_BOARDS} boards were compared for overlaps."));
    }
    // The furniture's size: boards only, not dimensions or texts drawn around them.
    let extents = all.iter().filter_map(|id| board_bounds(model, *id)).reduce(|x, y| (x.0.min(y.0), x.1.max(y.1)));
    if let Some((lo, hi)) = extents {
        out["model_bounds_mm"] = json!({"min": point_mm(lo), "max": point_mm(hi), "size": point_mm(hi - lo)});
    }
    out
}
