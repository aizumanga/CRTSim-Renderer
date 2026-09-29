//! Runs a shader port, the effect as a RetroArch preset (`.slangp`), through librashader's wgpu
//! runtime on a headless device, so tests can compare its pixels with the renderer's and the
//! benchmark can time it. See docs/adr/0003-games-get-shader-ports-not-the-renderer.md.
//!
//! librashader is MPL-2.0 OR GPL-3.0-only. This crate is for tests and benchmarks and never
//! ships, and nothing that ships may depend on it.

use anyhow::{bail, ensure, Context, Result};
use image::RgbaImage;
use librashader::presets::ShaderFeatures;
use librashader::runtime::wgpu::{FilterChain, FilterChainOptions, FrameOptions, WgpuOutputView};
use librashader::runtime::{FilterChainParameters, Size, Viewport};
use std::collections::BTreeSet;
use std::path::Path;
use std::time::{Duration, Instant};

/// The RetroArch a preset runs in, which decides the uniforms its shaders are told they have.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetroArch {
    /// One without `OriginalAspect` and `FrameTimeDelta`: `_HAS_ORIGINALASPECT_UNIFORMS` and
    /// `_HAS_FRAMETIME_UNIFORMS` are undefined, so a port takes its fallback.
    Older,
    /// One that defines both, and binds `OriginalAspect`, `OriginalAspectRotated`,
    /// `FrameTimeDelta` and `OriginalFPS`.
    Newer,
}

impl RetroArch {
    fn features(self) -> ShaderFeatures {
        match self {
            Self::Older => ShaderFeatures::NONE,
            Self::Newer => {
                ShaderFeatures::ORIGINAL_ASPECT_UNIFORMS | ShaderFeatures::FRAMETIME_UNIFORMS
            }
        }
    }
}

/// A preset loaded on a device of its own, and the frames it has rendered so far.
pub struct Port {
    device: wgpu::Device,
    queue: wgpu::Queue,
    adapter: wgpu::AdapterInfo,
    chain: FilterChain,
    /// The parameters the preset's shaders declare. librashader adds any name it is given
    /// without complaint, so `set` checks against these instead.
    parameters: BTreeSet<String>,
    /// The next frame's `FrameCount`.
    frame: usize,
}

/// How long frames took.
#[derive(Clone, Debug)]
pub struct Timing {
    /// The time from the first submission until the device finished the last frame, divided
    /// by the frames.
    pub wall: Duration,
    /// Each frame on the GPU alone, from timestamp queries, when the device has them.
    pub gpu: Option<Vec<Duration>>,
}

impl Port {
    /// Loads `preset` on a headless Vulkan device: on a machine with only a software driver,
    /// lavapipe.
    pub fn load(preset: &Path, retroarch: RetroArch) -> Result<Self> {
        Self::load_on(wgpu::Backends::VULKAN, preset, retroarch)
    }

    /// Loads `preset` on the fastest adapter of `backends`.
    pub fn load_on(backends: wgpu::Backends, preset: &Path, retroarch: RetroArch) -> Result<Self> {
        let (device, queue, adapter) = pollster::block_on(open(backends))?;
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let options = FilterChainOptions {
            force_no_mipmaps: false,
            enable_cache: false,
            adapter_info: None,
        };
        let chain = FilterChain::load_from_path(
            preset,
            retroarch.features(),
            &device,
            &queue,
            Some(&options),
        )
        .with_context(|| format!("librashader could not load {}", preset.display()))?;
        if let Some(error) = pollster::block_on(scope.pop()) {
            bail!("wgpu rejected {}: {error}", preset.display());
        }
        let parameters = chain
            .parameters()
            .parameters()
            .keys()
            .map(|name| name.to_string())
            .collect();
        Ok(Self {
            device,
            queue,
            adapter,
            chain,
            parameters,
            frame: 0,
        })
    }

    pub fn adapter(&self) -> &wgpu::AdapterInfo {
        &self.adapter
    }

    /// Sets one of the preset's parameters for the frames rendered from now on.
    pub fn set(&mut self, parameter: &str, value: f32) -> Result<()> {
        ensure!(
            self.parameters.contains(parameter),
            "the preset has no parameter {parameter}; it has {:?}",
            self.parameters
        );
        self.chain
            .parameters()
            .set_parameter_value(parameter, value);
        Ok(())
    }

