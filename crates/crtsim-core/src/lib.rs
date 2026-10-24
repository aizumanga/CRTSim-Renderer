#![doc = include_str!("../README.md")]

pub mod bezel;
pub mod color;
pub mod config;
pub mod game_palette;
pub mod glass;
mod gpu;
mod gpu_prepare;
pub mod input;
pub mod mesh;
pub mod nes_luts;
pub mod palette;
mod params;
pub mod presets;
mod sequence;
pub mod settings;
pub mod sheet;
pub mod test_clip;
#[cfg(test)]
mod tests;
pub mod workflow;

/// The original's two textures as images: the composite artifact pattern, and the shadow mask
/// at full size. Shader ports carry them as files, to read what the renderer reads.
pub mod textures {
    pub use crate::gpu::{artifacts_image as artifacts, mask_image as shadow_mask};
}

use anyhow::{ensure, Context, Result};
use config::{ColorMode, Config};
use gpu::{Readback, Target, FORMAT};
use image::RgbaImage;
use params::{Params, Uniforms, BATCH, PARAMS_SIZE, SURFACE_SLOT};
use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

pub use gpu_prepare::PrepareOn;
use sequence::Follow;
pub use sequence::{Sequence, Shape, Timing};

pub const SHADER: &str = include_str!("../shaders/crtsim.wgsl");

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
    /// Kept to make the pass that hands a frame to a host, for each format it asks for.
    module: wgpu::ShaderModule,
    layout: wgpu::PipelineLayout,
}

/// What follows the signal: the curved glass and its bezel, the mipmaps the bloom may read,
/// then the bloom's two blurs.
struct SurfacePasses {
    surface: wgpu::RenderPipeline,
    /// The picture flat in its place, with the CRT off.
    flat: wgpu::RenderPipeline,
    halve: wgpu::RenderPipeline,
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
    /// Each mip level of the glass and bezel's picture alone, its own size first: the picture is
    /// drawn into the first, and each level after from the one before. The bloom and present
    /// passes read the whole chain.
    full_levels: Vec<wgpu::TextureView>,
    down: Target,
    up: Target,
    final_target: Target,
    /// Made on the first read back, which a frame only shown on the device never needs.
    readback: Option<Readback>,
    prepare: gpu_prepare::Cache,
    uniforms: Uniforms,
    bindings: Bindings,
    /// What its targets are made for.
    shape: Shape,
}

/// The mip levels of the glass and bezel's picture of `size` that the bloom may read: down to the
/// level below a sixteenth, between which a target a sixteenth its size reads, or as many as
/// the size has.
fn bloom_levels((width, height): (u32, u32)) -> u32 {
    (u32::BITS - width.max(height).leading_zeros()).min(6)
}

/// The most memory a render's targets may take, as `Renderer::fits` estimates them.
const BUDGET: u64 = 1_500_000_000;

/// The bind groups of a workspace's passes, made once with it.
struct Bindings {
    /// By the history target a tick writes; it reads the other.
    composite: [wgpu::BindGroup; 2],
    /// By the history target holding the signal the glass shows.
    glass: [wgpu::BindGroup; 2],
    /// The clean signal, drawn flat with the CRT off.
    flat: wgpu::BindGroup,
    /// By the mip level drawn, from the second: each reads the level before.
    halve: Vec<wgpu::BindGroup>,
    downsample: wgpu::BindGroup,
    upsample: wgpu::BindGroup,
    present: wgpu::BindGroup,
}

/// What a render of one input with one configuration makes, checked against the device.
struct Plan {
    shape: Shape,
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
    /// The glass's shade and outline, baked from its mesh.
    glass: Target,
    artifacts: Target,
    mask: Target,
    prepare_pipelines: gpu_prepare::Pipelines,
    /// What a frame recorded for a host is drawn into its target with, made for each format
    /// a host asks for.
    deliver: Mutex<HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>>,
    /// Where the prepare step runs. The GPU unless a caller asks otherwise, to compare the two.
    pub prepare: PrepareOn,
    pub adapter: wgpu::AdapterInfo,
}

