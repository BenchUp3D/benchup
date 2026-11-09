//! Retain world geometry, project and shade on the GPU when ordinary opaque
//! geometry needs no CPU clipping. The precision/annotation renderer remains
//! available for every other display mode and transient editing preview.
use super::*;

use crate::theme::{FacePalette, Palette};
#[path = "resident_appearance.rs"]
mod appearance;
#[path = "resident_bounds.rs"]
mod bounds;
#[path = "resident_shadows.rs"]
mod shadows;
#[path = "resident_markers.rs"]
mod markers;

// Match fragment_visibility: face ID followed by the unmodified f32 depth bits.
#[cfg(any(test, target_arch = "wasm32"))]
#[path = "resident_regression.rs"]
mod regression;
const VISIBILITY_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg32Uint;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct FaceVertex {
    position: [f32; 3],
    normal: [f32; 3],
    anchor: [f32; 3],
    color: [u8; 4],
    face_id: u32,
    interaction: [u32; 2],
}
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct EdgeInstance {
    a: [f32; 3],
    b: [f32; 3],
    normal0: [f32; 3],
    inward0: [f32; 3],
    normal1: [f32; 3],
    inward1: [f32; 3],
    normal2: [f32; 3],
    inward2: [f32; 3],
    normal3: [f32; 3],
    inward3: [f32; 3],
    color: [u8; 4],
    width: f32,
    faces: [u32; 4],
    interaction: [u32; 2],
}
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct MarkerInstance {
    position: [f32; 3],
    edges: [u32; 4],
}
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniform {
    matrix: [[f32; 4]; 4],
    shadow_matrix: [[f32; 4]; 4],
    shadow_params: [f32; 4],
    plane_matrix: [[f32; 4]; 4],
    eye: [f32; 4],
    light: [f32; 4],
    direction: [f32; 4],
    viewport: [f32; 4],
    flags: [f32; 4],
    interaction: [u32; 4],
    face_selected: [f32; 4],
    face_hovered: [f32; 4],
    face_inactive: [f32; 4],
    edge_selected: [f32; 4],
    edge_hovered: [f32; 4],
    edge_inactive: [f32; 4],
    edge_muted: [f32; 4],
    edge_normal: [f32; 4],
    marker_fill: [f32; 4],
}
#[derive(Default)]
pub(super) struct Cache {
    key: Option<SceneKey>,
    geometry: Option<Arc<Geometry>>,
    builds: usize,
    appearance: appearance::Cache,
    source: Option<Model>,
    removed: std::collections::BTreeSet<u64>,
    bounds_index: bounds::Index,
    hierarchy: Option<appearance::Hierarchy>,
    overlay: Option<Arc<Geometry>>,
    bounds: Option<(DVec3, DVec3)>,
    states: Arc<OnceLock<appearance::StateTexture>>,
    state_capacity: usize,
    overlay_builds: usize,
    shadows: Option<Arc<shadows::Map>>,
    shadow_builds: usize,
}
struct Geometry {
    faces: Vec<FaceVertex>,
    edges: Vec<EdgeInstance>,
    markers: Vec<MarkerInstance>,
    origin: DVec3,
    bounds: (DVec3, DVec3),
    gpu: OnceLock<GeometryBuffers>,
}
struct GeometryBuffers {
    faces: wgpu::Buffer,
    edges: wgpu::Buffer,
    markers: wgpu::Buffer,
    visibility: Mutex<Option<Arc<Visibility>>>,
}
struct Visibility {
    size: [u32; 2],
    depth: wgpu::TextureView,
    faces: wgpu::TextureView,
}
struct LayerBuffers {
    _uniform: wgpu::Buffer,
    bind: wgpu::BindGroup,
}
struct DrawBuffers {
    layers: Vec<LayerBuffers>,
    visibility_bind: wgpu::BindGroup,
    visibility: Arc<Visibility>,
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
}
struct Draw {
    geometry: Arc<Geometry>,
    overlay: Option<Arc<Geometry>>,
    states: Arc<OnceLock<appearance::StateTexture>>,
    uniform: Uniform,
    shadows: Option<Arc<shadows::Map>>,
    markers: bool,
    appearance: Arc<Vec<u32>>,
    references: Vec<Vertex>,
    reference_indices: Vec<u32>,
    gpu: OnceLock<DrawBuffers>,
}
struct Pipelines {
    face: wgpu::RenderPipeline,
    marker: wgpu::RenderPipeline,
    shadow: wgpu::RenderPipeline,
    unshadowed: wgpu::TextureView,
    visibility: wgpu::RenderPipeline,
    edge_depth: wgpu::RenderPipeline,
    edge: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    visibility_layout: wgpu::BindGroupLayout,
    srgb: bool,
}

fn geometry_key(key: &SceneKey) -> SceneKey {
    let mut key = SceneKey {
        joinery_members: key.joinery_members,
        joint_preview: key.joint_preview.clone(),
        model_revision: key.model_revision,
        edit_context: None, camera: Camera::default(), size: DVec2::ZERO,
        pixels_per_point: 1., display: key.display.clone(), dark: key.dark,
        tool: benchup_core::Tool::Orbit, selected: Default::default(), hover: None,
        outliner_hover: Default::default(), erasing: Default::default(),
    };
    key.display.group_emphasis = false;
    key.display.edges = true;
    // Light, intensity and filtering affect the cached shadow map/uniform,
    // never the retained world geometry.
    key.display.shadows = false;
    key.display.shadow_darkness = 0.;
    key.display.sun_angle = 0.;
    key.display.sun_elevation = 0.;
    key.display.shadow_antialiasing = false;
    key
}

/// A deletion (and undo back to the original geometry) can use the same GPU
/// buffers. Compare ordered collections once, then mask just the removed IDs.
#[cfg(test)]
fn removed_geometry(source: &Model, model: &Model) -> Option<std::collections::BTreeSet<u64>> {
    use std::collections::BTreeSet;
    fn removed<T>(
        before: &BTreeMap<u64, T>,
        after: &BTreeMap<u64, T>,
        same: impl Fn(&T, &T) -> bool,
    ) -> Option<BTreeSet<u64>> {
        let mut next = after.iter().peekable();
        let mut removed = BTreeSet::new();
        for (id, value) in before {
            match next.peek() {
                Some((key, other)) if *key == id => {
                    if !same(value, other) {
                        return None;
                    }
                    next.next();
                }
                Some((key, _)) if *key < id => return None,
                _ => {
                    removed.insert(*id);
                }
            }
        }
        next.next().is_none().then_some(removed)
    }
    if source.hidden != model.hidden
        || source.locked != model.locked
        || source.tags != model.tags
        || source.entity_tags != model.entity_tags
    {
        return None;
    }
    removed(&source.vertices, &model.vertices, PartialEq::eq)?;
    let mut ids = removed(&source.edges, &model.edges, PartialEq::eq)?;
    let faces = removed(&source.faces, &model.faces, PartialEq::eq)?;
    let colors = removed(&source.face_colors, &model.face_colors, PartialEq::eq)?;
    if !colors.is_subset(&faces) {
        return None;
    }
    ids.extend(faces);
    let objects = removed(&source.objects, &model.objects, |before, after| {
        if before == after {
            return true;
        }
        if !after.entities.is_subset(&before.entities) {
            return false;
        }
        let mut before = before.clone();
        before.entities.clone_from(&after.entities);
        before == *after
    })?;
    ids.extend(&objects);
    // A removed ownership relation must describe removed geometry, not a move
    // or dissolution that leaves the same entity under a different parent.
    for (id, before) in &source.objects {
        let safe = if let Some(after) = model.objects.get(id) {
            before
                .entities
                .difference(&after.entities)
                .all(|child| ids.contains(child))
        } else {
            before.entities.iter().all(|child| ids.contains(child))
        };
        if !safe {
            return None;
        }
    }
    Some(ids)
}

