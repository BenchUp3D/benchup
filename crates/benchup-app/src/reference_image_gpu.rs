//! Retained GPU image textures/quads; camera projection, UV transforms and
//! clipping stay on the GPU on both WebGPU and the WebGL fallback.
use crate::renderer::EntityCache;
use benchup_core::{DVec3, DisplaySettings, ImageSettings, Model};
use eframe::{egui, egui_wgpu, wgpu};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, OnceLock},
};
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    position: [f32; 3],
    uv: [f32; 2],
}
#[repr(C)]
#[derive(Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniform {
    matrix: [[f32; 4]; 4],
    size: [f32; 4],
    placement: [f32; 4],
    rotation: [f32; 4],
    appearance: [f32; 4],
    flags: [f32; 4],
    depth: [f32; 4],
    section: [f32; 4],
}
struct Texture {
    _bytes: Arc<[u8]>,
    gpu: OnceLock<Option<wgpu::TextureView>>,
}
struct Quad {
    relative: [DVec3; 4],
    gpu: OnceLock<wgpu::Buffer>,
}
#[derive(Default)]
struct Entries {
    textures: BTreeMap<usize, Arc<Texture>>,
    quads: BTreeMap<u64, Arc<Quad>>,
    draws: BTreeMap<(u64, [u32; 4]), Arc<Draw>>,
}
#[derive(Default)]
pub(crate) struct Cache(Mutex<Entries>);
struct Resources {
    pipeline: wgpu::RenderPipeline,
    linear: wgpu::Sampler,
    nearest: wgpu::Sampler,
    srgb: bool,
}
pub(crate) fn install(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    renderer: &mut egui_wgpu::Renderer,
) {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Reference images"),
        source: wgpu::ShaderSource::Wgsl(include_str!("reference_image_gpu.wgsl").into()),
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Reference images"),
        layout: None,
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vertex_main"),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<Vertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x2],
            })],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fragment_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: Default::default(),
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: Default::default(),
        multiview_mask: None,
        cache: None,
    });
    let sampler = |filter| {
        device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: filter,
            min_filter: filter,
            ..Default::default()
        })
    };
    renderer.callback_resources.insert(Resources {
        pipeline,
        linear: sampler(wgpu::FilterMode::Linear),
        nearest: sampler(wgpu::FilterMode::Nearest),
        srgb: format.is_srgb(),
    });
}