    /// Renders `frames` frames of `input` into an output of `size`, each lasting
    /// `frame_time_us` microseconds (`FrameTimeDelta`), with `aspect` as `OriginalAspect`, and
    /// returns the last as a host shows it: opaque, whatever alpha the last pass wrote.
    /// Frames carry on from earlier calls: `FrameCount` keeps counting, and the feedback and
    /// history the preset keeps are the previous frames'.
    pub fn render(
        &mut self,
        input: &RgbaImage,
        size: (u32, u32),
        frames: u32,
        frame_time_us: u32,
        aspect: f32,
    ) -> Result<RgbaImage> {
        let output = self.run(input, size, frames, frame_time_us, aspect, None)?;
        let mut picture = self.read(&output)?;
        for pixel in picture.pixels_mut() {
            pixel[3] = 255;
        }
        Ok(picture)
    }

    /// Renders as `render` does, without reading the frames back, and times them.
    pub fn time(
        &mut self,
        input: &RgbaImage,
        size: (u32, u32),
        frames: u32,
        frame_time_us: u32,
        aspect: f32,
    ) -> Result<Timing> {
        let timer = self
            .device
            .features()
            .contains(wgpu::Features::TIMESTAMP_QUERY)
            .then(|| Timer::new(&self.device, frames));
        let start = Instant::now();
        self.run(input, size, frames, frame_time_us, aspect, timer.as_ref())?;
        let wall = start.elapsed() / frames;
        let gpu = match timer {
            Some(timer) => Some(timer.read(self)?),
            None => None,
        };
        Ok(Timing { wall, gpu })
    }

    /// Submits the frames one by one, since librashader writes each frame's uniforms through
    /// the queue as it records, and returns the output once the device has finished them.
    fn run(
        &mut self,
        input: &RgbaImage,
        (width, height): (u32, u32),
        frames: u32,
        frame_time_us: u32,
        aspect: f32,
        timer: Option<&Timer>,
    ) -> Result<wgpu::Texture> {
        ensure!(frames > 0, "render at least one frame");
        let scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let source = self.device.create_texture(&texture(
            "port input",
            input.dimensions(),
            wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC,
        ));
        self.queue.write_texture(
            source.as_image_copy(),
            input.as_raw(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * input.width()),
                rows_per_image: Some(input.height()),
            },
            source.size(),
        );
        let output = self.device.create_texture(&texture(
            "port output",
            (width, height),
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        ));
        let viewport = Viewport {
            x: 0.0,
            y: 0.0,
            mvp: None,
            output: WgpuOutputView::from(&output),
            size: Size { width, height },
        };
        let options = FrameOptions {
            aspect_ratio: aspect,
            frametime_delta: frame_time_us,
            frames_per_second: if frame_time_us == 0 {
                1.0
            } else {
                1e6 / frame_time_us as f32
            },
            ..FrameOptions::default()
        };
        for index in 0..frames {
            let mut encoder = self.device.create_command_encoder(&Default::default());
            if let Some(timer) = timer {
                timer.mark(&mut encoder, 0);
            }
            self.chain
                .frame(&source, &viewport, &mut encoder, self.frame, Some(&options))
                .context("librashader could not record a frame")?;
            if let Some(timer) = timer {
                timer.mark(&mut encoder, 1);
                timer.resolve(&mut encoder, index);
            }
            self.queue.submit([encoder.finish()]);
            self.frame += 1;
        }
        self.device.poll(wgpu::PollType::wait_indefinitely())?;
        if let Some(error) = pollster::block_on(scope.pop()) {
            bail!("wgpu rejected a frame: {error}");
        }
        Ok(output)
    }

    fn read(&self, texture: &wgpu::Texture) -> Result<RgbaImage> {
        let (width, height) = (texture.width(), texture.height());
        let pitch = (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("port readback"),
            size: u64::from(pitch) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(pitch),
                    rows_per_image: Some(height),
                },
            },
            texture.size(),
        );
        self.queue.submit([encoder.finish()]);
        let padded = map(&self.device, &buffer)?;
        let pixels = padded
            .chunks_exact(pitch as usize)
            .flat_map(|row| &row[..width as usize * 4])
            .copied()
            .collect();
        RgbaImage::from_raw(width, height, pixels).context("the readback is the wrong length")
    }
}

