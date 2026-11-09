//! Reuse render geometry and a spatial hierarchy for pointer queries. No query
//! reconstructs face planes or searches every group's membership for each edge.
use crate::renderer::{EntityCache, FaceCache};
use ketchup_core::{Camera, DVec2, DVec3, Model, Ray, selection};
#[path = "snapping_index.rs"]
mod snapping_index;
#[cfg(any(test, target_arch = "wasm32"))]
#[path = "box_selection_regression.rs"]
mod box_selection_regression;

#[derive(Clone, Copy, PartialEq)]
struct Bounds {
    lo: DVec3,
    hi: DVec3,
}

fn deleted_keys<'a, T: PartialEq + 'a>(
    before: impl IntoIterator<Item = (u64, &'a T)>,
    after: impl IntoIterator<Item = (u64, &'a T)>,
) -> Option<std::collections::HashSet<u64>> {
    let mut next = after.into_iter().peekable();
    let mut removed = std::collections::HashSet::new();
    for (id, value) in before {
        match next.peek() {
            Some((key, other)) if *key == id => {
                if value != *other {
                    return None;
                }
                next.next();
            }
            Some((key, _)) if *key < id => return None,
            _ => {
                removed.insert(id);
            }
        }
    }
    next.next().is_none().then_some(removed)
}
impl Bounds {
    fn entry(self, ray: Ray, limit: f64) -> Option<f64> {
        let mut enter = 0.0_f64;
        let mut exit = limit;
        for axis in 0..3 {
            let origin = ray.origin[axis];
            let direction = ray.direction[axis];
            if direction.abs() < 1e-15 {
                if origin < self.lo[axis] - 1e-6 || origin > self.hi[axis] + 1e-6 {
                    return None;
                }
            } else {
                let a = (self.lo[axis] - 1e-6 - origin) / direction;
                let b = (self.hi[axis] + 1e-6 - origin) / direction;
                enter = enter.max(a.min(b));
                exit = exit.min(a.max(b));
                if enter > exit {
                    return None;
                }
            }
        }
        Some(enter)
    }
}
struct Item {
    id: u64,
    bounds: Bounds,
}
struct Node {
    bounds: Bounds,
    start: usize,
    end: usize,
    children: Option<(usize, usize)>,
}
#[derive(Default)]
pub struct PickingCache {
    revision: Option<u64>,
    display: ketchup_core::DisplaySettings,
    items: Vec<Item>,
    nodes: Vec<Node>,
    source_bounds: Vec<(u64, Bounds)>,
    extra: Vec<Item>,
    removed: std::collections::HashSet<u64>,
    #[cfg(any(test, target_arch = "wasm32"))]
    hierarchy_builds: usize,
    edges: EdgePickingCache,
    snapping: snapping_index::Index,
}
pub struct SnapQuery<'a> {
    picking: &'a PickingCache,
    faces: &'a FaceCache,
    entities: &'a EntityCache,
    excluded: Option<&'a std::collections::BTreeSet<u64>>,
}
impl<'a> SnapQuery<'a> {
    pub fn excluding(mut self, excluded: Option<&'a std::collections::BTreeSet<u64>>) -> Self {
        self.excluded = excluded;
        self
    }
}
impl ketchup_core::inference::FaceQuery for SnapQuery<'_> {
    fn contains_target(
        &self,
        model: &Model,
        camera: &Camera,
        size: DVec2,
        point: DVec3,
        kind: ketchup_core::Inference,
    ) -> Option<bool> {
        use ketchup_core::inference::{Inference, SnapTargets};
        let cursor = camera.view(size).project(point)?;
        let geometry = self.nearby_geometry(
            camera,
            size,
            cursor,
            SnapTargets {
                edges: kind == Inference::OnEdge,
                faces: kind == Inference::OnFace,
                face_centers: false,
            },
        )?;
        match kind {
            Inference::OnFace => Some(
                geometry
                    .faces
                    .into_iter()
                    .any(|id| model.face_contains(&model.faces[&id], point, true)),
            ),
            Inference::OnEdge => Some(geometry.edges.into_iter().any(|id| {
                let (a, b) = model.points(&model.edges[&id]);
                let t = ((point - a).dot(b - a) / (b - a).length_squared()).clamp(0., 1.);
                (a + (b - a) * t).distance(point) < 1e-7
            })),
            _ => None,
        }
    }
    fn nearby_geometry(
        &self,
        camera: &Camera,
        size: DVec2,
        cursor: DVec2,
        targets: ketchup_core::inference::SnapTargets,
    ) -> Option<ketchup_core::inference::SnapGeometry> {
        if self.excluded.is_some()
            || !self
                .picking
                .snapping
                .matches(self.faces, self.entities, camera, size)
        {
            return None;
        }
        Some(
            self.picking
                .snapping
                .nearby(cursor, targets, &self.picking.edges, self.entities),
        )
    }
    fn alignment_references(
        &self,
        camera: &Camera,
        size: DVec2,
        cursor: DVec2,
    ) -> Option<[Vec<DVec3>; 3]> {
        if self.excluded.is_some()
            || !self
                .picking
                .snapping
                .matches(self.faces, self.entities, camera, size)
        {
            return None;
        }
        let view = camera.view(size);
        self.picking.snapping.alignment(cursor, |point| {
            let Some(pixel) = view.project(point) else {
                return false;
            };
            let ray = view.ray(pixel);
            self.nearest(ray, true)
                .is_none_or(|(_, t, _)| t + 1e-5 >= point.distance(ray.origin))
        })
    }
    fn nearest(&self, ray: Ray, scoped: bool) -> Option<(u64, f64, DVec3)> {
        self.picking.nearest_matching(
            self.faces,
            self.entities,
            ray,
            false,
            scoped,
            &mut 0,
            |id| self.excluded.is_none_or(|ids| !ids.contains(&id)),
        )
    }

    fn is_hidden(&self, _model: &Model, id: u64) -> bool {
        self.entities.get(id).hidden || self.excluded.is_some_and(|ids| ids.contains(&id))
    }
}
impl PickingCache {
    #[cfg(any(test, target_arch = "wasm32"))]
    pub(crate) fn validate_box_selection() -> Result<usize, String> {
        box_selection_regression::validate()
    }
    #[cfg(any(test, target_arch = "wasm32"))]
    pub(crate) fn benchmark_builds(&self) -> [usize; 3] {
        [
            self.hierarchy_builds,
            self.edges.geometry_builds,
            self.edges.projection_builds,
        ]
    }
    pub(crate) fn reference_edges(
        &self,
        faces: &FaceCache,
        entities: &EntityCache,
        camera: &Camera,
        size: DVec2,
        point: DVec3,
    ) -> Option<Vec<u64>> {
        if !self.snapping.matches(faces, entities, camera, size) {
            return None;
        }
        let pixel = camera.view(size).project(point)?;
        Some(
            self.snapping
                .nearby(
                    pixel,
                    ketchup_core::inference::SnapTargets {
                        edges: true,
                        faces: false,
                        face_centers: false,
                    },
                    &self.edges,
                    entities,
                )
                .edges,
        )
    }
    pub(crate) fn prepare_snapping(
        &mut self,
        model: &Model,
        faces: &FaceCache,
        entities: &EntityCache,
        camera: &Camera,
        size: DVec2,
    ) {
        self.prepare_view(model, faces, camera, size);
        self.snapping.prepare(model, faces, entities, camera, size);
    }
    pub(crate) fn prepare_view(
        &mut self,
        model: &Model,
        faces: &FaceCache,
        camera: &Camera,
        size: DVec2,
    ) {
        self.update(faces);
        self.edges
            .update_for_revision(model, camera, size, faces.source_revision());
    }

