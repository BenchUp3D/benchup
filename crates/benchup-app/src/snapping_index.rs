//! Screen-space broad phase for drawing. Exact inference and visibility remain
//! shared with the core; immutable targets survive pointer motion.
use super::*;
use benchup_core::inference::{Axis, Inference, SnapGeometry, SnapTargets};
use std::{
    cell::OnceCell,
    collections::{BTreeSet, HashMap},
};
const TILE: f64 = 32.;
const ANGLES: usize = 1024;
#[derive(PartialEq)]
struct Key {
    source: Option<u64>,
    faces: u64,
    entities: u64,
    camera: Camera,
    size: DVec2,
}
struct Point {
    id: u64,
    world: DVec3,
    pixel: DVec2,
    kind: Inference,
    directions: [Option<DVec2>; 3],
    visible: OnceCell<bool>,
}
enum Lines {
    Radial {
        origin: DVec2,
        bins: Vec<Vec<usize>>,
    },
    Parallel {
        normal: DVec2,
        bins: HashMap<i64, Vec<usize>>,
    },
}
impl Lines {
    fn new(camera: &Camera, size: DVec2, axis: Axis) -> Self {
        let clip = camera.matrix(size) * axis.vector().extend(0.);
        if clip.w.abs() > 1e-9 {
            let ndc = clip.truncate().truncate() / clip.w;
            let origin = (ndc * DVec2::new(0.5, -0.5) + DVec2::splat(0.5)) * size;
            Self::Radial {
                origin,
                bins: vec![Vec::new(); ANGLES],
            }
        } else {
            let direction =
                (clip.truncate().truncate() * size * DVec2::new(1., -1.)).normalize_or_zero();
            Self::Parallel {
                normal: DVec2::new(-direction.y, direction.x),
                bins: HashMap::new(),
            }
        }
    }
    fn angle(delta: DVec2) -> usize {
        ((delta.y.atan2(delta.x).rem_euclid(std::f64::consts::PI) / std::f64::consts::PI
            * ANGLES as f64) as usize)
            .min(ANGLES - 1)
    }
    fn insert(&mut self, index: usize, pixel: DVec2) {
        match self {
            Self::Radial { origin, bins } => bins[Self::angle(pixel - *origin)].push(index),
            Self::Parallel { normal, bins } => bins
                .entry((normal.dot(pixel) / TILE).floor() as i64)
                .or_default()
                .push(index),
        }
    }
    fn nearby(&self, cursor: DVec2) -> Vec<usize> {
        let mut result = Vec::new();
        match self {
            Self::Radial { origin, bins } => {
                let delta = cursor - *origin;
                let radius = if delta.length() <= 10. {
                    ANGLES as i32 / 2
                } else {
                    ((10. / delta.length()).asin() / std::f64::consts::PI * ANGLES as f64).ceil()
                        as i32
                        + 1
                };
                if radius >= ANGLES as i32 / 2 {
                    result.extend(bins.iter().flatten().copied());
                } else {
                    let middle = Self::angle(delta) as i32;
                    for offset in -radius..=radius {
                        result.extend(&bins[(middle + offset).rem_euclid(ANGLES as i32) as usize]);
                    }
                }
            }
            Self::Parallel { normal, bins } => {
                let center = normal.dot(cursor);
                for tile in
                    ((center - 10.) / TILE).floor() as i64..=((center + 10.) / TILE).floor() as i64
                {
                    if let Some(ids) = bins.get(&tile) {
                        result.extend(ids);
                    }
                }
            }
        }
        result.sort_unstable();
        result.dedup();
        result
    }
}
#[derive(Default)]
pub(super) struct Index {
    key: Option<Key>,
    points: Vec<Point>,
    tiles: HashMap<(i32, i32), Vec<usize>>,
    lines: Option<[Lines; 3]>,
    faces: Vec<ScreenEdge>,
    face_nodes: Vec<ScreenEdgeNode>,
    source: Option<Box<Model>>,
    dirty: BTreeSet<u64>,
    extra: Option<Box<Index>>,
}
impl Index {
    fn key(faces: &FaceCache, entities: &EntityCache, camera: &Camera, size: DVec2) -> Key {
        Key {
            source: faces.source_revision(),
            faces: faces.revision(),
            entities: entities.revision(),
            camera: camera.clone(),
            size,
        }
    }
    pub fn matches(
        &self,
        faces: &FaceCache,
        entities: &EntityCache,
        camera: &Camera,
        size: DVec2,
    ) -> bool {
        self.key.as_ref() == Some(&Self::key(faces, entities, camera, size))
    }
    pub fn prepare(
        &mut self,
        model: &Model,
        faces: &FaceCache,
        entities: &EntityCache,
        camera: &Camera,
        size: DVec2,
    ) {
        let key = Self::key(faces, entities, camera, size);
        if key.source.is_some() && self.key.as_ref() == Some(&key) {
            return;
        }
        if self.key.as_ref().is_some_and(|old| {
            old.source == key.source
                && old.faces == key.faces
                && old.camera == key.camera
                && old.size == key.size
        }) {
            for point in &mut self.points {
                point.visible.take();
            }
            if let Some(extra) = &mut self.extra {
                for point in &mut extra.points {
                    point.visible.take();
                }
            }
            self.key = Some(key);
            return;
        }
        if self
            .key
            .as_ref()
            .is_some_and(|old| old.camera == key.camera && old.size == key.size)
        {
            if let Some(source) = self.source.as_ref() {
                if let Some(mut dirty) = crate::renderer::geometry_changes(source, model)
                    .filter(|dirty| dirty.len() <= 512)
                {
                    let vertices: std::collections::HashSet<_> = dirty
                        .iter()
                        .flat_map(|id| [source.edges.get(id), model.edges.get(id)])
                        .flatten()
                        .flat_map(|edge| [edge.a, edge.b])
                        .collect();
                    dirty.extend(
                        model
                            .edges
                            .values()
                            .filter(|edge| vertices.contains(&edge.a) || vertices.contains(&edge.b))
                            .map(|edge| edge.id),
                    );
                    let subset = model.copy_entities(&dirty);
                    dirty.extend(&vertices);
                    dirty.extend(subset.vertices.keys());
                    dirty.extend(subset.edges.keys());
                    dirty.extend(subset.faces.keys());
                    let mut extra = Box::new(Index::default());
                    extra.prepare(&subset, faces, entities, camera, size);
                    self.extra = Some(extra);
                    self.dirty = dirty;
                    for point in &mut self.points {
                        point.visible.take();
                    }
                    self.key = Some(key);
                    return;
                }
            }
        }
        self.source = Some(Box::new(model.clone()));
        self.dirty.clear();
        self.extra = None;
        self.key = Some(key);
        self.points.clear();
        self.tiles.clear();
        self.faces.clear();
        self.face_nodes.clear();
        let view = camera.view(size);
        let mut lines = [
            Lines::new(camera, size, Axis::X),
            Lines::new(camera, size, Axis::Y),
            Lines::new(camera, size, Axis::Z),
        ];
        let visible_edges: Vec<_> = model
            .edges
            .values()
            .filter(|edge| !entities.get(edge.id).hidden)
            .collect();
        let vertices: std::collections::HashSet<_> = visible_edges
            .iter()
            .flat_map(|edge| [edge.a, edge.b])
            .collect();
        let mut vertices: Vec<_> = vertices.into_iter().collect();
        vertices.sort_unstable();
        let mut positions: Vec<_> = vertices
            .into_iter()
            .map(|id| (id, model.vertices[&id].position, Inference::Endpoint))
            .collect();
        positions.extend(visible_edges.into_iter().map(|edge| {
            let (a, b) = model.points(edge);
            (edge.id, (a + b) * 0.5, Inference::Midpoint)
        }));
        positions.extend(
            model
                .faces
                .values()
                .filter(|face| !entities.get(face.id).hidden)
                .filter_map(|face| {
                    faces
                        .snap_center(model, face.id)
                        .map(|center| (face.id, center, Inference::FaceCenter))
                }),
        );
        for (id, world, kind) in positions {
            let Some(pixel) = view.project(world) else {
                continue;
            };
            let directions = [Axis::X, Axis::Y, Axis::Z].map(|axis| {
                let delta = view
                    .project(world + axis.vector() * camera.distance)
                    .or_else(|| view.project(world - axis.vector() * camera.distance))?
                    - pixel;
                (delta.length_squared() >= 1e-8).then_some(delta)
            });
            let index = self.points.len();
            self.tiles
                .entry((
                    (pixel.x / TILE).floor() as i32,
                    (pixel.y / TILE).floor() as i32,
                ))
                .or_default()
                .push(index);
            if kind != Inference::FaceCenter {
                for axis in 0..3 {
                    if directions[axis].is_some() {
                        lines[axis].insert(index, pixel);
                    }
                }
            }
            self.points.push(Point {
                id,
                world,
                pixel,
                kind,
                directions,
                visible: OnceCell::new(),
            });
        }
        self.lines = Some(lines);
        for id in model
            .faces
            .keys()
            .copied()
            .filter(|id| !entities.get(*id).hidden)
        {
            let (lo, hi) = faces.get(id).bounds;
            let corners = [
                DVec3::new(lo.x, lo.y, lo.z),
                DVec3::new(hi.x, lo.y, lo.z),
                DVec3::new(lo.x, hi.y, lo.z),
                DVec3::new(hi.x, hi.y, lo.z),
                DVec3::new(lo.x, lo.y, hi.z),
                DVec3::new(hi.x, lo.y, hi.z),
                DVec3::new(lo.x, hi.y, hi.z),
                DVec3::new(hi.x, hi.y, hi.z),
            ];
            let mut a = DVec2::splat(f64::INFINITY);
            let mut b = DVec2::splat(f64::NEG_INFINITY);
            for corner in corners {
                if let Some(pixel) = view.project(corner) {
                    a = a.min(pixel);
                    b = b.max(pixel);
                } else {
                    a = DVec2::splat(f64::NEG_INFINITY);
                    b = DVec2::splat(f64::INFINITY);
                    break;
                }
            }
            self.faces.push(ScreenEdge { id, a, b });
        }
        if !self.faces.is_empty() {
            self.build_faces(0, self.faces.len());
        }
    }
    fn build_faces(&mut self, start: usize, end: usize) -> usize {
        let (lo, hi) = self.faces[start..end].iter().fold(
            (DVec2::splat(f64::INFINITY), DVec2::splat(f64::NEG_INFINITY)),
            |(lo, hi), f| (lo.min(f.a), hi.max(f.b)),
        );
        let index = self.face_nodes.len();
        self.face_nodes.push(ScreenEdgeNode {
            lo,
            hi,
            start,
            end,
            children: None,
        });
        if end - start > 8 {
            let axis = usize::from((hi - lo).y > (hi - lo).x);
            let mid = (start + end) / 2;
            self.faces[start..end].select_nth_unstable_by(mid - start, |a, b| {
                (a.a[axis] + a.b[axis]).total_cmp(&(b.a[axis] + b.b[axis]))
            });
            let left = self.build_faces(start, mid);
            let right = self.build_faces(mid, end);
            self.face_nodes[index].children = Some((left, right));
        }
        index
    }
    pub fn nearby(
        &self,
        cursor: DVec2,
        targets: SnapTargets,
        edges: &EdgePickingCache,
        entities: &EntityCache,
    ) -> SnapGeometry {
        let mut indices: Vec<usize> = Vec::new();
        for x in ((cursor.x - 11.) / TILE).floor() as i32..=((cursor.x + 11.) / TILE).floor() as i32
        {
            for y in
                ((cursor.y - 11.) / TILE).floor() as i32..=((cursor.y + 11.) / TILE).floor() as i32
            {
                if let Some(ids) = self.tiles.get(&(x, y)) {
                    indices.extend(ids);
                }
            }
        }
        indices.sort_unstable();
        let mut points: Vec<_> = indices
            .into_iter()
            .map(|index| &self.points[index])
            .filter(|point| !self.dirty.contains(&point.id))
            .collect();
        if let Some(extra) = &self.extra {
            points.extend(&extra.points);
        }
        points.retain(|p| {
            p.pixel.distance(cursor) < 11.
                && if p.kind == Inference::FaceCenter {
                    targets.face_centers
                } else {
                    targets.edges
                }
        });
        points.sort_by_key(|p| {
            (
                if p.kind == Inference::Endpoint {
                    0
                } else if p.kind == Inference::Midpoint {
                    1
                } else {
                    2
                },
                p.id,
            )
        });
        let points = points.into_iter().map(|p| (p.world, p.kind)).collect();
        let mut result = SnapGeometry {
            points,
            edges: Vec::new(),
            faces: Vec::new(),
        };
        if targets.edges {
            for_rect(&edges.nodes, cursor, 11., |node| {
                for edge in &edges.items[node.start..node.end] {
                    if edges.removed.contains(&edge.id) {
                        continue;
                    }
                    if !entities.get(edge.id).hidden
                        && selection::distance_to_segment(cursor, edge.a, edge.b) <= 11.
                    {
                        result.edges.push(edge.id);
                    }
                }
            });
            for edge in &edges.extra {
                if !entities.get(edge.id).hidden
                    && selection::distance_to_segment(cursor, edge.a, edge.b) <= 11.
                {
                    result.edges.push(edge.id);
                }
            }
            result.edges.sort_unstable();
        }
        if targets.faces {
            for_rect(&self.face_nodes, cursor, 11., |node| {
                for face in &self.faces[node.start..node.end] {
                    if !self.dirty.contains(&face.id)
                        && face.a.x <= cursor.x + 11.
                        && face.b.x >= cursor.x - 11.
                        && face.a.y <= cursor.y + 11.
                        && face.b.y >= cursor.y - 11.
                    {
                        result.faces.push(face.id);
                    }
                }
            });
            if let Some(extra) = &self.extra {
                for face in &extra.faces {
                    if face.a.x <= cursor.x + 11.
                        && face.b.x >= cursor.x - 11.
                        && face.a.y <= cursor.y + 11.
                        && face.b.y >= cursor.y - 11.
                    {
                        result.faces.push(face.id);
                    }
                }
            }
            result.faces.sort_unstable();
        }
        result
    }
    pub fn alignment(
        &self,
        cursor: DVec2,
        mut visible: impl FnMut(DVec3) -> bool,
    ) -> Option<[Vec<DVec3>; 3]> {
        let lines = self.lines.as_ref()?;
        Some(std::array::from_fn(|axis| {
            let mut points: Vec<_> = lines[axis]
                .nearby(cursor)
                .into_iter()
                .map(|index| &self.points[index])
                .filter(|point| !self.dirty.contains(&point.id))
                .collect();
            if let Some(extra) = &self.extra {
                points.extend(&extra.points);
            }
            points.sort_by_key(|p| (if p.kind == Inference::Endpoint { 0 } else { 1 }, p.id));
            points
                .into_iter()
                .filter_map(|point| {
                    if point.kind == Inference::FaceCenter {
                        return None;
                    }
                    let direction = point.directions[axis]?;
                    if (cursor - point.pixel).perp_dot(direction).abs() / direction.length() > 10. {
                        return None;
                    }
                    if !*point.visible.get_or_init(|| visible(point.world)) {
                        return None;
                    }
                    Some(point.world)
                })
                .collect()
        }))
    }
}
fn for_rect(
    nodes: &[ScreenEdgeNode],
    pixel: DVec2,
    radius: f64,
    mut leaf: impl FnMut(&ScreenEdgeNode),
) {
    if nodes.is_empty() {
        return;
    }
    let mut pending = vec![0];
    while let Some(index) = pending.pop() {
        let node = &nodes[index];
        if node.hi.x < pixel.x - radius
            || node.lo.x > pixel.x + radius
            || node.hi.y < pixel.y - radius
            || node.lo.y > pixel.y + radius
        {
            continue;
        }
        if let Some((a, b)) = node.children {
            pending.extend([a, b]);
        } else {
            leaf(node);
        }
    }
}

