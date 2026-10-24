//! Shows pictures in an X window, each for a number of frames, presenting every frame through a
//! Vulkan swapchain, then saves what the window shows. Run with vkBasalt's layer enabled, the
//! window shows the pictures after vkBasalt's effects; `tests/reshade_vkbasalt.rs` runs it so.
//!
//! ```text
//! present SHOWN.png PICTURE.png FRAMES [PICTURE.png FRAMES]...
//! ```
//!
//! Each picture is copied into the swapchain's images as it is, byte for byte, and the window's
//! contents are read back from the X server once the last frame has been presented and the
//! window has stopped changing.

use anyhow::{bail, ensure, Context, Result};
use image::RgbaImage;
use std::ffi::c_void;
use std::ptr::NonNull;
use wgpu::rwh::{RawDisplayHandle, RawWindowHandle, XlibDisplayHandle, XlibWindowHandle};
use x11_dl::xlib;

/// The pause after each frame has finished, before the next.
const PACE: std::time::Duration = std::time::Duration::from_millis(50);

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let usage = "usage: present SHOWN.png PICTURE.png FRAMES [PICTURE.png FRAMES]...";
    let Some((shown, steps)) = args.split_first() else {
        bail!(usage);
    };
    ensure!(!steps.is_empty() && steps.len() % 2 == 0, usage);
    let steps = steps
        .as_chunks::<2>()
        .0
        .iter()
        .map(|[picture, frames]| {
            let picture = image::open(picture)
                .with_context(|| format!("cannot read {picture}"))?
                .to_rgba8();
            let frames: u32 = frames.parse().context("FRAMES is a number")?;
            Ok((picture, frames))
        })
        .collect::<Result<Vec<_>>>()?;
    let size = steps[0].0.dimensions();
    ensure!(
        steps
            .iter()
            .all(|(picture, _)| picture.dimensions() == size),
        "the pictures are of different sizes"
    );
    let window = Window::open(size)?;
    present(&window, &steps)?;
    window.read()?.save(shown)?;
    Ok(())
}

/// An X window of the picture's size, on its own connection.
struct Window {
    xlib: xlib::Xlib,
    display: *mut xlib::Display,
    screen: i32,
    window: xlib::Window,
    size: (u32, u32),
}

impl Window {
    fn open((width, height): (u32, u32)) -> Result<Self> {
        let xlib = xlib::Xlib::open().context("cannot load Xlib")?;
        // SAFETY: Xlib's calls on a display it opened, and a window on it; the display is
        // used from other threads by the Vulkan driver, which XInitThreads allows.
        unsafe {
            (xlib.XInitThreads)();
            let display = (xlib.XOpenDisplay)(std::ptr::null());
            ensure!(!display.is_null(), "cannot open the X display");
            let screen = (xlib.XDefaultScreen)(display);
            let root = (xlib.XRootWindow)(display, screen);
            let black = (xlib.XBlackPixel)(display, screen);
            let window =
                (xlib.XCreateSimpleWindow)(display, root, 0, 0, width, height, 0, black, black);
            (xlib.XSelectInput)(display, window, xlib::StructureNotifyMask);
            (xlib.XMapWindow)(display, window);
            let mut event: xlib::XEvent = std::mem::zeroed();
            loop {
                (xlib.XNextEvent)(display, &mut event);
                if event.get_type() == xlib::MapNotify {
                    break;
                }
            }
            Ok(Self {
                xlib,
                display,
                screen,
                window,
                size: (width, height),
            })
        }
    }

    /// What the window shows once it has stopped changing: the same in reads a tenth of a
    /// second apart for half a second, since the driver may hand the last frame to the X server
    /// from a thread of its own, after presenting returns.
    fn read(&self) -> Result<RgbaImage> {
        let start = std::time::Instant::now();
        let mut shown = self.read_now()?;
        let mut same = 0;
        while same < 5 {
            ensure!(
                start.elapsed() < std::time::Duration::from_secs(20),
                "the window kept changing after the last frame"
            );
            std::thread::sleep(std::time::Duration::from_millis(100));
            let now = self.read_now()?;
            same = if now == shown { same + 1 } else { 0 };
            shown = now;
        }
        Ok(shown)
    }