    /// Attachment eligibility needs neither barycentric weights nor another
    /// triangulation. Query the existing edge and face indices at the snap.
    pub fn contains_geometry_point(&mut self, model: &Model, faces: &FaceCache,
        entities: &EntityCache, camera: &Camera, size: DVec2, point: DVec3) -> bool
    {
        let epsilon = ketchup_core::document::EPS;
        self.edges.update_for_revision(model, camera, size,faces.source_revision());
        if let Some(pixel) = camera.view(size).project(point) {
            for (id, _) in self.edges.candidates(pixel, 0.01, entities) {
                let (a, b) = model.points(&model.edges[&id]);
                let delta = b - a;
                let t = ((point - a).dot(delta) / delta.length_squared()).clamp(0., 1.);
                if (point - (a + delta * t)).length() < epsilon { return true; }
            }
        }
        if self.nodes.is_empty() { return false; }
        let contains = |bounds: Bounds| (point.cmpge(bounds.lo - DVec3::splat(epsilon))
            & point.cmple(bounds.hi + DVec3::splat(epsilon))).all();
        let mut pending = [0; usize::BITS as usize + 1];
        let mut len = 1;
        while len != 0 {
            len -= 1;
            let node = &self.nodes[pending[len]];
            if !contains(node.bounds) { continue; }
            if let Some((a, b)) = node.children {
                pending[len..len + 2].copy_from_slice(&[a, b]); len += 2;
            } else {
                for item in &self.items[node.start..node.end] {
                    if self.removed.contains(&item.id) {
                        continue;
                    }
                    let state = entities.get(item.id);
                    if !state.hidden && (model.edit_context.is_none() || state.context.is_some())
                        && contains(item.bounds) && faces.get(item.id).contains_point(point)
                    { return true; }
                }
            }
        }
        for item in &self.extra {
            let state = entities.get(item.id);
            if !state.hidden
                && (model.edit_context.is_none() || state.context.is_some())
                && contains(item.bounds)
                && faces.get(item.id).contains_point(point)
            {
                return true;
            }
        }
        false
    }

