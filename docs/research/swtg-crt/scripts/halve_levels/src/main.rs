//! Makes the mip levels of one picture with CRTSim-Renderer's own `halve` pass (crtsim.wgsl's
//! `quad` and `halve` entry points, the renderer's linear clamp sampler and 8-bit target), as
//! the renderer draws them for `bloom_from_mipmaps`, so they can be checked against the levels
//! a game's driver made from the same picture.
//!
//! usage: halve_levels LEVEL0.rgba WIDTH HEIGHT OUTDIR
//!   LEVEL0.rgba  raw RGBA, WIDTH x HEIGHT, in any row order (kept as given)
//! Writes OUTDIR/level{n}.rgba for every level after the first, as many as the renderer makes
//! for that size (down to the level below a sixteenth, at most 6 in all).
use anyhow::{Context, Result};
use std::path::Path;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let pixels = std::fs::read(&args[1])?;
    let (width, height): (u32, u32) = (args[2].parse()?, args[3].parse()?);
    anyhow::ensure!(pixels.len() == (width * height * 4) as usize, "expected {width}x{height} RGBA");
    let out = Path::new(&args[4]);
    std::fs::create_dir_all(out)?;
    // As the renderer's bloom_levels.
    let levels = (u32::BITS - width.max(height).leading_zeros()).min(6);

    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
        apply_limit_buckets: false,
    }))
    .context("no adapter")?;
    eprintln!("adapter: {} ({:?})", adapter.get_info().name, adapter.get_info().driver);
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default()))?;

    let format = wgpu::TextureFormat::Rgba8Unorm;
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("picture"),
        size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        mip_level_count: levels,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &pixels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4 * width),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
    );
    let views: Vec<_> = (0..levels)
        .map(|level| {
            texture.create_view(&wgpu::TextureViewDescriptor {
                base_mip_level: level,
                mip_level_count: Some(1),
                ..Default::default()
            })
        })
        .collect();
    // The renderer's linear_clamp sampler.
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Linear,
        ..Default::default()
    });
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("CRT WGSL"),
        source: wgpu::ShaderSource::Wgsl(crtsim_core::SHADER.into()),
    });
    // Layout derived from what quad and halve use: source (1) and linear_clamp (6).
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("halve"),
        layout: None,
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("quad"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("halve"),
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
    });
    let layout = pipeline.get_bind_group_layout(0);
    let mut encoder = device.create_command_encoder(&Default::default());
    for level in 1..levels as usize {
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&views[level - 1]),
                },
                wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::Sampler(&sampler) },
            ],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("halve"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &views[level],
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.draw(0..3, 0..1);
    }
    queue.submit(Some(encoder.finish()));

    for level in 1..levels {
        let (w, h) = ((width >> level).max(1), (height >> level).max(1));
        let pitch = (w * 4).div_ceil(256) * 256;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(pitch * h),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: level,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(pitch),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        queue.submit(Some(encoder.finish()));
        let slice = buffer.slice(..);
        let (send, recv) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = send.send(r);
        });
        device.poll(wgpu::PollType::wait_indefinitely())?;
        recv.recv()??;
        let mapped = slice.get_mapped_range()?;
        let mut data = Vec::with_capacity((w * h * 4) as usize);
        for row in mapped.chunks_exact(pitch as usize) {
            data.extend_from_slice(&row[..w as usize * 4]);
        }
        drop(mapped);
        buffer.unmap();
        std::fs::write(out.join(format!("level{level}.rgba")), data)?;
        eprintln!("level {level}: {w}x{h}");
    }
    Ok(())
}
