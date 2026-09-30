pub mod bezel;
pub mod config;
mod gpu;
mod gpu_prepare;
pub mod input;
pub mod mesh;
pub mod nes_luts;
pub mod palette;
pub mod retroarch;
mod sequence;
pub mod settings;
pub mod test_clip;
#[cfg(test)]
mod tests;
pub mod workflow;

use anyhow::{ensure, Context, Result};
use bytemuck::{Pod, Zeroable};
use config::{ColorMode, Config, Phase};
use gpu::{Readback, Target, FORMAT};
use image::RgbaImage;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

pub use gpu_prepare::PrepareOn;
pub use sequence::{Sequence, Timing};

pub const SHADER: &str = include_str!("../../../shaders/crtsim.wgsl");

/// The shader's settings, laid out as `Params` in `crtsim.wgsl`.
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
    bezel: [f32; 4],
    light: [f32; 4],
    camera: [f32; 4],
    bloom: [f32; 4],
    processing: [f32; 4],
}

/// The size of `Params`, which each settings slot holds.
const PARAMS_SIZE: Option<std::num::NonZeroU64> =
    std::num::NonZeroU64::new(std::mem::size_of::<Params>() as u64);

impl Params {
    /// Everything but the composite phase and the interlaced field, which `tick` sets.
    fn new(c: &Config, signal: (u32, u32), output: (u32, u32)) -> Self {
        let fov = c.fov.to_radians();
        let distance = 1. / (fov * 0.5).tan();
        let camera = glam::Vec3::new(-distance, 0., 0.);
        let view = glam::Mat4::look_at_rh(camera, glam::Vec3::ZERO, glam::Vec3::Z);
        let projection =
            glam::Mat4::perspective_rh(fov, output.0 as f32 / output.1 as f32, 0.1, 100.);
        let uv = c.uv_scale(signal);
        let mask = c.mask_repeats.resolve(signal);
        let flag = |on: bool| if on { 1. } else { 0. };
        Self {
            mvp: (projection * view).to_cols_array_2d(),
            size: [
                signal.0 as f32,
                signal.1 as f32,
                output.0 as f32,
                output.1 as f32,
            ],
            signal: [c.sharpness, c.bleed, c.artifacts, 0.5],
            persistence: [c.persistence[0], c.persistence[1], c.persistence[2], 0.],
            geometry: [uv[0], uv[1], c.overscan, c.barrel],
            mask: [mask[0], mask[1], c.mask_brightness, c.mask_opacity],
            lighting: [c.diffuse, c.specular, c.specular_power, c.rim],
            surface: [
                c.dimming,
                c.reflection,
                c.saturation,
                flag(c.mask_antialias),
            ],
            bezel: [c.frame_color[0], c.frame_color[1], c.frame_color[2], 1.],
            light: [
                c.light_position[0],
                c.light_position[1],
                c.light_position[2],
                0.,
            ],
            camera: [camera.x, camera.y, camera.z, 0.],
            bloom: [c.bloom, c.bloom_power, c.bloom_spread, 0.],
            processing: [
                flag(c.color_mode == ColorMode::LinearLight),
                flag(c.interlace),
                0.,
                flag(c.screen_only),
            ],
        }
    }

    /// The composite phase for `tick`, and the field it scans when interlaced. `blending` moves
    /// the two phases towards each other.
    fn tick(&mut self, phase: Phase, blending: f32, tick: u64) {
        let (a, b) = (blending, 1. - blending);
        self.signal[3] = match phase {
            Phase::Stable => 0.5,
            Phase::A => a,
            Phase::B => b,
            Phase::Alternating if tick.is_multiple_of(2) => a,
            Phase::Alternating => b,
        };
        self.processing[2] = (tick % 2) as f32;
    }
}

/// The format a color mode draws the glass, lighting and bloom in.
fn surface_format(mode: ColorMode) -> wgpu::TextureFormat {
    match mode {
        ColorMode::Reference => FORMAT,
        ColorMode::LinearLight => wgpu::TextureFormat::Rgba16Float,
    }
}

/// The render pipelines. The composite and present passes write 8-bit targets in either color
/// mode; the surface passes write the mode's own format.
struct Pipelines {
    composite: wgpu::RenderPipeline,
    present: wgpu::RenderPipeline,
    reference: SurfacePasses,
    linear: SurfacePasses,
}

/// What follows the signal: the curved glass and its bezel, then the bloom's two blurs.
struct SurfacePasses {
    surface: wgpu::RenderPipeline,
    downsample: wgpu::RenderPipeline,
    upsample: wgpu::RenderPipeline,
}