pub(crate) fn edited_geometry(
    source: &Model,
    model: &Model,
) -> Option<std::collections::BTreeSet<u64>> {
    use std::collections::{BTreeSet, HashSet};
    fn changed<T: PartialEq>(before: &BTreeMap<u64, T>, after: &BTreeMap<u64, T>) -> BTreeSet<u64> {
        let mut old = before.iter().peekable();
        let mut new = after.iter().peekable();
        let mut ids = BTreeSet::new();
        loop {
            match (old.peek(), new.peek()) {
                (Some((a, _)), Some((b, _))) if a == b => {
                    let (id, a) = old.next().unwrap();
                    let (_, b) = new.next().unwrap();
                    if a != b {
                        ids.insert(*id);
                    }
                }
                (Some((a, _)), Some((b, _))) if a < b => {
                    ids.insert(*old.next().unwrap().0);
                }
                (_, Some(_)) => {
                    ids.insert(*new.next().unwrap().0);
                }
                (Some(_), None) => {
                    ids.insert(*old.next().unwrap().0);
                }
                (None, None) => break,
            }
        }
        ids
    }
    // Removing a hidden/tagged entity (or undoing its removal) does not change
    // the appearance of surviving primitives. Keep those deletions incremental.
    let survives = |id: &u64| source.contains_entity(id) && model.contains_entity(id);
    if source
        .hidden
        .symmetric_difference(&model.hidden)
        .any(survives)
        || source
            .locked
            .symmetric_difference(&model.locked)
            .any(survives)
        || source.tags != model.tags
        || source
            .entity_tags
            .keys()
            .chain(model.entity_tags.keys())
            .any(|id| source.entity_tags.get(id) != model.entity_tags.get(id) && survives(id))
    {
        return None;
    }
    let vertices: HashSet<_> = changed(&source.vertices, &model.vertices)
        .into_iter()
        .collect();
    let mut faces = changed(&source.faces, &model.faces);
    faces.extend(changed(&source.face_colors, &model.face_colors));
    if !vertices.is_empty() {
        faces.extend(
            model
                .faces
                .values()
                .filter(|face| {
                    face.vertices
                        .iter()
                        .chain(face.holes.iter().flatten())
                        .any(|id| vertices.contains(id))
                })
                .map(|face| face.id),
        );
    }
    let mut edges = changed(&source.edges, &model.edges);
    if !vertices.is_empty() {
        edges.extend(
            model
                .edges
                .values()
                .filter(|edge| vertices.contains(&edge.a) || vertices.contains(&edge.b))
                .map(|edge| edge.id),
        );
    }
    // Any changed incident plane must update its edge depth allowances too.
    let pairs: HashSet<_> = faces
        .iter()
        .flat_map(|id| [source.faces.get(id), model.faces.get(id)])
        .flatten()
        .flat_map(|face| std::iter::once(&face.vertices).chain(face.holes.iter()))
        .flat_map(|ring| {
            (0..ring.len()).map(|i| {
                let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
                (a.min(b), a.max(b))
            })
        })
        .collect();
    if !pairs.is_empty() {
        edges.extend(
            model
                .edges
                .values()
                .filter(|edge| pairs.contains(&(edge.a.min(edge.b), edge.a.max(edge.b))))
                .map(|edge| edge.id),
        );
    }
    // Membership changes require new owner slots in the affected primitives.
    for (&id, object) in &source.objects {
        if let Some(after) = model.objects.get(&id) {
            edges.extend(
                object
                    .entities
                    .symmetric_difference(&after.entities)
                    .copied(),
            );
        } else {
            edges.insert(id);
            edges.extend(&object.entities);
        }
    }
    for object in model
        .objects
        .values()
        .filter(|object| !source.objects.contains_key(&object.id))
    {
        edges.extend(&object.entities);
    }
    if edges.len() + faces.len() >= 4096 {
        return None;
    }
    for id in &edges {
        if source.contains_entity(id)
            && model.contains_entity(id)
            && source.parent(*id) != model.parent(*id)
        {
            return None;
        }
    }
    edges.extend(faces);
    Some(edges)
}

pub(crate) fn supported(key: &SceneKey, model: &Model, faces: &FaceCache) -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        thread_local! {
            static LEGACY: bool = web_sys::window().and_then(|window|
                js_sys::Reflect::get(&window, &"benchupViewportBenchmarkLegacy".into()).ok())
                .and_then(|value| value.as_bool()).unwrap_or(false);
        }
        if LEGACY.with(|value| *value) {
            return false;
        }
    }
    let d = &key.display;
    model.faces.len() >= 1024
        && faces.max_edge_faces <= 4
        && key.camera.projection == benchup_core::Projection::Perspective
        && matches!(
            d.face_style,
            benchup_core::FaceStyle::Materials | benchup_core::FaceStyle::Shaded
        )
        && key.joinery_members.is_none()
        && key.joint_preview.is_none()
        && !(d.x_ray
            || d.fog
            || d.depth_cue
            || d.profiles
            || d.back_edges
            || d.endpoints
            || d.extensions
            || d.grain_direction
            || d.hidden_geometry
            || d.hidden_objects
            || d.section_planes
            || d.section_cuts
            || d.section_fill)
        && model.dimensions.is_empty()
        && model.texts.is_empty()
        && model.connections.is_empty()
        && !matches!(
            key.tool,
            benchup_core::Tool::Move | benchup_core::Tool::Paint
        )
}

