//! Paired pockets for purchased inserts and continuous grooves. Like the
//! traditional profiles, these machine source stock only; inserts are separate
//! workshop parts. All tessellation happens during cached CSG evaluation.
use super::*;

pub(super) fn fits(stock: &JointStock, settings: &JointSettings) -> bool {
    fits_bounds(stock.geometry, stock.bounds, settings)
}

pub(super) fn fits_bounds(geometry: JoineryOverlap, bounds: [(DVec3, DVec3); 2], settings: &JointSettings) -> bool {
    if !settings.valid() {
        return false;
    }
    let (lo, hi) = geometry.bounds;
    let (d, u) = (settings.depth_axis, settings.width_axis);
    let v = 3 - d - u;
    let center = (lo + hi) * 0.5;
    let allowance = settings.clearance;
    if settings.pocket_thickness + 2. * allowance >= hi[v] - lo[v] - EPS {
        return false;
    }
    if matches!(settings.kind, JointKind::Biscuit | JointKind::Domino) {
        if (settings.pocket_width + 2. * allowance) * settings.pocket_count as f64
            >= hi[u] - lo[u] - EPS
        {
            return false;
        }
        if settings.kind == JointKind::Domino && settings.pocket_width < settings.pocket_thickness {
            return false;
        }
        // A biscuit's two circular arcs must meet at pointed ends.
        if settings.kind == JointKind::Biscuit
            && settings.pocket_depth >= settings.pocket_width * 0.5
        {
            return false;
        }
    }
    let forward = joint_direction(bounds[0], bounds[1], lo, hi, d).unwrap_or(
        bounds[0].0[d] + bounds[0].1[d] <= bounds[1].0[d] + bounds[1].1[d],
    );
    bounds.iter().enumerate().all(|(i, bounds)| {
        if settings.kind == JointKind::Groove && i == 0 {
            return true;
        }
        let available = if (i == 0) == forward {
            center[d] - bounds.0[d]
        } else {
            bounds.1[d] - center[d]
        };
        settings.pocket_depth < available - EPS
    })
}

/// Extrude a convex ring along a stock axis, orienting every face outward in
/// world coordinates (also handles axis permutations and rotated boards).
fn extrusion(
    ring: &[DVec3],
    axis: usize,
    low: f64,
    high: f64,
    axes: JoineryFrame,
    frame: Frame,
) -> Result<Vec<Polygon>, &'static str> {
    let mut ring = ring.to_vec();
    ring.dedup_by(|a, b| a.distance_squared(*b) < EPS.powi(2));
    if ring.len() > 1 && ring[0].distance_squared(*ring.last().unwrap()) < EPS.powi(2) {
        ring.pop();
    }
    let layer = |height: f64| {
        ring.iter()
            .map(|p| {
                let mut p = *p;
                p[axis] = height;
                frame.local(axes.world(p))
            })
            .collect::<Vec<_>>()
    };
    let bottom = layer(low);
    let top = layer(high);
    let center = bottom.iter().chain(&top).copied().sum::<DVec3>() / (2 * ring.len()) as f64;
    let mut faces = vec![bottom.clone(), top.clone()];
    for i in 0..ring.len() {
        let j = (i + 1) % ring.len();
        faces.push(vec![bottom[i], bottom[j], top[j], top[i]]);
    }
    faces
        .into_iter()
        .map(|vertices| {
            let mut face = Polygon::new(vertices, None, frame.epsilon).ok_or(tr!(
                "core.joinery.joint_details_are_too_small_for_the_model"
            ))?;
            if face.normal.dot(face.vertices[0] - center) < 0. {
                face.flip();
            }
            Ok(face)
        })
        .collect()
}