    pub fn set_display(&mut self, display: &ketchup_core::DisplaySettings) {
        self.display = display.clone();
    }
    pub fn snap_query<'a>(
        &'a self,
        faces: &'a FaceCache,
        entities: &'a EntityCache,
    ) -> SnapQuery<'a> {
        SnapQuery {
            picking: self,
            faces,
            entities,
            excluded: None,
        }
    }
    pub fn update(&mut self, faces: &FaceCache) {
        if self.revision == Some(faces.revision()) {
            return;
        }
        self.revision = Some(faces.revision());
        let bounds: Vec<_> = faces
            .iter()
            .filter(|(_, face)| !face.triangles.is_empty())
            .map(|(id, face)| {
                (
                    id,
                    Bounds {
                        lo: face.bounds.0,
                        hi: face.bounds.1,
                    },
                )
            })
            .collect();
        let dirty = crate::renderer::changed_ids(
            self.source_bounds.iter().map(|(id, bounds)| (*id, bounds)),
            bounds.iter().map(|(id, bounds)| (*id, bounds)),
        );
        if !bounds.is_empty() && !self.source_bounds.is_empty() && dirty.len() <= 512 {
            self.extra = bounds
                .iter()
                .filter(|(id, _)| dirty.contains(id))
                .map(|&(id, bounds)| Item { id, bounds })
                .collect();
            self.removed = dirty.into_iter().collect();
            return;
        }
        self.extra.clear();
        self.source_bounds = bounds;
        self.removed.clear();
        self.items = self
            .source_bounds
            .iter()
            .map(|&(id, bounds)| Item { id, bounds })
            .collect();
        self.nodes.clear();
        if !self.items.is_empty() {
            self.build(0, self.items.len());
        }
        #[cfg(any(test, target_arch = "wasm32"))]
        {
            self.hierarchy_builds += 1;
        }
    }
    fn build(&mut self, start: usize, end: usize) -> usize {
        let bounds = self.items[start..end].iter().fold(
            Bounds {
                lo: DVec3::splat(f64::INFINITY),
                hi: DVec3::splat(f64::NEG_INFINITY),
            },
            |bounds, item| Bounds {
                lo: bounds.lo.min(item.bounds.lo),
                hi: bounds.hi.max(item.bounds.hi),
            },
        );
        let index = self.nodes.len();
        self.nodes.push(Node {
            bounds,
            start,
            end,
            children: None,
        });
        if end - start > 8 {
            let extent = bounds.hi - bounds.lo;
            let axis = if extent.x >= extent.y && extent.x >= extent.z {
                0
            } else if extent.y >= extent.z {
                1
            } else {
                2
            };
            let middle = start + (end - start) / 2;
            self.items[start..end].select_nth_unstable_by(middle - start, |a, b| {
                (a.bounds.lo[axis] + a.bounds.hi[axis])
                    .total_cmp(&(b.bounds.lo[axis] + b.bounds.hi[axis]))
            });
            let left = self.build(start, middle);
            let right = self.build(middle, end);
            self.nodes[index].children = Some((left, right));
        }
        index
    }
    pub fn nearest(
        &self,
        faces: &FaceCache,
        entities: &EntityCache,
        ray: Ray,
        editable: bool,
    ) -> Option<(u64, f64, DVec3)> {
        self.nearest_filtered(faces, entities, ray, editable, &mut 0)
    }
    /// Navigation can place a camera on geometry outside the current group's
    /// editing context, without changing or closing that context.
    pub fn nearest_unscoped(&self, faces: &FaceCache, entities: &EntityCache, ray: Ray) -> Option<(u64, f64, DVec3)> {
        self.nearest_in_scope(faces, entities, ray, false, false, &mut 0)
    }
    /// Test a world marker against cached surfaces in the active editing level.
    pub fn visible(
        &self,
        faces: &FaceCache,
        entities: &EntityCache,
        camera: &Camera,
        size: DVec2,
        point: DVec3,
    ) -> bool {
        let view = camera.view(size);
        let Some(pixel) = view.project(point) else {
            return false;
        };
        let ray = view.ray(pixel);
        self.nearest(faces, entities, ray, false)
            .is_none_or(|(_, t, _)| t + 1e-5 >= point.distance(ray.origin))
    }
    fn nearest_filtered(
        &self,
        faces: &FaceCache,
        entities: &EntityCache,
        ray: Ray,
        editable: bool,
        visited: &mut usize,
    ) -> Option<(u64, f64, DVec3)> {
        self.nearest_in_scope(faces, entities, ray, editable, true, visited)
    }
    fn nearest_in_scope(
        &self,
        faces: &FaceCache,
        entities: &EntityCache,
        ray: Ray,
        editable: bool,
        scoped: bool,
        visited: &mut usize,
    ) -> Option<(u64, f64, DVec3)> {
        self.nearest_matching(faces, entities, ray, editable, scoped, visited, |_| true)
    }

    /// World markers remain visible over their owners, but not through other objects.
    pub fn marker_visible(
        &self,
        faces: &FaceCache,
        entities: &EntityCache,
        camera: &Camera,
        size: DVec2,
        point: DVec3,
        owners: &std::collections::BTreeSet<u64>,
    ) -> bool {
        let view = camera.view(size);
        let Some(pixel) = view.project(point) else { return false; };
        if !self.display.section_contains(point) { return false; }
        let ray = view.ray(pixel);
        self.nearest_matching(faces, entities, ray, false, false, &mut 0,
            |id| !owners.contains(&id))
            .is_none_or(|(_, t, _)| t + 1e-5 >= point.distance(ray.origin))
    }

    fn nearest_matching(
        &self,
        faces: &FaceCache,
        entities: &EntityCache,
        ray: Ray,
        editable: bool,
        scoped: bool,
        visited: &mut usize,
        include: impl Fn(u64) -> bool,
    ) -> Option<(u64, f64, DVec3)> {
        if self.nodes.is_empty() {
            return None;
        }
        let mut best: Option<(u64, f64, DVec3)> = None;
        // The hierarchy halves each range, so its traversal needs at most one
        // pending sibling per bit of usize. Keep this hot occlusion query off
        // the allocator (it also runs for every edge candidate and snap marker).
        let mut pending = [0; usize::BITS as usize + 1];
        let mut pending_len = 1;
        while pending_len != 0 {
            pending_len -= 1;
            let index = pending[pending_len];
            let node = &self.nodes[index];
            let limit = best.map_or(f64::INFINITY, |(_, t, _)| t);
            if node.bounds.entry(ray, limit).is_none() {
                continue;
            }
            if let Some((left, right)) = node.children {
                let a = self.nodes[left].bounds.entry(ray, limit);
                let b = self.nodes[right].bounds.entry(ray, limit);
                match (a, b) {
                    (Some(a), Some(b)) => {
                        if a < b {
                            pending[pending_len..pending_len + 2].copy_from_slice(&[right, left]);
                            pending_len += 2;
                        } else {
                            pending[pending_len..pending_len + 2].copy_from_slice(&[left, right]);
                            pending_len += 2;
                        }
                    }
                    (Some(_), None) => {
                        pending[pending_len] = left;
                        pending_len += 1;
                    }
                    (None, Some(_)) => {
                        pending[pending_len] = right;
                        pending_len += 1;
                    }
                    _ => {}
                }
                continue;
            }
            for item in &self.items[node.start..node.end] {
                if self.removed.contains(&item.id) {
                    continue;
                }
                let state = entities.get(item.id);
                if !include(item.id)
                    || state.hidden
                    || scoped && state.context.is_none()
                    || editable && state.context != Some(item.id)
                {
                    continue;
                }
                if item.bounds.entry(ray, limit).is_none() {
                    continue;
                }
                *visited += 1;
                if let Some((t, p)) = faces.get(item.id).hit(ray)
                    && self.display.section_contains(p)
                {
                    if best.is_none_or(|(id, best, _)| t < best || t == best && item.id > id) {
                        best = Some((item.id, t, p));
                    }
                }
            }
        }
        for item in &self.extra {
            let state = entities.get(item.id);
            if !include(item.id)
                || state.hidden
                || scoped && state.context.is_none()
                || editable && state.context != Some(item.id)
            {
                continue;
            }
            if item
                .bounds
                .entry(ray, best.map_or(f64::INFINITY, |(_, t, _)| t))
                .is_none()
            {
                continue;
            }
            *visited += 1;
            if let Some((t, p)) = faces.get(item.id).hit(ray)
                && self.display.section_contains(p)
            {
                if best.is_none_or(|(id, old, _)| t < old || t == old && item.id > id) {
                    best = Some((item.id, t, p));
                }
            }
        }
        best
    }
    pub fn pick(
        &mut self,
        model: &Model,
        faces: &FaceCache,
        entities: &EntityCache,
        camera: &Camera,
        size: DVec2,
        pixel: DVec2,
        radius: f64,
        include_faces: bool,
    ) -> Option<u64> {
        if include_faces {
            if let Some(id) = selection::pick_text(model, camera, size, pixel, radius) {
                return Some(id);
            }
            if let Some(id) = selection::pick_dimension(model, camera, size, pixel, radius) {
                return Some(id);
            }
        }
        if let Some((edge, _)) = self.pick_edge(model, faces, entities, camera, size, pixel, radius) {
            return entities.get(edge).context;
        }
        if include_faces {
            self.nearest(faces, entities, camera.ray(pixel, size), false)
                .and_then(|(id, _, _)| entities.get(id).context)
        } else {
            None
        }
    }

    pub(crate) fn pick_edge(
        &mut self,
        model: &Model,
        faces: &FaceCache,
        entities: &EntityCache,
        camera: &Camera,
        size: DVec2,
        pixel: DVec2,
        radius: f64,
    ) -> Option<(u64, DVec3)> {
        let view = camera.view(size);
        let ray = view.ray(pixel);
        self.edges.update_for_revision(model, camera, size,faces.source_revision());
        let candidates = self.edges.candidates(pixel, radius, entities);
        for (id, _) in candidates {
            let edge = &model.edges[&id];
            let (a, b) = model.points(edge);
            let Some((a, b)) = self.display.section_segment(a, b) else {
                continue;
            };
            let point = ray.segment_point(a, b);
            let Some(projected) = view.project(point) else {
                continue;
            };
            if self.display.section_cuts && projected.distance(pixel) > radius {
                continue;
            }
            let point_ray = view.ray(projected);
            let obstruction = self.nearest(faces, entities, point_ray, false);
            if obstruction.is_none_or(|(_, t, _)| t + 1e-5 >= point.distance(point_ray.origin)) {
                return Some((id, point));
            }
        }
        None
    }

    /// Reuse the projected-edge hierarchy for the whole eraser stroke, then
    /// test only nearby edges against the cached surfaces in the editing scope.
    pub(crate) fn sweep_edges(
        &mut self,
        model: &Model,
        faces: &FaceCache,
        entities: &EntityCache,
        camera: &Camera,
        size: DVec2,
        from: DVec2,
        to: DVec2,
        radius: f64,
    ) -> std::collections::BTreeSet<u64> {
        self.prepare_view(model, faces, camera, size);
        let view = camera.view(size);
        self.edges
            .sweep_candidates(from, to, radius, entities)
            .into_iter()
            .filter_map(|(id, _)| {
                let state = entities.get(id);
                if state.locked {
                    return None;
                }
                let (a, b) = model.points(&model.edges[&id]);
                let (a, b) = self.display.section_segment(a, b)?;
                let (c, d) = view.segment(a, b)?;
                if selection::segment_distance(from, to, c, d) >= radius {
                    return None;
                }
                let pixel = selection::closest_on_second_segment(from, to, c, d);
                let point = view.ray(pixel).segment_point(a, b);
                self.visible(faces, entities, camera, size, point)
                    .then_some(state.context)
                    .flatten()
            })
            .collect()
    }
}