impl Pipelines {
    fn surface(&self, mode: ColorMode) -> &SurfacePasses {
        match mode {
            ColorMode::Reference => &self.reference,
            ColorMode::LinearLight => &self.linear,
        }
    }
}

/// The targets one sequence renders through, made for its sizes and color mode, with the bind
/// groups its passes use.
pub(crate) struct Workspace {
    /// The prepared signal: the source resized, edited and graded, before the simulation.
    source: Target,
    /// The simulated signal's feedback: each tick reads one and writes the other.
    history: [Target; 2],
    full: Target,
    down: Target,
    up: Target,
    final_target: Target,
    /// Made on the first read back, which a frame only shown on the device never needs.
    readback: Option<Readback>,
    prepare: gpu_prepare::Cache,
    uniforms: Uniforms,
    bindings: Bindings,
    signal_size: (u32, u32),
    output_size: (u32, u32),
    surface_format: wgpu::TextureFormat,
}

impl Workspace {
    fn matches(&self, plan: &Plan) -> bool {
        self.signal_size == plan.signal
            && self.output_size == plan.output
            && self.surface_format == plan.surface_format
    }
}

/// Ticks encoded before each submission, which is also where a frame reports progress and
/// notices a cancel.
const BATCH: u32 = 8;

/// A workspace's settings, one slot per tick of a batch and one for the surface passes, each
/// reached by dynamic offset. `write_buffer` takes effect when the batch is submitted, so every
/// tick needs a slot of its own: sharing one would give them all the last tick's settings.
struct Uniforms {
    buffer: wgpu::Buffer,
    stride: u32,
}

/// The surface passes' slot, after the ticks'.
const SURFACE_SLOT: u32 = BATCH;

impl Uniforms {
    fn new(device: &wgpu::Device) -> Self {
        let alignment = device.limits().min_uniform_buffer_offset_alignment;
        let stride = (std::mem::size_of::<Params>() as u32).div_ceil(alignment) * alignment;
        Self {
            buffer: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("settings"),
                size: u64::from(stride * (SURFACE_SLOT + 1)),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            stride,
        }
    }

    /// Puts `params` in `slot` for the next submission, and returns the slot's offset.
    fn write(&self, queue: &wgpu::Queue, slot: u32, params: &Params) -> u32 {
        let offset = slot * self.stride;
        queue.write_buffer(&self.buffer, offset.into(), bytemuck::bytes_of(params));
        offset
    }
}

/// The bind groups of a workspace's passes, made once with it.
struct Bindings {
    /// By the history target a tick writes; it reads the other.
    composite: [wgpu::BindGroup; 2],
    /// By the history target holding the signal the glass shows.
    glass: [wgpu::BindGroup; 2],
    downsample: wgpu::BindGroup,
    upsample: wgpu::BindGroup,
    present: wgpu::BindGroup,
}

/// What a render of one input with one configuration makes, checked against the device.
struct Plan {
    signal: (u32, u32),
    output: (u32, u32),
    surface_format: wgpu::TextureFormat,
    /// Whether to prepare the signal on the device rather than on this thread.
    prepare_on_gpu: bool,
}

/// One reusable GPU device, which renders sequences of frames.
pub struct Renderer {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    layout: wgpu::BindGroupLayout,
    /// In binding order: point and linear filtering clamped to the edge, then repeating.
    samplers: [wgpu::Sampler; 4],
    pipelines: Pipelines,
    /// The bezel as the surface pass traces it, in the order `bezel::Maps` lists them.
    bezel: [Target; 3],
    artifacts: Target,
    mask: Target,
    prepare_pipelines: gpu_prepare::Pipelines,
    /// Where the prepare step runs. The GPU unless a caller asks otherwise, to compare the two.
    pub prepare: PrepareOn,
    pub adapter: wgpu::AdapterInfo,
}

/// The images a frame was made from, for inspection and tests.
pub struct Signals {
    /// The prepared signal: the source resized, edited and graded, before the simulation.
    pub clean: RgbaImage,
    /// The simulated signal, before the glass.
    pub signal: RgbaImage,
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

/// Which history target tick `tick` writes. It reads the other, which the tick before wrote.
fn written(tick: u64) -> usize {
    (tick % 2) as usize
}

/// How far a render has got.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderProgress {
    /// Completed, weighted stages; not an estimate of elapsed time.
    pub fraction: f32,
    pub stage: Stage,
}

