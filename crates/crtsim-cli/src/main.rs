use anyhow::{ensure, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use crtsim_core::{
    config::{self, Config, Filter, Fit, Phase},
    mesh, Renderer,
};
use image::{DynamicImage, ImageOutputFormat};
use std::{
    fs::{self, OpenOptions},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
};

#[derive(Parser)]
#[command(
    version,
    about = "Phase 0: headless CRTSim still-image renderer (no GUI/video yet)"
)]
struct Args {
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    /// Render an image, or the built-in original test card if --input is omitted.
    Render {
        #[arg(short, long)]
        input: Option<PathBuf>,
        #[arg(short, long)]
        output: PathBuf,
        /// JSON overrides; omitted fields use the public-reference defaults.
        #[arg(long)]
        config: Option<PathBuf>,
        /// original, auto, native, 240p, 360p, 480p or WIDTHxHEIGHT
        #[arg(long)]
        signal: Option<String>,
        /// reference, 720p, 1080p, 1440p, 4k, match-input or WIDTHxHEIGHT
        #[arg(long)]
        size: Option<String>,
        #[arg(long, value_enum)]
        phase: Option<PhaseArg>,
        /// Additional ticks before saving (0..240). Default 16, not a convergence guarantee.
        #[arg(long)]
        warmup: Option<u32>,
        #[arg(long, value_enum, default_value = "auto")]
        backend: Backend,
        /// Save clean.png, signal.png and settings.json into a NEW directory.
        #[arg(long)]
        debug_dir: Option<PathBuf>,
    },
    /// Print the resolution presets. Sizes never select image type automatically.
    Presets,
    /// Write a starting configuration. Never overwrites an existing file.
    Config {
        #[arg(short, long)]
        output: PathBuf,
        #[arg(long)]
        general: bool,
    },
    /// Validate original meshes or export all attributes to portable JSON.
    InspectMeshes {
        #[arg(long)]
        export_dir: Option<PathBuf>,
    },
}
#[derive(Clone, Copy, ValueEnum)]
enum Backend {
    Auto,
    Vulkan,
    Dx12,
    Metal,
}
#[derive(Clone, Copy, ValueEnum)]
enum PhaseArg {
    Stable,
    A,
    B,
    Alternating,
}

fn new_file(path: &Path) -> Result<fs::File> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| {
            format!(
                "Cannot create {} (parent must exist; files are never overwritten)",
                path.display()
            )
        })
}
fn save_png(path: &Path, img: image::RgbaImage) -> Result<()> {
    let mut file = BufWriter::new(new_file(path)?);
    DynamicImage::ImageRgba8(img).write_to(&mut file, ImageOutputFormat::Png)?;
    file.flush()?;
    Ok(())
}
fn main() -> Result<()> {
    match Args::parse().command {
        Command::Presets => {
            println!("Signal: original (256x224), auto (up to 480 rows, preserves aspect), native, 240p, 360p, 480p, WIDTHxHEIGHT\nOutput: reference (1600x900), 720p, 1080p, 1440p, 4k, match-input, WIDTHxHEIGHT\nUse config --general for square-pixel images and contain fitting. Default is CRTSim Reference.");
        }
        Command::Config { output, general } => {
            let mut c = Config::default();
            if general {
                c.signal = "auto".into();
                c.output = "1080p".into();
                c.pixel_aspect = 1.;
                c.fit = Fit::Contain;
                c.filter = Filter::Lanczos;
            }
            let mut file = new_file(&output)?;
            file.write_all(serde_json::to_string_pretty(&c)?.as_bytes())?;
        }
        Command::InspectMeshes { export_dir } => {
            if let Some(ref dir) = export_dir {
                fs::create_dir(dir)
                    .context("export directory must be new and its parent must exist")?;
            }
            for (name, bytes) in [("screen", mesh::SCREEN), ("frame", mesh::FRAME)] {
                let m = mesh::Mesh::read(bytes)?;
                println!(
                    "{name}: {} vertices, {} triangles, streams {:?}",
                    m.vertices.len(),
                    m.indices.len() / 3,
                    m.streams
                );
                if let Some(ref dir) = export_dir {
                    let mut file = new_file(&dir.join(format!("{name}.json")))?;
                    file.write_all(serde_json::to_string(&m)?.as_bytes())?;
                }
            }
        }
        Command::Render {
            input,
            output,
            config,
            signal,
            size,
            phase,
            warmup,
            backend,
            debug_dir,
        } => {
            ensure!(!output.exists(), "Output already exists; choose a new path");
            ensure!(
                output
                    .extension()
                    .is_some_and(|s| s.eq_ignore_ascii_case("png")),
                "Phase 0 output must use .png"
            );
            if let Some(ref dir) = debug_dir {
                ensure!(!dir.exists(), "debug directory must be new");
            }
            let mut c = if let Some(path) = config {
                serde_json::from_slice::<Config>(&fs::read(path)?)?
            } else {
                Config::default()
            };
            if let Some(v) = signal {
                c.signal = v;
            }
            if let Some(v) = size {
                c.output = v;
            }
            if let Some(v) = warmup {
                c.warmup = v;
            }
            if let Some(v) = phase {
                c.phase = match v {
                    PhaseArg::Stable => Phase::Stable,
                    PhaseArg::A => Phase::A,
                    PhaseArg::B => Phase::B,
                    PhaseArg::Alternating => Phase::Alternating,
                };
            }
            c.validate()?;
            let src = if let Some(path) = input {
                let dims =
                    image::image_dimensions(&path).context("cannot inspect input dimensions")?;
                config::validate_size(dims)?;
                let mut reader = image::io::Reader::open(path)?.with_guessed_format()?;
                let mut limits = image::io::Limits::default();
                limits.max_alloc = Some(512 * 1024 * 1024);
                reader.limits(limits);
                reader.decode()?.to_rgba8()
            } else {
                config::test_card()
            };
            let sig = c.signal_size(src.dimensions())?;
            let out = c.output_size(src.dimensions())?;
            eprintln!(
                "Input {}x{} -> signal {}x{} -> output {}x{}; {} ticks",
                src.width(),
                src.height(),
                sig.0,
                sig.1,
                out.0,
                out.1,
                c.warmup + 1
            );
            let backends = match backend {
                Backend::Auto => wgpu::Backends::PRIMARY,
                Backend::Vulkan => wgpu::Backends::VULKAN,
                Backend::Dx12 => wgpu::Backends::DX12,
                Backend::Metal => wgpu::Backends::METAL,
            };
            let renderer = pollster::block_on(Renderer::new(backends))?;
            eprintln!(
                "Adapter: {} ({:?}, {:?})",
                renderer.adapter.name, renderer.adapter.backend, renderer.adapter.device_type
            );
            let started = std::time::Instant::now();
            let rendered = renderer.render(&src, &c)?;
            save_png(&output, rendered.crt)?;
            if let Some(dir) = debug_dir {
                fs::create_dir(&dir)?;
                save_png(&dir.join("clean.png"), rendered.clean)?;
                save_png(&dir.join("signal.png"), rendered.signal)?;
                let mut file = new_file(&dir.join("settings.json"))?;
                file.write_all(serde_json::to_string_pretty(&c)?.as_bytes())?;
            }
            eprintln!(
                "Saved {} in {:.2}s",
                output.display(),
                started.elapsed().as_secs_f64()
            );
        }
    }
    Ok(())
}