#[derive(Default)]
struct EdgePickingCache {
    view: Option<(Camera, DVec2)>,
    known_revision:Option<u64>,
    source_edges: Vec<ketchup_core::Edge>,
    source_vertices: Vec<(u64, DVec3)>,
    extra: Vec<ScreenEdge>,
    removed: std::collections::HashSet<u64>,
    world_edges: Vec<(u64, DVec3, DVec3)>,
    items: Vec<ScreenEdge>,
    nodes: Vec<ScreenEdgeNode>,
    pending: Vec<usize>,
    #[cfg(any(test, target_arch = "wasm32"))]
    projection_builds: usize,
    #[cfg(any(test, target_arch = "wasm32"))]
    geometry_builds: usize,
    #[cfg(test)]
    tested_edges: usize,
}

struct ScreenEdge {
    id: u64,
    a: DVec2,
    b: DVec2,
}

struct ScreenEdgeNode {
    lo: DVec2,
    hi: DVec2,
    start: usize,
    end: usize,
    children: Option<(usize, usize)>,
}

impl EdgePickingCache {
    fn update_for_revision(
        &mut self,
        model: &Model,
        camera: &Camera,
        size: DVec2,
        revision: Option<u64>,
    ) {
        if revision.is_some()
            && self.known_revision == revision
            && self
                .view
                .as_ref()
                .is_some_and(|(c, s)| c == camera && *s == size)
        {
            return;
        }
        self.update(model, camera, size);
        self.known_revision = revision;
    }
    fn update(&mut self, model: &Model, camera: &Camera, size: DVec2) {
        self.known_revision=None;
        // Public model collections permit direct edits, so validate the source
        // with linear iterators. An unchanged hover does no edge projection,
        // map lookup per endpoint, or spatial-index construction.
        let moved = crate::renderer::changed_ids(
            self.source_vertices.iter().map(|(id, p)| (*id, p)),
            model.vertices.iter().map(|(&id, v)| (id, &v.position)),
        );
        let mut dirty = crate::renderer::changed_ids(
            self.source_edges.iter().map(|edge| (edge.id, edge)),
            model.edges.iter().map(|(&id, edge)| (id, edge)),
        );
        if !moved.is_empty() {
            dirty.extend(
                model
                    .edges
                    .values()
                    .filter(|edge| moved.contains(&edge.a) || moved.contains(&edge.b))
                    .map(|edge| edge.id),
            );
        }
        let same_geometry = dirty.is_empty() && moved.is_empty();
        if !self.source_edges.is_empty()
            && model.edges.len() >= 1024
            && dirty.len() <= 512
            && self
                .view
                .as_ref()
                .is_some_and(|(saved, dimensions)| saved == camera && *dimensions == size)
        {
            self.removed = dirty.into_iter().collect();
            let view = camera.view(size);
            self.extra = self
                .removed
                .iter()
                .filter_map(|id| {
                    let edge = model.edges.get(id)?;
                    let (a, b) = model.points(edge);
                    let (a, b) = view.segment(a, b)?;
                    Some(ScreenEdge { id: *id, a, b })
                })
                .collect();
            return;
        }
        self.extra.clear();
        if same_geometry
            && self
                .view
                .as_ref()
                .is_some_and(|(saved, dimensions)| saved == camera && *dimensions == size)
        {
            return;
        }
        if !same_geometry {
            self.removed.clear();
            // Camera changes only reproject these endpoints; resolve the model's
            // vertex maps once per geometry change, including direct public edits.
            self.world_edges.clear();
            self.world_edges.extend(model.edges.values().map(|edge| {
                let (a, b) = model.points(edge);
                (edge.id, a, b)
            }));
            #[cfg(any(test, target_arch = "wasm32"))]
            {
                self.geometry_builds += 1;
            }
            self.source_edges.clear();
            self.source_edges.extend(model.edges.values().cloned());
            self.source_vertices.clear();
            self.source_vertices
                .extend(model.vertices.iter().map(|(&id, v)| (id, v.position)));
        }
        self.removed.clear();
        self.view = Some((camera.clone(), size));
        self.items.clear();
        let view = camera.view(size);
        self.items
            .extend(self.world_edges.iter().filter_map(|&(id, a, b)| {
                let (a, b) = view.segment(a, b)?;
                Some(ScreenEdge { id, a, b })
            }));
        self.nodes.clear();
        if !self.items.is_empty() {
            self.build(0, self.items.len());
        }
        #[cfg(any(test, target_arch = "wasm32"))]
        {
            self.projection_builds += 1;
        }
    }

