use anyhow::{ensure, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use crtsim_core::{
    config::{self, Config, Phase},
    mesh, ports, presets, sheet, PrepareOn, Renderer, Sequence,
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
    about = "Headless CRTSim renderer for images, videos and animations."
)]
struct Args {
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    /// Render an image, video or animation; the built-in test card, or the video test card for
    /// a moving output, if --input is omitted. The output's extension picks what is made:
    /// .mp4, .mkv or .webm a video; .gif or .webp an animation; .png a still, or an animated
    /// PNG of a moving input. Video and animation output, and video input, need FFmpeg.
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
        /// Save clean.png, signal.png and settings.json into a NEW directory. Stills only.
        #[arg(long)]
        debug_dir: Option<PathBuf>,
        #[command(flatten)]
        moving: Moving,
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
    /// Write looks as RetroArch shader presets into a NEW directory.
    ExportRetroarch(PortExport),
    /// Write looks as ReShade presets, with the effect, into a NEW directory.
    ExportReshade(PortExport),
    /// Write looks for the OBS filter into a NEW directory.
    ExportObs(PortExport),
    /// Write the OBS filter's data folder, its effect and images, into a NEW directory.
    ObsPluginData {
        #[arg(short, long)]
        output_dir: PathBuf,
    },
    /// Validate original meshes or export all attributes to portable JSON.
    InspectMeshes {
        #[arg(long)]
        export_dir: Option<PathBuf>,
    },
}
/// Looks to write as a shader port's presets.
#[derive(clap::Args)]
struct PortExport {
    #[arg(short, long)]
    output_dir: PathBuf,
    /// A look, as NAME=SETTINGS.json; the shipped looks when none is given.
    #[arg(long = "look", value_name = "NAME=SETTINGS.json")]
    looks: Vec<String>,
}
/// How a video or animation is made.
#[derive(clap::Args)]
#[command(next_help_heading = "Videos and animations")]
struct Moving {
    /// Video quality; lossless is FFV1 and needs .mkv.
    #[arg(long, value_enum, default_value = "balanced")]
    quality: QualityArg,
    /// stable: the source's rate, artifacts held still; ntsc60: 60 per second, alternating
    /// (videos only); off: no glow carried between frames.
    #[arg(long, value_enum, default_value = "stable")]
    timing: TimingArg,
    /// Video audio: keep it where the container holds it, re-encode it, or leave it out.
    #[arg(long, value_enum, default_value = "auto")]
    audio: AudioArg,
    /// Video encoder; hardware ones write H.264 to .mp4 or .mkv.
    #[arg(long, value_enum, default_value = "software")]
    encoder: EncoderArg,
    /// Animations: frames per second (GIF up to 30, WebP and PNG up to 60).
    #[arg(long, default_value_t = 24)]
    fps: u32,
    /// Animations: the CRT canvas's longest side, in pixels.
    #[arg(long, default_value_t = 640)]
    max_side: u32,
    /// Animations: where in the source to start, in seconds.
    #[arg(long, default_value_t = 0.)]
    start: f64,
    /// Animations: at most this many seconds; 0 for the rest of the source.
    #[arg(long, default_value_t = 10.)]
    seconds: f64,
    /// Lossless WebP animation.
    #[arg(long)]
    lossless: bool,
}
#[derive(Clone, Copy, ValueEnum)]
enum QualityArg {
    Draft,
    Balanced,
    High,
    Archival,
    Lossless,
}
#[derive(Clone, Copy, ValueEnum)]
enum TimingArg {
    Stable,
    Ntsc60,
    Off,
}
#[derive(Clone, Copy, ValueEnum)]
enum AudioArg {
    Auto,
    Encode,
    Mute,
}
#[derive(Clone, Copy, ValueEnum)]
enum EncoderArg {
    Software,
    Nvenc,
    Qsv,
    Amf,
    Videotoolbox,
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

/// Writes looks as `host`'s presets and prints each preset's path.
fn export_port(host: ports::Host, PortExport { output_dir, looks }: PortExport) -> Result<()> {
    let looks = if looks.is_empty() {
        ports::shipped()
    } else {
        looks_from_files(&looks)?
    };
    let looks: Vec<_> = looks.iter().map(|(name, c)| (name.as_str(), c)).collect();
    ports::write(&output_dir, &ports::export(host, &looks)?)?;
    for (name, _) in &looks {
        let preset = format!("{name}.{}", host.preset_extension());
        println!("{}", output_dir.join(preset).display());
    }
    Ok(())
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

/// What a render makes, by its output's extension and whether its input moves.
enum Making {
    Still,
    Video,
    Animation,
}

fn making(input: Option<&Path>, output: &Path) -> Result<Making> {
    let moving = input.is_some_and(|i| crtsim_media::MediaKind::of(i).is_moving());
    if crtsim_media::Container::of(output).is_some() {
        Ok(Making::Video)
    } else if let Some(format) = crtsim_media::AnimationFormat::of(output) {
        Ok(match format {
            crtsim_media::AnimationFormat::Apng if !moving => Making::Still,
            _ => Making::Animation,
        })
    } else {
        anyhow::bail!("The output must be .png, .mp4, .mkv, .webm, .gif or .webp")
    }
}

/// Renders a video or an animation of `input`, or of the video test card.
fn render_moving(
    input: Option<&Path>,
    output: &Path,
    config: &Config,
    moving: &Moving,
    animation: bool,
    backend: Backend,
) -> Result<()> {
    use crtsim_media::{AnimationOptions, Audio, Encoder, Options, Quality, Timing};
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let video = match input {
        Some(path) => crtsim_media::probe(path, &cancel)?,
        None => crtsim_media::test_clip(),
    };
    let timing = match moving.timing {
        TimingArg::Stable => Timing::Stable,
        TimingArg::Ntsc60 => Timing::Ntsc60,
        TimingArg::Off => Timing::Disabled,
    };
    eprintln!(
        "Input {}x{} at {:.3} frames per second, {:.2}s",
        video.size.0, video.size.1, video.fps, video.duration
    );
    let renderer = renderer(backend)?;
    let started = std::time::Instant::now();
    // Each stage once, and its progress in whole tenths.
    let mut said = (String::new(), -1);
    let progress = |p: crtsim_media::Progress| {
        let tenth = (p.fraction * 10.).floor() as i32;
        if p.stage != said.0 || tenth != said.1 {
            eprintln!("{:3.0}% {}", p.fraction * 100., p.stage);
            said = (p.stage, tenth);
        }
    };
    if animation {
        let options = AnimationOptions {
            max_side: moving.max_side,
            fps: moving.fps,
            timing,
            start: moving.start,
            max_seconds: (moving.seconds > 0.).then_some(moving.seconds),
            lossless: moving.lossless,
            ..AnimationOptions::default()
        };
        crtsim_media::export_animation(
            &video, output, config, &options, &renderer, &cancel, progress,
        )?;
    } else {
        let options = Options {
            timing,
            audio: match moving.audio {
                AudioArg::Auto => Audio::Auto,
                AudioArg::Encode => Audio::Encode,
                AudioArg::Mute => Audio::Mute,
            },
            quality: match moving.quality {
                QualityArg::Draft => Quality::Draft,
                QualityArg::Balanced => Quality::Balanced,
                QualityArg::High => Quality::High,
                QualityArg::Archival => Quality::Archival,
                QualityArg::Lossless => Quality::Lossless,
            },
            encoder: match moving.encoder {
                EncoderArg::Software => Encoder::Software,
                EncoderArg::Nvenc => Encoder::Nvenc,
                EncoderArg::Qsv => Encoder::Qsv,
                EncoderArg::Amf => Encoder::Amf,
                EncoderArg::Videotoolbox => Encoder::VideoToolbox,
            },
            ..Options::default()
        };
        options.validate()?;
        crtsim_media::export(
            &video, output, config, &options, &renderer, &cancel, progress,
        )?;
    }
    eprintln!(
        "Saved {} in {:.2}s",
        output.display(),
        started.elapsed().as_secs_f64()
    );
    Ok(())
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
        Command::ExportRetroarch(export) => export_port(ports::Host::RetroArch, export)?,
        Command::ExportReshade(export) => export_port(ports::Host::ReShade, export)?,
        Command::ExportObs(export) => export_port(ports::Host::Obs, export)?,
        Command::ObsPluginData { output_dir } => {
            ports::write(&output_dir, &ports::obs::plugin_data()?)?
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
            moving,
        } => {
            let making = making(input.as_deref(), &output)?;
            ensure!(!output.exists(), "Output already exists; choose a new path");
            if !matches!(making, Making::Still) {
                ensure!(debug_dir.is_none(), "--debug-dir is for stills");
            }
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
            if let Making::Video | Making::Animation = making {
                let animation = matches!(making, Making::Animation);
                return render_moving(input.as_deref(), &output, &c, &moving, animation, backend);
            }
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
