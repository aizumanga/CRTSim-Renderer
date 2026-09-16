pub mod config;
pub mod mesh;

use anyhow::{ensure, Context, Result};
use bytemuck::{Pod, Zeroable};
use config::{ColorMode, Config, Phase};
use image::RgbaImage;
use wgpu::util::DeviceExt;

pub const SHADER: &str = include_str!("../../../shaders/crtsim.wgsl");
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Params {
    mvp: [[f32; 4]; 4],
    size: [f32; 4],
    signal: [f32; 4],
    persistence: [f32; 4],
    geometry: [f32; 4],
    mask: [f32; 4],
    lighting: [f32; 4],
    surface: [f32; 4],
    frame: [f32; 4],
    light: [f32; 4],
    camera: [f32; 4],
    bloom: [f32; 4],
    processing: [f32; 4],
}

struct Target {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    width: u32,
    height: u32,
}
impl Target {
    fn new(device: &wgpu::Device, name: &str, (width, height): (u32, u32)) -> Self {
        Self::with_format(device, name, (width, height), FORMAT, 1)
    }
    fn with_format(
        device: &wgpu::Device,
        name: &str,
        (width, height): (u32, u32),
        format: wgpu::TextureFormat,
        mip_level_count: u32,
    ) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(name),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        Self {
            texture,
            view,
            width,
            height,
        }
    }
    fn upload(&self, queue: &wgpu::Queue, image: &RgbaImage) {
        queue.write_texture(
            self.texture.as_image_copy(),
            image.as_raw(),
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(4 * self.width),
                rows_per_image: Some(self.height),
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );
    }
}

struct GpuMesh {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    count: u32,
}
impl GpuMesh {
    fn new(device: &wgpu::Device, bytes: &[u8]) -> Result<Self> {
        let m = mesh::Mesh::read(bytes)?;
        Ok(Self {
            vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("mesh vertices"),
                contents: bytemuck::cast_slice(&m.vertices),
                usage: wgpu::BufferUsages::VERTEX,
            }),
            indices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("mesh indices"),
                contents: bytemuck::cast_slice(&m.indices),
                usage: wgpu::BufferUsages::INDEX,
            }),
            count: m.indices.len() as u32,
        })
    }
}

/// One reusable GPU device; individual still jobs own and reset their history.
pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    layout: wgpu::BindGroupLayout,
    samplers: Vec<wgpu::Sampler>,
    pipelines: Vec<wgpu::RenderPipeline>,
    screen: GpuMesh,
    frame: GpuMesh,
    artifacts: Target,
    mask: Target,
    pub adapter: wgpu::AdapterInfo,
}
pub struct Rendered {
    pub clean: RgbaImage,
    pub signal: RgbaImage,
    pub crt: RgbaImage,
}

#[derive(Clone, Debug)]
pub struct RenderProgress {
    /// Completed, weighted stages; not an estimate of elapsed time.
    pub fraction: f32,
    pub stage: String,
}