    fn build(&mut self, start: usize, end: usize) -> usize {
        let (lo, hi) = self.items[start..end].iter().fold(
            (DVec2::splat(f64::INFINITY), DVec2::splat(f64::NEG_INFINITY)),
            |(lo, hi), edge| (lo.min(edge.a).min(edge.b), hi.max(edge.a).max(edge.b)),
        );
        let index = self.nodes.len();
        self.nodes.push(ScreenEdgeNode {
            lo,
            hi,
            start,
            end,
            children: None,
        });
        if end - start > 8 {
            let extent = hi - lo;
            let axis = usize::from(extent.y > extent.x);
            let middle = start + (end - start) / 2;
            self.items[start..end].select_nth_unstable_by(middle - start, |a, b| {
                (a.a[axis] + a.b[axis]).total_cmp(&(b.a[axis] + b.b[axis]))
            });
            let left = self.build(start, middle);
            let right = self.build(middle, end);
            self.nodes[index].children = Some((left, right));
        }
        index
    }

    fn candidates(&mut self, pixel: DVec2, radius: f64, entities: &EntityCache) -> Vec<(u64, f64)> {
        self.candidates_in_bounds(
            pixel - DVec2::splat(radius),
            pixel + DVec2::splat(radius),
            entities,
            |edge| {
                let distance = selection::distance_to_segment(pixel, edge.a, edge.b);
                (distance <= radius).then_some(distance)
            },
        )
    }

