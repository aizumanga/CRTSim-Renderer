pub mod config;
mod gpu;
mod gpu_prepare;
pub mod input;
pub mod mesh;
pub mod nes_luts;
pub mod settings;
#[cfg(test)]
mod tests;
pub mod workflow;

use anyhow::{ensure, Context, Result};
use bytemuck::{Pod, Zeroable};
use config::{ColorMode, Config, Phase};
use gpu::{GpuMesh, Readback, Target, FORMAT};
use image::RgbaImage;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use wgpu::util::DeviceExt;

pub use gpu_prepare::PrepareOn;

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
    frame: [f32; 4],
    light: [f32; 4],
    camera: [f32; 4],
    bloom: [f32; 4],
    processing: [f32; 4],
}

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
                flag(c.mask_antialias),
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
                flag(c.color_mode == ColorMode::LinearLight),
                flag(c.interlace),
                0.,
                0.,
            ],
        }
    }

    /// The composite phase for `tick`, and the field it scans when interlaced.
    fn tick(&mut self, phase: Phase, tick: u64) {
        self.signal[3] = match phase {
            Phase::Stable => 0.5,
            Phase::A => 0.,
            Phase::B => 1.,
            Phase::Alternating => (tick % 2) as f32,
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
    screen: wgpu::RenderPipeline,
    frame: wgpu::RenderPipeline,
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

/// The targets one sequence renders through, made for its sizes and color mode.
struct Workspace {
    /// The prepared signal: the source resized, edited and graded, before the simulation.
    source: Target,
    /// The simulated signal's feedback: each tick reads one and writes the other.
    history: [Target; 2],
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
        let depth = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("depth"),
            size: gpu::extent(output_size.0, output_size.1),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let depth_view = depth.create_view(&Default::default());
        let surface = |name, size| Target::with_format(device, name, size, surface_format, 1);
        Self {
            source: Target::new(device, "clean signal", signal_size),
            history: [
                Target::new(device, "history A", signal_size),
                Target::new(device, "history B", signal_size),
            ],
            full: surface("screen and frame", output_size),
            down: surface(
                "bloom downsample",
                ((output_size.0 / 16).max(1), (output_size.1 / 16).max(1)),
            ),
            up: surface("bloom upsample", output_size),
            final_target: Target::new(device, "output", output_size),
            _depth: depth,
            depth_view,
            readback: Readback::new(device, output_size),
            prepare: Default::default(),
            signal_size,
            output_size,
            surface_format,
        }
    }

    fn matches(&self, plan: &Plan) -> bool {
        self.signal_size == plan.signal
            && self.output_size == plan.output
            && self.surface_format == plan.surface_format
    }
}

/// What a render of one input with one configuration makes, checked against the device.
struct Plan {
    signal: (u32, u32),
    output: (u32, u32),
    surface_format: wgpu::TextureFormat,
    /// Whether to prepare the signal on the device rather than on this thread.
    prepare_on_gpu: bool,
}

/// One reusable GPU device; individual still jobs own and reset their history.
pub struct Renderer {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    layout: wgpu::BindGroupLayout,
    /// In binding order: point and linear filtering clamped to the edge, then repeating.
    samplers: [wgpu::Sampler; 4],
    pipelines: Pipelines,
    screen: GpuMesh,
    frame: GpuMesh,
    artifacts: Target,
    mask: Target,
    prepare_pipelines: gpu_prepare::Pipelines,
    /// Where the prepare step runs. The GPU unless a caller asks otherwise, to compare the two.
    pub prepare: PrepareOn,
    pub adapter: wgpu::AdapterInfo,
}

/// A still with the images it was made from, for inspection and tests.
pub struct Rendered {
    /// The prepared signal: the source resized, edited and graded, before the simulation.
    pub clean: RgbaImage,
    /// The simulated signal, before the glass.
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

/// Feedback belongs to a single sequence on this renderer. Start a new sequence after seeking
/// or changing settings. Reuse it only with the same device and signal dimensions.
#[derive(Default)]
pub struct Sequence {
    workspace: Option<Workspace>,
    tick: u64,
}

/// Which history target tick `tick` writes. It reads the other, which the tick before wrote.
fn written(tick: u64) -> usize {
    (tick % 2) as usize
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
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
            })
            .await
            .context(
                "No compatible graphics adapter. Install a Vulkan/DX12/Metal driver; no window \
                 is required.",
            )?;
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
        device.push_error_scope(wgpu::ErrorFilter::Validation);
        let pipelines = Self::pipelines(&device, &layout);
        let prepare_pipelines = gpu_prepare::Pipelines::new(&device, &queue);
        if let Some(error) = device.pop_error_scope().await {
            anyhow::bail!("shader/pipeline validation: {error}");
        }
        let sampler = |address, filter| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                address_mode_u: address,
                address_mode_v: address,
                mag_filter: filter,
                min_filter: filter,
                mipmap_filter: wgpu::FilterMode::Linear,
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
            screen: GpuMesh::new(&device, mesh::SCREEN)?,
            frame: GpuMesh::new(&device, mesh::FRAME)?,
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
            bind_group_layouts: &[layout],
            push_constant_ranges: &[],
        });
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
        let pipeline = |entry: &str, format| {
            // The screen and bezel are meshes, depth-tested; everything else covers its target.
            let mesh = entry == "screen" || entry == "frame";
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: if mesh { "mesh" } else { "quad" },
                    buffers: if mesh { &vertex_layout } else { &[] },
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: entry,
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: mesh.then(|| wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: true,
                    depth_compare: wgpu::CompareFunction::Less,
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                multiview: None,
            })
        };
        let surface = |mode| {
            let format = surface_format(mode);
            SurfacePasses {
                screen: pipeline("screen", format),
                frame: pipeline("frame", format),
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

    /// A deterministic still from cleared history, with the prepared and simulated signals it
    /// was made from; warmup=0 means one tick. An export wants only the result, from
    /// `render_frame`.
    pub fn render(&self, input: &RgbaImage, c: &Config) -> Result<Rendered> {
        let mut sequence = Sequence::default();
        let clean = self.run(input, c, &mut sequence, None, &mut |_| {})?;
        let latest = written(sequence.tick - 1);
        let workspace = sequence
            .workspace
            .as_mut()
            .context("render made no workspace")?;
        let signal = self.readback(&workspace.history[latest])?;
        let clean = match clean {
            Some(clean) => clean,
            None => self.readback(&workspace.source)?,
        };
        let crt = workspace
            .readback
            .read(&self.device, &self.queue, &workspace.final_target)?;
        Ok(Rendered { clean, signal, crt })
    }

    /// The next frame of `sequence`, read back. A new sequence makes a still: history starts
    /// cleared and warm-up runs first; after that, history and phase carry on frame to frame.
    /// `cancel` is checked between bounded GPU batches.
    pub fn render_frame(
        &self,
        input: &RgbaImage,
        c: &Config,
        sequence: &mut Sequence,
        cancel: Option<&AtomicBool>,
        mut progress: impl FnMut(RenderProgress),
    ) -> Result<RgbaImage> {
        self.run(input, c, sequence, cancel, &mut progress)?;
        progress(RenderProgress {
            fraction: 0.9,
            stage: "Glass, lighting and bloom complete; reading pixels".into(),
        });
        let workspace = sequence
            .workspace
            .as_mut()
            .context("render made no workspace")?;
        let crt = workspace
            .readback
            .read(&self.device, &self.queue, &workspace.final_target)?;
        progress(RenderProgress {
            fraction: 1.,
            stage: "Render complete".into(),
        });
        Ok(crt)
    }

    /// A still for display on this renderer's own device, skipping the read back to system
    /// memory that a saved or encoded frame needs.
    pub fn render_preview(&self, input: &RgbaImage, c: &Config) -> Result<PreviewFrame> {
        let mut sequence = Sequence::default();
        self.run(input, c, &mut sequence, None, &mut |_| {})?;
        let workspace = sequence
            .workspace
            .as_ref()
            .context("render made no workspace")?;
        Ok(self.copy_out(&workspace.final_target))
    }

    /// Everything up to the final image, which stays in the sequence's workspace. Returns the
    /// prepared signal when it was prepared on this thread rather than on the device.
    fn run(
        &self,
        input: &RgbaImage,
        c: &Config,
        sequence: &mut Sequence,
        cancel: Option<&AtomicBool>,
        progress: &mut dyn FnMut(RenderProgress),
    ) -> Result<Option<RgbaImage>> {
        let cancelled = || cancel.is_some_and(|cancel| cancel.load(Ordering::Relaxed));
        ensure!(!cancelled(), "Render cancelled");
        let mut report = |fraction, stage: String| progress(RenderProgress { fraction, stage });
        report(0., "Preparing image".into());
        let plan = self.plan(input, c)?;
        let clean = if plan.prepare_on_gpu {
            None
        } else {
            Some(config::prepare(input, c)?)
        };
        let workspace = sequence.workspace.get_or_insert_with(|| {
            Workspace::new(&self.device, plan.signal, plan.output, plan.surface_format)
        });
        ensure!(
            workspace.matches(&plan),
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
        let mut params = Params::new(c, plan.signal, plan.output);
        // A sequence starts from cleared history, then warms up; each job is independent.
        let first = sequence.tick == 0;
        if first {
            for target in &workspace.history {
                gpu::clear(&mut encoder, target);
            }
        }
        // Separate immutable uniform per tick avoids queue.write_buffer ordering bugs.
        let ticks = if first { c.warmup + 1 } else { 1 };
        for step in 0..ticks {
            ensure!(!cancelled(), "Render cancelled");
            let tick = sequence.tick;
            params.tick(c.phase, tick);
            let uniform = self.uniform(&params, "tick settings");
            let bindings = self.bind(
                &uniform,
                &workspace.source,
                &workspace.history[written(tick + 1)],
            );
            gpu::fullscreen(
                &mut encoder,
                &workspace.history[written(tick)],
                &self.pipelines.composite,
                &bindings,
            );
            sequence.tick += 1;
            // Report completed GPU work, not just command encoding. Small batches keep overhead bounded.
            if (step + 1) % 8 == 0 || step + 1 == ticks {
                self.queue.submit(Some(encoder.finish()));
                self.device.poll(wgpu::Maintain::Wait);
                report(
                    0.05 + 0.75 * (step + 1) as f32 / ticks as f32,
                    format!("Simulation {}/{}", step + 1, ticks),
                );
                encoder = self.device.create_command_encoder(&Default::default());
            }
        }
        let signal = &workspace.history[written(sequence.tick - 1)];
        self.surface(&mut encoder, workspace, signal, &params, c);
        self.queue.submit(Some(encoder.finish()));
        ensure!(!cancelled(), "Render cancelled");
        Ok(clean)
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
        Ok(Plan {
            signal,
            output,
            surface_format: surface_format(c.color_mode),
            prepare_on_gpu,
        })
    }

    /// The curved glass and its bezel over the simulated signal, then bloom, into the final
    /// target.
    fn surface(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        workspace: &Workspace,
        signal: &Target,
        params: &Params,
        c: &Config,
    ) {
        let uniform = self.uniform(params, "surface settings");
        let passes = self.pipelines.surface(c.color_mode);
        let bindings = self.bind(&uniform, signal, &workspace.source);
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("curved glass and frame"),
                color_attachments: &[gpu::cleared(&workspace.full.view)],
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
            draw(&mut pass, &passes.screen, &self.screen);
            if !c.screen_only {
                draw(&mut pass, &passes.frame, &self.frame);
            }
        }
        gpu::fullscreen(
            encoder,
            &workspace.down,
            &passes.downsample,
            &self.bind(&uniform, &workspace.full, &workspace.source),
        );
        gpu::fullscreen(
            encoder,
            &workspace.up,
            &passes.upsample,
            &self.bind(&uniform, &workspace.down, &workspace.source),
        );
        gpu::fullscreen(
            encoder,
            &workspace.final_target,
            &self.pipelines.present,
            &self.bind(&uniform, &workspace.full, &workspace.up),
        );
    }

    fn uniform(&self, params: &Params, label: &str) -> wgpu::Buffer {
        self.device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: bytemuck::bytes_of(params),
                usage: wgpu::BufferUsages::UNIFORM,
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

    /// The final target is reused by the next render, so the frame is copied out rather than
    /// handed over: a blit on the device, not a round trip through system memory. sRGB because
    /// that is the format egui requires of a texture it is given, and the copy is legal because
    /// the two differ only in that.
    fn copy_out(&self, final_target: &Target) -> PreviewFrame {
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("preview frame"),
            size: gpu::extent(final_target.width, final_target.height),
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
            gpu::extent(final_target.width, final_target.height),
        );
        self.queue.submit(Some(encoder.finish()));
        PreviewFrame {
            texture,
            width: final_target.width,
            height: final_target.height,
        }
    }

    fn readback(&self, target: &Target) -> Result<RgbaImage> {
        Readback::new(&self.device, target.size()).read(&self.device, &self.queue, target)
    }
}

fn draw<'a>(
    pass: &mut wgpu::RenderPass<'a>,
    pipeline: &'a wgpu::RenderPipeline,
    mesh: &'a GpuMesh,
) {
    pass.set_pipeline(pipeline);
    pass.set_vertex_buffer(0, mesh.vertices.slice(..));
    pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint16);
    pass.draw_indexed(0..mesh.count, 0, 0..1);
}
