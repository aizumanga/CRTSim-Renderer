pub mod config;
mod gpu_prepare;
pub mod mesh;
pub mod nes_luts;
pub mod workflow;

use anyhow::{ensure, Context, Result};
use bytemuck::{Pod, Zeroable};
use config::{ColorMode, Config, Phase};
use image::RgbaImage;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use wgpu::util::DeviceExt;

pub use gpu_prepare::PrepareOn;

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

struct Readback {
    buffer: wgpu::Buffer,
    pitch: u32,
    width: u32,
    height: u32,
}

impl Readback {
    fn new(device: &wgpu::Device, (width, height): (u32, u32)) -> Self {
        let pitch = (width * 4).div_ceil(256) * 256;
        Self {
            buffer: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("readback"),
                size: u64::from(pitch) * u64::from(height),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }),
            pitch,
            width,
            height,
        }
    }
}

struct Workspace {
    source: Target,
    full: Target,
    down: Target,
    up: Target,
    final_target: Target,
    _depth: wgpu::Texture,
    depth_view: wgpu::TextureView,
    readback: Readback,
    prepare: gpu_prepare::Cache,
    signal_size: (u32, u32),
    output_size: (u32, u32),
    surface_format: wgpu::TextureFormat,
}

impl Workspace {
    fn new(
        device: &wgpu::Device,
        signal_size: (u32, u32),
        output_size: (u32, u32),
        surface_format: wgpu::TextureFormat,
    ) -> Self {
        let source = Target::new(device, "clean signal", signal_size);
        let full = Target::with_format(device, "screen and frame", output_size, surface_format, 1);
        let down = Target::with_format(
            device,
            "bloom downsample",
            ((output_size.0 / 16).max(1), (output_size.1 / 16).max(1)),
            surface_format,
            1,
        );
        let up = Target::with_format(device, "bloom upsample", output_size, surface_format, 1);
        let final_target = Target::new(device, "output", output_size);
        let depth = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("depth"),
            size: wgpu::Extent3d {
                width: output_size.0,
                height: output_size.1,
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
        Self {
            source,
            full,
            down,
            up,
            final_target,
            _depth: depth,
            depth_view,
            readback: Readback::new(device, output_size),
            prepare: Default::default(),
            signal_size,
            output_size,
            surface_format,
        }
    }

    fn matches(
        &self,
        signal_size: (u32, u32),
        output_size: (u32, u32),
        surface_format: wgpu::TextureFormat,
    ) -> bool {
        self.signal_size == signal_size
            && self.output_size == output_size
            && self.surface_format == surface_format
    }
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
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    layout: wgpu::BindGroupLayout,
    samplers: Vec<wgpu::Sampler>,
    pipelines: Vec<wgpu::RenderPipeline>,
    screen: GpuMesh,
    frame: GpuMesh,
    artifacts: Target,
    mask: Target,
    prepare_pipelines: gpu_prepare::Pipelines,
    /// Where the prepare step runs. The GPU unless a caller asks otherwise, to compare the two.
    pub prepare: PrepareOn,
    pub adapter: wgpu::AdapterInfo,
}
/// A rendered still. `clean` and `signal` are read back only by a render that asks for them --
/// `render` and its progress variants do; a video frame or a preview does not -- and are
/// otherwise empty.
pub struct Rendered {
    /// The prepared signal: the source resized, edited and graded, before the simulation.
    pub clean: RgbaImage,
    pub signal: RgbaImage,
    pub crt: RgbaImage,
}

/// A rendered frame left on the render device, for a caller that is only going to draw it
/// again on that same device. Reading pixels back costs a full-frame transfer and a wait on
/// the GPU -- measured at 14% of a 1280x720 render and 25% of a 4K one, before the re-upload
/// on the other side -- and a preview pays all of it for nothing.
///
/// The texture is the caller's; drop it when the frame is no longer on screen.
pub struct PreviewFrame {
    pub texture: wgpu::Texture,
    pub width: u32,
    pub height: u32,
}

/// What a render hands back.
#[derive(Clone, Copy, PartialEq)]
enum Output {
    /// Pixels in system memory, for saving, encoding or a test.
    Pixels,
    /// A texture on the render device.
    Texture,
}

/// Feedback belongs to a single sequence on this renderer. Start a new sequence after seeking
/// or changing settings. Reuse it only with the same device and signal dimensions.
#[derive(Default)]
pub struct Sequence {
    history: Option<[Target; 2]>,
    workspace: Option<Workspace>,
    tick: u64,
}

#[derive(Clone, Debug)]
pub struct RenderProgress {
    /// Completed, weighted stages; not an estimate of elapsed time.
    pub fraction: f32,
    pub stage: String,
}

impl Renderer {
    /// The limits the renderer needs. A host sharing its own device must request at least
    /// these, or a large export will fail validation on a device that only ever had to be big
    /// enough for a window.
    pub fn limits(adapter: &wgpu::Adapter) -> wgpu::Limits {
        wgpu::Limits::default().using_resolution(adapter.limits())
    }