impl SceneCache {
    pub(crate) fn resident_context_bounds(
        &mut self,
        context: u64,
        model: &Model,
        entities: &EntityCache,
    ) -> Option<(DVec3, DVec3)> {
        let _ = (model, entities);
        self.resident.bounds_index.bounds(Some(context))
    }
    #[cfg(any(test, target_arch = "wasm32"))]
    pub(crate) fn resident_builds(&self) -> usize {
        self.resident.builds
    }
    #[cfg(any(test, target_arch = "wasm32"))]
    pub(crate) fn resident_shadow_builds(&self) -> usize {
        self.resident.shadow_builds
    }
    pub(crate) fn prepare_resident(
        &mut self,
        key: &SceneKey,
        model: &Model,
        faces: &FaceCache,
        entities: &EntityCache,
        ui: &egui::Ui,
    ) {
        let geometry_key = geometry_key(key);
        if self.resident.key.as_ref() != Some(&geometry_key) {
            let same_style = self.resident.key.as_ref().is_some_and(|previous| {
                let mut previous = previous.clone();
                previous.model_revision = geometry_key.model_revision;
                previous == geometry_key
            });
            let dirty = same_style
                .then(|| {
                    self.resident
                        .source
                        .as_ref()
                        .and_then(|source| edited_geometry(source, model))
                })
                .flatten();
            if let Some(dirty) = dirty {
                let source = self.resident.source.as_ref().unwrap();
                let hierarchy = self.resident.hierarchy.as_mut().unwrap();
                let touched = self.resident.removed.union(&dirty).copied().collect();
                hierarchy.synchronize(source, model, entities, &touched);
                self.resident
                    .bounds_index
                    .update(model, entities, &key.display, &touched);
                self.resident.removed = dirty;
                let live = self
                    .resident
                    .removed
                    .iter()
                    .any(|id| model.faces.contains_key(id) || model.edges.contains_key(id));
                self.resident.overlay = live.then(|| {
                    Arc::new(Geometry::subset(
                        key,
                        model,
                        faces,
                        entities,
                        ui,
                        hierarchy,
                        Some(&self.resident.removed),
                        Some(self.resident.geometry.as_ref().unwrap().origin),
                    ))
                });
                self.resident.overlay_builds += usize::from(live);
                self.resident.appearance = appearance::Cache::default();
            } else {
                let hierarchy = appearance::Hierarchy::new(model, entities, faces);
                self.resident.geometry = Some(Arc::new(Geometry::subset(
                    key, model, faces, entities, ui, &hierarchy, None, None,
                )));
                self.resident.hierarchy = Some(hierarchy);
                self.resident.bounds_index = bounds::Index::new(model, entities, &key.display);
                self.resident.source = Some(model.clone());
                self.resident.overlay = None;
                self.resident.removed.clear();
                self.resident.builds += 1;
                self.resident.appearance = appearance::Cache::default();
            }
            self.resident.bounds = self.resident.bounds_index.bounds(None);
            self.resident.key = Some(geometry_key);
        }
    }
    pub(crate) fn paint_resident(
        &mut self,
        key: &SceneKey,
        model: &Model,
        faces: &FaceCache,
        entities: &EntityCache,
        ui: &egui::Ui,
        rect: Rect,
        references: Mesh,
    ) {
        self.prepare_resident(key, model, faces, entities, ui);
        let geometry = Arc::clone(self.resident.geometry.as_ref().unwrap());
        let (near, far) = clip_planes(&key.camera, self.resident.bounds.unwrap_or(geometry.bounds));
        let matrix = key.camera.matrix_with_clip_planes(key.size, near, far)
            * glam::DMat4::from_translation(geometry.origin);
        let eye = key.camera.eye();
        let toward_eye = (eye - key.camera.target).normalize();
        let light =
            (toward_eye * 0.8 - key.camera.right() * 0.45 + key.camera.up() * 0.65).normalize();
        let appearance = self.resident.hierarchy.as_ref().unwrap().update(
            &mut self.resident.appearance,
            key,
            &self.resident.removed,
        );
        if self.resident.state_capacity != appearance.len() {
            self.resident.states = Arc::new(OnceLock::new());
            self.resident.state_capacity = appearance.len();
        }
        let palette = Palette::for_ui(ui);
        let face_palette = FacePalette::for_ui(ui);
        let rgba = |color: Color32| color.to_array().map(|v| v as f32 / 255.);
        let previous_shadow = self.resident.shadows.clone();
        let shadows = shadows::Map::for_scene(
            &mut self.resident.shadows, key, self.resident.bounds.unwrap_or(geometry.bounds),
            geometry.origin,
        );
        if shadows.as_ref().is_some_and(|map| previous_shadow.as_ref().is_none_or(|old| !Arc::ptr_eq(old, map))) {
            self.resident.shadow_builds += 1;
        }
        let callback = Draw {
            shadows: shadows.clone(),
            markers: key.selected.iter().any(|id| model.edges.contains_key(id)),
            appearance,
            geometry: Arc::clone(&geometry),
            overlay: self.resident.overlay.clone(),
            states: Arc::clone(&self.resident.states),
            uniform: Uniform {
                matrix: matrix.as_mat4().to_cols_array_2d(),
                shadow_matrix: shadows.as_ref().map_or(glam::Mat4::IDENTITY.to_cols_array_2d(),
                    |map| map.matrix.as_mat4().to_cols_array_2d()),
                shadow_params: [
                    if shadows.is_some() { key.display.shadow_darkness as f32 } else { 0. },
                    if key.display.shadow_antialiasing { 1. } else { 0. }, 0.000002, 0.,
                ],
                plane_matrix: matrix.inverse().transpose().as_mat4().to_cols_array_2d(),
                eye: (eye - self.resident.geometry.as_ref().unwrap().origin)
                    .extend(0.)
                    .as_vec4()
                    .to_array(),
                light: light.extend(0.).as_vec4().to_array(),
                direction: toward_eye.extend(0.).as_vec4().to_array(),
                viewport: [
                    key.size.x as f32,
                    key.size.y as f32,
                    key.pixels_per_point,
                    1.8,
                ],
                interaction: [
                    self.resident
                        .hierarchy
                        .as_ref()
                        .unwrap()
                        .slot(key.hover.unwrap_or(0)),
                    u32::from(key.edit_context.is_some() && key.display.group_emphasis),
                    u32::from(key.display.edges),
                    u32::from(key.tool == benchup_core::Tool::Erase)
                        | (u32::from(!self.resident.removed.is_empty()) << 1),
                ],
                face_selected: rgba(face_palette.selected),
                face_hovered: rgba(face_palette.hovered),
                face_inactive: rgba(face_palette.inactive),
                edge_selected: rgba(crate::theme::ACCENT),
                edge_hovered: rgba(palette.green),
                edge_inactive: rgba(if key.dark {
                    palette.border
                } else {
                    Color32::from_rgb(184, 193, 181)
                }),
                edge_muted: rgba(palette.muted.gamma_multiply(0.55)),
                edge_normal: rgba(palette.ink),
                marker_fill: rgba(palette.paper),
                flags: [
                    0.,
                    if key.camera.projection == benchup_core::Projection::Parallel {
                        1.
                    } else {
                        0.
                    },
                    0.,
                    0.,
                ],
            },
            references: references.vertices,
            reference_indices: references.indices,
            gpu: OnceLock::new(),
        };
        ui.painter()
            .with_clip_rect(rect)
            .add(egui_wgpu::Callback::new_paint_callback(rect, callback));
    }
}

