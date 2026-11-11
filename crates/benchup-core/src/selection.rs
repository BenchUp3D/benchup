use crate::{Camera, CameraView, Model};
use glam::DVec2;
use std::collections::BTreeSet;

pub fn visible(model: &Model, camera: &Camera, size: DVec2, point: crate::DVec3) -> bool {
    visible_in_view(model, &camera.view(size), point)
}

pub(crate) fn visible_in_view(model: &Model, view: &CameraView, point: crate::DVec3) -> bool {
    let Some(pixel) = view.project(point) else {
        return false;
    };
    let ray = view.ray(pixel);
    model
        .nearest_context_face(ray)
        .is_none_or(|(_, t, _)| t + 1e-5 >= point.distance(ray.origin))
}

pub fn pick_entity(
    model: &Model,
    camera: &Camera,
    size: DVec2,
    point: DVec2,
    radius: f64,
) -> Option<u64> {
    pick_text(model, camera, size, point, radius)
        .or_else(|| pick_dimension(model, camera, size, point, radius))
        .or_else(|| pick_guide(model, camera, size, point, radius))
        .or_else(|| pick(model, camera, size, point, radius))
        .or_else(|| {
            model
                .nearest_context_face(camera.ray(point, size))
                .and_then(|(id, _, _)| model.entity_in_context(id))
        })
}

/// Guides are drawn over geometry; hit the visible line, including its dash gaps.
pub fn pick_guide(
    model: &Model,
    camera: &Camera,
    size: DVec2,
    point: DVec2,
    radius: f64,
) -> Option<u64> {
    model
        .guides
        .iter()
        .filter(|(id, _)| !model.is_hidden(**id))
        .filter_map(|(id, guide)| {
            let context = model.entity_in_context(*id)?;
            let distance = if guide.direction.is_some() {
                let (a, b) = guide.screen_segment(camera, size)?;
                distance_to_segment(point, a, b)
            } else {
                camera.project(guide.origin, size)?.distance(point)
            };
            (distance <= radius).then_some((context, distance))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1).then_with(|| b.0.cmp(&a.0)))
        .map(|(id, _)| id)
}