    fn sweep_candidates(
        &mut self,
        from: DVec2,
        to: DVec2,
        radius: f64,
        entities: &EntityCache,
    ) -> Vec<(u64, f64)> {
        self.candidates_in_bounds(
            from.min(to) - DVec2::splat(radius),
            from.max(to) + DVec2::splat(radius),
            entities,
            |edge| {
                let distance = selection::segment_distance(from, to, edge.a, edge.b);
                (distance < radius).then_some(distance)
            },
        )
    }

    fn candidates_in_bounds(
        &mut self,
        lo: DVec2,
        hi: DVec2,
        entities: &EntityCache,
        distance: impl Fn(&ScreenEdge) -> Option<f64>,
    ) -> Vec<(u64, f64)> {
        #[cfg(test)]
        {
            self.tested_edges = 0;
        }
        let mut candidates = Vec::new();
        self.pending.clear();
        if !self.nodes.is_empty() {
            self.pending.push(0);
        }
        while let Some(index) = self.pending.pop() {
            let node = &self.nodes[index];
            if node.hi.x < lo.x || node.lo.x > hi.x || node.hi.y < lo.y || node.lo.y > hi.y {
                continue;
            }
            if let Some((left, right)) = node.children {
                self.pending.extend([left, right]);
                continue;
            }
            for edge in &self.items[node.start..node.end] {
                if self.removed.contains(&edge.id) {
                    continue;
                }
                let state = entities.get(edge.id);
                if state.hidden || state.context.is_none() {
                    continue;
                }
                #[cfg(test)]
                {
                    self.tested_edges += 1;
                }
                if let Some(distance) = distance(edge) {
                    candidates.push((edge.id, distance));
                }
            }
        }
        for edge in &self.extra {
            let state = entities.get(edge.id);
            if !state.hidden && state.context.is_some() {
                if let Some(distance) = distance(edge) {
                    candidates.push((edge.id, distance));
                }
            }
        }
        candidates.sort_unstable_by(|(a, x), (b, y)| x.total_cmp(y).then_with(|| b.cmp(a)));
        candidates
    }
}

