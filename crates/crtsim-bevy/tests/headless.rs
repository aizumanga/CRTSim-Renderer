//! A Bevy app with no window draws a camera's picture through the CRT, and it is the picture
//! the renderer itself draws of the same frames.

use bevy::{
    camera::{ClearColorConfig, Hdr, RenderTarget},
    core_pipeline::tonemapping::{DebandDither, Tonemapping},
    prelude::*,
    render::{
        gpu_readback::{Readback, ReadbackComplete},
        render_resource::{TextureFormat, TextureUsages},
        settings::WgpuSettings,
        view::Msaa,
        RenderPlugin,
    },
    time::TimeUpdateStrategy,
    window::ExitCondition,
    winit::WinitPlugin,
};
use crtsim_bevy::{Crt, CrtPlugin};
use crtsim_core::{config::Phase, HostFrame, Renderer, Sequence};
use std::time::Duration;

const SIZE: (u32, u32) = (320, 240);
const FRAMES: u32 = 90;
/// The picture the camera clears to, as sRGB bytes.
const COLOR: [u8; 3] = [200, 120, 40];

fn look() -> Crt {
    let mut crt = Crt::preset("General image").unwrap();
    // Every frame alike, so the glow settles whatever frame it started on.
    crt.config.phase = Phase::Stable;
    crt.config.interlace = false;
    crt
}

/// The last frame a camera of `SIZE` clearing to `COLOR` shows after `FRAMES` frames a
/// sixtieth of a second each, read back as sRGB bytes.
fn bevy_frame(hdr: bool) -> Vec<u8> {
    #[derive(Resource, Default)]
    struct Last(Vec<u8>);

    let mut app = App::new();
    app.add_plugins((
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: None,
                exit_condition: ExitCondition::DontExit,
                close_when_requested: false,
                ..default()
            })
            .set(RenderPlugin {
                render_creation: WgpuSettings {
                    backends: Some(wgpu::Backends::VULKAN),
                    ..default()
                }
                .into(),
                synchronous_pipeline_compilation: true,
                ..default()
            })
            .disable::<WinitPlugin>(),
        CrtPlugin,
    ))
    .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
        1. / 60.,
    )))
    .init_resource::<Last>();
    let target = app.world_mut().resource_mut::<Assets<Image>>().add({
        let mut image =
            Image::new_target_texture(SIZE.0, SIZE.1, TextureFormat::Rgba8UnormSrgb, None);
        image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
        image
    });
    let mut camera = app.world_mut().spawn((
        Camera2d,
        Camera {
            clear_color: ClearColorConfig::Custom(Color::srgb_u8(COLOR[0], COLOR[1], COLOR[2])),
            ..default()
        },
        RenderTarget::Image(target.clone().into()),
        Tonemapping::None,
        DebandDither::Disabled,
        Msaa::Off,
        look(),
    ));
    if hdr {
        camera.insert(Hdr);
    }
    app.world_mut().spawn(Readback::texture(target)).observe(
        |read: On<ReadbackComplete>, mut last: ResMut<Last>| last.0.clone_from(&read.data),
    );
    app.finish();
    app.cleanup();
    for _ in 0..FRAMES {
        app.update();
    }
    app.world_mut().remove_resource::<Last>().unwrap().0
}

/// The same frames drawn by the renderer itself, on a device of its own.
fn renderer_frame() -> Vec<u8> {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                required_limits: Renderer::limits(&adapter),
                ..Default::default()
            })
            .await
            .unwrap();
        let renderer = Renderer::with_device(
            device.clone().into(),
            queue.clone().into(),
            adapter.get_info(),
        )
        .await
        .unwrap();
        let texture = |usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: SIZE.0,
                    height: SIZE.1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage,
                view_formats: &[],
            })
        };
        let source = texture(wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST);
        let target =
            texture(wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC);
        let pixels: Vec<u8> = (0..SIZE.0 * SIZE.1)
            .flat_map(|_| [COLOR[0], COLOR[1], COLOR[2], 255])
            .collect();
        queue.write_texture(
            source.as_image_copy(),
            &pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * SIZE.0),
                rows_per_image: None,
            },
            source.size(),
        );
        let config = look().config;
        let mut sequence = Sequence::host();
        for _ in 0..FRAMES {
            let mut encoder = device.create_command_encoder(&Default::default());
            let frame = HostFrame {
                source: &source,
                target: &target,
                seconds: 1. / 60.,
            };
            renderer
                .encode(&mut encoder, &mut sequence, &frame, &config)
                .unwrap();
            queue.submit(Some(encoder.finish()));
        }
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(4 * SIZE.0 * SIZE.1),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * SIZE.0),
                    rows_per_image: None,
                },
            },
            target.size(),
        );
        queue.submit(Some(encoder.finish()));
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, |mapped| mapped.unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let pixels = buffer.slice(..).get_mapped_range().unwrap().to_vec();
        pixels
    })
}

/// The most two pictures' colours differ by, in 8-bit steps.
fn most_apart(a: &[u8], b: &[u8]) -> u8 {
    assert_eq!(a.len(), b.len());
    a.chunks(4)
        .zip(b.chunks(4))
        .flat_map(|(a, b)| (0..3).map(move |i| a[i].abs_diff(b[i])))
        .max()
        .unwrap()
}

#[test]
#[ignore = "requires a Vulkan adapter"]
fn a_camera_is_drawn_through_the_crt_as_the_renderer_draws_it() {
    let expected = renderer_frame();
    let flat: Vec<u8> = (0..SIZE.0 * SIZE.1)
        .flat_map(|_| [COLOR[0], COLOR[1], COLOR[2], 255])
        .collect();
    assert!(
        most_apart(&expected, &flat) > 64,
        "the CRT changes the picture"
    );
    for hdr in [false, true] {
        let drawn = bevy_frame(hdr);
        let most = most_apart(&drawn, &expected);
        assert!(
            most <= 2,
            "with hdr {hdr}, {most} steps from the renderer's"
        );
    }
}
