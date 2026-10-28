use std::mem::size_of;
use std::sync::Arc;

use winit::window::Window;

use snowhush::simulation::{COLUMN_WIDTH, SnowSimulation, Sprite};

const SPRITE_STRIDE: u64 = 48;
const ATTRS: [wgpu::VertexAttribute; 8] = [
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x2,
        offset: 0,
        shader_location: 0,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x2,
        offset: 8,
        shader_location: 1,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32,
        offset: 16,
        shader_location: 2,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32,
        offset: 20,
        shader_location: 3,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32,
        offset: 24,
        shader_location: 4,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32,
        offset: 28,
        shader_location: 5,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x3,
        offset: 32,
        shader_location: 6,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32,
        offset: 44,
        shader_location: 7,
    },
];

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SpriteUniforms {
    view_size: [f32; 2],
    scale: f32,
    time: f32,
    light: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct PileUniforms {
    view_size: [f32; 2],
    scale: f32,
    time: f32,
    column_width: f32,
    column_count: i32,
    pile_top: f32,
    occluder_count: i32,
    origin: [f32; 2],
    pile_bottom: f32,
    height_offset: i32,
    light: [f32; 4],
    shadow_tint: [f32; 4],
    occluders: [[f32; 4]; 16],
}

/// The light the snow is drawn in.
#[derive(Clone, Copy)]
pub struct Ambient {
    pub light: [f32; 3],
}

struct Pipelines {
    sprite: wgpu::RenderPipeline,
    pile: wgpu::RenderPipeline,
    sprite_layout: wgpu::BindGroupLayout,
    pile_layout: wgpu::BindGroupLayout,
}

/// The GPU, its device and the pipelines shared by every display's surface.
pub struct Gpu {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub adapter_name: String,
    pub format: Option<wgpu::TextureFormat>,
    pipelines: Option<Pipelines>,
}

impl Gpu {
    pub fn new() -> Result<Gpu, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN | wgpu::Backends::GL,
            ..Default::default()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: None,
            force_fallback_adapter: false,
        }))
        .ok_or("no graphics adapter could be opened")?;
        let info = adapter.get_info();
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("Snowhush"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
            },
            None,
        ))
        .map_err(|error| format!("opening the graphics device: {error}"))?;
        Ok(Gpu {
            instance,
            adapter,
            device,
            queue,
            adapter_name: format!("{} ({:?}, {:?})", info.name, info.backend, info.device_type),
            format: None,
            pipelines: None,
        })
    }

    pub fn create_surface(&self, window: Arc<Window>) -> Result<wgpu::Surface<'static>, String> {
        self.instance
            .create_surface(window)
            .map_err(|error| format!("creating a drawing surface: {error}"))
    }

    /// The first display picks the format; every other surface uses the same one so one set of pipelines serves all.
    /// `None` means this surface cannot use the already-chosen format (or has no formats at all).
    pub fn preferred_format(&self, surface: &wgpu::Surface<'_>) -> Option<wgpu::TextureFormat> {
        let caps = surface.get_capabilities(&self.adapter);
        if let Some(format) = self.format {
            return caps.formats.contains(&format).then_some(format);
        }
        caps.formats
            .iter()
            .find(|format| **format == wgpu::TextureFormat::Bgra8Unorm)
            .or_else(|| caps.formats.iter().find(|format| !format.is_srgb()))
            .copied()
    }

    pub fn preferred_alpha(&self, surface: &wgpu::Surface<'_>) -> Option<wgpu::CompositeAlphaMode> {
        let caps = surface.get_capabilities(&self.adapter);
        caps.alpha_modes
            .iter()
            .find(|mode| **mode == wgpu::CompositeAlphaMode::PreMultiplied)
            .or_else(|| {
                caps.alpha_modes
                    .iter()
                    .find(|mode| **mode == wgpu::CompositeAlphaMode::PostMultiplied)
            })
            .or_else(|| {
                caps.alpha_modes
                    .iter()
                    .find(|mode| **mode == wgpu::CompositeAlphaMode::Inherit)
            })
            .copied()
    }

    pub fn ensure_pipelines(&mut self, format: wgpu::TextureFormat) {
        if self.pipelines.is_some() {
            return;
        }
        self.format = Some(format);
        let sprite_shader = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("snow sprite"),
                source: wgpu::ShaderSource::Wgsl(include_str!("sprite.wgsl").into()),
            });
        let pile_shader = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("snow drift"),
                source: wgpu::ShaderSource::Wgsl(include_str!("pile.wgsl").into()),
            });

        let sprite_layout =
            self.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("sprite uniforms"),
                    entries: &[wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    }],
                });
        let pile_layout = self
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("drift uniforms and heights"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });

        let sprite_pipeline_layout =
            self.device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("snow sprite"),
                    bind_group_layouts: &[&sprite_layout],
                    push_constant_ranges: &[],
                });
        let pile_pipeline_layout =
            self.device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("snow drift"),
                    bind_group_layouts: &[&pile_layout],
                    push_constant_ranges: &[],
                });
        let blend = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
        };
        let target = Some(wgpu::ColorTargetState {
            format,
            blend: Some(blend),
            write_mask: wgpu::ColorWrites::ALL,
        });
        let buffers = [wgpu::VertexBufferLayout {
            array_stride: SPRITE_STRIDE,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &ATTRS,
        }];

        let sprite = self
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("snow sprite"),
                layout: Some(&sprite_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &sprite_shader,
                    entry_point: "sprite_vertex",
                    compilation_options: Default::default(),
                    buffers: &buffers,
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleStrip,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &sprite_shader,
                    entry_point: "sprite_fragment",
                    compilation_options: Default::default(),
                    targets: std::slice::from_ref(&target),
                }),
                multiview: None,
            });
        let pile = self
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("snow drift"),
                layout: Some(&pile_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &pile_shader,
                    entry_point: "pile_vertex",
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleStrip,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &pile_shader,
                    entry_point: "pile_fragment",
                    compilation_options: Default::default(),
                    targets: &[target],
                }),
                multiview: None,
            });
        self.pipelines = Some(Pipelines {
            sprite,
            pile,
            sprite_layout,
            pile_layout,
        });
    }
}