async fn open(backends: wgpu::Backends) -> Result<(wgpu::Device, wgpu::Queue, wgpu::AdapterInfo)> {
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
        .with_context(|| format!("no {backends:?} adapter"))?;
    // Presets clamp to a transparent border unless they ask otherwise. Without the feature
    // librashader clamps to the edge instead, which shows at the picture's edges.
    let features = adapter.features()
        & (wgpu::Features::ADDRESS_MODE_CLAMP_TO_BORDER
            | wgpu::Features::FLOAT32_FILTERABLE
            | wgpu::Features::TIMESTAMP_QUERY);
    if !features.contains(wgpu::Features::ADDRESS_MODE_CLAMP_TO_BORDER) {
        eprintln!("this adapter cannot clamp to a border; librashader clamps to the edge instead");
    }
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("crtsim-ports"),
            required_features: features,
            required_limits: wgpu::Limits::default().using_resolution(adapter.limits()),
            ..Default::default()
        })
        .await?;
    Ok((device, queue, adapter.get_info()))
}

fn texture(
    label: &str,
    (width, height): (u32, u32),
    usage: wgpu::TextureUsages,
) -> wgpu::TextureDescriptor<'_> {
    wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage,
        view_formats: &[],
    }
}

/// Maps `buffer`, whose copies have been submitted, and returns its contents.
fn map(device: &wgpu::Device, buffer: &wgpu::Buffer) -> Result<Vec<u8>> {
    let (sender, mapped) = std::sync::mpsc::channel();
    buffer.map_async(wgpu::MapMode::Read, .., move |result| {
        let _ = sender.send(result);
    });
    device.poll(wgpu::PollType::wait_indefinitely())?;
    mapped.recv().context("the device dropped a readback")??;
    let bytes = buffer.get_mapped_range(..)?.to_vec();
    buffer.unmap();
    Ok(bytes)
}

/// Timestamps either side of each frame, written by empty compute passes because only passes
/// can write them without an extra device feature.
struct Timer {
    set: wgpu::QuerySet,
    /// Each frame's pair, at the 256-byte alignment resolving needs.
    resolved: wgpu::Buffer,
    frames: u32,
}

impl Timer {
    const STRIDE: u64 = wgpu::QUERY_RESOLVE_BUFFER_ALIGNMENT;

    fn new(device: &wgpu::Device, frames: u32) -> Self {
        Self {
            set: device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("frame timestamps"),
                ty: wgpu::QueryType::Timestamp,
                count: 2,
            }),
            resolved: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("resolved timestamps"),
                size: Self::STRIDE * u64::from(frames),
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            frames,
        }
    }

    fn mark(&self, encoder: &mut wgpu::CommandEncoder, query: u32) {
        encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("timestamp"),
            timestamp_writes: Some(wgpu::ComputePassTimestampWrites {
                query_set: &self.set,
                beginning_of_pass_write_index: Some(query),
                end_of_pass_write_index: None,
            }),
        });
    }

    fn resolve(&self, encoder: &mut wgpu::CommandEncoder, frame: u32) {
        encoder.resolve_query_set(
            &self.set,
            0..2,
            &self.resolved,
            Self::STRIDE * u64::from(frame),
        );
    }

    fn read(self, port: &Port) -> Result<Vec<Duration>> {
        let readable = port.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readable timestamps"),
            size: self.resolved.size(),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = port.device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(&self.resolved, 0, &readable, 0, None);
        port.queue.submit([encoder.finish()]);
        let bytes = map(&port.device, &readable)?;
        let period = f64::from(port.queue.get_timestamp_period());
        Ok((0..self.frames as usize)
            .map(|frame| {
                let at = |query: usize| {
                    let start = frame * Self::STRIDE as usize + query * 8;
                    u64::from_le_bytes(bytes[start..start + 8].try_into().unwrap())
                };
                Duration::from_nanos((at(1).saturating_sub(at(0)) as f64 * period) as u64)
            })
            .collect())
    }
}
