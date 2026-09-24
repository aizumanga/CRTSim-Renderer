//! GPU building blocks the passes share: render targets, reading them back, the meshes, and the
//! textures loaded once per renderer.

use crate::mesh;
use anyhow::{ensure, Context, Result};
use image::RgbaImage;
use wgpu::util::DeviceExt;

/// The 8-bit format of the signal, its history and the final image.
pub(crate) const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

pub(crate) struct Target {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub width: u32,
    pub height: u32,
}

impl Target {
    pub fn new(device: &wgpu::Device, name: &str, size: (u32, u32)) -> Self {
        Self::with_format(device, name, size, FORMAT, 1)
    }

    pub fn with_format(
        device: &wgpu::Device,
        name: &str,
        (width, height): (u32, u32),
        format: wgpu::TextureFormat,
        mip_level_count: u32,
    ) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(name),
            size: extent(width, height),
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

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn upload(&self, queue: &wgpu::Queue, image: &RgbaImage) {
        self.upload_level(queue, 0, image);
    }

    fn upload_level(&self, queue: &wgpu::Queue, mip_level: u32, image: &RgbaImage) {
        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &self.texture,
                mip_level,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            image.as_raw(),
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(4 * image.width()),
                rows_per_image: Some(image.height()),
            },
            extent(image.width(), image.height()),
        );
    }
}

pub(crate) fn extent(width: u32, height: u32) -> wgpu::Extent3d {
    wgpu::Extent3d {
        width,
        height,
        depth_or_array_layers: 1,
    }
}

/// A buffer a target of one size is copied into to be read on the CPU. Rows are padded to the
/// 256-byte alignment copies require.
pub(crate) struct Readback {
    buffer: wgpu::Buffer,
    pitch: u32,
    size: (u32, u32),
}

impl Readback {
    pub fn new(device: &wgpu::Device, (width, height): (u32, u32)) -> Self {
        let pitch = (width * 4).div_ceil(256) * 256;
        Self {
            buffer: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("readback"),
                size: u64::from(pitch) * u64::from(height),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }),
            pitch,
            size: (width, height),
        }
    }

    /// Copies `target` into the buffer, waits for the GPU and returns its pixels.
    pub fn read(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: &Target,
    ) -> Result<RgbaImage> {
        ensure!(
            self.size == target.size(),
            "readback dimensions do not match render target"
        );
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            target.texture.as_image_copy(),
            wgpu::ImageCopyBuffer {
                buffer: &self.buffer,
                layout: wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(self.pitch),
                    rows_per_image: Some(target.height),
                },
            },
            extent(target.width, target.height),
        );
        queue.submit(Some(encoder.finish()));
        let slice = self.buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        device.poll(wgpu::Maintain::Wait);
        rx.recv()??;
        let mapped = slice.get_mapped_range();
        let mut pixels = Vec::with_capacity((target.width * target.height * 4) as usize);
        for row in mapped.chunks_exact(self.pitch as usize) {
            pixels.extend_from_slice(&row[..target.width as usize * 4]);
        }
        drop(mapped);
        self.buffer.unmap();
        RgbaImage::from_raw(target.width, target.height, pixels).context("invalid readback length")
    }
}

pub(crate) struct GpuMesh {
    pub vertices: wgpu::Buffer,
    pub indices: wgpu::Buffer,
    pub count: u32,
}

impl GpuMesh {
    pub fn new(device: &wgpu::Device, bytes: &[u8]) -> Result<Self> {
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

/// An attachment that starts the pass by clearing `view` to black.
pub(crate) fn cleared(view: &wgpu::TextureView) -> Option<wgpu::RenderPassColorAttachment<'_>> {
    Some(wgpu::RenderPassColorAttachment {
        view,
        resolve_target: None,
        ops: wgpu::Operations {
            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
            store: wgpu::StoreOp::Store,
        },
    })
}

/// Clears `target` to black.
pub(crate) fn clear(encoder: &mut wgpu::CommandEncoder, target: &Target) {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("clear"),
        color_attachments: &[cleared(&target.view)],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
    });
}

/// Draws one triangle over all of `dst`, the fullscreen pass every image-space step uses.
pub(crate) fn fullscreen(
    encoder: &mut wgpu::CommandEncoder,
    dst: &Target,
    pipeline: &wgpu::RenderPipeline,
    bindings: &wgpu::BindGroup,
) {
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("fullscreen pass"),
        color_attachments: &[cleared(&dst.view)],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
    });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, bindings, &[]);
    pass.draw(0..3, 0..1);
}

fn bmp(bytes: &[u8]) -> Result<RgbaImage> {
    Ok(image::load_from_memory_with_format(bytes, image::ImageFormat::Bmp)?.to_rgba8())
}

/// The artifact pattern the composite pass tints neighbouring pixels by.
pub(crate) fn artifacts(device: &wgpu::Device, queue: &wgpu::Queue) -> Result<Target> {
    let image = bmp(include_bytes!(
        "../../../assets/original-crtsim/artifacts.bmp"
    ))?;
    let target = Target::new(device, "NTSC texture", image.dimensions());
    target.upload(queue, &image);
    Ok(target)
}

/// The shadow mask with a full mip chain, for sampling it smaller than it is.
pub(crate) fn shadow_mask(device: &wgpu::Device, queue: &wgpu::Queue) -> Result<Target> {
    let mut level = bmp(include_bytes!("../../../assets/original-crtsim/mask.bmp"))?;
    let levels = level.width().max(level.height()).ilog2() + 1;
    let mask = Target::with_format(device, "shadow mask", level.dimensions(), FORMAT, levels);
    for mip in 0..levels {
        mask.upload_level(queue, mip, &level);
        level = image::imageops::resize(
            &level,
            (level.width() / 2).max(1),
            (level.height() / 2).max(1),
            image::imageops::FilterType::Triangle,
        );
    }
    Ok(mask)
}