/// One frame of a host's picture, for `Renderer::encode`: its texture in, its target out, and
/// how long the frame before it was shown.
pub struct HostFrame<'a> {
    /// The host's picture: a 2D texture on the renderer's device, bound for sampling, of a
    /// color format with float samples, such as `Rgba8Unorm`, `Bgra8UnormSrgb` or
    /// `Rgba16Float`. An sRGB or float format holds linear light, as an engine's HDR target
    /// does, so it is read as the colors it encodes; any other holds them encoded.
    pub source: &'a wgpu::Texture,
    /// Where the CRT is drawn, filling it: a 2D texture of a color format with float samples,
    /// bound as a render attachment, such as a window's surface texture. Its size is the
    /// output's, whatever the settings' export size says. An sRGB or float format is given
    /// linear light, the colors it is to encode.
    pub target: &'a wgpu::Texture,
    /// How long the frame before this one was on screen, in seconds, which this one's glow
    /// decays by: a sixtieth for a host running at 60 frames a second.
    pub seconds: f64,
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

/// The settings a frame is drawn with: with the CRT off, no colour table, which belongs to the
/// CRT's signal, as the game draws its picture without one.
fn drawn(mut c: Config) -> Config {
    if c.crt_off {
        c.palette = None;
        c.lut = None;
    }
    c
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
        for binding in (1..=4).chain(9..=12) {
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
            glass: {
                let map = glass::map()?;
                let target = Target::new(&device, "glass", map.dimensions());
                target.upload(&queue, &map);
                target
            },
            device,
            queue,
            layout,
            samplers,
            pipelines,
            prepare_pipelines,
            deliver: Default::default(),
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
        let pipeline = |entry: &str, format| {
            Self::fullscreen_pipeline(device, &pipeline_layout, &module, entry, format)
        };
        let surface = |mode| {
            let format = surface_format(mode);
            SurfacePasses {
                surface: pipeline("surface", format),
                flat: pipeline("flat", format),
                halve: pipeline("halve", format),
                downsample: pipeline("downsample", format),
                upsample: pipeline("upsample", format),
            }
        };
        Pipelines {
            composite: pipeline("composite", FORMAT),
            present: pipeline("present", FORMAT),
            reference: surface(ColorMode::Reference),
            linear: surface(ColorMode::LinearLight),
            module,
            layout: pipeline_layout,
        }
    }

    /// A pass of `entry` writing `format`, covering its target with one triangle: every pass
    /// is one, the glass and bezel being ray-traced.
    fn fullscreen_pipeline(
        device: &wgpu::Device,
        layout: &wgpu::PipelineLayout,
        module: &wgpu::ShaderModule,
        entry: &str,
        format: wgpu::TextureFormat,
    ) -> wgpu::RenderPipeline {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(entry),
            layout: Some(layout),
            vertex: wgpu::VertexState {
                module,
                entry_point: Some("quad"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module,
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
        config.validate()?;
        let c = &drawn(sequence.timed(config));
        let plan = self.plan(input, c)?;
        let clean = if plan.prepare_on_gpu {
            None
        } else {
            Some(config::prepare(input, c)?)
        };
        self.follow(sequence, plan.shape)?;
        let ticks = sequence.ticks(c.warmup);
        let clocks = sequence.clocks(ticks);
        // A still drawn as the game's screenshots ends on a tick of both patterns averaged.
        let screenshot =
            sequence.still_frame() && c.screenshot_stills && c.phase == config::Phase::Alternating;
        let workspace = sequence.workspace.as_mut().expect("made by follow");
        self.finish_at(workspace, c.tate.turned(plan.shape.output));
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
                gpu_prepare::Picture::Image(input),
                c,
                &workspace.source,
            ),
        }
        let mut params = Params::new(c, plan.shape.signal, plan.shape.output);
        report(0.05, Stage::Simulating { done: 0, of: ticks });
        for step in 0..ticks {
            ensure!(!cancelled(), "Render cancelled");
            let phase = if screenshot && step + 1 == ticks {
                config::Phase::Stable
            } else {
                c.phase
            };
            let tick = sequence.tick;
            params.tick(phase, c.ntsc_blending, tick, clocks[step as usize]);
            // With the CRT off nothing glows: the picture is drawn flat, its static still ticking.
            if !c.crt_off {
                self.composite(&mut encoder, workspace, &params, tick, step % BATCH);
            }
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
        let size = workspace.final_target.size();
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

    /// Records `sequence`'s next frame into a host's `encoder`, from the host's picture into
    /// its target, both already on this renderer's device: the way a game, an emulator or an
    /// engine draws the CRT over its own frames. It submits nothing and waits for nothing;
    /// the frame is drawn when the host submits the encoder.
    ///
    /// `sequence` must be a host's, from `Sequence::host`: no warm-up, one tick a frame, and
    /// each frame's glow decaying by `frame.seconds`. The target's size is the output's, and a
    /// new one keeps the glow; a new signal size or color mode starts the sequence again.
    ///
    /// The settings reach the device through the queue, as `Queue::write_buffer` writes, so
    /// submit each frame's commands before encoding the sequence's next frame.
    pub fn encode(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        sequence: &mut Sequence,
        frame: &HostFrame,
        config: &Config,
    ) -> Result<()> {
        ensure!(
            sequence.hosted(),
            "Encode a host's sequence, made with Sequence::host"
        );
        let (source, target) = (frame.source, frame.target);
        let float = |texture: &wgpu::Texture| {
            texture.dimension() == wgpu::TextureDimension::D2
                && texture.sample_count() == 1
                && texture.format().has_color_aspect()
                && matches!(
                    texture
                        .format()
                        .sample_type(None, Some(self.device.features())),
                    Some(wgpu::TextureSampleType::Float { .. })
                )
        };
        ensure!(
            float(source)
                && source
                    .usage()
                    .contains(wgpu::TextureUsages::TEXTURE_BINDING),
            "The host's picture must be a 2D color texture with float samples, bound for sampling"
        );
        ensure!(
            float(target)
                && target
                    .usage()
                    .contains(wgpu::TextureUsages::RENDER_ATTACHMENT),
            "The host's target must be a 2D color texture with float samples, bound as a render \
             attachment"
        );
        sequence.lasted(frame.seconds);
        // The settings as given, since a frame of no time holds the glow as it is, which
        // settings could not ask for.
        config.validate()?;
        let c = &drawn(sequence.timed(config));
        let input = (source.width(), source.height());
        let limit = self.device.limits().max_texture_dimension_2d;
        ensure!(
            input.0 <= limit && input.1 <= limit,
            "the host's picture exceeds adapter texture limit {limit}"
        );
        // The target is the finished picture's size; tate turns the tube's into it.
        let finished = (target.width(), target.height());
        let shape = Shape {
            signal: c.signal_size(input)?,
            output: c.tate.turned(finished),
            color_mode: c.color_mode,
        };
        self.fits(shape)?;
        self.follow(sequence, shape)?;
        let clock = sequence.clocks(1)[0];
        let workspace = sequence.workspace.as_mut().expect("made by follow");
        self.finish_at(workspace, finished);
        self.prepare_pipelines.encode(
            &self.device,
            &self.queue,
            encoder,
            &mut workspace.prepare,
            gpu_prepare::Picture::Texture(source),
            c,
            &workspace.source,
        );
        let tick = sequence.tick;
        let mut params = Params::new(c, shape.signal, shape.output);
        params.tick(c.phase, c.ntsc_blending, tick, clock);
        let offset = if c.crt_off {
            workspace.uniforms.write(&self.queue, 0, &params)
        } else {
            self.composite(encoder, workspace, &params, tick, 0)
        };
        sequence.tick += 1;
        self.surface(encoder, workspace, written(tick), &params, c);
        let format = target.format();
        let mut pipelines = self
            .deliver
            .lock()
            .map_err(|_| anyhow::anyhow!("a frame for a host failed while making its pass"))?;
        let deliver = pipelines.entry(format).or_insert_with(|| {
            let entry = if gpu::holds_linear_light(format) {
                "deliver_srgb"
            } else {
                "deliver"
            };
            Self::fullscreen_pipeline(
                &self.device,
                &self.pipelines.layout,
                &self.pipelines.module,
                entry,
                format,
            )
        });
        let bindings = self.bind(
            &workspace.uniforms.buffer,
            &workspace.final_target.view,
            &workspace.source.view,
        );
        gpu::draw(
            encoder,
            &target.create_view(&Default::default()),
            deliver,
            &bindings,
            &[offset],
        );
        Ok(())
    }

    /// Records tick `tick` of a sequence: the clean signal composited onto the history the tick
    /// before wrote, with the tick's settings, `params`, in uniform slot `slot`, whose offset it
    /// returns. A sequence's first tick clears its history first, so it starts from a dark tube.
    fn composite(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        workspace: &Workspace,
        params: &Params,
        tick: u64,
        slot: u32,
    ) -> u32 {
        if tick == 0 {
            for target in &workspace.history {
                gpu::clear(encoder, target);
            }
        }
        let offset = workspace.uniforms.write(&self.queue, slot, params);
        gpu::fullscreen(
            encoder,
            &workspace.history[written(tick)],
            &self.pipelines.composite,
            &workspace.bindings.composite[written(tick)],
            &[offset],
        );
        offset
    }

    /// Makes `workspace`'s finished picture `size`, which tate may have turned from the tube's.
    fn finish_at(&self, workspace: &mut Workspace, size: (u32, u32)) {
        if workspace.final_target.size() != size {
            workspace.final_target = Target::new(&self.device, "output", size);
            workspace.readback = None;
        }
    }

    /// Makes `sequence` ready for a frame of `shape`, as the sequence says it follows into
    /// one: its workspace kept, drawn at the new output size with its history, or made afresh.
    fn follow(&self, sequence: &mut Sequence, shape: Shape) -> Result<()> {
        let from = sequence.workspace.as_ref().map(|workspace| workspace.shape);
        match from.map_or(Ok(Follow::Same), |from| sequence.follow(&from, &shape))? {
            Follow::Same => {}
            Follow::Reshape => {
                let workspace = sequence
                    .workspace
                    .take()
                    .expect("a sequence that reshapes has one");
                sequence.workspace = Some(self.reshaped(workspace, shape.output));
            }
            Follow::Restart => sequence.restart(),
        }
        sequence
            .workspace
            .get_or_insert_with(|| self.workspace(shape));
        Ok(())
    }

    /// The targets for `shape`, and the bind groups their passes use.
    fn workspace(&self, shape: Shape) -> Workspace {
        let device = &self.device;
        let source = Target::new(device, "clean signal", shape.signal);
        let history = [
            Target::new(device, "history A", shape.signal),
            Target::new(device, "history B", shape.signal),
        ];
        self.around(
            source,
            history,
            Default::default(),
            Uniforms::new(device),
            shape,
        )
    }

    /// `workspace` drawn at another output size, keeping its signal and history, so the glow
    /// goes on where the frames before left it.
    fn reshaped(&self, workspace: Workspace, output: (u32, u32)) -> Workspace {
        let shape = Shape {
            output,
            ..workspace.shape
        };
        self.around(
            workspace.source,
            workspace.history,
            workspace.prepare,
            workspace.uniforms,
            shape,
        )
    }

    /// The output's targets for `shape`, around a signal and its history, and the bind groups
    /// of all their passes.
    fn around(
        &self,
        source: Target,
        history: [Target; 2],
        prepare: gpu_prepare::Cache,
        uniforms: Uniforms,
        shape: Shape,
    ) -> Workspace {
        let device = &self.device;
        let output_size = shape.output;
        let format = surface_format(shape.color_mode);
        let surface = |name, size, levels| Target::with_format(device, name, size, format, levels);
        let full = surface("screen and bezel", output_size, bloom_levels(output_size));
        let full_levels: Vec<_> = (0..full.texture.mip_level_count())
            .map(|level| {
                full.texture.create_view(&wgpu::TextureViewDescriptor {
                    base_mip_level: level,
                    mip_level_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();
        let down = surface(
            "bloom downsample",
            ((output_size.0 / 16).max(1), (output_size.1 / 16).max(1)),
            1,
        );
        let up = surface("bloom upsample", output_size, 1);
        let bind = |a: &Target, b: &Target| self.bind(&uniforms.buffer, &a.view, &b.view);
        let bindings = Bindings {
            composite: [bind(&source, &history[1]), bind(&source, &history[0])],
            glass: [bind(&history[0], &source), bind(&history[1], &source)],
            flat: bind(&source, &source),
            halve: full_levels[..full_levels.len() - 1]
                .iter()
                .map(|above| self.bind(&uniforms.buffer, above, &source.view))
                .collect(),
            downsample: bind(&full, &source),
            upsample: bind(&down, &source),
            present: bind(&full, &up),
        };
        Workspace {
            final_target: Target::new(device, "output", output_size),
            readback: None,
            prepare,
            uniforms,
            bindings,
            source,
            history,
            full_levels,
            down,
            up,
            shape,
        }
    }

    /// How to render `input` with `c`, settings already checked before being timed.
    fn plan(&self, input: &RgbaImage, c: &Config) -> Result<Plan> {
        let shape = c.sequence_shape(input.dimensions())?;
        let estimated = self.fits(shape)?;
        // Preparing on the device puts the input there as it is, plus the resize's intermediate.
        // Where that does not fit -- a 16384-pixel image on a device limited to 8192, or a huge
        // input kept at native size -- the render is still accepted, as it always was, by
        // preparing here instead.
        let (width, height) = input.dimensions();
        let limit = self.device.limits().max_texture_dimension_2d;
        let prepare_on_gpu = self.prepare == PrepareOn::Gpu
            && width <= limit
            && height <= limit
            && estimated
                + u64::from(width) * (u64::from(height) * 4 + u64::from(shape.signal.1) * 16)
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
            shape,
            prepare_on_gpu,
        })
    }

    /// Checks that a sequence of `shape` fits the device, and returns its working set's
    /// estimated size in bytes.
    fn fits(&self, shape: Shape) -> Result<u64> {
        let (signal, output) = (shape.signal, shape.output);
        let limit = self.device.limits().max_texture_dimension_2d;
        ensure!(
            [signal.0, signal.1, output.0, output.1]
                .iter()
                .all(|v| *v <= limit),
            "image exceeds adapter texture limit {limit}"
        );
        // Conservative working-set guard: color targets, the glass's mipmaps, depth, readback and
        // history.
        let linear = shape.color_mode == ColorMode::LinearLight;
        let estimated = u64::from(output.0) * u64::from(output.1) * if linear { 48 } else { 24 }
            + u64::from(signal.0) * u64::from(signal.1) * 16;
        ensure!(
            estimated <= BUDGET,
            "estimated working set exceeds Phase 0 budget; choose a smaller preset"
        );
        Ok(estimated)
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
        if c.crt_off {
            gpu::draw(
                encoder,
                &workspace.full_levels[0],
                &passes.flat,
                &bindings.flat,
                &[offset],
            );
            gpu::fullscreen(
                encoder,
                &workspace.final_target,
                &self.pipelines.present,
                &bindings.present,
                &[offset],
            );
            return;
        }
        gpu::draw(
            encoder,
            &workspace.full_levels[0],
            &passes.surface,
            &bindings.glass[latest],
            &[offset],
        );
        if c.bloom_from_mipmaps {
            for (level, above) in workspace.full_levels[1..].iter().zip(&bindings.halve) {
                gpu::draw(encoder, level, &passes.halve, above, &[offset]);
            }
        }
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
    fn bind(
        &self,
        uniforms: &wgpu::Buffer,
        source: &wgpu::TextureView,
        previous: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        let mut entries = vec![wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                buffer: uniforms,
                offset: 0,
                size: PARAMS_SIZE,
            }),
        }];
        for (i, view) in [source, previous, &self.artifacts.view, &self.mask.view]
            .into_iter()
            .enumerate()
        {
            entries.push(wgpu::BindGroupEntry {
                binding: i as u32 + 1,
                resource: wgpu::BindingResource::TextureView(view),
            });
        }
        for (i, s) in self.samplers.iter().enumerate() {
            entries.push(wgpu::BindGroupEntry {
                binding: i as u32 + 5,
                resource: wgpu::BindingResource::Sampler(s),
            });
        }
        for (i, t) in self.bezel.iter().chain([&self.glass]).enumerate() {
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