    /// What the X server shows in the window now, once every request sent on its connection
    /// has been handled.
    fn read_now(&self) -> Result<RgbaImage> {
        let (width, height) = self.size;
        // SAFETY: the image Xlib returns is read within its bounds, then destroyed by Xlib.
        unsafe {
            (self.xlib.XSync)(self.display, xlib::False);
            let image = (self.xlib.XGetImage)(
                self.display,
                self.window,
                0,
                0,
                width,
                height,
                !0,
                xlib::ZPixmap,
            );
            ensure!(!image.is_null(), "cannot read the window back");
            let read = &*image;
            let result = if read.bits_per_pixel == 32 && read.byte_order == xlib::LSBFirst {
                let channel = |pixel: u32, mask: u64| -> u8 {
                    ((u64::from(pixel) & mask) >> mask.trailing_zeros()) as u8
                };
                let data = read.data as *const u8;
                let mut out = RgbaImage::new(width, height);
                for (x, y, pixel) in out.enumerate_pixels_mut() {
                    let at = data.add(y as usize * read.bytes_per_line as usize + x as usize * 4);
                    let mut value = [0; 4];
                    std::ptr::copy_nonoverlapping(at, value.as_mut_ptr(), 4);
                    let value = u32::from_le_bytes(value);
                    *pixel = image::Rgba([
                        channel(value, read.red_mask),
                        channel(value, read.green_mask),
                        channel(value, read.blue_mask),
                        255,
                    ]);
                }
                Ok(out)
            } else {
                Err(anyhow::anyhow!(
                    "the window reads back at {} bits a pixel, not 32: run X at depth 24",
                    read.bits_per_pixel
                ))
            };
            if let Some(destroy) = read.funcs.destroy_image {
                destroy(image);
            }
            result
        }
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        // SAFETY: the window and display this opened, which nothing uses any more.
        unsafe {
            (self.xlib.XDestroyWindow)(self.display, self.window);
            (self.xlib.XCloseDisplay)(self.display);
        }
    }
}

/// Presents each picture for its frames in `window`, waiting for each frame to finish.
fn present(window: &Window, steps: &[(RgbaImage, u32)]) -> Result<()> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let target = wgpu::SurfaceTargetUnsafe::RawHandle {
        raw_display_handle: Some(RawDisplayHandle::Xlib(XlibDisplayHandle::new(
            NonNull::new(window.display.cast::<c_void>()),
            window.screen,
        ))),
        raw_window_handle: RawWindowHandle::Xlib(XlibWindowHandle::new(window.window)),
    };
    // SAFETY: the display and window outlive the surface, which is dropped at the end here.
    let surface = unsafe { instance.create_surface_unsafe(target) }?;
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: Some(&surface),
        force_fallback_adapter: false,
        apply_limit_buckets: false,
    }))
    .context("no Vulkan adapter for the window")?;
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default()))?;
    let (width, height) = window.size;
    let capabilities = surface.get_capabilities(&adapter);
    // Copied in as they are and read as they are: an 8-bit format with no sRGB curve.
    let format = [
        wgpu::TextureFormat::Bgra8Unorm,
        wgpu::TextureFormat::Rgba8Unorm,
    ]
    .into_iter()
    .find(|format| capabilities.formats.contains(format))
    .with_context(|| format!("the window offers only {:?}", capabilities.formats))?;
    ensure!(
        capabilities.usages.contains(wgpu::TextureUsages::COPY_DST),
        "the window's images cannot be copied into"
    );
    let mut config = surface
        .get_default_config(&adapter, width, height)
        .context("the window has no configuration")?;
    config.format = format;
    config.usage = wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::RENDER_ATTACHMENT;
    // Presented as each frame finishes, by the thread presenting it, not later by a queue.
    if capabilities
        .present_modes
        .contains(&wgpu::PresentMode::Immediate)
    {
        config.present_mode = wgpu::PresentMode::Immediate;
    }
    surface.configure(&device, &config);
    for (picture, frames) in steps {
        let mut bytes = picture.as_raw().clone();
        if format == wgpu::TextureFormat::Bgra8Unorm {
            for pixel in bytes.as_chunks_mut::<4>().0 {
                pixel.swap(0, 2);
            }
        }
        for _ in 0..*frames {
            let texture = match surface.get_current_texture() {
                wgpu::CurrentSurfaceTexture::Success(texture) => texture,
                other => bail!("the window gave no image to draw in: {other:?}"),
            };
            queue.write_texture(
                texture.texture.as_image_copy(),
                &bytes,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * width),
                    rows_per_image: Some(height),
                },
                texture.texture.size(),
            );
            queue.submit([]);
            queue.present(texture);
            // A layer presenting submits its own work to the queue first; waiting for work
            // submitted after it waits for that too, so the layer never rewrites its uniforms
            // while a frame still reads them.
            queue.submit([device.create_command_encoder(&Default::default()).finish()]);
            device.poll(wgpu::PollType::wait_indefinitely())?;
            // And a moment more. Without it, on a busy machine, lavapipe and Xvfb sometimes
            // left the window showing the frame before the last for good: in 2 of 6 runs of a
            // heavy effect beside another test's, and in none of 12 with the pause.
            std::thread::sleep(PACE);
        }
    }
    Ok(())
}