    /// Creates a renderer that owns its device. Headless callers -- the CLI, the tests --
    /// want this; a windowed application should share its own device with `with_device`
    /// instead, so rendered frames and the interface drawing them are on one device.
    pub async fn new(backends: wgpu::Backends) -> Result<Self> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends,
            ..Default::default()
        });
        let adapter=instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference:wgpu::PowerPreference::HighPerformance,compatible_surface:None,force_fallback_adapter:false,
        }).await.context("No compatible graphics adapter. Install a Vulkan/DX12/Metal driver; no window is required.")?;
        let info = adapter.get_info();
        let limits = Self::limits(&adapter);
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
        Self::with_device(Arc::new(device), Arc::new(queue), info).await
    }

    /// Builds a renderer on a device someone else owns, so its output can be handed to that
    /// device's other users without a round trip through system memory. The device must have
    /// been requested with at least `Renderer::limits`.
    ///
    /// Note that a shared device cannot be replaced: where `new` lets a caller rebuild after a
    /// driver failure, here a lost device takes its owner down too.
    pub async fn with_device(
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        info: wgpu::AdapterInfo,
    ) -> Result<Self> {
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
        let prepare_pipelines = gpu_prepare::Pipelines::new(&device, &queue);
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
            prepare_pipelines,
            prepare: PrepareOn::default(),
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
        progress: impl FnMut(RenderProgress),
    ) -> Result<Rendered> {
        Ok(self
            .render_sequence(
                input,
                c,
                &mut Sequence::default(),
                true,
                || false,
                progress,
                Output::Pixels,
            )?
            .0)
    }

    /// A cancellable still export. Cancellation is checked between bounded GPU batches.
    pub fn render_with_progress_and_cancel(
        &self,
        input: &RgbaImage,
        c: &Config,
        cancel: &AtomicBool,
        progress: impl FnMut(RenderProgress),
    ) -> Result<Rendered> {
        Ok(self
            .render_sequence(
                input,
                c,
                &mut Sequence::default(),
                true,
                || cancel.load(Ordering::Relaxed),
                progress,
                Output::Pixels,
            )?
            .0)
    }

    /// One ordered video frame. Warm-up applies once, then history and phase persist.
    pub fn render_video_frame(
        &self,
        input: &RgbaImage,
        c: &Config,
        sequence: &mut Sequence,
    ) -> Result<RgbaImage> {
        Ok(self
            .render_sequence(input, c, sequence, false, || false, |_| {}, Output::Pixels)?
            .0
            .crt)
    }

    pub fn render_video_frame_with_cancel(
        &self,
        input: &RgbaImage,
        c: &Config,
        sequence: &mut Sequence,
        cancel: &AtomicBool,
    ) -> Result<RgbaImage> {
        Ok(self
            .render_sequence(
                input,
                c,
                sequence,
                false,
                || cancel.load(Ordering::Relaxed),
                |_| {},
                Output::Pixels,
            )?
            .0
            .crt)
    }

    /// One frame for display on this renderer's own device, skipping the read back to system
    /// memory that a saved or encoded frame needs.
    pub fn render_preview(&self, input: &RgbaImage, c: &Config) -> Result<PreviewFrame> {
        let (_, preview) = self.render_sequence(
            input,
            c,
            &mut Sequence::default(),
            false,
            || false,
            |_| {},
            Output::Texture,
        )?;
        preview.context("render did not produce a preview frame")
    }

    #[allow(clippy::too_many_arguments)]
    fn render_sequence(
        &self,
        input: &RgbaImage,
        c: &Config,
        sequence: &mut Sequence,
        debug: bool,
        mut cancelled: impl FnMut() -> bool,
        mut progress: impl FnMut(RenderProgress),
        output: Output,
    ) -> Result<(Rendered, Option<PreviewFrame>)> {
        ensure!(!cancelled(), "Render cancelled");
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
        const BUDGET: u64 = 1_500_000_000;
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
            estimated <= BUDGET,
            "estimated working set exceeds Phase 0 budget; choose a smaller preset"
        );
        // Preparing on the device puts the input there as it is, plus the resize's intermediate.
        // Where that does not fit -- a 16384-pixel image on a device limited to 8192, or a huge
        // input kept at native size -- the render is still accepted, as it always was, by
        // preparing here instead.
        let (width, height) = input.dimensions();
        let gpu_prepare = self.prepare == PrepareOn::Gpu
            && width <= limit
            && height <= limit
            && estimated + u64::from(width) * (u64::from(height) * 4 + u64::from(sig.1) * 16)
                <= BUDGET;
        let clean = if gpu_prepare {
            None
        } else {
            Some(config::prepare(input, c)?)
        };
        let workspace = sequence
            .workspace
            .get_or_insert_with(|| Workspace::new(&self.device, sig, out, surface_format));
        ensure!(
            workspace.matches(sig, out, surface_format),
            "Start a new video sequence after changing signal size, output size or color mode"
        );
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("still job"),
            });
        match &clean {
            Some(clean) => workspace.source.upload(&self.queue, clean),
            None => self.prepare_pipelines.encode(
                &self.device,
                &self.queue,
                &mut encoder,
                &mut workspace.prepare,
                input,
                c,
                &workspace.source,
            ),
        }
        report(0.05, "Image prepared".into());
        let first = sequence.history.is_none();
        let history = sequence.history.get_or_insert_with(|| {
            [
                Target::new(&self.device, "history A", sig),
                Target::new(&self.device, "history B", sig),
            ]
        });
        ensure!(
            (history[0].width, history[0].height) == sig,
            "Start a new video sequence after changing signal size"
        );
        let source = &workspace.source;
        let full = &workspace.full;
        let down = &workspace.down;
        let up = &workspace.up;
        let final_target = &workspace.final_target;
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
            processing: [
                if linear { 1. } else { 0. },
                if c.interlace { 1. } else { 0. },
                0.,
                0.,
            ],
        };
        // Explicitly reset both feedback surfaces; each job is independent.
        for t in history.iter().filter(|_| first) {
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
        let count = if first { c.warmup + 1 } else { 1 };
        for step in 0..count {
            ensure!(!cancelled(), "Render cancelled");
            let tick = sequence.tick;
            p.signal[3] = match c.phase {
                Phase::Stable => 0.5,
                Phase::A => 0.,
                Phase::B => 1.,
                Phase::Alternating => (tick % 2) as f32,
            };
            // The field this tick scans, when interlaced.
            p.processing[2] = (tick % 2) as f32;
            let uniform = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("tick settings"),
                    contents: bytemuck::bytes_of(&p),
                    usage: wgpu::BufferUsages::UNIFORM,
                });
            let bindings = self.bind(&uniform, source, &history[(1 - tick % 2) as usize]);
            self.pass(&mut encoder, &history[(tick % 2) as usize], &bindings, 0);
            // Report completed GPU work, not just command encoding. Small batches keep overhead bounded.
            sequence.tick += 1;
            if (step + 1) % 8 == 0 || step + 1 == count {
                self.queue.submit(Some(encoder.finish()));
                self.device.poll(wgpu::Maintain::Wait);
                report(
                    0.05 + 0.75 * (step + 1) as f32 / count as f32,
                    format!("Simulation {}/{}", step + 1, count),
                );
                encoder = self.device.create_command_encoder(&Default::default());
            }
        }
        let signal = &history[((sequence.tick - 1) % 2) as usize];
        let uniform = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("surface settings"),
                contents: bytemuck::bytes_of(&p),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let bindings = self.bind(&uniform, signal, source);
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
                    view: &workspace.depth_view,
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
                if c.screen_only && index == 2 {
                    continue;
                }
                pass.set_pipeline(&self.pipelines[index + base_pipeline]);
                pass.set_vertex_buffer(0, m.vertices.slice(..));
                pass.set_index_buffer(m.indices.slice(..), wgpu::IndexFormat::Uint16);
                pass.draw_indexed(0..m.count, 0, 0..1);
            }
        }
        self.pass(
            &mut encoder,
            down,
            &self.bind(&uniform, full, source),
            3 + base_pipeline,
        );
        self.pass(
            &mut encoder,
            up,
            &self.bind(&uniform, down, source),
            4 + base_pipeline,
        );
        self.pass(
            &mut encoder,
            final_target,
            &self.bind(&uniform, full, up),
            5 + base_pipeline,
        );
        self.queue.submit(Some(encoder.finish()));
        ensure!(!cancelled(), "Render cancelled");
        let signal = if debug {
            self.readback(signal)?
        } else {
            RgbaImage::new(0, 0)
        };
        let clean = match clean {
            Some(clean) => clean,
            None if debug => self.readback(source)?,
            None => RgbaImage::new(0, 0),
        };
        report(
            0.9,
            "Glass, lighting and bloom complete; reading pixels".into(),
        );
        let (crt, preview) = match output {
            Output::Pixels => (
                self.readback_into(final_target, &mut workspace.readback)?,
                None,
            ),
            // The final target is reused by the next render, so the frame is copied out
            // rather than handed over: a blit on the device, not a round trip through system
            // memory. sRGB because that is the format egui requires of a texture it is given,
            // and the copy is legal because the two differ only in that.
            Output::Texture => {
                let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("preview frame"),
                    size: wgpu::Extent3d {
                        width: final_target.width,
                        height: final_target.height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8UnormSrgb,
                    usage: wgpu::TextureUsages::COPY_DST
                        | wgpu::TextureUsages::COPY_SRC
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                });
                let mut encoder = self.device.create_command_encoder(&Default::default());
                encoder.copy_texture_to_texture(
                    final_target.texture.as_image_copy(),
                    texture.as_image_copy(),
                    wgpu::Extent3d {
                        width: final_target.width,
                        height: final_target.height,
                        depth_or_array_layers: 1,
                    },
                );
                self.queue.submit(Some(encoder.finish()));
                (
                    RgbaImage::new(0, 0),
                    Some(PreviewFrame {
                        texture,
                        width: final_target.width,
                        height: final_target.height,
                    }),
                )
            }
        };
        report(1., "Render complete".into());
        Ok((Rendered { clean, signal, crt }, preview))
    }

    fn readback(&self, target: &Target) -> Result<RgbaImage> {
        let mut readback = Readback::new(&self.device, (target.width, target.height));
        self.readback_into(target, &mut readback)
    }

    fn readback_into(&self, target: &Target, readback: &mut Readback) -> Result<RgbaImage> {
        ensure!(
            (readback.width, readback.height) == (target.width, target.height),
            "readback dimensions do not match render target"
        );
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            target.texture.as_image_copy(),
            wgpu::ImageCopyBuffer {
                buffer: &readback.buffer,
                layout: wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(readback.pitch),
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
        let slice = readback.buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.device.poll(wgpu::Maintain::Wait);
        rx.recv()??;
        let mapped = slice.get_mapped_range();
        let mut pixels = Vec::with_capacity((target.width * target.height * 4) as usize);
        for row in mapped.chunks_exact(readback.pitch as usize) {
            pixels.extend_from_slice(&row[..target.width as usize * 4]);
        }
        drop(mapped);
        readback.buffer.unmap();
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
    #[ignore = "requires a Vulkan adapter"]
    fn video_history_survives_between_frames_and_resets_for_new_sequence() {
        let r = pollster::block_on(Renderer::new(wgpu::Backends::VULKAN)).unwrap();
        let c = Config {
            output: "160x120".into(),
            signal: "32x32".into(),
            warmup: 0,
            persistence: [0.9; 3],
            ..Config::default()
        };
        let white = RgbaImage::from_pixel(32, 32, image::Rgba([255; 4]));
        let black = RgbaImage::from_pixel(32, 32, image::Rgba([0, 0, 0, 255]));
        let mut sequence = Sequence::default();
        r.render_video_frame(&white, &c, &mut sequence).unwrap();
        let trailing = r.render_video_frame(&black, &c, &mut sequence).unwrap();
        let reset = r
            .render_video_frame(&black, &c, &mut Sequence::default())
            .unwrap();
        assert_ne!(trailing, reset, "history was reset between video frames");
        assert_eq!(reset, r.render(&black, &c).unwrap().crt);
    }
    #[test]
    #[ignore = "requires a Vulkan adapter"]
    fn preview_frame_matches_the_pixels_a_read_back_render_produces() {
        let r = pollster::block_on(Renderer::new(wgpu::Backends::VULKAN)).unwrap();
        let c = Config {
            output: "320x180".into(),
            ..Config::default()
        };
        let source = config::test_card();
        let preview = r.render_preview(&source, &c).unwrap();
        assert_eq!((preview.width, preview.height), (320, 180));
        // Read the frame back the same way an export would, to compare like with like. The
        // copy into the preview texture keeps the bytes and only relabels them as sRGB, which
        // is the conversion egui would otherwise apply when it uploads the pixels itself.
        let view = preview.texture.create_view(&Default::default());
        let target = Target {
            texture: preview.texture,
            view,
            width: preview.width,
            height: preview.height,
        };
        assert_eq!(
            r.readback(&target).unwrap(),
            r.render(&source, &c).unwrap().crt
        );
    }

    /// The GPU prepare step against the CPU one it replaced, across a spread of routes and
    /// settings wider than the goldens pin. Not tied to a recorded driver like the goldens: the
    /// shader repeats the CPU's arithmetic, so on any driver the two differ only where float
    /// rounding lands a value on the other side of a half step.
    #[test]
    #[ignore = "requires a Vulkan adapter"]
    fn gpu_prepare_matches_cpu_prepare() {
        let mut r = pollster::block_on(Renderer::new(wgpu::Backends::VULKAN)).unwrap();
        let source = RgbaImage::from_fn(301, 187, |x, y| {
            let v = (x * 37 + y * 101 + x * y * 13) % 256;
            let a = if (x / 40 + y / 30) % 3 == 0 {
                (x + y) % 256
            } else {
                255
            };
            image::Rgba([v as u8, (v * 7 % 256) as u8, (255 - v) as u8, a as u8])
        });
        let lut = Arc::new(nes_luts::load(7).unwrap());
        let mut state = 0x2545_f491_u32;
        let mut next = |range: f32| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            (state as f32 / u32::MAX as f32 * 2. - 1.) * range
        };
        let mut worst = 0;
        let mut differing = 0;
        let mut total = 0;
        for case in 0..48 {
            let signal = ["original", "native", "240p", "480p", "97x301", "600x100"][case % 6];
            let mut c = Config {
                signal: signal.into(),
                output: "64x48".into(),
                warmup: 0,
                filter: if case % 2 == 0 {
                    config::Filter::Lanczos
                } else {
                    config::Filter::Nearest
                },
                ..Config::default()
            };
            c.source.background = [(case * 40 % 256) as u8, 90, 200];
            if case % 3 == 0 {
                c.source.rotation = next(180.);
                c.source.zoom = 1. + next(0.6);
                c.source.position = [next(0.3), next(0.3)];
                c.source.crop = [next(0.2).abs(), next(0.2).abs(), 0.1, 0.];
                c.source.checkerboard = case % 4 == 0;
            }
            if case % 4 == 1 {
                c.lut = Some(lut.clone());
            }
            if case % 5 < 2 {
                c.hue = next(180.);
                c.chroma = 1. + next(1.);
            }
            r.prepare = PrepareOn::Cpu;
            let cpu = r.render(&source, &c).unwrap().clean;
            r.prepare = PrepareOn::Gpu;
            let gpu = r.render(&source, &c).unwrap().clean;
            assert_eq!(cpu.dimensions(), gpu.dimensions(), "case {case}");
            for (a, b) in cpu.pixels().zip(gpu.pixels()) {
                for channel in 0..4 {
                    let delta = a[channel].abs_diff(b[channel]);
                    worst = worst.max(delta);
                    differing += usize::from(delta > 0);
                    total += 1;
                }
            }
        }
        println!("{differing} of {total} channels differ, by at most {worst} step(s)");
        assert!(worst <= 1, "GPU prepare differs by {worst} steps");
    }

    /// Read in the signal, where rows are rows: a tick scans one field, and the other keeps only
    /// what persistence leaves of its last scan.
    #[test]
    #[ignore = "requires a Vulkan adapter"]
    fn interlaced_ticks_scan_alternate_fields() {
        let r = pollster::block_on(Renderer::new(wgpu::Backends::VULKAN)).unwrap();
        let c = Config {
            output: "64x48".into(),
            signal: "32x16".into(),
            warmup: 0,
            persistence: [0.; 3],
            artifacts: 0.,
            sharpness: 0.,
            interlace: true,
            ..Config::default()
        };
        let white = RgbaImage::from_pixel(32, 16, image::Rgba([255; 4]));
        let rows = |c: &Config| -> Vec<u8> {
            let signal = r.render(&white, c).unwrap().signal;
            (0..16).map(|y| signal.get_pixel(16, y)[0]).collect()
        };
        // One tick scans the first field only.
        let first = rows(&c);
        assert!(first.iter().step_by(2).all(|&v| v == 255), "{first:?}");
        assert!(
            first.iter().skip(1).step_by(2).all(|&v| v == 0),
            "{first:?}"
        );
        // The second tick scans the other field; with no persistence the first goes dark.
        let second = rows(&Config {
            warmup: 1,
            ..c.clone()
        });
        assert!(second.iter().step_by(2).all(|&v| v == 0), "{second:?}");
        assert!(
            second.iter().skip(1).step_by(2).all(|&v| v == 255),
            "{second:?}"
        );
        // With persistence the unscanned field keeps a decayed copy instead.
        let decayed = rows(&Config {
            warmup: 1,
            persistence: [0.5; 3],
            ..c.clone()
        });
        assert!(
            decayed.iter().step_by(2).all(|&v| (120..=135).contains(&v)),
            "{decayed:?}"
        );
        assert!(
            decayed.iter().skip(1).step_by(2).all(|&v| v == 255),
            "{decayed:?}"
        );
        // Off, every tick scans every row.
        let progressive = rows(&Config {
            interlace: false,
            ..c
        });
        assert!(progressive.iter().all(|&v| v == 255), "{progressive:?}");
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
        let cancel = AtomicBool::new(true);
        assert!(r
            .render_with_progress_and_cancel(&source, &c, &cancel, |_| {})
            .err()
            .unwrap()
            .to_string()
            .contains("cancelled"));
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
