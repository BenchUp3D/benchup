//! Retain world geometry, project and shade on the GPU when ordinary opaque
//! geometry needs no CPU clipping. The precision/annotation renderer remains
//! available for every other display mode and transient editing preview.
use super::*;

use crate::theme::{FacePalette, Palette};
#[path = "resident_appearance.rs"]
mod appearance;

// Match fragment_visibility: face ID followed by the unmodified f32 depth bits.
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
struct Uniform {
    matrix: [[f32; 4]; 4],
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
}
#[derive(Default)]
pub(super) struct Cache {
    key: Option<SceneKey>,
    geometry: Option<Arc<Geometry>>,
    builds: usize,
    appearance: appearance::Cache,
}
struct Geometry {
    faces: Vec<FaceVertex>,
    edges: Vec<EdgeInstance>,
    hierarchy: appearance::Hierarchy,
    origin: DVec3,
    bounds: (DVec3, DVec3),
    gpu: OnceLock<GeometryBuffers>,
}
struct GeometryBuffers {
    faces: wgpu::Buffer,
    edges: wgpu::Buffer,
    visibility: Mutex<Option<Arc<Visibility>>>,
    states: appearance::StateTexture,
}
struct Visibility {
    size: [u32; 2],
    depth: wgpu::TextureView,
    faces: wgpu::TextureView,
}
struct DrawBuffers {
    _uniform: wgpu::Buffer,
    bind: wgpu::BindGroup,
    visibility_bind: wgpu::BindGroup,
    visibility: Arc<Visibility>,
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
}
struct Draw {
    geometry: Arc<Geometry>,
    uniform: Uniform,
    appearance: Arc<Vec<u32>>,
    references: Vec<Vertex>,
    reference_indices: Vec<u32>,
    gpu: OnceLock<DrawBuffers>,
}
struct Pipelines {
    face: wgpu::RenderPipeline,
    visibility: wgpu::RenderPipeline,
    edge_depth: wgpu::RenderPipeline,
    edge: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    visibility_layout: wgpu::BindGroupLayout,
    srgb: bool,
}

fn geometry_key(key: &SceneKey) -> SceneKey {
    let mut key = key.clone();
    key.camera = Camera::default();
    key.size = DVec2::ZERO;
    key.pixels_per_point = 1.;
    key.selected.clear();
    key.hover = None;
    key.outliner_hover.clear();
    key.edit_context = None;
    key.tool = ketchup_core::Tool::Orbit;
    key.display.group_emphasis = false;
    key.display.edges = true;
    key
}

pub(crate) fn supported(key: &SceneKey, model: &Model, faces: &FaceCache) -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        thread_local! {
            static LEGACY: bool = web_sys::window().and_then(|window|
                js_sys::Reflect::get(&window, &"ketchupViewportBenchmarkLegacy".into()).ok())
                .and_then(|value| value.as_bool()).unwrap_or(false);
        }
        if LEGACY.with(|value| *value) {
            return false;
        }
    }
    let d = &key.display;
    model.faces.len() >= 1024
        && faces.max_edge_faces <= 4
        && key.camera.projection == ketchup_core::Projection::Perspective
        && matches!(
            d.face_style,
            ketchup_core::FaceStyle::Materials | ketchup_core::FaceStyle::Shaded
        )
        && key.joinery_members.is_none()
        && key.joint_preview.is_none()
        && !(d.shadows
            || d.x_ray
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
            ketchup_core::Tool::Move | ketchup_core::Tool::Erase | ketchup_core::Tool::Paint
        )
}

