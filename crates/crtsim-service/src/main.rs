use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};
use std::time::Duration;

/// Renders pictures, animations and videos through the CRT over HTTP. See docs/SERVICE.md.
#[derive(Parser)]
#[command(version, about)]
struct Args {
    /// Where to listen. Anything but this computer should reach it through a reverse proxy
    /// that speaks HTTPS.
    #[arg(long, default_value = "127.0.0.1:8080")]
    bind: std::net::SocketAddr,
    /// A file holding the token every request but /health must send, as Authorization:
    /// Bearer <token>. CRTSIM_SERVICE_TOKEN holds it instead, when set.
    #[arg(long)]
    token_file: Option<std::path::PathBuf>,
    #[arg(long, value_enum, default_value = "auto")]
    backend: Backend,
    /// The largest upload, in megabytes.
    #[arg(long, default_value_t = 64)]
    max_upload_mb: u64,
    /// The most pixels a still may have, in millions.
    #[arg(long, default_value_t = 16.8)]
    max_input_megapixels: f64,
    /// The most pixels a still's CRT canvas may have, in millions: 8.3 is 4K.
    #[arg(long, default_value_t = 8.3)]
    max_output_megapixels: f64,
    /// The longest video, or stretch of an animation, in seconds.
    #[arg(long, default_value_t = 30.)]
    max_seconds: f64,
    /// The longest side of an animation's or a video's canvas, in pixels.
    #[arg(long, default_value_t = 1280)]
    max_moving_side: u32,
    /// Jobs that may wait for the renderer; more are refused until some finish.
    #[arg(long, default_value_t = 16)]
    max_queue: usize,
    /// Connections handled at once; more are refused.
    #[arg(long, default_value_t = 64)]
    max_connections: usize,
    /// The longest a still may render, in seconds.
    #[arg(long, default_value_t = 120)]
    still_timeout: u64,
    /// The longest an animation or a video may render, in seconds.
    #[arg(long, default_value_t = 900)]
    moving_timeout: u64,
    /// How long POST /render waits for its result before answering with the job instead.
    #[arg(long, default_value_t = 60)]
    wait: u64,
    /// How long a finished job's result is kept, in seconds.
    #[arg(long, default_value_t = 600)]
    keep: u64,
}

#[derive(Clone, Copy, ValueEnum)]
enum Backend {
    Auto,
    Vulkan,
    Dx12,
    Metal,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let token = match (std::env::var("CRTSIM_SERVICE_TOKEN"), &args.token_file) {
        (Ok(token), _) => Some(token),
        (Err(_), Some(path)) => Some(
            std::fs::read_to_string(path)
                .with_context(|| format!("Cannot read {}", path.display()))?,
        ),
        (Err(_), None) => None,
    }
    .map(|token| token.trim().to_owned());
    anyhow::ensure!(
        token.as_ref().is_none_or(|t| t.len() >= 16),
        "The token must be at least 16 characters"
    );
    if token.is_none() && !args.bind.ip().is_loopback() {
        eprintln!(
            "Warning: listening on {} with no token; anyone who reaches it can queue work",
            args.bind
        );
    }
    let megapixels = |m: f64| (m * 1_000_000.) as u64;
    let settings = crtsim_service::Settings {
        bind: args.bind,
        token,
        limits: crtsim_service::spec::Limits {
            upload: args.max_upload_mb * 1024 * 1024,
            input_pixels: megapixels(args.max_input_megapixels),
            output_pixels: megapixels(args.max_output_megapixels),
            seconds: args.max_seconds,
            moving_side: args.max_moving_side,
        },
        jobs: crtsim_service::jobs::Settings {
            waiting: args.max_queue,
            still_timeout: Duration::from_secs(args.still_timeout),
            moving_timeout: Duration::from_secs(args.moving_timeout),
            keep: Duration::from_secs(args.keep),
        },
        connections: args.max_connections,
        wait: Duration::from_secs(args.wait),
        ..Default::default()
    };
    let backends = match args.backend {
        Backend::Auto => wgpu::Backends::PRIMARY,
        Backend::Vulkan => wgpu::Backends::VULKAN,
        Backend::Dx12 => wgpu::Backends::DX12,
        Backend::Metal => wgpu::Backends::METAL,
    };
    let renderer = pollster::block_on(crtsim_core::Renderer::new(backends))?;
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let tools = crtsim_media::ToolCheck::run(&cancel);
    let ffmpeg = tools.ready();
    eprintln!(
        "Adapter: {} ({:?}); animations and videos: {}",
        renderer.adapter.name,
        renderer.adapter.backend,
        if ffmpeg {
            "yes"
        } else {
            "no, FFmpeg was not found"
        }
    );
    let (service, listener) = crtsim_service::Service::start(settings, renderer, ffmpeg)?;
    eprintln!("Listening on http://{}", service.address);
    service.serve(listener);
    Ok(())
}