pub(crate) fn paint(
    painter: &egui::Painter,
    rect: egui::Rect,
    model: &Model,
    entities: &EntityCache,
    projection: (glam::DMat4, [f32; 4]),
    display: &DisplaySettings,
    cache: &Cache,
    preview: Option<(u64, &ImageSettings)>,
) {
    let (matrix, depth) = projection;
    let Ok(mut entries) = cache.0.lock() else {
        return;
    };
    // Dropping a project also releases its GPU assets; copies share one texture.
    entries
        .quads
        .retain(|id, _| model.reference_images.contains_key(id));
    let assets: std::collections::BTreeSet<_> = model
        .reference_images
        .values()
        .filter_map(|i| model.image_assets.get(&i.asset))
        .map(|i| i.png.as_ptr() as usize)
        .collect();
    entries.textures.retain(|key, _| assets.contains(key));
    entries
        .draws
        .retain(|(id, _), _| model.reference_images.contains_key(id));
    if entries.draws.len() > model.reference_images.len().saturating_mul(16) {
        entries.draws.clear();
    }
    let viewport = [
        rect.min.x.to_bits(),
        rect.min.y.to_bits(),
        rect.width().to_bits(),
        rect.height().to_bits(),
    ];
    let mut pictures = Vec::new();
    for (&id, image) in &model.reference_images {
        if !entities.get(id).displayed(display) {
            continue;
        }
        let Some(points) = image.points(model) else {
            continue;
        };
        let Some(asset) = model.image_assets.get(&image.asset) else {
            continue;
        };
        let origin = points[0];
        let relative = points.map(|p| p - origin);
        let quad = entries.quads.entry(id).or_insert_with(|| {
            Arc::new(Quad {
                relative,
                gpu: OnceLock::new(),
            })
        });
        if quad.relative != relative {
            *quad = Arc::new(Quad {
                relative,
                gpu: OnceLock::new(),
            });
        }
        let quad = Arc::clone(quad);
        let texture = Arc::clone(
            entries
                .textures
                .entry(asset.png.as_ptr() as usize)
                .or_insert_with(|| {
                    Arc::new(Texture {
                        _bytes: Arc::clone(&asset.png),
                        gpu: OnceLock::new(),
                    })
                }),
        );
        let frame = [relative[1].length(), relative[3].length()];
        let s = preview
            .filter(|(preview_id, _)| *preview_id == id)
            .map_or(&image.settings, |(_, settings)| settings);
        let size = s.fitted_size(frame, asset.pixels);
        let rotation = s.rotation.to_radians();
        let uniform = Uniform {
            matrix: (matrix * glam::DMat4::from_translation(origin))
                .as_mat4()
                .to_cols_array_2d(),
            size: [
                size[0] as f32,
                size[1] as f32,
                frame[0] as f32,
                frame[1] as f32,
            ],
            placement: [
                s.anchor[0] as f32,
                s.anchor[1] as f32,
                s.pan[0] as f32,
                s.pan[1] as f32,
            ],
            rotation: [
                rotation.cos() as f32,
                rotation.sin() as f32,
                if s.flip_horizontal { -1. } else { 1. },
                if s.flip_vertical { -1. } else { 1. },
            ],
            appearance: [
                s.opacity as f32,
                s.brightness as f32,
                s.contrast as f32,
                u32::from(s.grayscale) as f32,
            ],
            flags: [u32::from(s.double_sided) as f32, 0., 0., 0.],
            depth,
            section: {
                let limits = display.section_limits();
                [
                    limits[0].map_or(1e30, |v| (v - origin.x) as f32),
                    limits[1].map_or(1e30, |v| (v - origin.y) as f32),
                    limits[2].map_or(1e30, |v| (v - origin.z) as f32),
                    0.,
                ]
            },
        };
        let center = points.iter().sum::<DVec3>() * 0.25;
        let clip = matrix * center.extend(1.);
        let key = (id, viewport);
        let draw = entries.draws.entry(key).or_insert_with(|| {
            Arc::new(Draw {
                texture: Arc::clone(&texture),
                quad: Arc::clone(&quad),
                uniform,
                smoothing: s.smoothing,
                gpu: Default::default(),
            })
        });
        if draw.uniform != uniform
            || draw.smoothing != s.smoothing
            || !Arc::ptr_eq(&draw.texture, &texture)
            || !Arc::ptr_eq(&draw.quad, &quad)
        {
            // Camera and appearance changes only upload a small uniform. Keep
            // the buffer and bind group unless their bound resources changed.
            let gpu = if draw.smoothing == s.smoothing
                && Arc::ptr_eq(&draw.texture, &texture)
                && Arc::ptr_eq(&draw.quad, &quad)
            {
                Arc::clone(&draw.gpu)
            } else {
                Default::default()
            };
            *draw = Arc::new(Draw {
                texture,
                quad,
                uniform,
                smoothing: s.smoothing,
                gpu,
            });
        }
        pictures.push((clip.z / clip.w, Arc::clone(draw)));
    }
    // Only one scalar per reference, never project scene geometry on the CPU.
    pictures.sort_by(|a, b| b.0.total_cmp(&a.0));
    for (_, picture) in pictures {
        painter.add(egui_wgpu::Callback::new_paint_callback(
            rect,
            SharedDraw(picture),
        ));
    }
}
struct Prepared {
    bind: wgpu::BindGroup,
    uniform: wgpu::Buffer,
    uploaded: Mutex<Uniform>,
}
struct Draw {
    texture: Arc<Texture>,
    quad: Arc<Quad>,
    uniform: Uniform,
    smoothing: bool,
    gpu: Arc<OnceLock<Prepared>>,
}
struct SharedDraw(Arc<Draw>);
impl egui_wgpu::CallbackTrait for SharedDraw {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen: &egui_wgpu::ScreenDescriptor,
        encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        self.0.prepare(device, queue, screen, encoder, resources)
    }
    fn paint(
        &self,
        info: egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        self.0.paint(info, pass, resources);
    }
}
impl egui_wgpu::CallbackTrait for Draw {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen: &egui_wgpu::ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let Some(resources) = resources.get::<Resources>() else {
            return vec![];
        };
        let texture = self.texture.gpu.get_or_init(|| {
            let mut reader = image::ImageReader::with_format(
                std::io::Cursor::new(&self.texture._bytes),
                image::ImageFormat::Png,
            );
            let mut limits = image::Limits::default();
            limits.max_image_width = Some(4096);
            limits.max_image_height = Some(4096);
            limits.max_alloc = Some(128 * 1024 * 1024);
            reader.limits(limits);
            let image = reader.decode().ok()?;
            let max = device.limits().max_texture_dimension_2d.min(4096);
            let image = if image.width() > max || image.height() > max {
                image.resize(max, max, image::imageops::FilterType::Triangle)
            } else {
                image
            }
            .to_rgba8();
            let texture = device.create_texture_with_data(
                queue,
                &wgpu::TextureDescriptor {
                    label: Some("Reference image"),
                    size: wgpu::Extent3d {
                        width: image.width(),
                        height: image.height(),
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                wgpu::util::TextureDataOrder::LayerMajor,
                &image,
            );
            Some(texture.create_view(&Default::default()))
        });
        let Some(texture) = texture else {
            return vec![];
        };
        self.quad.gpu.get_or_init(|| {
            let uv = [[0., 1.], [1., 1.], [1., 0.], [0., 0.]];
            let vertices = [0, 1, 2, 0, 2, 3].map(|i| Vertex {
                position: self.quad.relative[i].as_vec3().to_array(),
                uv: uv[i],
            });
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Reference image quad"),
                contents: bytemuck::cast_slice(&vertices),
                usage: wgpu::BufferUsages::VERTEX,
            })
        });
        let mut uniform = self.uniform;
        uniform.flags[1] = u32::from(resources.srgb) as f32;
        let prepared = self.gpu.get_or_init(|| {
            let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Reference image settings"),
                contents: bytemuck::bytes_of(&uniform),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Reference image"),
                layout: &resources.pipeline.get_bind_group_layout(0),
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(texture),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(if self.smoothing {
                            &resources.linear
                        } else {
                            &resources.nearest
                        }),
                    },
                ],
            });
            Prepared {
                bind,
                uniform: buffer,
                uploaded: Mutex::new(uniform),
            }
        });
        if let Ok(mut uploaded) = prepared.uploaded.lock()
            && *uploaded != uniform
        {
            queue.write_buffer(&prepared.uniform, 0, bytemuck::bytes_of(&uniform));
            *uploaded = uniform;
        }
        vec![]
    }
    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        let (Some(resources), Some(prepared), Some(quad)) = (
            resources.get::<Resources>(),
            self.gpu.get(),
            self.quad.gpu.get(),
        ) else {
            return;
        };
        pass.set_pipeline(&resources.pipeline);
        pass.set_bind_group(0, &prepared.bind, &[]);
        pass.set_vertex_buffer(0, quad.slice(..));
        pass.draw(0..6, 0..1);
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    #[test]
    fn reference_shader_validates_for_gpu_backends() {
        let module =
            wgpu::naga::front::wgsl::parse_str(include_str!("reference_image_gpu.wgsl")).unwrap();
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .unwrap();
        assert_eq!(std::mem::size_of::<Uniform>(), 176);
    }
    fn image_model() -> Model {
        use benchup_core::{ImageAsset, ImageSettings, ReferenceImage};
        let mut model = Model::default();
        let mut out = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(2, 1)
            .write_to(&mut out, image::ImageFormat::Png)
            .unwrap();
        let image = ReferenceImage {
            name: "Plan".into(),
            asset: benchup_core::reference_images::ImageAssetId::new_v4(),
            vertices: [0; 4],
            settings: ImageSettings::default(),
            placement_offset: DVec3::ZERO,
        };
        model
            .add_reference_image(
                [DVec3::ZERO, DVec3::X * 2., DVec3::new(2., 1., 0.), DVec3::Y],
                image,
                ImageAsset {
                    png: out.into_inner().into(),
                    pixels: [2, 1],
                },
            )
            .unwrap();
        model
    }
    #[test]
    fn drawing_again_reuses_quad_texture_and_settings_across_frames() {
        let mut model = image_model();
        let mut entities = EntityCache::default();
        entities.update(&model);
        let ctx = egui::Context::default();
        let painter = ctx.layer_painter(egui::LayerId::background());
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800., 600.));
        let matrix = benchup_core::Camera::default().matrix(benchup_core::DVec2::new(800., 600.));
        let cache = Cache::default();
        paint(
            &painter,
            rect,
            &model,
            &entities,
            (matrix, [0.; 4]),
            &DisplaySettings::default(),
            &cache,
            None,
        );
        let first = Arc::clone(cache.0.lock().unwrap().draws.values().next().unwrap());
        paint(
            &painter,
            rect,
            &model,
            &entities,
            (matrix, [0.; 4]),
            &DisplaySettings::default(),
            &cache,
            None,
        );
        let second = Arc::clone(cache.0.lock().unwrap().draws.values().next().unwrap());
        assert!(Arc::ptr_eq(&first, &second));
        model
            .reference_images
            .values_mut()
            .next()
            .unwrap()
            .settings
            .zoom = 2.;
        paint(
            &painter,
            rect,
            &model,
            &entities,
            (matrix, [0.; 4]),
            &DisplaySettings::default(),
            &cache,
            None,
        );
        let changed = Arc::clone(cache.0.lock().unwrap().draws.values().next().unwrap());
        assert!(!Arc::ptr_eq(&first, &changed));
        assert!(Arc::ptr_eq(&first.quad, &changed.quad));
        assert!(Arc::ptr_eq(&first.texture, &changed.texture));
        assert!(Arc::ptr_eq(&first.gpu, &changed.gpu));
        let id = *model.reference_images.keys().next().unwrap();
        let source = model.reference_images[&id].settings.clone();
        let mut preview = source.clone();
        preview.opacity = 0.25;
        paint(
            &painter,
            rect,
            &model,
            &entities,
            (matrix, [0.; 4]),
            &DisplaySettings::default(),
            &cache,
            Some((id, &preview)),
        );
        let previewed = Arc::clone(cache.0.lock().unwrap().draws.values().next().unwrap());
        assert_eq!(previewed.uniform.appearance[0], 0.25);
        assert_eq!(model.reference_images[&id].settings, source);
        assert!(Arc::ptr_eq(&changed.gpu, &previewed.gpu));
        // Independent panes cannot overwrite one another's camera uniforms.
        let other_rect = rect.translate(egui::vec2(800., 0.));
        paint(
            &painter,
            other_rect,
            &model,
            &entities,
            (matrix, [0.; 4]),
            &DisplaySettings::default(),
            &cache,
            Some((id, &preview)),
        );
        let entries = cache.0.lock().unwrap();
        assert_eq!(entries.draws.len(), 2);
        assert!(
            entries
                .draws
                .values()
                .any(|other| !Arc::ptr_eq(&other.gpu, &previewed.gpu))
        );
        drop(entries);
        model
            .reference_images
            .get_mut(&id)
            .unwrap()
            .settings
            .smoothing = false;
        paint(
            &painter,
            rect,
            &model,
            &entities,
            (matrix, [0.; 4]),
            &DisplaySettings::default(),
            &cache,
            None,
        );
        let replaced = Arc::clone(cache.0.lock().unwrap().draws.values().next().unwrap());
        assert!(!Arc::ptr_eq(&changed.gpu, &replaced.gpu));
    }

    #[test]
    #[ignore = "Requires a hardware GPU adapter; validates retained image uniform uploads"]
    fn image_adjustments_update_existing_gpu_bindings() {
        use egui_wgpu::CallbackTrait as _;
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
        let info = adapter.get_info();
        eprintln!("Image adjustment adapter: {info:?}");
        assert_ne!(info.device_type, wgpu::DeviceType::Cpu);
        let (device, queue) =
            pollster::block_on(adapter.request_device(&Default::default())).unwrap();
        let errors = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut renderer =
            egui_wgpu::Renderer::new(&device, wgpu::TextureFormat::Rgba8Unorm, Default::default());
        install(&device, wgpu::TextureFormat::Rgba8Unorm, &mut renderer);
        let model = image_model();
        let id = *model.reference_images.keys().next().unwrap();
        let mut entities = EntityCache::default();
        entities.update(&model);
        let ctx = egui::Context::default();
        let painter = ctx.layer_painter(egui::LayerId::background());
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800., 600.));
        let matrix = benchup_core::Camera::default().matrix(benchup_core::DVec2::new(800., 600.));
        let cache = Cache::default();
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [800, 600],
            pixels_per_point: 1.,
        };
        let mut settings = model.reference_images[&id].settings.clone();
        let mut retained = None;
        for n in 0..120 {
            settings.brightness = n as f64 / 60.;
            paint(
                &painter,
                rect,
                &model,
                &entities,
                (matrix, [0.; 4]),
                &DisplaySettings::default(),
                &cache,
                Some((id, &settings)),
            );
            let draw = Arc::clone(cache.0.lock().unwrap().draws.values().next().unwrap());
            let mut encoder = device.create_command_encoder(&Default::default());
            draw.prepare(
                &device,
                &queue,
                &screen,
                &mut encoder,
                &mut renderer.callback_resources,
            );
            queue.submit([encoder.finish()]);
            if let Some(previous) = &retained {
                assert!(Arc::ptr_eq(previous, &draw.gpu));
            }
            retained = Some(Arc::clone(&draw.gpu));
        }
        assert!(pollster::block_on(errors.pop()).is_none());
    }
}