#[cfg(target_arch = "wasm32")]
impl super::PickingCache {
    pub(crate) fn validate_snap_edits(mut model: Model, camera: &Camera) -> Result<usize, String> {
        model.edit_context = None;
        let point = camera.target + DVec3::Z * 0.123;
        let edge = model
            .add_edge(point, point + DVec3::X)
            .map_err(str::to_owned)?;
        let original = model.clone();
        let size = DVec2::new(1064., 748.);
        let mut retained = Self::default();
        let mut faces = FaceCache::default();
        let mut entities = EntityCache::default();
        let mut checks = 0;
        for step in 0..4 {
            match step {
                1 => model.erase(&BTreeSet::from([edge])),
                2 => {
                    model.edit_context = model
                        .root_entities()
                        .into_iter()
                        .find(|id| model.objects.contains_key(id));
                    model
                        .draw_polyline(
                            &[
                                point,
                                point + DVec3::X,
                                point + DVec3::X + DVec3::Y,
                                point + DVec3::Y,
                                point,
                            ],
                            true,
                        )
                        .map_err(str::to_owned)?;
                }
                3 => model = original.clone(),
                _ => {}
            }
            faces.update_for_revision(&model, step + 1);
            entities.update_for_revision(&model, step + 1);
            retained.prepare_snapping(&model, &faces, &entities, camera, size);
            let mut fresh = Self::default();
            fresh.prepare_snapping(&model, &faces, &entities, camera, size);
            for world in [
                point,
                point + DVec3::X,
                point + DVec3::X * 0.5,
                point + DVec3::Y,
            ] {
                let Some(cursor) = camera.project(world, size) else {
                    continue;
                };
                let targets = Default::default();
                let a = retained
                    .snapping
                    .nearby(cursor, targets, &retained.edges, &entities);
                let b = fresh
                    .snapping
                    .nearby(cursor, targets, &fresh.edges, &entities);
                if a.points != b.points || a.edges != b.edges || a.faces != b.faces {
                    return Err(format!(
                        "Retained snap candidates differ after edit {step} at {world:?}"
                    ));
                }
                checks += 1;
            }
        }
        Ok(checks)
    }
}