/// A conservative camera-space bounds test increases depth precision without
/// clipping any model geometry. Close/inside views retain the original near
/// plane; parallel views use finite scene bounds instead of a 100,000 km slab.
fn clip_planes(camera: &Camera, bounds: (DVec3, DVec3)) -> (f64, f64) {
    let center = (bounds.0 + bounds.1) * 0.5;
    let half = (bounds.1 - bounds.0) * 0.5;
    let forward = (camera.target - camera.eye()).normalize();
    let depth = (center - camera.eye()).dot(forward);
    let radius = half.dot(forward.abs());
    let near = ((depth - radius) * 0.5).max(0.005);
    let far = if camera.projection == benchup_core::Projection::Parallel {
        (depth + radius + camera.distance * 2.).max(near + 1.)
    } else {
        1e8_f64.max((depth + radius) * 2.).max(near * 2.)
    };
    (near, far)
}

impl Geometry {
    #[cfg(test)]
    fn new(
        key: &SceneKey,
        model: &Model,
        faces: &FaceCache,
        entities: &EntityCache,
        ui: &egui::Ui,
    ) -> Self {
        let hierarchy = appearance::Hierarchy::new(model, entities, faces);
        Self::subset(key, model, faces, entities, ui, &hierarchy, None, None)
    }
    fn subset(
        key: &SceneKey,
        model: &Model,
        faces: &FaceCache,
        entities: &EntityCache,
        ui: &egui::Ui,
        hierarchy: &appearance::Hierarchy,
        dirty: Option<&std::collections::BTreeSet<u64>>,
        origin: Option<DVec3>,
    ) -> Self {
        let palette = Palette::for_ui(ui);
        let face_palette = FacePalette::for_ui(ui);
        let bounds = model.vertices.values().map(|v| v.position).fold(
            (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY)),
            |(lo, hi), p| (lo.min(p), hi.max(p)),
        );
        let origin = origin.unwrap_or((bounds.0 + bounds.1) * 0.5);
        let point = |p: DVec3| (p - origin).as_vec3().to_array();
        let mut vertices = Vec::new();
        let mut surfaces: BTreeMap<(u64, u64), Vec<(DVec3, DVec3, u32)>> = BTreeMap::new();
        for face in model.faces.values().filter(|f| {
            dirty.is_none_or(|dirty| dirty.contains(&f.id))
                && entities.get(f.id).displayed(&key.display)
        }) {
            // Visibility IDs also address interaction states. Retained edges
            // can then ignore depth allowances from an erased incident face.
            let face_id = hierarchy.slot(face.id);
            let entity = entities.get(face.id);
            let cached = faces.get(face.id);
            let material = if key.display.color_by_tag {
                entity.tag_color.as_ref()
            } else if key.display.face_style == benchup_core::FaceStyle::Materials {
                model.face_colors.get(&face.id)
            } else {
                None
            };
            let material = material.map_or(face_palette.front, |rgb| {
                Color32::from_rgb(rgb[0], rgb[1], rgb[2])
            });
            let color = material;
            let anchor = model.vertices[&face.vertices[0]].position;
            for triangle in &cached.triangles {
                for &p in triangle {
                    vertices.push(FaceVertex {
                        position: point(p),
                        normal: cached.normal.as_vec3().to_array(),
                        anchor: point(anchor),
                        color: color.to_array(),
                        face_id,
                        interaction: hierarchy.pair(face.id),
                    });
                }
            }
        }
        let surface_faces: std::collections::BTreeSet<u64> = if let Some(dirty) = dirty {
            dirty
                .iter()
                .filter_map(|id| model.edges.get(id))
                .flat_map(|edge| faces.adjacent_faces(edge.a, edge.b).iter().copied())
                .collect()
        } else {
            model.faces.keys().copied().collect()
        };
        for id in surface_faces {
            let face = &model.faces[&id];
            if !entities.get(id).displayed(&key.display) {
                continue;
            }
            let cached = faces.get(id);
            let face_id = hierarchy.slot(id);
            // Reuse the actual ring winding, including holes. Each half of a
            // stroke follows only the incident face on that side of its edge.
            for (ring_index, ring) in std::iter::once(&face.vertices)
                .chain(&face.holes)
                .enumerate()
            {
                let base = model.vertices[&ring[0]].position;
                let area_normal: DVec3 = (0..ring.len())
                    .map(|i| {
                        (model.vertices[&ring[i]].position - base)
                            .cross(model.vertices[&ring[(i + 1) % ring.len()]].position - base)
                    })
                    .sum();
                let sign = area_normal.dot(cached.normal).signum()
                    * if ring_index == 0 { 1. } else { -1. };
                for i in 0..ring.len() {
                    let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
                    let inward = cached
                        .normal
                        .cross(model.vertices[&b].position - model.vertices[&a].position)
                        .normalize_or_zero()
                        * sign;
                    surfaces.entry((a.min(b), a.max(b))).or_default().push((
                        cached.normal,
                        inward,
                        face_id,
                    ));
                }
            }
        }
        let mut edges = Vec::new();
        let mut endpoints: std::collections::BTreeMap<u64, Vec<u32>> = Default::default();
        for edge in model.edges.values().filter(|e| {
            dirty.is_none_or(|dirty| dirty.contains(&e.id))
                && entities.get(e.id).displayed(&key.display)
        }) {
            for vertex in [edge.a, edge.b] {
                endpoints.entry(vertex).or_default().push(hierarchy.slot(edge.id));
            }
            let state = entities.get(edge.id);
            let color = if key.display.color_by_tag {
                state
                    .tag_color
                    .map_or(palette.ink, |rgb| Color32::from_rgb(rgb[0], rgb[1], rgb[2]))
            } else {
                palette.ink
            };
            let neighbors = surfaces.get(&(edge.a.min(edge.b), edge.a.max(edge.b)));
            let surface = |i| {
                neighbors
                    .and_then(|v| v.get(i))
                    .copied()
                    .unwrap_or_default()
            };
            let (n0, i0, f0) = surface(0);
            let (n1, i1, f1) = surface(1);
            let (n2, i2, f2) = surface(2);
            let (n3, i3, f3) = surface(3);
            let (a, b) = model.points(edge);
            let width = 1.8;
            edges.push(EdgeInstance {
                a: point(a),
                b: point(b),
                normal0: n0.as_vec3().to_array(),
                inward0: i0.as_vec3().to_array(),
                normal1: n1.as_vec3().to_array(),
                inward1: i1.as_vec3().to_array(),
                normal2: n2.as_vec3().to_array(),
                inward2: i2.as_vec3().to_array(),
                normal3: n3.as_vec3().to_array(),
                inward3: i3.as_vec3().to_array(),
                color: color.to_array(),
                width,
                faces: [f0, f1, f2, f3],
                interaction: hierarchy.pair(edge.id),
            });
        }
        let markers = endpoints.into_iter().flat_map(|(id, edges)| {
            let position = point(model.vertices[&id].position);
            edges.chunks(4).map(|chunk| {
                let mut edges = [0; 4];
                edges[..chunk.len()].copy_from_slice(chunk);
                MarkerInstance { position, edges }
            }).collect::<Vec<_>>()
        }).collect();
        Self {
            faces: vertices,
            edges,
            markers,
            origin,
            bounds,
            gpu: OnceLock::new(),
        }
    }
}

