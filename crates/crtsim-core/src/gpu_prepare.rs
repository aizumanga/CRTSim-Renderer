//! The prepare step on the GPU: the decoded input goes up once, and the alpha composite, source
//! edits, resize, LUT and grade run as passes writing the signal texture the simulation reads.
//!
//! `config::prepare` stays as the reference this is tested against, and as the fallback for an
//! input too large to prepare on the device. The shader keeps that code's arithmetic, so the
//! two agree to within float rounding; see `shaders/prepare.wgsl`.
//!
//! These are passes of their own rather than being fused into the composite pass. The composite
//! reads its source at seven taps per pixel and runs once per simulated tick, 17 times for a
//! default still, so fusing a resize into it would repeat a downscale kernel of dozens of taps
//! roughly a hundred times over for every signal pixel. Written once, the prepared signal costs
//! one small texture, which is also what a debug render reads back as the clean image.

use crate::config::{Config, Filter};
use crate::gpu::{self, Target, FORMAT};
use crate::workflow::Lut;
use bytemuck::{Pod, Zeroable};
use image::RgbaImage;
use std::sync::Arc;
use wgpu::util::DeviceExt;

pub const SHADER: &str = include_str!("../../../shaders/prepare.wgsl");

/// Where a render runs the prepare step.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PrepareOn {
    #[default]
    Gpu,
    /// `config::prepare`, on this thread. Kept for comparing the two directly; a render also
    /// falls back to it on its own when the input is too large for the device's textures or
    /// its working-set budget.
    Cpu,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Params {
    background: [f32; 4],
    edit: [f32; 4],
    crop: [f32; 4],
    canvas: [f32; 4],
    lut_min: [f32; 4],
    lut_max: [f32; 4],
    grade: [f32; 4],
    lut_strength: [f32; 4],
}

impl Params {
    fn new(input: (u32, u32), signal: (u32, u32), c: &Config) -> Self {
        let edit = &c.source;
        let (width, height) = edit.size(input);
        let (sin, cos) = edit.rotation.to_radians().sin_cos();
        let (hue_sin, hue_cos) = c.hue.to_radians().sin_cos();
        let flag = |on: bool| if on { 1. } else { 0. };
        let [r, g, b] = edit.background.map(f32::from);
        let (lut_min, lut_max) = match &c.lut {
            Some(lut) => (
                [lut.domain_min[0], lut.domain_min[1], lut.domain_min[2], 1.],
                [
                    lut.domain_max[0],
                    lut.domain_max[1],
                    lut.domain_max[2],
                    lut.size as f32,
                ],
            ),
            None => ([0.; 4], [1., 1., 1., 2.]),
        };
        Self {
            background: [r, g, b, flag(edit.checkerboard)],
            edit: [sin, cos, edit.zoom, flag(c.filter == Filter::Nearest)],
            // Computed here, where they are computed the same way as the CPU computes them.
            crop: [
                input.0 as f32 * edit.crop[0],
                input.1 as f32 * edit.crop[1],
                width as f32,
                height as f32,
            ],
            canvas: [
                edit.position[0],
                edit.position[1],
                signal.0 as f32,
                signal.1 as f32,
            ],
            lut_min,
            lut_max,
            grade: [hue_sin, hue_cos, c.chroma, flag(c.grades())],
            lut_strength: [c.lut_strength, 0., 0., 0.],
        }
    }
}

/// Resampling weights along one axis, computed exactly as `image::imageops::resize` computes
/// them -- same span, same f32 kernel, same normalisation -- so the pass sums the terms the CPU
/// would. Computing them on the GPU instead would put a `sin` of the driver's own precision
/// into every weight, and WGSL only bounds that to 2^-11.
#[derive(Debug, PartialEq)]
pub(crate) struct Kernel {
    /// Per output pixel: first source pixel, tap count, offset into `weights`, unused.
    spans: Vec<[u32; 4]>,
    weights: Vec<f32>,
}

impl Kernel {
    /// `resize` copies an image whose size does not change instead of filtering it. That holds
    /// only when neither axis changes: an axis kept while the other is scaled is still filtered.
    fn identity(length: u32) -> Self {
        Self {
            spans: (0..length).map(|i| [i, 1, i, 0]).collect(),
            weights: vec![1.; length as usize],
        }
    }

    fn new(input: u32, output: u32, filter: Filter) -> Self {
        let (kernel, support): (fn(f32) -> f32, f32) = match filter {
            // A box of no width: one tap, the source pixel under the output pixel's centre.
            Filter::Nearest => (|_| 1., 0.),
            Filter::Lanczos => (lanczos3, 3.),
        };
        let ratio = input as f32 / output as f32;
        let sratio = if ratio < 1. { 1. } else { ratio };
        let src_support = support * sratio;
        let mut spans = Vec::with_capacity(output as usize);
        let mut weights = Vec::new();
        for out in 0..output {
            let centre = (out as f32 + 0.5) * ratio;
            let left = ((centre - src_support).floor() as i64).clamp(0, i64::from(input) - 1);
            let right = ((centre + src_support).ceil() as i64).clamp(left + 1, i64::from(input));
            let centre = centre - 0.5;
            let start = weights.len();
            let mut sum = 0.;
            for i in left..right {
                let w = kernel((i as f32 - centre) / sratio);
                weights.push(w);
                sum += w;
            }
            weights[start..].iter_mut().for_each(|w| *w /= sum);
            spans.push([left as u32, (right - left) as u32, start as u32, 0]);
        }
        Self { spans, weights }
    }

