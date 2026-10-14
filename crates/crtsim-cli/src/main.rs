use anyhow::{ensure, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use crtsim_core::{
    config::{self, Config, Phase},
    mesh, presets, retroarch, sheet, PrepareOn, Renderer, Sequence,
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
    about = "Headless CRTSim still-image renderer. The desktop app renders video."
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
        #[arg(long, conflicts_with = "preset")]
        config: Option<PathBuf>,
        /// An included preset by name, such as "NTSC 240p"; `presets` lists them.
        #[arg(long)]
        preset: Option<String>,
        /// original, auto, native, 240p, 288p, 360p, 480p, 576p or WIDTHxHEIGHT
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
        /// Where to resize, edit and grade the input. The GPU by default; cpu runs the
        /// original implementation, for comparing the two.
        #[arg(long, value_enum, default_value = "gpu")]
        prepare: PrepareArg,
        /// Save clean.png, signal.png and settings.json into a NEW directory.
        #[arg(long)]
        debug_dir: Option<PathBuf>,
    },
    /// Print the included presets and the resolution presets.
    Presets,
    /// Render one image through many looks into a labelled grid, to compare them: every
    /// included preset, or the looks given.
    ContactSheet {
        /// The image; the built-in test card if omitted.
        #[arg(short, long)]
        input: Option<PathBuf>,
        #[arg(short, long)]
        output: PathBuf,
        /// An included preset to show, by name; repeat for more.
        #[arg(long = "preset", value_name = "NAME")]
        presets: Vec<String>,
        /// Your own look to show, as NAME=SETTINGS.json; repeat for more.
        #[arg(long = "look", value_name = "NAME=SETTINGS.json")]
        looks: Vec<String>,
        /// Each tile's size, WIDTHxHEIGHT.
        #[arg(long, default_value = "480x360")]
        tile: String,
        #[arg(long, default_value_t = 4)]
        columns: u32,
        #[arg(long, value_enum, default_value = "auto")]
        backend: Backend,
    },
    /// Write a starting configuration. Never overwrites an existing file.
    Config {
        #[arg(short, long)]
        output: PathBuf,
        /// The desktop's General image preset instead of the public-reference defaults.
        #[arg(long)]
        general: bool,
    },
    /// Write looks as RetroArch shader presets into a NEW directory: the shipped looks, or
    /// your own, each given as NAME=SETTINGS.json.
    ExportRetroarch {
        #[arg(short, long)]
        output_dir: PathBuf,
        #[arg(long = "look", value_name = "NAME=SETTINGS.json")]
        looks: Vec<String>,
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
enum PrepareArg {
    Gpu,
    Cpu,
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
/// The included preset called `name`, or an error listing them.
fn preset(name: &str) -> Result<Config> {
    presets::named(name).map(|p| p.config).with_context(|| {
        let names: Vec<_> = presets::included().iter().map(|p| p.name).collect();
        format!(
            "No included preset is called {name:?}. They are: {}",
            names.join(", ")
        )
    })
}

/// Looks given as NAME=SETTINGS.json.
fn looks_from_files(looks: &[String]) -> Result<Vec<(String, Config)>> {
    looks
        .iter()
        .map(|look| {
            let (name, path) = look
                .split_once('=')
                .context("Give each look as NAME=SETTINGS.json")?;
            let bytes = fs::read(path).with_context(|| format!("Cannot read {path}"))?;
            Ok((name.to_string(), Config::from_json_slice(&bytes)?))
        })
        .collect()
}

fn renderer(backend: Backend) -> Result<Renderer> {
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
    Ok(renderer)
}

/// The image at `input`, or the built-in test card.
fn source(input: Option<&Path>) -> Result<image::RgbaImage> {
    match input {
        Some(path) => crtsim_core::input::load_image(path),
        None => Ok(config::test_card()),
    }
}

fn ensure_new_png(output: &Path) -> Result<()> {
    ensure!(!output.exists(), "Output already exists; choose a new path");
    ensure!(
        output
            .extension()
            .is_some_and(|s| s.eq_ignore_ascii_case("png")),
        "The output must be a .png"
    );
    Ok(())
}

fn main() -> Result<()> {
    match Args::parse().command {
        Command::Presets => {
            println!("Included presets, for render --preset and contact-sheet --preset:");
            for preset in presets::included() {
                println!("  {}: {}", preset.name, preset.description);
            }
            println!("Signal: original (256x224), auto (up to 480 rows, preserves aspect), native, 240p, 288p, 360p, 480p, 576p, WIDTHxHEIGHT\nOutput: reference (1600x900), 720p, 1080p, 1440p, 4k, match-input, WIDTHxHEIGHT\nUse config --general for square-pixel images and contain fitting. Default is CRTSim Reference.");
        }
        Command::Config { output, general } => {
            let c = if general {
                Config::general()
            } else {
                Config::default()
            };
            let mut file = new_file(&output)?;
            file.write_all(serde_json::to_string_pretty(&c)?.as_bytes())?;
        }
        Command::ExportRetroarch { output_dir, looks } => {
            let looks = if looks.is_empty() {
                retroarch::shipped()
            } else {
                looks_from_files(&looks)?
            };
            let looks: Vec<_> = looks.iter().map(|(name, c)| (name.as_str(), c)).collect();
            retroarch::write(&output_dir, &retroarch::export(&looks)?)?;
            for (name, _) in &looks {
                println!("{}", output_dir.join(format!("{name}.slangp")).display());
            }
        }
        Command::ContactSheet {
            input,
            output,
            presets: names,
            looks,
            tile,
            columns,
            backend,
        } => {
            ensure_new_png(&output)?;
            let size = |text: &str| -> Option<(u32, u32)> {
                let (w, h) = text.split_once('x')?;
                Some((w.parse().ok()?, h.parse().ok()?))
            };
            let tile = size(&tile)
                .filter(|&(w, h)| (16..=4096).contains(&w) && (16..=4096).contains(&h))
                .context("--tile must be WIDTHxHEIGHT, each 16 to 4096 pixels")?;
            let mut chosen: Vec<sheet::Look> = names
                .iter()
                .map(|name| {
                    Ok(sheet::Look {
                        name: presets::named(name).map_or(name.clone(), |p| p.name.into()),
                        config: preset(name)?,
                    })
                })
                .collect::<Result<_>>()?;
            chosen.extend(
                looks_from_files(&looks)?
                    .into_iter()
                    .map(|(name, config)| sheet::Look { name, config }),
            );
            if chosen.is_empty() {
                chosen = presets::included()
                    .into_iter()
                    .map(|p| sheet::Look {
                        name: p.name.into(),
                        config: p.config,
                    })
                    .collect();
            }
            let src = source(input.as_deref())?;
            let renderer = renderer(backend)?;
            let started = std::time::Instant::now();
            let count = chosen.len();
            let made = pollster::block_on(sheet::render(
                &renderer,
                &src,
                &chosen,
                tile,
                columns,
                None,
                |done| eprintln!("Rendered {done} of {count}: {}", chosen[done - 1].name),
            ))?;
            save_png(&output, made)?;
            eprintln!(
                "Saved {} in {:.2}s",
                output.display(),
                started.elapsed().as_secs_f64()
            );
        }
        Command::InspectMeshes { export_dir } => {
            if let Some(ref dir) = export_dir {
                fs::create_dir(dir)
                    .context("export directory must be new and its parent must exist")?;
            }
            for (name, bytes) in [("screen", mesh::SCREEN), ("bezel", mesh::BEZEL)] {
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
            preset: preset_name,
            signal,
            size,
            phase,
            warmup,
            backend,
            prepare,
            debug_dir,
        } => {
            ensure_new_png(&output)?;
            if let Some(ref dir) = debug_dir {
                ensure!(!dir.exists(), "debug directory must be new");
            }
            let mut c = if let Some(path) = config {
                Config::from_json_slice(&fs::read(path)?)?
            } else if let Some(name) = preset_name {
                preset(&name)?
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
            let src = source(input.as_deref())?;
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
            let mut renderer = renderer(backend)?;
            renderer.prepare = match prepare {
                PrepareArg::Gpu => PrepareOn::Gpu,
                PrepareArg::Cpu => PrepareOn::Cpu,
            };
            let started = std::time::Instant::now();
            let mut sequence = Sequence::still();
            pollster::block_on(renderer.frame(&mut sequence, &src, &c, None, |_| {}))?;
            save_png(&output, pollster::block_on(renderer.read(&mut sequence))?)?;
            // Only a debug run reads back the images the frame was made from.
            if let Some(dir) = debug_dir {
                fs::create_dir(&dir)?;
                let signals = pollster::block_on(renderer.signals(&sequence))?;
                save_png(&dir.join("clean.png"), signals.clean)?;
                save_png(&dir.join("signal.png"), signals.signal)?;
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