pub(super) fn install(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    renderer: &mut egui_wgpu::Renderer,
) {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("BenchUp resident world geometry"),
        source: wgpu::ShaderSource::Wgsl(include_str!("resident_scene.wgsl").into()),
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("Resident camera"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Uint,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
        ],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Resident viewport"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let visibility_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("Resident visibility and shadows"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Uint,
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        }, wgpu::BindGroupLayoutEntry {
            binding: 1,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Uint,
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        }],
    });
    let edge_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Resident edges with centerline visibility"),
        bind_group_layouts: &[Some(&layout), Some(&visibility_layout)],
        immediate_size: 0,
    });
    let create = |edge: bool, depth: bool, visibility: bool| {
        let attributes = if edge {
            wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x3,2=>Float32x3,3=>Float32x3,4=>Float32x3,5=>Float32x3,6=>Float32x3,7=>Float32x3,8=>Float32x3,9=>Float32x3,10=>Unorm8x4,11=>Float32,12=>Uint32x4,13=>Uint32x2].to_vec()
        } else {
            wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x3,2=>Float32x3,3=>Unorm8x4,4=>Uint32,5=>Uint32x2]
                .to_vec()
        };
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Resident viewport geometry"),
            layout: Some(if visibility { &pipeline_layout } else { &edge_layout }),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some(if edge { "edge_vertex" } else { "face_vertex" }),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: if edge {
                        std::mem::size_of::<EdgeInstance>()
                    } else {
                        std::mem::size_of::<FaceVertex>()
                    } as u64,
                    step_mode: if edge {
                        wgpu::VertexStepMode::Instance
                    } else {
                        wgpu::VertexStepMode::Vertex
                    },
                    attributes: &attributes,
                })],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some(if visibility {
                    "fragment_visibility"
                } else if depth {
                    "edge_depth"
                } else if edge {
                    "edge_color"
                } else {
                    "face_color"
                }),
                targets: &[Some(wgpu::ColorTargetState {
                    format: if visibility {
                        VISIBILITY_FORMAT
                    } else {
                        format
                    },
                    blend: if visibility {
                        None
                    } else {
                        Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING)
                    },
                    write_mask: if depth {
                        wgpu::ColorWrites::empty()
                    } else {
                        wgpu::ColorWrites::ALL
                    },
                })],
                compilation_options: Default::default(),
            }),
            primitive: Default::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(!edge || depth),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        })
    };
    renderer.callback_resources.insert(Pipelines {
        face: create(false, false, false),
        marker: markers::pipeline(device, &shader, &pipeline_layout, format),
        shadow: shadows::pipeline(device, &shader, &pipeline_layout),
        unshadowed: shadows::target(device, 1).faces,
        visibility: create(false, false, true),
        edge_depth: create(true, true, false),
        edge: create(true, false, false),
        layout,
        visibility_layout,
        srgb: format.is_srgb(),
    });
}
impl egui_wgpu::CallbackTrait for Draw {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen: &egui_wgpu::ScreenDescriptor,
        encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let Some(pipelines) = resources.get::<Pipelines>() else {
            return Vec::new();
        };
        let geometries: Vec<_> = std::iter::once(&self.geometry)
            .chain(self.overlay.iter())
            .collect();
        for geometry in &geometries {
            geometry.gpu.get_or_init(|| GeometryBuffers {
                visibility: Mutex::new(None),
                markers: upload(device, queue, &wgpu::util::BufferInitDescriptor {
                    label: Some("Resident endpoint handles"),
                    contents: bytemuck::cast_slice(&geometry.markers),
                    usage: wgpu::BufferUsages::VERTEX,
                }),
                faces: upload(
                    device,
                    queue,
                    &wgpu::util::BufferInitDescriptor {
                        label: Some("Resident face vertices"),
                        contents: bytemuck::cast_slice(&geometry.faces),
                        usage: wgpu::BufferUsages::VERTEX,
                    },
                ),
                edges: upload(
                    device,
                    queue,
                    &wgpu::util::BufferInitDescriptor {
                        label: Some("Resident edge instances"),
                        contents: bytemuck::cast_slice(&geometry.edges),
                        usage: wgpu::BufferUsages::VERTEX,
                    },
                ),
            });
        }
        let geometry = self.geometry.gpu.get().unwrap();
        let states = self
            .states
            .get_or_init(|| appearance::StateTexture::new(device, self.appearance.len()));
        states.update(queue, &self.appearance);
        let size = [
            (self.uniform.viewport[0] * self.uniform.viewport[2])
                .round()
                .max(1.) as u32,
            (self.uniform.viewport[1] * self.uniform.viewport[2])
                .round()
                .max(1.) as u32,
        ];
        let visibility = {
            let mut saved = geometry.visibility.lock().unwrap();
            if saved.as_ref().is_none_or(|target| target.size != size) {
                let texture = |format, usage| {
                    device
                        .create_texture(&wgpu::TextureDescriptor {
                            label: Some("Resident face visibility"),
                            size: wgpu::Extent3d {
                                width: size[0],
                                height: size[1],
                                depth_or_array_layers: 1,
                            },
                            mip_level_count: 1,
                            sample_count: 1,
                            dimension: wgpu::TextureDimension::D2,
                            format,
                            usage,
                            view_formats: &[],
                        })
                        .create_view(&Default::default())
                };
                *saved = Some(Arc::new(Visibility {
                    size,
                    depth: texture(
                        wgpu::TextureFormat::Depth32Float,
                        wgpu::TextureUsages::RENDER_ATTACHMENT,
                    ),
                    faces: texture(
                        VISIBILITY_FORMAT,
                        wgpu::TextureUsages::RENDER_ATTACHMENT
                            | wgpu::TextureUsages::TEXTURE_BINDING,
                    ),
                }));
            }
            Arc::clone(saved.as_ref().unwrap())
        };
        let new_shadow = self.shadows.as_ref().is_some_and(|map| map.prepared.get().is_none());
        let shadow = self.shadows.as_ref().map(|map|
            map.gpu.get_or_init(|| shadows::target(device, 4096.min(device.limits().max_texture_dimension_2d))));
        let draw = self.gpu.get_or_init(|| {
            let layers = geometries
                .iter()
                .enumerate()
                .map(|(index, _)| {
                    let mut uniform = self.uniform;
                    uniform.flags[0] = if pipelines.srgb { 1. } else { 0. };
                    if index != 0 {
                        uniform.interaction[3] |= 4;
                    }
                    let buffer = upload(
                        device,
                        queue,
                        &wgpu::util::BufferInitDescriptor {
                            label: Some("Resident camera uniform"),
                            contents: bytemuck::bytes_of(&uniform),
                            usage: wgpu::BufferUsages::UNIFORM,
                        },
                    );
                    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("Resident camera"),
                        layout: &pipelines.layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: buffer.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::TextureView(&states.view),
                            },
                        ],
                    });
                    LayerBuffers {
                        _uniform: buffer,
                        bind,
                    }
                })
                .collect();
            let visibility_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Resident centerline visibility"),
                layout: &pipelines.visibility_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&visibility.faces),
                }, wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(
                        shadow.map_or(&pipelines.unshadowed, |target| &target.faces)),
                }],
            });
            DrawBuffers {
                layers,
                visibility_bind,
                visibility: Arc::clone(&visibility),
                vertices: upload(
                    device,
                    queue,
                    &wgpu::util::BufferInitDescriptor {
                        label: Some("Reference vertices"),
                        contents: bytemuck::cast_slice(&self.references),
                        usage: wgpu::BufferUsages::VERTEX,
                    },
                ),
                indices: upload(
                    device,
                    queue,
                    &wgpu::util::BufferInitDescriptor {
                        label: Some("Reference indices"),
                        contents: bytemuck::cast_slice(&self.reference_indices),
                        usage: wgpu::BufferUsages::INDEX,
                    },
                ),
            }
        });
        if new_shadow {
            let target = shadow.unwrap();
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Cached resident sunlight"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.faces, depth_slice: None, resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &target.depth,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.), store: wgpu::StoreOp::Store }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_pipeline(&pipelines.shadow);
            for (geometry, layer) in geometries.iter().zip(&draw.layers) {
                pass.set_bind_group(0, &layer.bind, &[]);
                pass.set_vertex_buffer(0, geometry.gpu.get().unwrap().faces.slice(..));
                pass.draw(0..geometry.faces.len() as u32, 0..1);
            }
        }
        if new_shadow {
            self.shadows.as_ref().unwrap().prepared.get_or_init(|| ());
        }
        if geometries.iter().any(|geometry| !geometry.edges.is_empty()) {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Resident edge centerline visibility"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &draw.visibility.faces,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &draw.visibility.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_pipeline(&pipelines.visibility);
            for (geometry, layer) in geometries.iter().zip(&draw.layers) {
                pass.set_bind_group(0, &layer.bind, &[]);
                pass.set_vertex_buffer(0, geometry.gpu.get().unwrap().faces.slice(..));
                pass.draw(0..geometry.faces.len() as u32, 0..1);
            }
        }
        Vec::new()
    }
    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        let (Some(pipelines), Some(references), Some(draw)) = (
            resources.get::<Pipelines>(),
            resources.get::<Resources>(),
            self.gpu.get(),
        ) else {
            return;
        };
        let geometries: Vec<_> = std::iter::once(&self.geometry)
            .chain(self.overlay.iter())
            .collect();
        pass.set_pipeline(&pipelines.face);
        pass.set_bind_group(1, &draw.visibility_bind, &[]);
        for (geometry, layer) in geometries.iter().zip(&draw.layers) {
            pass.set_bind_group(0, &layer.bind, &[]);
            pass.set_vertex_buffer(0, geometry.gpu.get().unwrap().faces.slice(..));
            pass.draw(0..geometry.faces.len() as u32, 0..1);
        }
        for (pipeline, reference_pipeline) in [
            (&pipelines.edge_depth, &references.edge_depth_pipeline),
            (&pipelines.edge, &references.edge_pipeline),
        ] {
            pass.set_pipeline(pipeline);
            pass.set_bind_group(1, &draw.visibility_bind, &[]);
            for (geometry, layer) in geometries.iter().zip(&draw.layers) {
                pass.set_bind_group(0, &layer.bind, &[]);
                pass.set_vertex_buffer(0, geometry.gpu.get().unwrap().edges.slice(..));
                pass.draw(0..12, 0..geometry.edges.len() as u32);
            }
            if !self.reference_indices.is_empty() {
                pass.set_pipeline(reference_pipeline);
                pass.set_vertex_buffer(0, draw.vertices.slice(..));
                pass.set_index_buffer(draw.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..self.reference_indices.len() as u32, 0, 0..1);
            }
        }
        if self.markers {
            pass.set_pipeline(&pipelines.marker);
            for (geometry, layer) in geometries.iter().zip(&draw.layers) {
                pass.set_bind_group(0, &layer.bind, &[]);
                pass.set_vertex_buffer(0, geometry.gpu.get().unwrap().markers.slice(..));
                pass.draw(0..6, 0..geometry.markers.len() as u32);
            }
        }
    }
}