fn pocket(
    stock: &JointStock,
    settings: &JointSettings,
    index: u32,
    clearance: f64,
) -> Result<Vec<Polygon>, &'static str> {
    let (lo, hi) = stock.geometry.bounds;
    let (d, u) = (settings.depth_axis, settings.width_axis);
    let v = 3 - d - u;
    let mut center = (lo + hi) * 0.5;
    center[u] = lo[u] + (hi[u] - lo[u]) * (index as f64 + 0.5) / settings.pocket_count as f64;
    let half_width = settings.pocket_width * 0.5 + clearance;
    let half_thickness = settings.pocket_thickness * 0.5 + clearance;
    let depth = settings.pocket_depth;
    let mut ring = Vec::new();
    match settings.kind {
        JointKind::Biscuit => {
            // Lens formed by two circular cutter arcs in the insertion plane.
            let radius = (half_width * half_width + depth * depth) / (2. * depth);
            let angle = (half_width / radius).asin();
            for side in [-1., 1.] {
                for i in 0..=12 {
                    let theta = -angle + 2. * angle * i as f64 / 12.;
                    let mut p = center;
                    p[u] += side * radius * theta.sin();
                    p[d] += side * (radius * theta.cos() - (radius - depth));
                    // The next arc shares this tip; do not duplicate vertices.
                    if i < 12 {
                        ring.push(p);
                    }
                }
            }
            extrusion(
                &ring,
                v,
                center[v] - half_thickness,
                center[v] + half_thickness,
                stock.geometry.frame,
                stock.frame,
            )
        }
        JointKind::Domino => {
            // Rounded-end mortise: semicircular ends in the width/thickness plane.
            let straight = half_width - half_thickness;
            for side in [-1., 1.] {
                for i in 0..=8 {
                    let theta = -std::f64::consts::FRAC_PI_2 + std::f64::consts::PI * i as f64 / 8.;
                    let mut p = center;
                    p[u] += side * (straight + half_thickness * theta.cos());
                    p[v] += side * half_thickness * theta.sin();
                    ring.push(p);
                }
            }
            extrusion(
                &ring,
                d,
                center[d] - depth,
                center[d] + depth,
                stock.geometry.frame,
                stock.frame,
            )
        }
        JointKind::Groove | JointKind::LooseTongue => {
            let (mut l, mut h) = (lo, hi);
            l[d] = center[d] - depth;
            h[d] = center[d] + depth;
            l[v] = center[v] - half_thickness;
            h[v] = center[v] + half_thickness;
            prism(
                box_points(l, h).map(|p| stock.geometry.frame.world(p)),
                stock.frame,
            )
        }
        _ => unreachable!(),
    }
}

