//! Sunlight is independent of the viewport camera and editing scope. Retain a
//! light-space depth map until geometry, visibility or the sun direction changes.
//! Integer color stores depth bits for WebGL2, where depth textureLoad is absent.
use super::*;

#[derive(Clone, Copy, PartialEq)]
struct Key {
    revision: u64,
    angle: f64,
    elevation: f64,
}
pub(super) struct Map {
    key: Key,
    pub matrix: glam::DMat4,
    pub gpu: Arc<OnceLock<Target>>,
    pub prepared: OnceLock<()>,
}
pub(super) struct Target {
    pub faces: wgpu::TextureView,
    pub depth: wgpu::TextureView,
}
impl Map {
    pub fn for_scene(
        cache: &mut Option<Arc<Self>>,
        scene: &SceneKey,
        bounds: (DVec3, DVec3),
        origin: DVec3,
    ) -> Option<Arc<Self>> {
        if !scene.display.shadows || scene.display.shadow_darkness <= 0. {
            return None;
        }
        let key = Key {
            revision: scene.model_revision,
            angle: scene.display.sun_angle,
            elevation: scene.display.sun_elevation,
        };
        if cache.as_ref().is_none_or(|map| map.key != key) {
            // Reuse the atlas allocation while editing or dragging the sun.
            // Each generation still schedules a fresh depth pass before paint.
            let gpu = cache
                .as_ref()
                .map_or_else(|| Arc::new(OnceLock::new()), |map| map.gpu.clone());
            *cache = Some(Arc::new(Self {
                key,
                matrix: light_matrix(bounds, origin, key.angle, key.elevation),
                gpu,
                prepared: OnceLock::new(),
            }));
        }
        cache.clone()
    }
}

fn light_matrix(bounds: (DVec3, DVec3), origin: DVec3, angle: f64, elevation: f64) -> glam::DMat4 {
    let (angle, elevation) = (angle.to_radians(), elevation.to_radians());
    let light = DVec3::new(
        angle.cos() * elevation.cos(),
        angle.sin() * elevation.cos(),
        elevation.sin(),
    );
    let u = DVec3::new(-angle.sin(), angle.cos(), 0.);
    let v = light.cross(u);
    // Depth increases away from the sun; WGPU depth is 0..1, XY is -1..1.
    let basis = glam::DMat4::from_cols(
        u.extend(0.),
        v.extend(0.),
        (-light).extend(0.),
        glam::DVec4::W,
    )
    .transpose();
    let mut lo = DVec3::splat(f64::INFINITY);
    let mut hi = DVec3::splat(f64::NEG_INFINITY);
    for x in [bounds.0.x, bounds.1.x] {
        for y in [bounds.0.y, bounds.1.y] {
            for z in [bounds.0.z, bounds.1.z] {
                let p = basis.transform_point3(DVec3::new(x, y, z) - origin);
                lo = lo.min(p);
                hi = hi.max(p);
            }
        }
    }
    let padding = ((hi - lo) * 0.002).max(DVec3::splat(0.001));
    lo -= padding;
    hi += padding;
    let scale = DVec3::new(2., 2., 1.) / (hi - lo);
    glam::DMat4::from_translation(DVec3::new(-1., -1., 0.))
        * glam::DMat4::from_scale(scale)
        * glam::DMat4::from_translation(-lo)
        * basis
}

pub(super) fn target(device: &wgpu::Device, size: u32) -> Target {
    let texture = |format, usage| {
        device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("Cached resident shadow map"),
                size: wgpu::Extent3d {
                    width: size,
                    height: size,
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
    Target {
        faces: texture(
            VISIBILITY_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        ),
        depth: texture(
            wgpu::TextureFormat::Depth32Float,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        ),
    }
}

pub(super) fn pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Resident sunlight depth"), layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader, entry_point: Some("shadow_vertex"),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<FaceVertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x3,2=>Float32x3,3=>Unorm8x4,4=>Uint32,5=>Uint32x2],
            })], compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: shader, entry_point: Some("fragment_visibility"),
            targets: &[Some(wgpu::ColorTargetState { format: VISIBILITY_FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL })],
            compilation_options: Default::default(),
        }),
        primitive: Default::default(),
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float, depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::LessEqual), stencil: Default::default(), bias: Default::default(),
        }),
        multisample: Default::default(), multiview_mask: None, cache: None,
    })
}

#[cfg(any(test, target_arch = "wasm32"))]
pub(super) fn validate(scene: &SceneKey, bounds: (DVec3, DVec3)) -> Result<usize, String> {
    let origin = (bounds.0 + bounds.1) * 0.5;
    let mut checks = 0;
    let mut cache = None;
    let mut key = scene.clone();
    key.display.shadows = true;
    key.display.shadow_darkness = 0.35;
    let first = Map::for_scene(&mut cache, &key, bounds, origin).unwrap();
    key.camera.yaw += 0.5;
    key.edit_context = Some(123);
    key.selected.insert(42);
    key.hover = Some(43);
    key.display.group_emphasis = !key.display.group_emphasis;
    key.display.shadow_darkness = 0.7;
    key.display.shadow_antialiasing = !key.display.shadow_antialiasing;
    let next = Map::for_scene(&mut cache, &key, bounds, origin).unwrap();
    if !Arc::ptr_eq(&first, &next) || geometry_key(scene) != geometry_key(&key) {
        return Err(
            "Camera, selection, scope and shadow intensity must reuse geometry and sunlight".into(),
        );
    }
    checks += 1;
    key.display.shadows = false;
    if Map::for_scene(&mut cache, &key, bounds, origin).is_some() {
        return Err("Disabled shadows must skip the sunlight pass".into());
    }
    key.display.shadows = true;
    if !Arc::ptr_eq(
        &first,
        &Map::for_scene(&mut cache, &key, bounds, origin).unwrap(),
    ) {
        return Err("Toggling unchanged shadows must reuse the cached map".into());
    }
    checks += 1;
    key.model_revision += 1;
    let edited = Map::for_scene(&mut cache, &key, bounds, origin).unwrap();
    if Arc::ptr_eq(&first, &edited)
        || !Arc::ptr_eq(&first.gpu, &edited.gpu)
        || edited.prepared.get().is_some()
    {
        return Err(
            "Geometry edits must refresh sunlight while reusing its atlas allocation".into(),
        );
    }
    checks += 1;
    for angle in [0_f64, 80., 135., 270.] {
        for elevation in [5_f64, 40., 85.] {
            key.display.sun_angle = angle;
            key.display.sun_elevation = elevation;
            let map = Map::for_scene(&mut cache, &key, bounds, origin).unwrap();
            let light = DVec3::new(
                angle.to_radians().cos() * elevation.to_radians().cos(),
                angle.to_radians().sin() * elevation.to_radians().cos(),
                elevation.to_radians().sin(),
            );
            for x in [bounds.0.x, bounds.1.x] {
                for y in [bounds.0.y, bounds.1.y] {
                    for z in [bounds.0.z, bounds.1.z] {
                        let p = map.matrix.transform_point3(DVec3::new(x, y, z) - origin);
                        if p.x.abs() > 1. || p.y.abs() > 1. || !(0. ..=1.).contains(&p.z) {
                            return Err("Sunlight projection clipped a scene corner".into());
                        }
                    }
                }
            }
            let a = map.matrix.transform_point3(DVec3::ZERO);
            let b = map.matrix.transform_point3(light);
            if a.truncate().distance(b.truncate()) > 1e-9 || b.z >= a.z {
                return Err("Sunlight ray changed XY or reversed caster depth".into());
            }
            checks += 2;
        }
    }
    Ok(checks)
}