impl SceneCache {
    pub(crate) fn resident_reference_mesh(&self, key: &SceneKey, model: &Model) -> Mesh {
        let bounds = self
            .resident
            .geometry
            .as_ref()
            .filter(|_| self.resident.key.as_ref() == Some(&geometry_key(key)))
            .map(|geometry| self.resident.bounds.unwrap_or(geometry.bounds))
            .unwrap_or_else(|| {
                model.vertices.values().map(|v| v.position).fold(
                    (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY)),
                    |(lo, hi), p| (lo.min(p), hi.max(p)),
                )
            });
        let (near, far) = clip_planes(&key.camera, bounds);
        let mut mesh = Mesh::new(&key.camera, key.size);
        mesh.view = key.camera.view_with_clip_planes(key.size, near, far);
        mesh.set_pixels_per_point(key.pixels_per_point);
        mesh
    }
}

fn upload(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    descriptor: &wgpu::util::BufferInitDescriptor<'_>,
) -> wgpu::Buffer {
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: descriptor.label,
        size: (descriptor.contents.len() as u64).max(4),
        usage: descriptor.usage | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    if !descriptor.contents.is_empty() {
        queue.write_buffer(&buffer, 0, descriptor.contents);
    }
    buffer
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ui::ContextExt as _;
    use benchup_core::{Edge, Vertex as ModelVertex};

    // The Linux GLES backend enables Naga's GLSL writer. Exercise the WebGL2
    // translation path without requiring a browser or GPU adapter.
    #[cfg(target_os = "linux")]
    #[test]
    fn resident_shader_translates_to_webgl2() {
        use wgpu::naga::{back::glsl, front::wgsl, valid};

        let module = wgsl::parse_str(include_str!("resident_scene.wgsl")).unwrap();
        let info = valid::Validator::new(Default::default(), Default::default())
            .validate(&module)
            .unwrap();
        let options = glsl::Options {
            version: glsl::Version::Embedded {
                version: 300,
                is_webgl: true,
            },
            ..Default::default()
        };
        for entry in &module.entry_points {
            let pipeline = glsl::PipelineOptions {
                shader_stage: entry.stage,
                entry_point: entry.name.clone(),
                multiview: None,
            };
            let mut source = String::new();
            glsl::Writer::new(
                &mut source,
                &module,
                &info,
                &options,
                &pipeline,
                Default::default(),
            )
            .and_then(|mut writer| writer.write())
            .unwrap_or_else(|error| panic!("WebGL2 entry point {}: {error}", entry.name));
        }
    }

    fn model(count: usize) -> Model {
        let mut model = Model::default();
        for i in 0..count {
            let base = 1 + i as u64 * 7;
            let offset = DVec3::new((i % 32) as f64 * 2., (i / 32) as f64 * 2., 0.);
            for (j, p) in [DVec3::ZERO, DVec3::X, DVec3::Y].into_iter().enumerate() {
                let id = base + j as u64;
                model.vertices.insert(
                    id,
                    ModelVertex {
                        id,
                        position: p + offset,
                    },
                );
                let id = base + 3 + j as u64;
                model.edges.insert(
                    id,
                    Edge {
                        id,
                        a: base + j as u64,
                        b: base + ((j + 1) % 3) as u64,
                    },
                );
            }
            let id = base + 6;
            model.faces.insert(
                id,
                Face {
                    id,
                    vertices: vec![base, base + 1, base + 2],
                    holes: Vec::new(),
                },
            );
        }
        let next_id = model.faces.keys().next_back().copied().unwrap_or(0);
        let mut value = serde_json::to_value(model).unwrap();
        value["next_id"] = serde_json::json!(next_id);
        serde_json::from_value(value).unwrap()
    }
    fn key() -> SceneKey {
        SceneKey {
            joinery_members: None,
            joint_preview: None,
            model_revision: 1,
            edit_context: None,
            camera: Camera::default(),
            size: DVec2::new(800., 600.),
            pixels_per_point: 1.,
            display: Default::default(),
            dark: false,
            tool: benchup_core::Tool::Orbit,
            selected: Default::default(),
            hover: None,
            outliner_hover: Default::default(),
            erasing: Default::default(),
        }
    }

    #[test]
    fn geometry_survives_camera_and_dpi_changes_but_tracks_edits_and_replacement_projects() {
        let ctx = egui::Context::default();
        let mut model = model(1024);
        let mut snapshot = ModelSnapshot::default();
        let mut faces = FaceCache::default();
        let mut entities = EntityCache::default();
        let mut cache = SceneCache::default();
        let mut key = key();
        let mut render = |model: &Model, key: &mut SceneKey, cache: &mut SceneCache| {
            key.model_revision = snapshot.update_scene(model);
            faces.update_for_revision(model, key.model_revision);
            entities.update_for_revision(model, key.model_revision);
            assert!(supported(key, model, &faces));
            let _ = ctx.run_test_ui(egui::RawInput::default(), |ui| {
                let references = cache.resident_reference_mesh(key, model);
                cache.paint_resident(
                    key,
                    model,
                    &faces,
                    &entities,
                    ui,
                    Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800., 600.)),
                    references,
                );
            });
        };
        render(&model, &mut key, &mut cache);
        let geometry = cache.resident.geometry.as_ref().unwrap().clone();
        key.camera.yaw += 0.5;
        key.camera.distance *= 0.7;
        key.size *= 1.2;
        key.pixels_per_point = 2.;
        render(&model, &mut key, &mut cache);
        assert!(Arc::ptr_eq(
            &geometry,
            cache.resident.geometry.as_ref().unwrap()
        ));
        assert_eq!(cache.resident.builds, 1);
        key.selected.insert(7);
        render(&model, &mut key, &mut cache);
        assert_eq!(cache.resident.builds, 1);
        let hierarchy = cache.resident.hierarchy.as_ref().unwrap();
        assert_ne!(
            cache.resident.appearance.values[hierarchy.slot(7) as usize] & appearance::SELECTED,
            0
        );
        key.hover = Some(14);
        key.outliner_hover.insert(21);
        key.edit_context = Some(900_000);
        model.edit_context = key.edit_context;
        key.display.edges = false;
        key.display.group_emphasis = true;
        render(&model, &mut key, &mut cache);
        assert_eq!(cache.resident.builds, 1);
        assert!(Arc::ptr_eq(
            &geometry,
            cache.resident.geometry.as_ref().unwrap()
        ));
        model.face_colors.insert(7, [210, 45, 30]);
        render(&model, &mut key, &mut cache);
        assert_eq!(cache.resident.builds, 1);
        assert!(cache.resident.overlay.is_some());
        assert!(Arc::ptr_eq(
            &geometry,
            cache.resident.geometry.as_ref().unwrap()
        ));
        model.vertices.get_mut(&1).unwrap().position.z = 0.01;
        render(&model, &mut key, &mut cache);
        assert_eq!(cache.resident.builds, 1);
        assert!(cache.resident.overlay.as_ref().unwrap().edges.len() < model.edges.len());
        let previous_origin = cache.resident.geometry.as_ref().unwrap().origin;
        for vertex in model.vertices.values_mut() {
            vertex.position.x += 1000.;
        }
        render(&model, &mut key, &mut cache);
        assert_eq!(cache.resident.builds, 2);
        assert!(
            (cache.resident.geometry.as_ref().unwrap().origin.x - previous_origin.x - 1000.).abs()
                < 1e-9
        );
    }

    #[test]
    fn appearance_inherits_selection_hover_locks_and_nested_scopes() {
        use appearance::{ACTIVE, ERASING, HOVERED, LOCKED, OUTLINER, REMOVED, SELECTED};
        use std::collections::BTreeSet;
        let mut model = model(1024);
        let child = model
            .make_object(&BTreeSet::from([7]), false, "Child")
            .unwrap();
        let root = model
            .make_object(&BTreeSet::from([child]), false, "Root")
            .unwrap();
        model.locked.insert(child);
        let face = *model
            .faces
            .keys()
            .find(|id| model.entity_in_context(**id) == Some(root))
            .unwrap();
        let loose = *model
            .faces
            .keys()
            .find(|id| model.entity_in_context(**id) != Some(root))
            .unwrap();
        let mut entities = EntityCache::default();
        let mut faces = FaceCache::default();
        entities.update(&model);
        faces.update(&model);
        let hierarchy = appearance::Hierarchy::new(&model, &entities, &faces);
        let mut cache = appearance::Cache::default();
        let mut key = key();
        key.selected.insert(root);
        let flags = |values: &[u32], id| {
            let [owner, own] = hierarchy.pair(id);
            values[owner as usize] | values[own as usize]
        };
        let values = hierarchy.update(&mut cache, &key, &Default::default());
        assert_eq!(
            flags(&values, face) & (SELECTED | LOCKED | ACTIVE),
            SELECTED | LOCKED | ACTIVE
        );
        assert_eq!(flags(&values, loose) & SELECTED, 0);
        assert!(Arc::ptr_eq(
            &values,
            &hierarchy.update(&mut cache, &key, &Default::default())
        ));
        key.selected.clear();
        key.edit_context = Some(root);
        key.hover = Some(child);
        let values = hierarchy.update(&mut cache, &key, &Default::default());
        assert_eq!(flags(&values, face) & SELECTED, 0);
        assert_ne!(flags(&values, face) & HOVERED, 0);
        assert_eq!(flags(&values, loose) & ACTIVE, 0);
        key.hover = None;
        key.edit_context = Some(child);
        key.outliner_hover.insert(root);
        let values = hierarchy.update(&mut cache, &key, &Default::default());
        assert_eq!(
            flags(&values, face) & (ACTIVE | OUTLINER),
            ACTIVE | OUTLINER
        );
        assert_eq!(flags(&values, face) & HOVERED, 0);
        key.edit_context = None;
        key.outliner_hover.clear();
        let values = hierarchy.update(&mut cache, &key, &Default::default());
        assert_eq!(flags(&values, face) & (SELECTED | HOVERED | OUTLINER), 0);
        assert_ne!(flags(&values, loose) & ACTIVE, 0);
        key.erasing.insert(root);
        let values = hierarchy.update(&mut cache, &key, &Default::default());
        assert_ne!(flags(&values, face) & ERASING, 0);
        assert_eq!(flags(&values, loose) & ERASING, 0);
        key.erasing.clear();
        let values = hierarchy.update(&mut cache, &key, &BTreeSet::from([face]));
        assert_eq!(flags(&values, face) & ERASING, 0);
        assert_ne!(flags(&values, face) & REMOVED, 0);
        let values = hierarchy.update(&mut cache, &key, &Default::default());
        assert_eq!(flags(&values, face) & (ERASING | REMOVED), 0);
    }

    #[test]
    fn visibility_face_ids_address_interaction_states_even_with_hidden_faces() {
        let mut model = model(1024);
        model.hidden.insert(*model.faces.keys().next().unwrap());
        let mut faces = FaceCache::default();
        let mut entities = EntityCache::default();
        faces.update(&model);
        entities.update(&model);
        let ctx = egui::Context::default();
        let _ = ctx.run_test_ui(egui::RawInput::default(), |ui| {
            let geometry = Geometry::new(&key(), &model, &faces, &entities, ui);
            assert!(!geometry.faces.is_empty());
            assert!(
                geometry
                    .faces
                    .iter()
                    .all(|face| face.face_id == face.interaction[1])
            );
        });
    }

    #[test]
    fn deletion_masks_reject_moves_material_changes_and_reparenting() {
        use std::collections::BTreeSet;
        let mut source = model(1024);
        let group = source
            .make_object(&source.selectable_entities(), false, "Assembly")
            .unwrap();
        source.edit_context = Some(group);
        let edge = *source.edges.keys().next().unwrap();
        let mut erased = source.clone();
        erased.erase(&BTreeSet::from([edge]));
        let removed = removed_geometry(&source, &erased).unwrap();
        assert!(removed.contains(&edge));
        assert!(removed.len() >= 2);
        assert!(removed_geometry(&source, &source).unwrap().is_empty());
        let mut changed = erased.clone();
        changed.vertices.values_mut().next().unwrap().position.z += 0.1;
        assert!(removed_geometry(&source, &changed).is_none());
        changed = erased;
        changed
            .face_colors
            .insert(*changed.faces.keys().next().unwrap(), [12, 34, 56]);
        assert!(removed_geometry(&source, &changed).is_none());
        changed = source.clone();
        changed
            .objects
            .get_mut(&group)
            .unwrap()
            .entities
            .remove(&edge);
        assert!(removed_geometry(&source, &changed).is_none());
    }

    #[test]
    fn shadow_projection_and_cache_invalidation() {
        let mut scene = key();
        scene.display.shadows = true;
        assert_eq!(shadows::validate(&scene, (DVec3::splat(-100.), DVec3::splat(100.))).unwrap(), 27);
    }

    #[test]
    fn unsupported_visibility_modes_keep_the_precision_renderer() {
        let model = model(1024);
        let mut faces = FaceCache::default();
        faces.update(&model);
        let key = key();
        assert!(supported(&key, &model, &faces));
        for modify in [
            (|k: &mut SceneKey| k.display.x_ray = true) as fn(&mut SceneKey),
            |k| k.display.section_cuts = true,
            |k| k.display.hidden_geometry = true,
            |k| k.display.back_edges = true,
            |k| k.display.profiles = true,
            |k| k.display.grain_direction = true,
            |k| k.camera.projection = benchup_core::Projection::Parallel,
        ] {
            let mut changed = key.clone();
            modify(&mut changed);
            assert!(!supported(&changed, &model, &faces));
        }
        faces.max_edge_faces = 5;
        assert!(!supported(&key, &model, &faces));
    }

    #[test]
    fn depth_range_preserves_projection_and_does_not_clip_scene_bounds() {
        let size = DVec2::new(1400., 900.);
        let camera = Camera::default();
        let bounds = (DVec3::splat(-2.), DVec3::splat(2.));
        let (near, far) = clip_planes(&camera, bounds);
        assert!(near > 0.005);
        let view = camera.view_with_clip_planes(size, near, far);
        for x in [-2., 2.] {
            for y in [-2., 2.] {
                for z in [-2., 2.] {
                    let p = DVec3::new(x, y, z);
                    let projected = view.project_depth(p).unwrap();
                    assert!((0. ..=1.).contains(&projected.z));
                    assert!(
                        projected
                            .truncate()
                            .distance(camera.project(p, size).unwrap())
                            < 1e-8
                    );
                }
            }
        }
        let inside = (camera.eye() - DVec3::ONE, camera.eye() + DVec3::ONE);
        assert_eq!(clip_planes(&camera, inside).0, 0.005);
    }
}