/// One display's drawing surface, with the buffers the snow is uploaded into.
pub struct Canvas {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    scale: f32,
    heights: wgpu::Buffer,
    height_data: Vec<f32>,
    height_capacity: usize,
    sprite_uniform: wgpu::Buffer,
    sprite_bind: wgpu::BindGroup,
    sprites: Vec<Sprite>,
    sprite_buffer: wgpu::Buffer,
    sprite_capacity: usize,
    piles: Vec<(wgpu::Buffer, wgpu::BindGroup)>,
}

impl Canvas {
    pub fn new(
        gpu: &Gpu,
        surface: wgpu::Surface<'static>,
        config: wgpu::SurfaceConfiguration,
        columns: usize,
        scale: f32,
    ) -> Canvas {
        let height_capacity = (columns * 2).max(4096);
        let height_bytes = (height_capacity * size_of::<f32>()) as u64;
        let heights = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("drift heights"),
            size: height_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sprite_uniform = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sprite uniforms"),
            size: size_of::<SpriteUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sprite_layout = &gpu.pipelines.as_ref().expect("pipelines").sprite_layout;
        let sprite_bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sprite uniforms"),
            layout: sprite_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: sprite_uniform.as_entire_binding(),
            }],
        });
        let sprite_capacity = 1024;
        let sprite_buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("snow sprites"),
            size: (sprite_capacity * size_of::<Sprite>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Canvas {
            surface,
            config,
            scale,
            heights,
            height_data: vec![0.0; height_capacity],
            height_capacity,
            sprite_uniform,
            sprite_bind,
            sprites: Vec::new(),
            sprite_buffer,
            sprite_capacity,
            piles: Vec::new(),
        }
    }

    pub fn configure(&self, gpu: &Gpu) {
        self.surface.configure(&gpu.device, &self.config);
    }

    pub fn draw(
        &mut self,
        gpu: &Gpu,
        simulation: &SnowSimulation,
        ambient: Ambient,
        time: f32,
    ) -> Result<(), wgpu::SurfaceError> {
        let frame = self.surface.get_current_texture()?;
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let pipelines = gpu.pipelines.as_ref().expect("pipelines");

        let light = [ambient.light[0], ambient.light[1], ambient.light[2], 1.0];
        gpu.queue.write_buffer(
            &self.sprite_uniform,
            0,
            bytemuck::bytes_of(&SpriteUniforms {
                view_size: [simulation.width(), simulation.height()],
                scale: self.scale,
                time,
                light,
            }),
        );

        let columns = simulation.column_count();
        // The simulation gains a layer per window, so the height field grows over time.
        if columns > self.height_capacity {
            self.height_capacity = (columns * 3 / 2).max(4096);
            self.height_data.resize(self.height_capacity, 0.0);
            self.heights = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("drift heights"),
                size: (self.height_capacity * size_of::<f32>()) as u64,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            for index in 0..self.piles.len() {
                let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("drift uniforms"),
                    layout: &pipelines.pile_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: self.piles[index].0.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: self.heights.as_entire_binding(),
                        },
                    ],
                });
                self.piles[index].1 = bind;
            }
        }
        let layers = simulation.write_layers(&mut self.height_data[..self.height_capacity]);
        if !layers.is_empty() {
            gpu.queue.write_buffer(
                &self.heights,
                0,
                bytemuck::cast_slice(&self.height_data[..columns]),
            );
        }
        while self.piles.len() < layers.len() {
            let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("drift uniforms"),
                size: size_of::<PileUniforms>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("drift uniforms"),
                layout: &pipelines.pile_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: self.heights.as_entire_binding(),
                    },
                ],
            });
            self.piles.push((buffer, bind));
        }

        let count = simulation.sprite_count();
        if count > 0 {
            if count > self.sprite_capacity {
                self.sprite_capacity = 1024.max(count * 3 / 2);
                self.sprite_buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("snow sprites"),
                    size: (self.sprite_capacity * size_of::<Sprite>()) as u64,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
            }
            if self.sprites.len() < count {
                self.sprites.resize(count, Sprite::default());
            }
            let written = simulation.write_sprites(&mut self.sprites[..count]);
            let bytes = sprite_bytes(&self.sprites[..written]);
            gpu.queue.write_buffer(&self.sprite_buffer, 0, bytes);
        }

        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("snow"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("snow"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            if !layers.is_empty() {
                pass.set_pipeline(&pipelines.pile);
                for (index, pile) in layers.iter().enumerate() {
                    let mut occluders = [[0.0; 4]; 16];
                    let used = pile.occluders.len().min(16);
                    occluders[..used].copy_from_slice(&pile.occluders[..used]);
                    let uniforms = PileUniforms {
                        view_size: [simulation.width(), simulation.height()],
                        scale: self.scale,
                        time,
                        column_width: COLUMN_WIDTH,
                        column_count: pile.count as i32,
                        pile_top: pile.top + 2.0,
                        occluder_count: used as i32,
                        origin: [pile.origin.x, pile.origin.y],
                        pile_bottom: pile.bottom,
                        height_offset: pile.offset as i32,
                        light,
                        shadow_tint: [0.0; 4],
                        occluders,
                    };
                    gpu.queue
                        .write_buffer(&self.piles[index].0, 0, bytemuck::bytes_of(&uniforms));
                    pass.set_bind_group(0, &self.piles[index].1, &[]);
                    pass.draw(0..4, 0..1);
                }
            }

            if count > 0 {
                pass.set_pipeline(&pipelines.sprite);
                pass.set_bind_group(0, &self.sprite_bind, &[]);
                pass.set_vertex_buffer(0, self.sprite_buffer.slice(..));
                pass.draw(0..4, 0..count as u32);
            }
        }
        gpu.queue.submit(Some(encoder.finish()));
        frame.present();
        Ok(())
    }
}

/// `Sprite` is `repr(C)` with eight fields of plain floats, laid out as the shader's instance buffer.
fn sprite_bytes(sprites: &[Sprite]) -> &[u8] {
    const _: () = assert!(size_of::<Sprite>() == SPRITE_STRIDE as usize);
    unsafe { std::slice::from_raw_parts(sprites.as_ptr().cast(), std::mem::size_of_val(sprites)) }
}