impl SceneCache {
    pub(crate) fn resident_context_bounds(&self, context: u64) -> Option<(DVec3, DVec3)> {
        self.resident
            .geometry
            .as_ref()?
            .hierarchy
            .bounds
            .get(&context)
            .copied()
    }
    #[cfg(any(test, target_arch = "wasm32"))]
    pub(crate) fn resident_builds(&self) -> usize {
        self.resident.builds
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
        let geometry_key = geometry_key(key);
        if self.resident.key.as_ref() != Some(&geometry_key) {
            self.resident.geometry = Some(Arc::new(Geometry::new(key, model, faces, entities, ui)));
            self.resident.key = Some(geometry_key);
            self.resident.builds += 1;
            self.resident.appearance = appearance::Cache::default();
        }
        let geometry = Arc::clone(self.resident.geometry.as_ref().unwrap());
        let (near, far) = clip_planes(&key.camera, geometry.bounds);
        let matrix = key.camera.matrix_with_clip_planes(key.size, near, far)
            * glam::DMat4::from_translation(geometry.origin);
        let eye = key.camera.eye();
        let toward_eye = (eye - key.camera.target).normalize();
        let light =
            (toward_eye * 0.8 - key.camera.right() * 0.45 + key.camera.up() * 0.65).normalize();
        let appearance = geometry
            .hierarchy
            .update(&mut self.resident.appearance, key);
        let palette = Palette::for_ui(ui);
        let face_palette = FacePalette::for_ui(ui);
        let rgba = |color: Color32| color.to_array().map(|v| v as f32 / 255.);
        let callback = Draw {
            appearance,
            geometry: Arc::clone(&geometry),
            uniform: Uniform {
                matrix: matrix.as_mat4().to_cols_array_2d(),
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
                    geometry.hierarchy.slot(key.hover.unwrap_or(0)),
                    u32::from(key.edit_context.is_some() && key.display.group_emphasis),
                    u32::from(key.display.edges),
                    0,
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
                flags: [
                    0.,
                    if key.camera.projection == ketchup_core::Projection::Parallel {
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
    let far = if camera.projection == ketchup_core::Projection::Parallel {
        (depth + radius + camera.distance * 2.).max(near + 1.)
    } else {
        1e8_f64.max((depth + radius) * 2.).max(near * 2.)
    };
    (near, far)
}

impl Geometry {
    fn new(
        key: &SceneKey,
        model: &Model,
        faces: &FaceCache,
        entities: &EntityCache,
        ui: &egui::Ui,
    ) -> Self {
        let palette = Palette::for_ui(ui);
        let face_palette = FacePalette::for_ui(ui);
        let bounds = model.vertices.values().map(|v| v.position).fold(
            (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY)),
            |(lo, hi), p| (lo.min(p), hi.max(p)),
        );
        let origin = (bounds.0 + bounds.1) * 0.5;
        let hierarchy = appearance::Hierarchy::new(model, entities, faces);
        let point = |p: DVec3| (p - origin).as_vec3().to_array();
        let mut vertices = Vec::new();
        let mut surfaces: BTreeMap<(u64, u64), Vec<(DVec3, DVec3, u32)>> = BTreeMap::new();
        let mut face_id = 0;
        for face in model
            .faces
            .values()
            .filter(|f| entities.get(f.id).displayed(&key.display))
        {
            face_id += 1;
            let entity = entities.get(face.id);
            let cached = faces.get(face.id);
            let material = if key.display.color_by_tag {
                entity.tag_color.as_ref()
            } else if key.display.face_style == ketchup_core::FaceStyle::Materials {
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
        for edge in model
            .edges
            .values()
            .filter(|e| entities.get(e.id).displayed(&key.display))
        {
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
        Self {
            hierarchy,
            faces: vertices,
            edges,
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
        label: Some("KetchUp resident world geometry"),
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
        label: Some("Resident centerline visibility"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
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
            layout: Some(if edge { &edge_layout } else { &pipeline_layout }),
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
                    "fragment_main"
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
        let geometry = self.geometry.gpu.get_or_init(|| GeometryBuffers {
            visibility: Mutex::new(None),
            states: appearance::StateTexture::new(device, self.appearance.len()),
            faces: upload(
                device,
                queue,
                &wgpu::util::BufferInitDescriptor {
                    label: Some("Resident face vertices"),
                    contents: bytemuck::cast_slice(&self.geometry.faces),
                    usage: wgpu::BufferUsages::VERTEX,
                },
            ),
            edges: upload(
                device,
                queue,
                &wgpu::util::BufferInitDescriptor {
                    label: Some("Resident edge instances"),
                    contents: bytemuck::cast_slice(&self.geometry.edges),
                    usage: wgpu::BufferUsages::VERTEX,
                },
            ),
        });
        geometry.states.update(queue, &self.appearance);
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
        let draw = self.gpu.get_or_init(|| {
            let mut uniform = self.uniform;
            uniform.flags[0] = if pipelines.srgb { 1. } else { 0. };
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
                        resource: wgpu::BindingResource::TextureView(&geometry.states.view),
                    },
                ],
            });
            let visibility_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Resident centerline visibility"),
                layout: &pipelines.visibility_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&visibility.faces),
                }],
            });
            DrawBuffers {
                visibility_bind,
                visibility: Arc::clone(&visibility),
                _uniform: buffer,
                bind,
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
        if !self.geometry.edges.is_empty() {
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
            pass.set_bind_group(0, &draw.bind, &[]);
            pass.set_vertex_buffer(0, geometry.faces.slice(..));
            pass.draw(0..self.geometry.faces.len() as u32, 0..1);
        }
        Vec::new()
    }
    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        let (Some(pipelines), Some(reference_pipelines), Some(geometry), Some(draw)) = (
            resources.get::<Pipelines>(),
            resources.get::<Resources>(),
            self.geometry.gpu.get(),
            self.gpu.get(),
        ) else {
            return;
        };
        pass.set_pipeline(&pipelines.face);
        pass.set_bind_group(0, &draw.bind, &[]);
        pass.set_vertex_buffer(0, geometry.faces.slice(..));
        pass.draw(0..self.geometry.faces.len() as u32, 0..1);
        for (pipeline, reference_pipeline) in [
            (
                &pipelines.edge_depth,
                &reference_pipelines.edge_depth_pipeline,
            ),
            (&pipelines.edge, &reference_pipelines.edge_pipeline),
        ] {
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &draw.bind, &[]);
            pass.set_bind_group(1, &draw.visibility_bind, &[]);
            pass.set_vertex_buffer(0, geometry.edges.slice(..));
            pass.draw(0..12, 0..self.geometry.edges.len() as u32);
            if !self.reference_indices.is_empty() {
                pass.set_pipeline(reference_pipeline);
                pass.set_vertex_buffer(0, draw.vertices.slice(..));
                pass.set_index_buffer(draw.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..self.reference_indices.len() as u32, 0, 0..1);
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
            .map(|geometry| geometry.bounds)
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
    use ketchup_core::{Edge, Vertex as ModelVertex};

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
            model.next_id = id;
        }
        model
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
            tool: ketchup_core::Tool::Orbit,
            selected: Default::default(),
            hover: None,
            outliner_hover: Default::default(),
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
            let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
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
        let hierarchy = &geometry.hierarchy;
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
        assert_eq!(cache.resident.builds, 2);
        model.vertices.get_mut(&1).unwrap().position.z = 0.01;
        render(&model, &mut key, &mut cache);
        assert_eq!(cache.resident.builds, 3);
        let previous_origin = cache.resident.geometry.as_ref().unwrap().origin;
        for vertex in model.vertices.values_mut() {
            vertex.position.x += 1000.;
        }
        render(&model, &mut key, &mut cache);
        assert_eq!(cache.resident.builds, 4);
        assert!(
            (cache.resident.geometry.as_ref().unwrap().origin.x - previous_origin.x - 1000.).abs()
                < 1e-9
        );
    }

    #[test]
    fn appearance_inherits_selection_hover_locks_and_nested_scopes() {
        use appearance::{ACTIVE, HOVERED, LOCKED, OUTLINER, SELECTED};
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
        let values = hierarchy.update(&mut cache, &key);
        assert_eq!(
            flags(&values, face) & (SELECTED | LOCKED | ACTIVE),
            SELECTED | LOCKED | ACTIVE
        );
        assert_eq!(flags(&values, loose) & SELECTED, 0);
        assert!(Arc::ptr_eq(&values, &hierarchy.update(&mut cache, &key)));
        key.selected.clear();
        key.edit_context = Some(root);
        key.hover = Some(child);
        let values = hierarchy.update(&mut cache, &key);
        assert_eq!(flags(&values, face) & SELECTED, 0);
        assert_ne!(flags(&values, face) & HOVERED, 0);
        assert_eq!(flags(&values, loose) & ACTIVE, 0);
        key.hover = None;
        key.edit_context = Some(child);
        key.outliner_hover.insert(root);
        let values = hierarchy.update(&mut cache, &key);
        assert_eq!(
            flags(&values, face) & (ACTIVE | OUTLINER),
            ACTIVE | OUTLINER
        );
        assert_eq!(flags(&values, face) & HOVERED, 0);
        key.edit_context = None;
        key.outliner_hover.clear();
        let values = hierarchy.update(&mut cache, &key);
        assert_eq!(flags(&values, face) & (SELECTED | HOVERED | OUTLINER), 0);
        assert_ne!(flags(&values, loose) & ACTIVE, 0);
    }

    #[test]
    fn unsupported_visibility_modes_keep_the_precision_renderer() {
        let model = model(1024);
        let mut faces = FaceCache::default();
        faces.update(&model);
        let key = key();
        assert!(supported(&key, &model, &faces));
        for modify in [
            (|k: &mut SceneKey| k.display.shadows = true) as fn(&mut SceneKey),
            |k| k.display.x_ray = true,
            |k| k.display.section_cuts = true,
            |k| k.display.hidden_geometry = true,
            |k| k.display.back_edges = true,
            |k| k.display.profiles = true,
            |k| k.display.grain_direction = true,
            |k| k.camera.projection = ketchup_core::Projection::Parallel,
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