impl Renderer {
    pub async fn new(backends: wgpu::Backends) -> Result<Self> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends,
            ..Default::default()
        });
        let adapter=instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference:wgpu::PowerPreference::HighPerformance,compatible_surface:None,force_fallback_adapter:false,
        }).await.context("No compatible graphics adapter. Install a Vulkan/DX12/Metal driver; no window is required.")?;
        let info = adapter.get_info();
        let limits = wgpu::Limits::default().using_resolution(adapter.limits());
        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("CRTSim"),
                    required_features: wgpu::Features::empty(),
                    required_limits: limits,
                },
                None,
            )
            .await?;
        let mut entries = vec![wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }];
        for binding in 1..=4 {
            entries.push(wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            });
        }
        for binding in 5..=8 {
            entries.push(wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            });
        }
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("CRT bindings"),
            entries: &entries,
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        device.push_error_scope(wgpu::ErrorFilter::Validation);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("CRT WGSL"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let attrs = wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x3,2=>Float32x4,3=>Float32x2,4=>Float32];
        let vertex_layout = [wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<mesh::Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &attrs,
        }];
        let mut pipelines = Vec::new();
        for (index, entry) in [
            "composite",
            "screen",
            "frame",
            "downsample",
            "upsample",
            "present",
            "composite",
            "screen",
            "frame",
            "downsample",
            "upsample",
            "present",
        ]
        .into_iter()
        .enumerate()
        {
            let geometry = entry == "screen" || entry == "frame";
            pipelines.push(
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some(entry),
                    layout: Some(&pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: &module,
                        entry_point: if geometry { "mesh" } else { "quad" },
                        buffers: if geometry { &vertex_layout } else { &[] },
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &module,
                        entry_point: entry,
                        targets: &[Some(wgpu::ColorTargetState {
                            format: if index >= 6 && entry != "composite" && entry != "present" {
                                wgpu::TextureFormat::Rgba16Float
                            } else {
                                FORMAT
                            },
                            blend: None,
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                    }),
                    primitive: wgpu::PrimitiveState {
                        cull_mode: None,
                        ..Default::default()
                    },
                    depth_stencil: if geometry {
                        Some(wgpu::DepthStencilState {
                            format: wgpu::TextureFormat::Depth32Float,
                            depth_write_enabled: true,
                            depth_compare: wgpu::CompareFunction::Less,
                            stencil: Default::default(),
                            bias: Default::default(),
                        })
                    } else {
                        None
                    },
                    multisample: Default::default(),
                    multiview: None,
                }),
            );
        }
        if let Some(error) = device.pop_error_scope().await {
            anyhow::bail!("shader/pipeline validation: {error}");
        }
        let samplers = (0..4)
            .map(|i| {
                let address = if i < 2 {
                    wgpu::AddressMode::ClampToEdge
                } else {
                    wgpu::AddressMode::Repeat
                };
                let filter = if i % 2 == 0 {
                    wgpu::FilterMode::Nearest
                } else {
                    wgpu::FilterMode::Linear
                };
                device.create_sampler(&wgpu::SamplerDescriptor {
                    address_mode_u: address,
                    address_mode_v: address,
                    mag_filter: filter,
                    min_filter: filter,
                    mipmap_filter: wgpu::FilterMode::Linear,
                    ..Default::default()
                })
            })
            .collect();
        let load = |bytes: &[u8], name: &str| -> Result<Target> {
            let image =
                image::load_from_memory_with_format(bytes, image::ImageFormat::Bmp)?.to_rgba8();
            let target = Target::new(&device, name, image.dimensions());
            target.upload(&queue, &image);
            Ok(target)
        };
        let artifacts = load(
            include_bytes!("../../../assets/original-crtsim/artifacts.bmp"),
            "NTSC texture",
        )?;
        let mut mask_image = image::load_from_memory_with_format(
            include_bytes!("../../../assets/original-crtsim/mask.bmp"),
            image::ImageFormat::Bmp,
        )?
        .to_rgba8();
        let levels = mask_image.width().max(mask_image.height()).ilog2() + 1;
        let mask = Target::with_format(
            &device,
            "shadow mask",
            mask_image.dimensions(),
            FORMAT,
            levels,
        );
        for level in 0..levels {
            queue.write_texture(
                wgpu::ImageCopyTexture {
                    texture: &mask.texture,
                    mip_level: level,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                mask_image.as_raw(),
                wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(mask_image.width() * 4),
                    rows_per_image: Some(mask_image.height()),
                },
                wgpu::Extent3d {
                    width: mask_image.width(),
                    height: mask_image.height(),
                    depth_or_array_layers: 1,
                },
            );
            mask_image = image::imageops::resize(
                &mask_image,
                (mask_image.width() / 2).max(1),
                (mask_image.height() / 2).max(1),
                image::imageops::FilterType::Triangle,
            );
        }
        let screen = GpuMesh::new(&device, mesh::SCREEN)?;
        let frame = GpuMesh::new(&device, mesh::FRAME)?;
        Ok(Self {
            device,
            queue,
            layout,
            samplers,
            pipelines,
            screen,
            frame,
            artifacts,
            mask,
            adapter: info,
        })
    }

    fn bind(&self, uniform: &wgpu::Buffer, source: &Target, previous: &Target) -> wgpu::BindGroup {
        let mut entries = vec![wgpu::BindGroupEntry {
            binding: 0,
            resource: uniform.as_entire_binding(),
        }];
        for (i, t) in [source, previous, &self.artifacts, &self.mask]
            .iter()
            .enumerate()
        {
            entries.push(wgpu::BindGroupEntry {
                binding: i as u32 + 1,
                resource: wgpu::BindingResource::TextureView(&t.view),
            });
        }
        for (i, s) in self.samplers.iter().enumerate() {
            entries.push(wgpu::BindGroupEntry {
                binding: i as u32 + 5,
                resource: wgpu::BindingResource::Sampler(s),
            });
        }
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.layout,
            entries: &entries,
        })
    }
    fn pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        dst: &Target,
        bindings: &wgpu::BindGroup,
        pipeline: usize,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("fullscreen CRT pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &dst.view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_pipeline(&self.pipelines[pipeline]);
        pass.set_bind_group(0, bindings, &[]);
        pass.draw(0..3, 0..1);
    }

    /// Deterministic still export. warmup=0 means one tick from cleared history.
    pub fn render(&self, input: &RgbaImage, c: &Config) -> Result<Rendered> {
        self.render_with_progress(input, c, |_| {})
    }

    pub fn render_with_progress(
        &self,
        input: &RgbaImage,
        c: &Config,
        mut progress: impl FnMut(RenderProgress),
    ) -> Result<Rendered> {
        let mut report = |fraction, stage: String| progress(RenderProgress { fraction, stage });
        report(0., "Preparing image".into());
        c.validate()?;
        let sig = c.signal_size(input.dimensions())?;
        let out = c.output_size(input.dimensions())?;
        let limit = self.device.limits().max_texture_dimension_2d;
        ensure!(
            [sig.0, sig.1, out.0, out.1].iter().all(|v| *v <= limit),
            "image exceeds adapter texture limit {limit}"
        );
        // Conservative working-set guard: color targets, depth, readback and history.
        let linear = c.color_mode == ColorMode::LinearLight;
        let base_pipeline = if linear { 6 } else { 0 };
        let surface_format = if linear {
            wgpu::TextureFormat::Rgba16Float
        } else {
            FORMAT
        };
        let estimated = u64::from(out.0) * u64::from(out.1) * if linear { 48 } else { 24 }
            + u64::from(sig.0) * u64::from(sig.1) * 16;
        ensure!(
            estimated <= 1_500_000_000,
            "estimated working set exceeds Phase 0 budget; choose a smaller preset"
        );
        let clean = config::prepare(input, c)?;
        let source = Target::new(&self.device, "clean signal", sig);
        source.upload(&self.queue, &clean);
        report(0.05, "Image prepared".into());
        let history = [
            Target::new(&self.device, "history A", sig),
            Target::new(&self.device, "history B", sig),
        ];
        let full = Target::with_format(&self.device, "screen and frame", out, surface_format, 1);
        let down = Target::with_format(
            &self.device,
            "bloom downsample",
            ((out.0 / 16).max(1), (out.1 / 16).max(1)),
            surface_format,
            1,
        );
        let up = Target::with_format(&self.device, "bloom upsample", out, surface_format, 1);
        let final_target = Target::new(&self.device, "output", out);
        let depth = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("depth"),
            size: wgpu::Extent3d {
                width: out.0,
                height: out.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let depth_view = depth.create_view(&Default::default());
        let fov = c.fov.to_radians();
        let distance = 1. / (fov * 0.5).tan();
        let camera = glam::Vec3::new(-distance, 0., 0.);
        let view = glam::Mat4::look_at_rh(camera, glam::Vec3::ZERO, glam::Vec3::Z);
        let projection = glam::Mat4::perspective_rh(fov, out.0 as f32 / out.1 as f32, 0.1, 100.);
        let uv = c.uv_scale(sig);
        let mut p = Params {
            mvp: (projection * view).to_cols_array_2d(),
            size: [sig.0 as f32, sig.1 as f32, out.0 as f32, out.1 as f32],
            signal: [c.sharpness, c.bleed, c.artifacts, 0.5],
            persistence: [c.persistence[0], c.persistence[1], c.persistence[2], 0.],
            geometry: [uv[0], uv[1], c.overscan, c.barrel],
            mask: [
                c.mask_repeats[0],
                c.mask_repeats[1],
                c.mask_brightness,
                c.mask_opacity,
            ],
            lighting: [c.diffuse, c.specular, c.specular_power, c.rim],
            surface: [
                c.dimming,
                c.reflection,
                c.saturation,
                if c.mask_antialias { 1. } else { 0. },
            ],
            frame: [c.frame_color[0], c.frame_color[1], c.frame_color[2], 1.],
            light: [
                c.light_position[0],
                c.light_position[1],
                c.light_position[2],
                0.,
            ],
            camera: [camera.x, camera.y, camera.z, 0.],
            bloom: [c.bloom, c.bloom_power, c.bloom_spread, 0.],
            processing: [if linear { 1. } else { 0. }, 0., 0., 0.],
        };
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("still job"),
            });
        // Explicitly reset both feedback surfaces; each job is independent.
        for t in &history {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("reset history"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &t.view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
        }
        // Separate immutable uniform per tick avoids queue.write_buffer ordering bugs.
        for tick in 0..=c.warmup {
            p.signal[3] = match c.phase {
                Phase::Stable => 0.5,
                Phase::A => 0.,
                Phase::B => 1.,
                Phase::Alternating => (tick % 2) as f32,
            };
            let uniform = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("tick settings"),
                    contents: bytemuck::bytes_of(&p),
                    usage: wgpu::BufferUsages::UNIFORM,
                });
            let bindings = self.bind(&uniform, &source, &history[(1 - tick % 2) as usize]);
            self.pass(&mut encoder, &history[(tick % 2) as usize], &bindings, 0);
            // Report completed GPU work, not just command encoding. Small batches keep overhead bounded.
            if (tick + 1) % 8 == 0 || tick == c.warmup {
                self.queue.submit(Some(encoder.finish()));
                self.device.poll(wgpu::Maintain::Wait);
                report(
                    0.05 + 0.75 * (tick + 1) as f32 / (c.warmup + 1) as f32,
                    format!("Warm-up {}/{}", tick + 1, c.warmup + 1),
                );
                encoder = self.device.create_command_encoder(&Default::default());
            }
        }
        let signal = &history[(c.warmup % 2) as usize];
        let uniform = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("surface settings"),
                contents: bytemuck::bytes_of(&p),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let bindings = self.bind(&uniform, signal, &source);
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("curved glass and frame"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &full.view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_bind_group(0, &bindings, &[]);
            for (index, m) in [(1, &self.screen), (2, &self.frame)] {
                pass.set_pipeline(&self.pipelines[index + base_pipeline]);
                pass.set_vertex_buffer(0, m.vertices.slice(..));
                pass.set_index_buffer(m.indices.slice(..), wgpu::IndexFormat::Uint16);
                pass.draw_indexed(0..m.count, 0, 0..1);
            }
        }
        self.pass(
            &mut encoder,
            &down,
            &self.bind(&uniform, &full, &source),
            3 + base_pipeline,
        );
        self.pass(
            &mut encoder,
            &up,
            &self.bind(&uniform, &down, &source),
            4 + base_pipeline,
        );
        self.pass(
            &mut encoder,
            &final_target,
            &self.bind(&uniform, &full, &up),
            5 + base_pipeline,
        );
        self.queue.submit(Some(encoder.finish()));
        let signal = self.readback(signal)?;
        report(
            0.9,
            "Glass, lighting and bloom complete; reading pixels".into(),
        );
        let crt = self.readback(&final_target)?;
        report(1., "Render complete".into());
        Ok(Rendered { clean, signal, crt })
    }

    fn readback(&self, target: &Target) -> Result<RgbaImage> {
        let pitch = (target.width * 4).div_ceil(256) * 256;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: u64::from(pitch) * u64::from(target.height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            target.texture.as_image_copy(),
            wgpu::ImageCopyBuffer {
                buffer: &buffer,
                layout: wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(pitch),
                    rows_per_image: Some(target.height),
                },
            },
            wgpu::Extent3d {
                width: target.width,
                height: target.height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit(Some(encoder.finish()));
        let slice = buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.device.poll(wgpu::Maintain::Wait);
        rx.recv()??;
        let mapped = slice.get_mapped_range();
        let mut pixels = Vec::with_capacity((target.width * target.height * 4) as usize);
        for row in mapped.chunks_exact(pitch as usize) {
            pixels.extend_from_slice(&row[..target.width as usize * 4]);
        }
        drop(mapped);
        buffer.unmap();
        RgbaImage::from_raw(target.width, target.height, pixels).context("invalid readback length")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wgsl_validates_without_a_gpu() {
        let module = naga::front::wgsl::parse_str(SHADER).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .unwrap();
    }
    #[test]
    #[ignore = "requires an explicitly provisioned GPU or software Vulkan driver"]
    fn gpu_smoke() {
        let r = pollster::block_on(Renderer::new(wgpu::Backends::all())).unwrap();
        let mut c = Config {
            output: "641x361".into(),
            ..Config::default()
        };
        let source = config::test_card();
        let a = r.render(&source, &c).unwrap();
        let b = r.render(&source, &c).unwrap();
        assert_eq!(a.crt.dimensions(), (641, 361));
        assert_eq!(a.crt, b.crt);
        assert!(a.crt.pixels().any(|p| p[0] > 180));
        assert!(a.crt.pixels().any(|p| p[0] < 20));
        c.phase = Phase::A;
        let a = r.render(&source, &c).unwrap();
        c.phase = Phase::B;
        let b = r.render(&source, &c).unwrap();
        assert_ne!(a.signal, b.signal);
        c.phase = Phase::Stable;
        c.artifacts = 0.;
        c.sharpness = 0.;
        c.persistence = [0.; 3];
        let a = r.render(&source, &c).unwrap();
        assert_eq!(a.clean, a.signal);
        c.color_mode = ColorMode::LinearLight;
        c.mask_antialias = true;
        let mut progress = vec![];
        let linear = r
            .render_with_progress(&source, &c, |p| progress.push(p.fraction))
            .unwrap();
        assert_eq!(linear.crt.dimensions(), (641, 361));
        assert_ne!(linear.crt, a.crt);
        assert_eq!(progress.first(), Some(&0.));
        assert_eq!(progress.last(), Some(&1.));
        assert!(progress.windows(2).all(|w| w[0] <= w[1]));
        c.color_mode = ColorMode::Reference;
        let filtered = r.render(&source, &c).unwrap();
        assert_ne!(filtered.crt, a.crt);
        if let Ok(dir) = std::env::var("CRTSIM_TEST_OUTPUT") {
            std::fs::create_dir_all(&dir).unwrap();
            linear
                .crt
                .save(std::path::Path::new(&dir).join("linear-light.png"))
                .unwrap();
            filtered
                .crt
                .save(std::path::Path::new(&dir).join("filtered-mask.png"))
                .unwrap();
        }
    }
}
