//! Renders a 256x224 RGBA probe through CRTSim-Renderer's Super Win the Game look as a 60 fps
//! alternating sequence, saving each kept frame's prepared signal, composite signal and final
//! picture. ARTIFACTS and BLOOM override those settings; FLIP_ARTIFACTS=1 flips the pattern.
//! usage: render_probe PROBE.rgba OUTDIR FRAMES KEEP_FROM [WIDTHxHEIGHT]
use anyhow::Result;
use crtsim_core::{config::{Config, Phase}, Renderer, Sequence, Timing};
use image::RgbaImage;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let probe = RgbaImage::from_raw(256, 224, std::fs::read(&args[1])?).expect("256x224 RGBA");
    let out = std::path::Path::new(&args[2]);
    std::fs::create_dir_all(out)?;
    let (frames, keep_from): (u64, u64) = (args[3].parse()?, args[4].parse()?);
    let size = args.get(5).cloned().unwrap_or("1280x960".into());
    // The gallery's Super Win the Game look.
    let config = Config {
        fov: 30.,
        phase: Phase::Alternating,
        ntsc_blending: 0.35,
        palette: Some(Default::default()),
        output: size,
        artifacts: std::env::var("ARTIFACTS").map_or(0.5, |v| v.parse().unwrap()),
        bloom: std::env::var("BLOOM").map_or(0.25, |v| v.parse().unwrap()),
        flip_artifacts: std::env::var("FLIP_ARTIFACTS").is_ok_and(|v| v == "1"),
        ..Config::default()
    };
    let renderer = pollster::block_on(Renderer::new(wgpu::Backends::VULKAN))?;
    // Ntsc60: alternating phase, persistence per tick as the game applies it.
    let mut sequence = Sequence::video(Timing::Ntsc60, 60.);
    for n in 0..frames {
        pollster::block_on(renderer.frame(&mut sequence, &probe, &config, None, |_| ()))?;
        if n >= keep_from {
            // The first frame draws ticks 0 to its warm-up; each after, one more.
            let tick = u64::from(config.warmup) + n;
            let lerp = if tick % 2 == 0 { 0.175 } else { 0.825 };
            let signals = pollster::block_on(renderer.signals(&sequence))?;
            let picture = pollster::block_on(renderer.read(&mut sequence))?;
            let name = |what: &str| out.join(format!("frame{n:03}_lerp{lerp}_{what}.png"));
            signals.clean.save(name("prepared"))?;
            signals.signal.save(name("composite"))?;
            picture.save(name("final"))?;
        }
    }
    Ok(())
}