/// Leader text is selectable both on its leader and over its readable label.
pub fn pick_text(
    model: &Model,
    camera: &Camera,
    size: DVec2,
    point: DVec2,
    radius: f64,
) -> Option<u64> {
    let view = camera.view(size);
    model
        .texts
        .values()
        .filter(|t| !model.is_hidden(t.id))
        .filter_map(|text| {
            let id = model.entity_in_context(text.id)?;
            let [a, b] = text.layout(model);
            let half = text.label_size() * 0.5;
            let label = view.project(b).is_some_and(|center| {
                (point - center).abs().cmple(half).all() && visible_in_view(model, &view, b)
            });
            let leader = view.segment(a, b).and_then(|(screen_a, screen_b)| {
                let distance = distance_to_segment(point, screen_a, screen_b);
                (distance <= radius
                    && visible_in_view(model, &view, view.ray(point).segment_point(a, b)))
                .then_some(distance)
            });
            if label {
                Some((id, 0.))
            } else {
                leader.map(|distance| (id, distance))
            }
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(id, _)| id)
}

pub fn pick_dimension(
    model: &Model,
    camera: &Camera,
    size: DVec2,
    point: DVec2,
    radius: f64,
) -> Option<u64> {
    pick_dimension_with_visibility(model, camera, size, point, radius,
        |position| visible(model, camera, size, position))
}

/// Allow viewport callers to reuse their retained face BVH for label/arc occlusion.
pub fn pick_dimension_with_visibility(
    model: &Model,
    camera: &Camera,
    size: DVec2,
    point: DVec2,
    radius: f64,
    mut visible: impl FnMut(crate::DVec3) -> bool,
) -> Option<u64> {
    model
        .dimensions
        .values()
        .filter(|d| !model.is_hidden(d.id))
        .filter_map(|d| {
            let id = model.entity_in_context(d.id)?;
            if d.vertex.is_some() {
                let center = d.label_position(model);
                let label = camera.project(center, size).is_some_and(|at| {
                    (point.x - at.x).abs() < 38. && (point.y - at.y).abs() < 11.
                        && visible(center)
                });
                if label { return Some((id, 0.)); }
                let distance = d.segments(model).into_iter().filter_map(|(a, b)| {
                    let (screen_a, screen_b) = camera.segment(a, b, size)?;
                    let distance = distance_to_segment(point, screen_a, screen_b);
                    (distance <= radius && visible(camera.ray(point, size).segment_point(a, b))).then_some(distance)
                }).min_by(f64::total_cmp);
                return distance.map(|distance| (id, distance));
            }
            let [_, _, a, b] = d.layout(model);
            let (a, b) = camera.segment(a, b, size)?;
            let center = (a + b) * 0.5;
            let distance = distance_to_segment(point, a, b);
            let label = (point.x - center.x).abs() < 38. && (point.y - center.y).abs() < 11.;
            (distance <= radius || label).then_some((id, if label { 0. } else { distance }))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(id, _)| id)
}

pub fn distance_to_segment(p: DVec2, a: DVec2, b: DVec2) -> f64 {
    p.distance(closest_point(p, a, b))
}
pub fn closest_point(p: DVec2, a: DVec2, b: DVec2) -> DVec2 {
    let d = b - a;
    let t = if d.length_squared() < 1e-12 {
        0.0
    } else {
        ((p - a).dot(d) / d.length_squared()).clamp(0.0, 1.0)
    };
    a + d * t
}
pub fn closest_on_second_segment(a: DVec2, b: DVec2, c: DVec2, d: DVec2) -> DVec2 {
    let ab = b - a;
    let cd = d - c;
    let denominator = ab.perp_dot(cd);
    if denominator.abs() > 1e-12 {
        let t = (c - a).perp_dot(cd) / denominator;
        let u = (c - a).perp_dot(ab) / denominator;
        if (0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u) {
            return c + cd * u;
        }
    }
    [c, d, closest_point(a, c, d), closest_point(b, c, d)]
        .into_iter()
        .min_by(|p, q| distance_to_segment(*p, a, b).total_cmp(&distance_to_segment(*q, a, b)))
        .unwrap()
}
pub fn segment_distance(a: DVec2, b: DVec2, c: DVec2, d: DVec2) -> f64 {
    let ab = b - a;
    let cd = d - c;
    let denominator = ab.perp_dot(cd);
    if denominator.abs() > 1e-12 {
        let t = (c - a).perp_dot(cd) / denominator;
        let u = (c - a).perp_dot(ab) / denominator;
        if (0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u) {
            return 0.0;
        }
    }
    distance_to_segment(a, c, d)
        .min(distance_to_segment(b, c, d))
        .min(distance_to_segment(c, a, b))
        .min(distance_to_segment(d, a, b))
}
pub fn pick(model: &Model, camera: &Camera, size: DVec2, point: DVec2, radius: f64) -> Option<u64> {
    let view = camera.view(size);
    let ray = view.ray(point);
    model
        .edges
        .values()
        .filter(|e| !model.is_hidden(e.id) && model.entity_in_context(e.id).is_some())
        .filter_map(|e| {
            let (a, b) = model.points(e);
            let (screen_a, screen_b) = view.segment(a, b)?;
            let d = distance_to_segment(point, screen_a, screen_b);
            // Reject distant edges before casting a ray against every face.
            if d > radius || !visible_in_view(model, &view, ray.segment_point(a, b)) {
                return None;
            }
            Some((e.id, d))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1).then_with(|| b.0.cmp(&a.0)))
        .and_then(|v| model.entity_in_context(v.0))
}

pub fn intersects_rect(a: DVec2, b: DVec2, lo: DVec2, hi: DVec2) -> bool {
    let delta = b - a;
    let mut enter: f64 = 0.0;
    let mut exit: f64 = 1.0;
    for i in 0..2 {
        if delta[i].abs() < 1e-12 {
            if a[i] < lo[i] || a[i] > hi[i] {
                return false;
            }
        } else {
            let t1 = (lo[i] - a[i]) / delta[i];
            let t2 = (hi[i] - a[i]) / delta[i];
            enter = enter.max(t1.min(t2));
            exit = exit.min(t1.max(t2));
            if enter > exit {
                return false;
            }
        }
    }
    true
}
/// Select entities fully enclosed by the rectangle, regardless of drag direction.
pub fn box_select(
    model: &Model,
    camera: &Camera,
    size: DVec2,
    start: DVec2,
    end: DVec2,
) -> BTreeSet<u64> {
    box_select_with_guides(model, camera, size, start, end, true)
}

pub fn box_select_with_guides(
    model: &Model,
    camera: &Camera,
    size: DVec2,
    start: DVec2,
    end: DVec2,
    include_guides: bool,
) -> BTreeSet<u64> {
    use std::collections::HashMap;
    // Reverse ownership once, instead of searching all assemblies for every
    // primitive, ancestor and descendant-completeness check.
    let parents: HashMap<_, _> = model
        .objects
        .values()
        .flat_map(|object| object.entities.iter().map(|id| (*id, object.id)))
        .collect();
    let own_hidden = |id: u64| {
        model.hidden.contains(&id)
            || model
                .entity_tags
                .get(&id)
                .and_then(|tag| model.tags.get(tag))
                .is_some_and(|tag| !tag.visible)
    };
    let objects: HashMap<_, _> = model
        .objects
        .keys()
        .map(|&id| {
            let mut current = id;
            let mut hidden = false;
            let mut context = None;
            loop {
                hidden |= own_hidden(current);
                let parent = parents.get(&current).copied();
                if Some(current) != model.edit_context && parent == model.edit_context {
                    context = Some(current);
                }
                let Some(parent) = parent else { break };
                current = parent;
            }
            (id, (hidden, context))
        })
        .collect();
    box_select_with_state(model, camera, size, start, end, include_guides, |id| {
        let parent = parents.get(&id).copied();
        let inherited = parent.and_then(|id| objects.get(&id)).copied();
        let hidden = own_hidden(id) || inherited.is_some_and(|state| state.0);
        let context = if parent == model.edit_context {
            Some(id)
        } else {
            inherited.and_then(|state| state.1)
        };
        (hidden, context)
    })
}

/// The application already caches inherited visibility and selectable owners.
/// Reuse those states without rebuilding the ownership index for each drag.
/// Each primitive contributes once to its owner's complete-enclosure test;
/// shared vertices are projected once using one camera view.
pub fn box_select_with_state(
    model: &Model,
    camera: &Camera,
    size: DVec2,
    start: DVec2,
    end: DVec2,
    include_guides: bool,
    state: impl Fn(u64) -> (bool, Option<u64>),
) -> BTreeSet<u64> {
    use std::collections::HashMap;
    fn record(
        id: u64,
        state: (bool, Option<u64>),
        groups: &mut HashMap<u64, bool>,
        selected: &mut BTreeSet<u64>,
        hit: impl FnOnce() -> bool,
    ) {
        let (hidden, Some(owner)) = state else { return };
        if hidden {
            return;
        }
        if owner == id {
            if hit() {
                selected.insert(id);
            }
        } else {
            // Once any visible descendant misses the box, that assembly cannot
            // be enclosed. Hidden descendants do not block its selection.
            let enclosed = groups.entry(owner).or_insert(true);
            if *enclosed {
                *enclosed = hit();
            }
        }
    }
    let lo = start.min(end);
    let hi = start.max(end);
    let inside = |p: DVec2| p.cmpge(lo).all() && p.cmple(hi).all();
    let view = camera.view(size);
    let mut vertices = HashMap::new();
    let mut vertex_inside = |id: u64| {
        *vertices.entry(id).or_insert_with(|| {
            view.project(model.vertices[&id].position)
                .is_some_and(inside)
        })
    };
    let mut selected = BTreeSet::new();
    let mut groups = HashMap::new();
    for edge in model.edges.values() {
        record(edge.id, state(edge.id), &mut groups, &mut selected, || {
            vertex_inside(edge.a) && vertex_inside(edge.b)
        });
    }
    for face in model.faces.values() {
        record(face.id, state(face.id), &mut groups, &mut selected, || {
            face.vertices.iter().all(|id| vertex_inside(*id))
        });
    }
    for dimension in model.dimensions.values() {
        record(
            dimension.id,
            state(dimension.id),
            &mut groups,
            &mut selected,
            || {
                dimension
                    .annotation_points(model)
                    .iter()
                    .all(|p| view.project(*p).is_some_and(inside))
            },
        );
    }
    for text in model.texts.values() {
        record(text.id, state(text.id), &mut groups, &mut selected, || {
            let [a, b] = text.layout(model);
            view.project(a).is_some_and(inside)
                && view.project(b).is_some_and(|center| {
                    let half = text.label_size() * 0.5;
                    inside(center - half) && inside(center + half)
                })
        });
    }
    for (&id, guide) in &model.guides {
        record(id, state(id), &mut groups, &mut selected, || {
            include_guides
                && if guide.direction.is_some() {
                    guide
                        .screen_segment(camera, size)
                        .is_some_and(|(a, b)| intersects_rect(a, b, lo, hi))
                } else {
                    view.project(guide.origin).is_some_and(inside)
                }
        });
    }
    selected.extend(
        groups
            .into_iter()
            .filter_map(|(id, enclosed)| enclosed.then_some(id)),
    );
    selected
}
