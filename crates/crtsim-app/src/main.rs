//! The desktop app's command line.
use crtsim_app::{
    native::{self, Launch},
    Smoke,
};
use std::path::PathBuf;

fn main() -> eframe::Result<()> {
    let mut input = None;
    let mut smoke = None;
    // The backend --backend asks for; none lets wgpu choose among the primary ones.
    let mut backend = None;
    let mut smoke_welcome = false;
    let mut smoke_gallery = false;
    let mut smoke_lut_gallery = false;
    let mut smoke_export = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--smoke-welcome" => smoke_welcome = true,
            "--smoke-gallery" => smoke_gallery = true,
            "--smoke-lut-gallery" => smoke_lut_gallery = true,
            "--smoke-export" => smoke_export = true,
            "--backend" => {
                backend = match args.next().as_deref() {
                    Some("vulkan") => Some(wgpu::Backends::VULKAN),
                    Some("dx12") => Some(wgpu::Backends::DX12),
                    Some("metal") => Some(wgpu::Backends::METAL),
                    Some("auto") => None,
                    _ => {
                        eprintln!("Expected --backend auto|vulkan|dx12|metal");
                        std::process::exit(2);
                    }
                }
            }
            "--smoke-test" => {
                smoke = Some(PathBuf::from(
                    args.next().expect("--smoke-test needs a new PNG path"),
                ))
            }
            "--help" | "-h" => {
                println!(
                    "crtsim-desktop [IMAGE_OR_VIDEO] [--backend auto|vulkan|dx12|metal]\n\
                     Open media, adjust effects, load/save presets and export PNG or video from \
                     the window.\n\
                     Video requires FFmpeg and ffprobe on PATH."
                );
                return Ok(());
            }
            s if s.starts_with('-') => {
                eprintln!("Unknown option: {s}");
                std::process::exit(2);
            }
            _ if input.is_none() => input = Some(PathBuf::from(arg)),
            _ => {
                eprintln!("Only one image or video can be opened at startup");
                std::process::exit(2);
            }
        }
    }
    let smoke = smoke.map(|screenshot| {
        let mut smoke = Smoke::new(screenshot);
        smoke.welcome = smoke_welcome;
        smoke.export = smoke_export;
        smoke.gallery = smoke_gallery;
        smoke.lut_gallery = smoke_lut_gallery;
        smoke
    });
    native::run(Launch {
        input,
        backend,
        smoke,
    })
}