    fn upload(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> KernelTextures {
        // Weights are laid out in rows of this many, so a steep downscale -- tens of taps per
        // output pixel -- still fits a texture. Must match the shader's WEIGHT_ROW.
        const ROW: usize = 4096;
        let height = self.weights.len().div_ceil(ROW);
        let mut weights = self.weights.clone();
        weights.resize(ROW * height, 0.);
        KernelTextures {
            spans: texture(
                device,
                queue,
                "resample spans",
                (self.spans.len() as u32, 1, 1),
                wgpu::TextureDimension::D2,
                wgpu::TextureFormat::Rgba32Uint,
                bytemuck::cast_slice(&self.spans),
            ),
            weights: texture(
                device,
                queue,
                "resample weights",
                (ROW as u32, height as u32, 1),
                wgpu::TextureDimension::D2,
                wgpu::TextureFormat::R32Float,
                bytemuck::cast_slice(&weights),
            ),
        }
    }
}

/// `image`'s Lanczos3, reproduced term for term.
fn lanczos3(x: f32) -> f32 {
    fn sinc(t: f32) -> f32 {
        let a = t * std::f32::consts::PI;
        if t == 0. {
            1.
        } else {
            a.sin() / a
        }
    }
    if x.abs() < 3. {
        sinc(x) * sinc(x / 3.)
    } else {
        0.
    }
}

fn texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    (width, height, depth): (u32, u32, u32),
    dimension: wgpu::TextureDimension,
    format: wgpu::TextureFormat,
    data: &[u8],
) -> wgpu::TextureView {
    let texel = format.block_copy_size(None).expect("uncompressed format");
    device
        .create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: depth,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &data[..(width * height * depth * texel) as usize],
        )
        .create_view(&Default::default())
}

fn lut_texture(device: &wgpu::Device, queue: &wgpu::Queue, lut: &Lut) -> wgpu::TextureView {
    let size = lut.size as u32;
    let values: Vec<[f32; 4]> = lut.values.iter().map(|&[r, g, b]| [r, g, b, 0.]).collect();
    texture(
        device,
        queue,
        "3D LUT",
        (size, size, size),
        wgpu::TextureDimension::D3,
        wgpu::TextureFormat::Rgba32Float,
        bytemuck::cast_slice(&values),
    )
}

struct KernelTextures {
    spans: wgpu::TextureView,
    weights: wgpu::TextureView,
}

/// The resize's two kernels, and the rows-resized intermediate between its passes.
struct Resample {
    key: ((u32, u32), (u32, u32), Filter),
    rows: KernelTextures,
    columns: KernelTextures,
    between: Target,
}

/// What a sequence keeps on the device between frames, so a video uploads only each new frame.
#[derive(Default)]
pub(crate) struct Cache {
    input: Option<Target>,
    resample: Option<Resample>,
    lut: Option<(Arc<Lut>, wgpu::TextureView)>,
}

pub(crate) struct Pipelines {
    layout: wgpu::BindGroupLayout,
    rows: wgpu::RenderPipeline,
    columns: wgpu::RenderPipeline,
    edit: wgpu::RenderPipeline,
    /// Bound where a pass does not use a table, since every binding must be filled.
    no_kernel: KernelTextures,
    no_lut: wgpu::TextureView,
}