/// An end-to-end rectangular board needs one face with pocket openings and
/// the clipped cutter walls. Construct that boundary directly: repeated BSP
/// subtraction otherwise splits every previous rounded wall for every insert.
/// Angled intersections, previously machined stock and multi-colored planes
/// retain the general solid-boolean path below.
fn cut_box(stock: &JointStock, settings: &JointSettings, incoming: &[Polygon], member: usize,
    tools: &[Vec<Polygon>], lower: bool) -> Option<Vec<Polygon>> {
    let (lo, hi) = stock.geometry.bounds;
    let (d, u) = (settings.depth_axis, settings.width_axis);
    let v = 3 - d - u;
    let (mut a, mut b) = stock.bounds[member];
    if [u, v].into_iter().any(|axis| a[axis] < lo[axis] - EPS || b[axis] > hi[axis] + EPS)
        || (lower && b[d] > hi[d] + EPS) || (!lower && a[d] < lo[d] - EPS) { return None; }
    let local = |p| stock.geometry.frame.local(stock.frame.world(p));
    let mut colors = [None; 6];
    let mut seen = [false; 6];
    for polygon in incoming {
        let points: Vec<_> = polygon.vertices.iter().copied().map(local).collect();
        let plane = (0..6).find(|&plane| points.iter().all(|p|
            (p[plane / 2] - if plane % 2 == 0 { a[plane / 2] } else { b[plane / 2] }).abs() <= EPS))?;
        if seen[plane] && colors[plane] != polygon.color { return None; }
        seen[plane] = true;
        colors[plane] = polygon.color;
    }
    if !seen.into_iter().all(|seen| seen) { return None; }
    let expected = (b - a).element_product();
    // Confirm the incoming shell is still the complete rectangular stock.
    let frame_scale = stock.frame.world(DVec3::X).distance(stock.frame.world(DVec3::ZERO));
    if (csg::volume(incoming).abs() * frame_scale.powi(3) - expected).abs() > expected * 1e-8 { return None; }
    let seam = (lo[d] + hi[d]) * 0.5;
    if lower { b[d] = seam; } else { a[d] = seam; }
    let mut shell = prism(box_points(a, b).map(|p| stock.geometry.frame.world(p)), stock.frame).ok()?;
    let face_index = shell.iter().position(|face| face.vertices.iter().all(|p| (local(*p)[d] - seam).abs() <= EPS))?;
    let opening_face = shell.remove(face_index);
    for face in &mut shell {
        let points: Vec<_> = face.vertices.iter().copied().map(local).collect();
        let (original_a, original_b) = stock.bounds[member];
        if let Some(plane) = (0..6).find(|&plane| points.iter().all(|p|
            (p[plane / 2] - if plane % 2 == 0 { original_a[plane / 2] } else { original_b[plane / 2] }).abs() <= EPS)) {
            face.color = colors[plane];
        }
    }
    let mut holes = Vec::new();
    for tool in tools {
        let mut opening: Vec<DVec3> = Vec::new();
        for face in tool {
            let points: Vec<_> = face.vertices.iter().copied().map(local).collect();
            let mut clipped = Vec::new();
            for (&p, &q) in points.iter().zip(points.iter().cycle().skip(1)) {
                let side = |p: DVec3| if lower { seam - p[d] } else { p[d] - seam };
                if side(p) >= -EPS { clipped.push(p); }
                if (side(p) > 0. && side(q) < 0.) || (side(p) < 0. && side(q) > 0.) {
                    clipped.push(p.lerp(q, side(p) / (side(p) - side(q))));
                }
            }
            for p in &mut clipped {
                if (p[d] - seam).abs() <= EPS {
                    p[d] = seam;
                    if !opening.iter().any(|q| q.distance(*p) <= EPS) { opening.push(*p); }
                }
            }
            if clipped.len() < 3 || clipped.iter().all(|p| (p[d] - seam).abs() <= EPS) { continue; }
            let mut wall = Polygon::new(clipped.into_iter().map(|p|
                stock.frame.local(stock.geometry.frame.world(p))).collect(), face.color, stock.frame.epsilon)?;
            wall.flip();
            shell.push(wall);
        }
        if opening.len() < 3 { return None; }
        let center = opening.iter().copied().sum::<DVec3>() / opening.len() as f64;
        opening.sort_by(|a, b| (a[v] - center[v]).atan2(a[u] - center[u])
            .total_cmp(&(b[v] - center[v]).atan2(b[u] - center[u])));
        holes.push(opening.into_iter().map(|p| stock.geometry.frame.world(p)).collect::<Vec<_>>());
    }
    // Triangulate the one planar face with all openings together. The ordinary
    // solid output still merges coplanar triangles and validates the result.
    let mut mesh = Model::default();
    let mut rings = Vec::new();
    for points in std::iter::once(opening_face.vertices.iter().map(|p| stock.frame.world(*p)).collect()).chain(holes) {
        let mut ring = Vec::new();
        for point in points {
            let id = mesh.id();
            mesh.vertices.insert(id, crate::Vertex { id, position: point });
            ring.push(id);
        }
        rings.push(ring);
    }
    let face = crate::Face { id: 0, vertices: rings.remove(0), holes: rings };
    let triangles = mesh.face_triangles(&face);
    if triangles.is_empty() { return None; }
    for triangle in triangles {
        // Aligned openings can produce collapsed triangulation fragments from
        // floating-point roundoff. Discard them as the solid importer does.
        let Some(mut polygon) = Polygon::new(triangle.map(|p| stock.frame.local(p)).to_vec(), opening_face.color, stock.frame.epsilon) else {
            continue;
        };
        if polygon.normal.dot(opening_face.normal) < 0. { polygon.flip(); }
        shell.push(polygon);
    }
    Some(shell)
}

