//! Read-only reviews an agent cannot do by looking at the model: a cut list and
//! a check for problems such as overlapping boards without a joint.
use crate::scene::{self, mm, point_mm};
use benchup_core::{DVec3, Model, StockMaterial, connections::ConnectionOperation, joinery::JointSettings};
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
/// Found through the assignment, not the name: two materials may share one.
fn material(model: &Model, id: u64) -> Option<&StockMaterial> {
    let mut current = Some(id);
    while let Some(object) = current.and_then(|c| model.objects.get(&c)) {
        if let Some(material) = model.object_stock_material(object) {
            return Some(material);
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
    // A material is its name and stock size, so same-named materials of
    // different sizes stay apart; `None` is unassigned.
    type Material = Option<(String, [u64; 3])>;
    /// Count, names and ids of the boards of one material and blank size.
    type Group = (usize, BTreeSet<String>, Vec<u64>);
    let mut groups: BTreeMap<(Material, [i64; 3]), Group> = BTreeMap::new();
    let mut skipped = Vec::new();
    for id in boards(model) {
        let Some(size) = blank(model, id) else {
            skipped.push(id);
            continue;
        };
        let material = material(model, id).map(|m| (m.name.clone(), m.dimensions));
        let key = (material, size.map(|m| (m * 10_000.).round() as i64));
        let entry = groups.entry(key).or_default();
        entry.0 += 1;
        entry.1.insert(model.objects[&id].name.clone());
        entry.2.push(id);
    }
    let name = |material: &Material| material.as_ref().map_or("unassigned", |(name, _)| name.as_str()).to_owned();
    let mut totals: BTreeMap<Material, (usize, f64, f64)> = BTreeMap::new();
    let pieces: Vec<Value> = groups
        .iter()
        .map(|((material, tenths), (count, names, ids))| {
            let [t, w, l] = tenths.map(|v| v as f64 / 10.);
            let total = totals.entry(material.clone()).or_default();
            total.0 += count;
            total.1 += l * w / 1.0e6 * *count as f64;
            total.2 += l * w * t / 1.0e9 * *count as f64;
            json!({"material": name(material), "length_mm": l, "width_mm": w, "thickness_mm": t, "quantity": count,
                "names": names, "ids": ids.iter().take(20).collect::<Vec<_>>()})
        })
        .collect();
    let materials: Vec<Value> = totals
        .iter()
        .map(|(material, (count, area, volume))| {
            let mut entry = json!({"material": name(material), "pieces": count,
                "area_m2": (area * 1000.).round() / 1000., "volume_m3": (volume * 10_000.).round() / 10_000.});
            match material {
                Some((_, dimensions)) if dimensions[1] > 0 && dimensions[2] > 0 => {
                    let [t, w, l] = dimensions.map(|v| v as f64 / 10.);
                    let stock_area = w * l / 1.0e6;
                    entry["stock_mm"] = json!({"thickness": t, "width": w, "length": l});
                    entry["minimum_stock_pieces"] = json!((area / stock_area).ceil() as u64);
                }
                Some(_) => {}
                None => {
                    entry["note"] = json!("Assign a stock material with sizes (create_stock_material, set_material) to get minimum_stock_pieces.");
                }
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
    summarize(model, collect(model), None)
}

/// Like [`check`], listing only the problems that are not in `baseline` (the
/// model before this batch). An issue is the same when its kind and ids match.
pub fn check_new(model: &Model, baseline: Option<&Model>) -> Value {
    let found = collect(model);
    let Some(baseline) = baseline else {
        let existing = found.issues.len();
        let mut out = summarize(model, Found { issues: Vec::new(), ..found }, Some(existing));
        out["note"] = json!("Nothing has changed yet in this batch, so there are no new problems. Run check_model without only_new to see all.");
        return out;
    };
    let key = |issue: &Value| -> (String, Vec<u64>) {
        let mut ids: Vec<u64> = issue["ids"].as_array().into_iter().flatten().filter_map(Value::as_u64).collect();
        ids.sort_unstable();
        (issue["kind"].as_str().unwrap_or_default().to_owned(), ids)
    };
    // Grouping gives boards new ids, so an issue also counts as existing when
    // its message (names, joint ids, sizes) is unchanged.
    let message = |issue: &Value| (issue["kind"].as_str().unwrap_or_default().to_owned(), issue["message"].as_str().unwrap_or_default().to_owned());
    let earlier = collect(baseline).issues;
    let old: BTreeSet<(String, Vec<u64>)> = earlier.iter().map(key).collect();
    let old_messages: BTreeSet<(String, String)> = earlier.iter().map(message).collect();
    let total = found.issues.len();
    let (existing, issues): (Vec<Value>, Vec<Value>) =
        found.issues.into_iter().partition(|issue| old.contains(&key(issue)) || old_messages.contains(&message(issue)));
    let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
    for issue in &existing {
        *kinds.entry(issue["kind"].as_str().unwrap_or_default().to_owned()).or_default() += 1;
    }
    debug_assert_eq!(total, issues.len() + existing.len());
    let mut out = summarize(model, Found { issues, ..found }, Some(existing.len()));
    out["existing_by_kind"] = json!(kinds);
    out
}

struct Found {
    issues: Vec<Value>,
    joint_errors: BTreeMap<u64, String>,
    boards: Vec<u64>,
}

fn summarize(model: &Model, found: Found, existing: Option<usize>) -> Value {
    let Found { mut issues, joint_errors, boards: all } = found;
    let total = issues.len();
    let errors = issues.iter().filter(|i| i["severity"] == "error").count();
    let warnings = issues.iter().filter(|i| i["severity"] == "warning").count();
    issues.truncate(MAX_LISTED_ISSUES);
    let wood_joints = model.connections.values().filter(|c| matches!(c.operation, ConnectionOperation::Joinery(_))).count();
    let failing: Vec<u64> = joint_errors.keys().copied().filter(|id| model.connections.contains_key(id)).collect();
    let mut out = json!({
        "passed": errors == 0 && warnings == 0,
        "boards": all.len(), "errors": errors, "warnings": warnings,
        "joints": {"total": wood_joints, "ok": wood_joints - failing.len().min(wood_joints), "failing": failing},
        "issues": issues,
    });
    if !model.routes.is_empty() {
        let failing: Vec<u64> = joint_errors.keys().copied().filter(|id| model.routes.contains_key(id)).collect();
        out["profiles"] = json!({"total": model.routes.len(), "failing": failing});
    }
    if let Some(existing) = existing {
        out["only_new"] = json!(true);
        out["existing_issues"] = json!(existing);
    }
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

fn collect(model: &Model) -> Found {
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
    let mut machined: Option<Model> = None;
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
            // Nor do they when a router profile (a rabbet, a groove) removed the shared wood.
            let routed = |id: u64| model.objects[&id].entities.iter().any(|e| model.routes.contains_key(e));
            if routed(*a) || routed(*b) {
                let evaluated = machined.get_or_insert_with(|| model.evaluate_joinery().model);
                if evaluated.objects.contains_key(a) && evaluated.objects.contains_key(b)
                    && evaluated.joinery_options([*a, *b], &JointSettings::default()).0.is_none()
                {
                    continue;
                }
            }
            let (name_a, name_b) = (&model.objects[a].name, &model.objects[b].name);
            // One board crossing the other completely, cutting it in two: the
            // shared region spans the other on two axes and it continues on
            // both sides along the third. A housed end only notches the board.
            let shared = (box_a.0.max(box_b.0), box_a.1.min(box_b.1));
            let splits = |board: (DVec3, DVec3)| {
                let full: Vec<usize> = (0..3).filter(|i| shared.1[*i] - shared.0[*i] >= board.1[*i] - board.0[*i] - 1.0e-4).collect();
                full.len() == 2 && {
                    let i = 3 - full[0] - full[1];
                    board.0[i] < shared.0[i] - 1.0e-4 && board.1[i] > shared.1[i] + 1.0e-4
                }
            };
            if splits(*box_a) || splits(*box_b) {
                let (through, pierced) = if splits(*box_b) { (name_a, name_b) } else { (name_b, name_a) };
                issue(
                    "error",
                    "passes_through",
                    format!("{through} passes right through {pierced}, cutting it in two; a joint cannot fix this. Shorten or move it."),
                    vec![*a, *b],
                );
                continue;
            }
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
    let joint_errors = scene::joint_errors(model);
    for (id, error) in &joint_errors {
        let id = *id;
        if model.routes.contains_key(&id) {
            let board = model.route_board(id);
            let name = board.and_then(|b| model.objects.get(&b)).map_or("loose geometry", |o| o.name.as_str());
            issue("error", "profile_not_cut", format!("The router profile on edge {id} of {name} cannot be cut: {error}"),
                board.into_iter().chain([id]).collect());
            continue;
        }
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
                        format!("Joint {id} looks reversed: {} goes into {}, so it should be a and {} b (flip_joint).",
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
    for id in &all {
        let Some(stock) = material(model, *id).filter(|m| m.dimensions[0] > 0) else { continue };
        let name = &stock.name;
        let Some([thickness, ..]) = blank(model, *id) else { continue };
        let expected = stock.dimensions[0] as f64 / 10.;
        if (mm(thickness) - expected).abs() > 0.5 {
            issue("warning", "thickness_mismatch",
                format!("{} is {} mm thick but its material {name} is {expected} mm.", model.objects[id].name, mm(thickness)), vec![*id]);
        }
    }
    let unassigned: Vec<u64> = all.iter().copied().filter(|id| material(model, *id).is_none()).collect();
    if !unassigned.is_empty() {
        issue("info", "no_material", format!("{} boards have no stock material.", unassigned.len()), unassigned.into_iter().take(20).collect());
    }
    Found { issues, joint_errors, boards: all }
}