impl Pipelines {
    /// Call inside the renderer's validation error scope, so a shader error surfaces there.
    pub(crate) fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let float = |binding, view_dimension| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("prepare bindings"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                float(1, wgpu::TextureViewDimension::D2),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Uint,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                float(3, wgpu::TextureViewDimension::D2),
                float(4, wgpu::TextureViewDimension::D3),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("prepare WGSL"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let pipeline = |entry: &str, format| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: "quad",
                    buffers: &[],
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
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview: None,
            })
        };
        Self {
            rows: pipeline("resample_rows", BETWEEN),
            columns: pipeline("resample_columns", FORMAT),
            edit: pipeline("edit", FORMAT),
            layout,
            no_kernel: Kernel::identity(1).upload(device, queue),
            no_lut: lut_texture(
                device,
                queue,
                &Lut {
                    name: String::new(),
                    size: 1,
                    domain_min: [0.; 3],
                    domain_max: [1.; 3],
                    values: vec![[0.; 3]],
                },
            ),
        }
    }

    /// Records the passes that turn `input` into the signal in `signal`. The input itself is
    /// uploaded here, through the queue, so it lands before the commands are submitted.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn encode(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        cache: &mut Cache,
        input: &RgbaImage,
        c: &Config,
        signal: &Target,
    ) {
        let size = input.dimensions();
        let signal_size = (signal.width, signal.height);
        let source = match cache.input.take() {
            Some(target) if (target.width, target.height) == size => target,
            _ => Target::new(device, "source image", size),
        };
        source.upload(queue, input);
        let source = cache.input.insert(source);

        let lut = match &c.lut {
            Some(lut) => {
                // The same table arrives every frame of a video, usually as the same allocation.
                if !matches!(&cache.lut, Some((cached, _)) if Arc::ptr_eq(cached, lut) || cached == lut)
                {
                    cache.lut = Some((lut.clone(), lut_texture(device, queue, lut)));
                }
                &cache.lut.as_ref().expect("just filled").1
            }
            None => &self.no_lut,
        };
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("prepare settings"),
            contents: bytemuck::bytes_of(&Params::new(size, signal_size, c)),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let bind = |input: &Target, kernel: &KernelTextures| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: uniform.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&input.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&kernel.spans),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(&kernel.weights),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::TextureView(lut),
                    },
                ],
            })
        };

        if c.edits_source() {
            gpu::fullscreen(encoder, signal, &self.edit, &bind(source, &self.no_kernel));
            return;
        }
        let key = (size, signal_size, c.filter);
        let resample = match cache.resample.take() {
            Some(resample) if resample.key == key => resample,
            _ => {
                let (rows, columns) = if size == signal_size {
                    (Kernel::identity(size.1), Kernel::identity(size.0))
                } else {
                    (
                        Kernel::new(size.1, signal_size.1, c.filter),
                        Kernel::new(size.0, signal_size.0, c.filter),
                    )
                };
                Resample {
                    key,
                    rows: rows.upload(device, queue),
                    columns: columns.upload(device, queue),
                    between: Target::with_format(
                        device,
                        "rows resampled",
                        (size.0, signal_size.1),
                        BETWEEN,
                        1,
                    ),
                }
            }
        };
        let resample = cache.resample.insert(resample);
        gpu::fullscreen(
            encoder,
            &resample.between,
            &self.rows,
            &bind(source, &resample.rows),
        );
        gpu::fullscreen(
            encoder,
            signal,
            &self.columns,
            &bind(&resample.between, &resample.columns),
        );
    }
}

/// The resize's intermediate: unclamped and unrounded, as `image` keeps it between its passes.
const BETWEEN: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Float;

#[cfg(test)]
mod tests {
    use super::*;
    use image::imageops::{self, FilterType};

    /// The GPU pass's sums, done on the CPU in the same order.
    fn resample(image: &RgbaImage, rows: &Kernel, columns: &Kernel) -> RgbaImage {
        let (width, height) = (image.width(), rows.spans.len() as u32);
        let mut between = vec![[0_f32; 3]; (width * height) as usize];
        for y in 0..height {
            let [first, count, offset, _] = rows.spans[y as usize];
            for x in 0..width {
                let mut total = [0_f32; 3];
                for i in 0..count {
                    let p = image.get_pixel(x, first + i);
                    let w = rows.weights[(offset + i) as usize];
                    for c in 0..3 {
                        total[c] += f32::from(p[c]) * w;
                    }
                }
                between[(y * width + x) as usize] = total;
            }
        }
        RgbaImage::from_fn(columns.spans.len() as u32, height, |x, y| {
            let [first, count, offset, _] = columns.spans[x as usize];
            let mut total = [0_f32; 3];
            for i in 0..count {
                let p = between[(y * width + first + i) as usize];
                let w = columns.weights[(offset + i) as usize];
                for c in 0..3 {
                    total[c] += p[c] * w;
                }
            }
            let [r, g, b] = total.map(|v| v.clamp(0., 255.).round() as u8);
            image::Rgba([r, g, b, 255])
        })
    }

    #[test]
    fn kernels_reproduce_the_image_crates_resize_exactly() {
        let image = RgbaImage::from_fn(97, 61, |x, y| {
            let v = (x * 37 + y * 101 + x * y * 13) % 256;
            image::Rgba([v as u8, (v * 7 % 256) as u8, (255 - v) as u8, 255])
        });
        for (filter, cpu) in [
            (Filter::Lanczos, FilterType::Lanczos3),
            (Filter::Nearest, FilterType::Nearest),
        ] {
            // Down, up, one axis at a time, a steep downscale and a single row.
            for size in [(40, 30), (200, 150), (97, 20), (13, 61), (5, 3), (97, 1)] {
                let expected = imageops::resize(&image, size.0, size.1, cpu);
                let rows = Kernel::new(61, size.1, filter);
                let columns = Kernel::new(97, size.0, filter);
                assert_eq!(
                    resample(&image, &rows, &columns),
                    expected,
                    "{filter:?} to {size:?}"
                );
            }
        }
        let same = resample(&image, &Kernel::identity(61), &Kernel::identity(97));
        assert_eq!(same, image);
    }

    #[test]
    fn prepare_shader_validates_without_a_gpu() {
        let module = naga::front::wgsl::parse_str(SHADER).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .unwrap();
    }
}