#[cfg(test)]
#[path = "picking_tests.rs"]
pub(crate) mod tests;

#[cfg(test)]
mod section_picking_tests {
    use super::*;

    #[test]
    fn cut_faces_are_skipped_on_each_axis() {
        for axis in 0..3 {
            let u = (axis + 1) % 3;
            let v = (axis + 2) % 3;
            let mut model = Model::default();
            for offset in [0., 2.] {
                let points: [DVec3; 4] = std::array::from_fn(|i| {
                    let mut point = DVec3::ZERO;
                    point[axis] = offset;
                    point[u] = if i == 1 || i == 2 { 2. } else { 0. };
                    point[v] = if i >= 2 { 2. } else { 0. };
                    point
                });
                for i in 0..4 {
                    model.add_edge(points[i], points[(i + 1) % 4]).unwrap();
                }
            }
            let mut faces = FaceCache::default();
            let mut entities = EntityCache::default();
            faces.update(&model);
            entities.update(&model);
            let mut picking = PickingCache::default();
            picking.update(&faces);
            let mut origin = DVec3::ONE;
            origin[axis] = 3.;
            let mut direction = DVec3::ZERO;
            direction[axis] = -1.;
            let ray = Ray { origin, direction };
            assert_eq!(
                picking.nearest(&faces, &entities, ray, false).unwrap().2[axis],
                2.
            );
            let mut display = ketchup_core::DisplaySettings {
                section_cuts: true,
                section_axes: [false; 3],
                ..Default::default()
            };
            display.section_axes[axis] = true;
            picking.set_display(&display);
            assert_eq!(
                picking.nearest(&faces, &entities, ray, false).unwrap().2[axis],
                0.
            );
        }
    }
}