/// What a render is doing. Callers put it into words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// Resizing, editing and grading the source into the signal.
    Preparing,
    /// Simulating the signal: a new sequence's warm-up, then the frame's own tick.
    Simulating { done: u32, of: u32 },
    /// Reading the finished frame back from the device.
    Reading,
    /// The frame is finished and read.
    Done,
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
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
                apply_limit_buckets: false,
            })
            .await
            .context(
                "No compatible graphics adapter. Install a Vulkan/DX12/Metal driver; no window \
                 is required.",
            )?;
        let info = adapter.get_info();
        let limits = Self::limits(&adapter);
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("CRTSim"),
                required_features: wgpu::Features::empty(),
                required_limits: limits,
                ..Default::default()
            })
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
                has_dynamic_offset: true,
                min_binding_size: PARAMS_SIZE,
            },
            count: None,
        }];
        for binding in (1..=4).chain(9..=11) {
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
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let pipelines = Self::pipelines(&device, &layout);
        let prepare_pipelines = gpu_prepare::Pipelines::new(&device, &queue);
        if let Some(error) = scope.pop().await {
            anyhow::bail!("shader/pipeline validation: {error}");
        }
        let sampler = |address, filter| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                address_mode_u: address,
                address_mode_v: address,
                mag_filter: filter,
                min_filter: filter,
                mipmap_filter: wgpu::MipmapFilterMode::Linear,
                ..Default::default()
            })
        };
        use wgpu::{AddressMode, FilterMode};
        let samplers = [
            sampler(AddressMode::ClampToEdge, FilterMode::Nearest),
            sampler(AddressMode::ClampToEdge, FilterMode::Linear),
            sampler(AddressMode::Repeat, FilterMode::Nearest),
            sampler(AddressMode::Repeat, FilterMode::Linear),
        ];
        Ok(Self {
            artifacts: gpu::artifacts(&device, &queue)?,
            mask: gpu::shadow_mask(&device, &queue)?,
            bezel: {
                let maps = bezel::maps()?;
                [
                    ("bezel shape", &maps.shape),
                    ("bezel uv", &maps.uv),
                    ("bezel normal", &maps.normal),
                ]
                .map(|(name, image)| {
                    let target = Target::new(&device, name, image.dimensions());
                    target.upload(&queue, image);
                    target
                })
            },
            device,
            queue,
            layout,
            samplers,
            pipelines,
            prepare_pipelines,
            prepare: PrepareOn::default(),
            adapter: info,
        })
    }

    /// Call inside a validation error scope, so a shader error surfaces there.
    fn pipelines(device: &wgpu::Device, layout: &wgpu::BindGroupLayout) -> Pipelines {
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(layout)],
            immediate_size: 0,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("CRT WGSL"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        // Every pass covers its target with one triangle; the glass and bezel are ray-traced.
        let pipeline = |entry: &str, format| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("quad"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let surface = |mode| {
            let format = surface_format(mode);
            SurfacePasses {
                surface: pipeline("surface", format),
                downsample: pipeline("downsample", format),
                upsample: pipeline("upsample", format),
            }
        };
        Pipelines {
            composite: pipeline("composite", FORMAT),
            present: pipeline("present", FORMAT),
            reference: surface(ColorMode::Reference),
            linear: surface(ColorMode::LinearLight),
        }
    }

    /// Renders `sequence`'s next frame of `input`, which stays on the device until it is read
    /// or shown. The sequence's first frame starts from cleared history and warms up; later
    /// ones carry its history and phase on. `cancel` is checked between batches of ticks.
    pub async fn frame(
        &self,
        sequence: &mut Sequence,
        input: &RgbaImage,
        config: &Config,
        cancel: Option<&AtomicBool>,
        mut progress: impl FnMut(RenderProgress),
    ) -> Result<()> {
        let cancelled = || cancel.is_some_and(|cancel| cancel.load(Ordering::Relaxed));
        ensure!(!cancelled(), "Render cancelled");
        let mut report = |fraction, stage| progress(RenderProgress { fraction, stage });
        report(0., Stage::Preparing);
        let c = &sequence.timed(config);
        let plan = self.plan(input, c)?;
        let clean = if plan.prepare_on_gpu {
            None
        } else {
            Some(config::prepare(input, c)?)
        };
        let workspace = sequence
            .workspace
            .get_or_insert_with(|| self.workspace(&plan));
        ensure!(
            workspace.matches(&plan),
            "Start a new video sequence after changing signal size, output size or color mode"
        );
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame"),
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
        let mut params = Params::new(c, plan.signal, plan.output);
        // A sequence starts from cleared history, then warms up.
        let first = sequence.tick == 0;
        if first {
            for target in &workspace.history {
                gpu::clear(&mut encoder, target);
            }
        }
        let ticks = if first { c.warmup + 1 } else { 1 };
        report(0.05, Stage::Simulating { done: 0, of: ticks });
        for step in 0..ticks {
            ensure!(!cancelled(), "Render cancelled");
            let tick = sequence.tick;
            params.tick(c.phase, c.ntsc_blending, tick);
            let offset = workspace.uniforms.write(&self.queue, step % BATCH, &params);
            gpu::fullscreen(
                &mut encoder,
                &workspace.history[written(tick)],
                &self.pipelines.composite,
                &workspace.bindings.composite[written(tick)],
                &[offset],
            );
            sequence.tick += 1;
            // Report completed GPU work, not just command encoding. Small batches keep overhead bounded.
            if (step + 1) % BATCH == 0 || step + 1 == ticks {
                self.queue.submit(Some(encoder.finish()));
                gpu::finished(&self.device, &self.queue).await?;
                gpu::yield_now().await;
                report(
                    0.05 + 0.75 * (step + 1) as f32 / ticks as f32,
                    Stage::Simulating {
                        done: step + 1,
                        of: ticks,
                    },
                );
                encoder = self.device.create_command_encoder(&Default::default());
            }
        }
        self.surface(
            &mut encoder,
            workspace,
            written(sequence.tick - 1),
            &params,
            c,
        );
        self.queue.submit(Some(encoder.finish()));
        ensure!(!cancelled(), "Render cancelled");
        Ok(())
    }

    /// The pixels of `sequence`'s latest frame.
    pub async fn read(&self, sequence: &mut Sequence) -> Result<RgbaImage> {
        let workspace = sequence
            .workspace
            .as_mut()
            .context("Render a frame before reading it")?;
        let size = workspace.output_size;
        workspace
            .readback
            .get_or_insert_with(|| Readback::new(&self.device, size))
            .read(&self.device, &self.queue, &workspace.final_target)
            .await
    }

    /// `sequence`'s latest frame, copied on the device into a texture of its own for drawing
    /// there. A preview wants this rather than a read back.
    pub fn show(&self, sequence: &Sequence) -> Result<PreviewFrame> {
        let workspace = sequence
            .workspace
            .as_ref()
            .context("Render a frame before showing it")?;
        Ok(self.copy_out(&workspace.final_target))
    }

    /// The prepared and simulated signals behind `sequence`'s latest frame.
    pub async fn signals(&self, sequence: &Sequence) -> Result<Signals> {
        let workspace = sequence
            .workspace
            .as_ref()
            .context("Render a frame before reading its signals")?;
        Ok(Signals {
            clean: self.read_target(&workspace.source).await?,
            signal: self
                .read_target(&workspace.history[written(sequence.tick - 1)])
                .await?,
        })
    }

    async fn read_target(&self, target: &Target) -> Result<RgbaImage> {
        Readback::new(&self.device, target.size())
            .read(&self.device, &self.queue, target)
            .await
    }

    /// A still: one frame from cleared history after warm-up, read back.
    pub async fn still(
        &self,
        input: &RgbaImage,
        config: &Config,
        cancel: Option<&AtomicBool>,
        mut progress: impl FnMut(RenderProgress),
    ) -> Result<RgbaImage> {
        let mut sequence = Sequence::still();
        self.frame(&mut sequence, input, config, cancel, &mut progress)
            .await?;
        progress(RenderProgress {
            fraction: 0.9,
            stage: Stage::Reading,
        });
        let image = self.read(&mut sequence).await?;
        progress(RenderProgress {
            fraction: 1.,
            stage: Stage::Done,
        });
        Ok(image)
    }

    /// The targets for `plan`, and the bind groups their passes use.
    fn workspace(&self, plan: &Plan) -> Workspace {
        let device = &self.device;
        let (signal_size, output_size) = (plan.signal, plan.output);
        let surface = |name, size| Target::with_format(device, name, size, plan.surface_format, 1);
        let source = Target::new(device, "clean signal", signal_size);
        let history = [
            Target::new(device, "history A", signal_size),
            Target::new(device, "history B", signal_size),
        ];
        let full = surface("screen and bezel", output_size);
        let down = surface(
            "bloom downsample",
            ((output_size.0 / 16).max(1), (output_size.1 / 16).max(1)),
        );
        let up = surface("bloom upsample", output_size);
        let uniforms = Uniforms::new(device);
        let bind = |a, b| self.bind(&uniforms.buffer, a, b);
        let bindings = Bindings {
            composite: [bind(&source, &history[1]), bind(&source, &history[0])],
            glass: [bind(&history[0], &source), bind(&history[1], &source)],
            downsample: bind(&full, &source),
            upsample: bind(&down, &source),
            present: bind(&full, &up),
        };
        Workspace {
            final_target: Target::new(device, "output", output_size),
            readback: None,
            prepare: Default::default(),
            uniforms,
            bindings,
            source,
            history,
            full,
            down,
            up,
            signal_size,
            output_size,
            surface_format: plan.surface_format,
        }
    }

    fn plan(&self, input: &RgbaImage, c: &Config) -> Result<Plan> {
        c.validate()?;
        let signal = c.signal_size(input.dimensions())?;
        let output = c.output_size(input.dimensions())?;
        let limit = self.device.limits().max_texture_dimension_2d;
        ensure!(
            [signal.0, signal.1, output.0, output.1]
                .iter()
                .all(|v| *v <= limit),
            "image exceeds adapter texture limit {limit}"
        );
        // Conservative working-set guard: color targets, depth, readback and history.
        const BUDGET: u64 = 1_500_000_000;
        let linear = c.color_mode == ColorMode::LinearLight;
        let estimated = u64::from(output.0) * u64::from(output.1) * if linear { 48 } else { 24 }
            + u64::from(signal.0) * u64::from(signal.1) * 16;
        ensure!(
            estimated <= BUDGET,
            "estimated working set exceeds Phase 0 budget; choose a smaller preset"
        );
        // Preparing on the device puts the input there as it is, plus the resize's intermediate.
        // Where that does not fit -- a 16384-pixel image on a device limited to 8192, or a huge
        // input kept at native size -- the render is still accepted, as it always was, by
        // preparing here instead.
        let (width, height) = input.dimensions();
        let prepare_on_gpu = self.prepare == PrepareOn::Gpu
            && width <= limit
            && height <= limit
            && estimated + u64::from(width) * (u64::from(height) * 4 + u64::from(signal.1) * 16)
                <= BUDGET;
        // In a browser, preparing here would hold the page's only thread for seconds.
        #[cfg(target_arch = "wasm32")]
        ensure!(
            prepare_on_gpu || self.prepare == PrepareOn::Cpu,
            "This image is larger than this browser's graphics allow ({limit} pixels a side, \
             within {} MB). The desktop app can open it.",
            BUDGET / 1_000_000
        );
        Ok(Plan {
            signal,
            output,
            surface_format: surface_format(c.color_mode),
            prepare_on_gpu,
        })
    }

    /// The curved glass and its bezel over the simulated signal in history target `latest`,
    /// then bloom, into the final target.
    fn surface(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        workspace: &Workspace,
        latest: usize,
        params: &Params,
        c: &Config,
    ) {
        let offset = workspace.uniforms.write(&self.queue, SURFACE_SLOT, params);
        let passes = self.pipelines.surface(c.color_mode);
        let bindings = &workspace.bindings;
        gpu::fullscreen(
            encoder,
            &workspace.full,
            &passes.surface,
            &bindings.glass[latest],
            &[offset],
        );
        gpu::fullscreen(
            encoder,
            &workspace.down,
            &passes.downsample,
            &bindings.downsample,
            &[offset],
        );
        gpu::fullscreen(
            encoder,
            &workspace.up,
            &passes.upsample,
            &bindings.upsample,
            &[offset],
        );
        gpu::fullscreen(
            encoder,
            &workspace.final_target,
            &self.pipelines.present,
            &bindings.present,
            &[offset],
        );
    }

    /// A bind group reading `source` and `previous`, with the settings slot the pass chooses
    /// by dynamic offset.
    fn bind(&self, uniforms: &wgpu::Buffer, source: &Target, previous: &Target) -> wgpu::BindGroup {
        let mut entries = vec![wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                buffer: uniforms,
                offset: 0,
                size: PARAMS_SIZE,
            }),
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
        for (i, t) in self.bezel.iter().enumerate() {
            entries.push(wgpu::BindGroupEntry {
                binding: i as u32 + 9,
                resource: wgpu::BindingResource::TextureView(&t.view),
            });
        }
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.layout,
            entries: &entries,
        })
    }

    /// The final target is reused by the next render, so the frame is copied out rather than
    /// handed over: a blit on the device, not a round trip through system memory. The copy
    /// keeps the target's gamma-encoded `Rgba8Unorm`, the format egui requires of a texture it
    /// is given.
    fn copy_out(&self, final_target: &Target) -> PreviewFrame {
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("preview frame"),
            size: gpu::extent(final_target.width, final_target.height),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_texture(
            final_target.texture.as_image_copy(),
            texture.as_image_copy(),
            gpu::extent(final_target.width, final_target.height),
        );
        self.queue.submit(Some(encoder.finish()));
        PreviewFrame {
            texture,
            width: final_target.width,
            height: final_target.height,
        }
    }
}
