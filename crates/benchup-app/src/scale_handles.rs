//! A procedural GPU cage: no projected CPU meshes or per-handle UI shapes.
use super::*;
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniform {
    matrix: [[f32; 4]; 4],
    lo: [f32; 4],
    hi: [f32; 4],
    viewport: [f32; 4],
    origin: [f32; 4],
}
struct Pipeline {
    pipeline: wgpu::RenderPipeline,
}
struct Packet {
    uniform: Uniform,
    gpu: OnceLock<(wgpu::Buffer, wgpu::BindGroup)>,
}
struct Callback(Arc<Packet>);
#[derive(Default)]
pub(crate) struct Cache {
    panes: BTreeMap<[u32; 4], Arc<Packet>>,
}

pub(crate) fn paint(
    painter: &egui::Painter,
    rect: Rect,
    camera: &Camera,
    lo: DVec3,
    hi: DVec3,
    hovered: Option<usize>,
    origin: Option<DVec3>,
    cache: &mut Cache,
) {
    let center = (lo + hi) * 0.5;
    let mut camera = camera.clone();
    camera.target -= center;
    let vector = |p: DVec3| [p.x as f32, p.y as f32, p.z as f32, 0.];
    let uniform = Uniform {
        matrix: camera
            .matrix(DVec2::new(rect.width() as f64, rect.height() as f64))
            .as_mat4()
            .to_cols_array_2d(),
        lo: vector(lo - center),
        hi: vector(hi - center),
        viewport: [
            rect.width(),
            rect.height(),
            hovered.map_or(-1., |i| i as f32),
            u8::from(origin.is_some()) as f32,
        ],
        origin: vector(origin.unwrap_or(center) - center),
    };
    let key = [
        rect.min.x.to_bits(),
        rect.min.y.to_bits(),
        rect.max.x.to_bits(),
        rect.max.y.to_bits(),
    ];
    // Each pane retains its own packet. prepare() for all panes precedes paint(),
    // so sharing a mutable uniform would put the last camera in every pane.
    if cache.panes.len() > 16 {
        cache.panes.clear();
    }
    let packet = cache.panes.entry(key).or_insert_with(|| {
        Arc::new(Packet {
            uniform,
            gpu: OnceLock::new(),
        })
    });
    if packet.uniform != uniform {
        *packet = Arc::new(Packet {
            uniform,
            gpu: OnceLock::new(),
        });
    }
    painter.add(egui_wgpu::Callback::new_paint_callback(
        rect,
        Callback(Arc::clone(packet)),
    ));
}

pub(crate) fn install(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    renderer: &mut egui_wgpu::Renderer,
) {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Scale cage"),
        source: wgpu::ShaderSource::Wgsl(include_str!("scale_handles.wgsl").into()),
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Scale cage billboards"),
        layout: None,
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vertex_main"),
            buffers: &[],
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
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::Always),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: Default::default(),
        multiview_mask: None,
        cache: None,
    });
    renderer.callback_resources.insert(Pipeline { pipeline });
}
impl egui_wgpu::CallbackTrait for Callback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
        _screen: &egui_wgpu::ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if let Some(pipeline) = resources.get::<Pipeline>() {
            self.0.gpu.get_or_init(|| {
                let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Scale cage parameters"),
                    contents: bytemuck::bytes_of(&self.0.uniform),
                    usage: wgpu::BufferUsages::UNIFORM,
                });
                let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("Scale cage parameters"),
                    layout: &pipeline.pipeline.get_bind_group_layout(0),
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: buffer.as_entire_binding(),
                    }],
                });
                (buffer, bind)
            });
        }
        vec![]
    }
    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        if let (Some(pipeline), Some((_, bind))) = (resources.get::<Pipeline>(), self.0.gpu.get()) {
            pass.set_pipeline(&pipeline.pipeline);
            pass.set_bind_group(0, bind, &[]);
            // Draw cage first so its lines cannot cover the yellow handles.
            pass.draw(28 * 6..40 * 6, 0..1);
            pass.draw(0..27 * 6, 0..1);
            let hovered = self.0.uniform.viewport[2];
            if hovered >= 0. {
                let index = hovered as u32;
                pass.draw(index * 6..(index + 1) * 6, 0..1);
            }
            pass.draw(27 * 6..28 * 6, 0..1);
        }
    }
}

#[cfg(test)]
mod tests {
    use eframe::wgpu;
    #[test]
    fn procedural_scale_shader_is_valid() {
        let module =
            wgpu::naga::front::wgsl::parse_str(include_str!("scale_handles.wgsl")).unwrap();
        wgpu::naga::valid::Validator::new(Default::default(), Default::default())
            .validate(&module)
            .unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn scale_shader_translates_to_webgl2() {
        use wgpu::naga::{back::glsl, front::wgsl, valid};
        let module = wgsl::parse_str(include_str!("scale_handles.wgsl")).unwrap();
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
            .unwrap()
            .write()
            .unwrap();
        }
    }
}