pub(super) fn cut(
    stock: &JointStock,
    settings: &JointSettings,
    mut solids: [Vec<Polygon>; 2],
) -> Result<[Vec<Polygon>; 2], &'static str> {
    if !fits(stock, settings) {
        return Err(tr!("core.joinery.pockets_do_not_fit"));
    }
    let (lo, hi) = stock.geometry.bounds;
    let d = settings.depth_axis;
    let seam = (lo[d] + hi[d]) * 0.5;
    let forward = joint_direction(stock.bounds[0], stock.bounds[1], lo, hi, d).unwrap_or(
        stock.bounds[0].0[d] + stock.bounds[0].1[d] <= stock.bounds[1].0[d] + stock.bounds[1].1[d],
    );
    for (i, solid) in solids.iter_mut().enumerate() {
        let rounded = matches!(settings.kind, JointKind::Biscuit | JointKind::Domino);
        let count = if rounded { settings.pocket_count } else { 1 };
        let tools = (0..count).map(|index| pocket(stock, settings, index,
            if i == 1 { settings.clearance } else { 0. })).collect::<Result<Vec<_>, _>>()?;
        if rounded && let Some(result) = cut_box(stock, settings, solid, i, &tools, (i == 0) == forward) {
            *solid = result;
            continue;
        }
        // Partition only the real shared material, never the overlap's box.
        let (mut l, mut h) = (lo, hi);
        if (i == 0) == forward {
            l[d] = seam;
        } else {
            h[d] = seam;
        }
        let half = prism(
            box_points(l, h).map(|p| stock.geometry.frame.world(p)),
            stock.frame,
        )?;
        let waste = csg::apply(
            stock.overlap.clone(),
            half,
            Boolean::Intersect,
            stock.frame.epsilon,
        )?;
        *solid = csg::apply(
            solid.clone(),
            waste,
            Boolean::Difference,
            stock.frame.epsilon,
        )?;
        if settings.kind == JointKind::Groove && i == 0 {
            continue;
        }
        for tool in tools {
            let before = csg::volume(solid).abs();
            *solid = csg::apply(
                solid.clone(),
                tool,
                Boolean::Difference,
                stock.frame.epsilon,
            )?;
            if before - csg::volume(solid).abs() <= stock.frame.epsilon.powi(3) * 8. {
                return Err(tr!("core.joinery.pockets_do_not_fit"));
            }
        }
    }
    Ok(solids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connections::{Connection, ConnectionOperation};
    use crate::{Document, SolidOperation};

    const KINDS: [JointKind; 4] = [
        JointKind::Biscuit,
        JointKind::Domino,
        JointKind::Groove,
        JointKind::LooseTongue,
    ];

    #[test]
    fn direct_rounded_pockets_match_boolean_machining_and_form_closed_solids() {
        for kind in [JointKind::Biscuit, JointKind::Domino] {
            for count in [1, 4, 16] {
                let mut model = Model::default();
                let members = [
                    super::super::tests::board_at(&mut model, DVec3::ZERO, DVec3::new(0.3, 0.8, 0.04), false),
                    super::super::tests::board_at(&mut model, DVec3::new(0.25, 0., 0.), DVec3::new(0.55, 0.8, 0.04), false),
                ];
                let stock = JointStock::new(&model, members).unwrap();
                let settings = JointSettings { kind, pocket_count: count, ..Default::default() };
                let (lo, hi) = stock.geometry.bounds;
                let seam = (lo.x + hi.x) * 0.5;
                for member in 0..2 {
                    let tools = (0..count).map(|index| pocket(&stock, &settings, index,
                        if member == 1 { settings.clearance } else { 0. }).unwrap()).collect::<Vec<_>>();
                    let fast = cut_box(&stock, &settings, &stock.solids[member], member, &tools, member == 0)
                        .unwrap_or_else(|| panic!("direct pocket cut failed: {kind:?}, count={count}, member={member}"));
                    let (mut l, mut h) = (lo, hi);
                    if member == 0 { l.x = seam; } else { h.x = seam; }
                    let half = prism(box_points(l, h).map(|p| stock.geometry.frame.world(p)), stock.frame).unwrap();
                    let waste = csg::apply(stock.overlap.clone(), half, Boolean::Intersect, stock.frame.epsilon).unwrap();
                    let mut reference = csg::apply(stock.solids[member].clone(), waste, Boolean::Difference, stock.frame.epsilon).unwrap();
                    for tool in tools {
                        reference = csg::apply(reference, tool, Boolean::Difference, stock.frame.epsilon).unwrap();
                    }
                    assert!((csg::volume(&fast) - csg::volume(&reference)).abs() < 1e-9,
                        "{kind:?}, count={count}, member={member}");
                    let mut result = Model::default();
                    let id = solids::output(&mut result, fast.clone(), stock.frame, "Pocket result", false, true).unwrap().unwrap();
                    result.validate().unwrap();
                    assert!(result.solid_volume(id).unwrap() > 0.);
                    assert!(cut_box(&stock, &settings, &fast, member, &[], member == 0).is_none(),
                        "previously machined boards must keep their existing cuts");
                }
            }
        }
    }

    #[test]
    fn pockets_remove_their_measured_volume_and_preserve_source_relations() {
        for kind in KINDS {
            for reverse in [false, true] {
                let (mut source, mut members) = super::super::tests::pair();
                if reverse {
                    members.swap(0, 1);
                }
                let settings = JointSettings {
                    kind,
                    clearance: 0.,
                    pocket_count: 2,
                    ..Default::default()
                };
                let stock = JointStock::new(&source, members).unwrap();
                let scale =
                    source.solid_volume(members[0]).unwrap() / csg::volume(&stock.solids[0]).abs();
                let count = if matches!(kind, JointKind::Biscuit | JointKind::Domino) {
                    2
                } else {
                    1
                };
                let pocket_volume = (0..count)
                    .map(|index| {
                        csg::volume(&pocket(&stock, &settings, index, 0.).unwrap()).abs() * scale
                    })
                    .sum::<f64>();
                let expected_removed =
                    pocket_volume * if kind == JointKind::Groove { 0.5 } else { 1. };
                let union_volume = members
                    .iter()
                    .map(|id| source.solid_volume(*id).unwrap())
                    .sum::<f64>()
                    - csg::volume(&stock.overlap).abs() * scale;
                let id = source
                    .add_connection(Connection {
                        members,
                        entry_face: None,
                        operation: ConnectionOperation::Joinery(settings),
                    })
                    .unwrap();
                let saved = source.clone();
                let result = source.evaluate_joinery();
                assert!(
                    result.errors.is_empty(),
                    "{kind:?}, reverse={reverse}: {:?}",
                    result.errors
                );
                result.model.validate().unwrap();
                let total = members
                    .iter()
                    .map(|id| result.model.solid_volume(*id).unwrap())
                    .sum::<f64>();
                assert!(
                    (union_volume - total - expected_removed).abs() < 1e-9,
                    "{kind:?}"
                );
                let mut model = result.model;
                assert!(
                    model
                        .boolean_solids(members[0], members[1], SolidOperation::Intersect)
                        .is_err()
                );
                assert_eq!(source, saved);
                let restored = Document::from_json(
                    &serde_json::to_string(&Document {
                        model: source.clone(),
                        ..Default::default()
                    })
                    .unwrap(),
                )
                .unwrap();
                assert_eq!(restored.model.connections[&id], source.connections[&id]);
                assert!(restored.model.evaluate_joinery().errors.is_empty());
            }
        }
    }

    #[test]
    fn groove_cuts_one_member_and_loose_tongue_cuts_both() {
        let (source, members) = super::super::tests::pair();
        let mut groove = source.clone();
        let mut tongue = source.clone();
        groove
            .test_cut_joint(
                members,
                &JointSettings {
                    kind: JointKind::Groove,
                    clearance: 0.,
                    ..Default::default()
                },
            )
            .unwrap();
        tongue
            .test_cut_joint(
                members,
                &JointSettings {
                    kind: JointKind::LooseTongue,
                    clearance: 0.,
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(
            (groove.solid_volume(members[1]).unwrap() - tongue.solid_volume(members[1]).unwrap())
                .abs()
                < 1e-10
        );
        let slot = 0.1 * 0.004 * 0.01;
        assert!(
            (groove.solid_volume(members[0]).unwrap()
                - tongue.solid_volume(members[0]).unwrap()
                - slot)
                .abs()
                < 1e-10
        );
    }

    #[test]
    fn domino_capsule_and_biscuit_lens_have_distinct_rounded_profiles() {
        let (model, members) = super::super::tests::pair();
        let stock = JointStock::new(&model, members).unwrap();
        let scale = model.solid_volume(members[0]).unwrap() / csg::volume(&stock.solids[0]).abs();
        let settings = JointSettings {
            kind: JointKind::Domino,
            clearance: 0.,
            ..Default::default()
        };
        let domino = pocket(&stock, &settings, 0, 0.).unwrap();
        let radius = settings.pocket_thickness * 0.5;
        // Rounded ends are polygonized with eight segments per semicircle.
        let circle_area = 8. * radius.powi(2) * (std::f64::consts::PI / 8.).sin();
        let expected = ((settings.pocket_width - settings.pocket_thickness)
            * settings.pocket_thickness
            + circle_area)
            * 2.
            * settings.pocket_depth;
        assert!((csg::volume(&domino).abs() * scale - expected).abs() < 1e-12);
        let biscuit = pocket(
            &stock,
            &JointSettings {
                kind: JointKind::Biscuit,
                ..settings.clone()
            },
            0,
            0.,
        )
        .unwrap();
        assert!(biscuit.len() > 8 && domino.len() > 8);
        assert!((csg::volume(&biscuit).abs() - csg::volume(&domino).abs()).abs() > 1e-8);
        // Equal width/thickness is a circular mortise, with no degenerate sides.
        assert!(
            pocket(
                &stock,
                &JointSettings {
                    pocket_width: settings.pocket_thickness,
                    ..settings
                },
                0,
                0.
            )
            .is_ok()
        );
    }

    #[test]
    fn invalid_dimensions_and_overcrowded_pockets_are_rejected_atomically() {
        let (source, members) = super::super::tests::pair();
        for kind in KINDS {
            let base = JointSettings {
                kind,
                ..Default::default()
            };
            for settings in [
                JointSettings {
                    pocket_thickness: 0.04,
                    ..base.clone()
                },
                JointSettings {
                    pocket_depth: 1.,
                    ..base.clone()
                },
                JointSettings {
                    pocket_width: f64::NAN,
                    ..base.clone()
                },
                JointSettings {
                    pocket_depth: 0.,
                    ..base.clone()
                },
                JointSettings {
                    pocket_count: 0,
                    ..base.clone()
                },
                JointSettings {
                    pocket_count: 17,
                    ..base.clone()
                },
            ] {
                let mut model = source.clone();
                assert!(
                    model.test_cut_joint(members, &settings).is_err(),
                    "{settings:?}"
                );
                assert_eq!(model, source);
            }
            if matches!(kind, JointKind::Biscuit | JointKind::Domino) {
                let mut model = source.clone();
                assert!(
                    model
                        .test_cut_joint(
                            members,
                            &JointSettings {
                                pocket_count: 4,
                                ..base
                            }
                        )
                        .is_err()
                );
                assert_eq!(model, source);
            }
        }
    }

    #[test]
    fn settings_edits_invalidate_cached_pockets_and_repeated_evaluation_reuses_them() {
        for kind in KINDS {
            let (mut source, members) = super::super::tests::pair();
            let settings = JointSettings {
                kind,
                ..Default::default()
            };
            let id = source
                .add_connection(Connection {
                    members,
                    entry_face: None,
                    operation: ConnectionOperation::Joinery(settings.clone()),
                })
                .unwrap();
            let mut cache = EvaluationCache::default();
            assert!(cache.evaluate(&source).errors.is_empty());
            assert!(cache.evaluate(&source).errors.is_empty());
            assert_eq!(cache.evaluations, 1);
            for edited in [
                JointSettings {
                    pocket_width: 0.035,
                    ..settings.clone()
                },
                JointSettings {
                    pocket_thickness: 0.006,
                    ..settings.clone()
                },
                JointSettings {
                    pocket_depth: 0.012,
                    ..settings.clone()
                },
                JointSettings {
                    pocket_count: 2,
                    ..settings.clone()
                },
            ] {
                source.connections.get_mut(&id).unwrap().operation =
                    ConnectionOperation::Joinery(edited);
                let before = cache.evaluations;
                let cached = cache.evaluate(&source);
                let fresh = source.evaluate_joinery();
                assert!(cached.errors.is_empty(), "{kind:?}: {:?}", cached.errors);
                assert_eq!(cached.errors, fresh.errors);
                assert_eq!(cache.evaluations, before + 1);
                for member in members {
                    assert!(
                        (cached.model.solid_volume(member).unwrap()
                            - fresh.model.solid_volume(member).unwrap())
                        .abs()
                            < 1e-10
                    );
                }
            }
        }
    }
}
