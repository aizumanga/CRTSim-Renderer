//! Renders one "shot" of a showcase video: a sequence of CRT frames with shared history.
//! Usage: shotgen SHOT.json
//! {
//!   "base": "default" | "general",
//!   "config": { ...overrides... },
//!   "source": {"test_clip": START, "step": 2} | {"dir": "frames/"} | {"image": "x.png"} | {"test_card": true},
//!   "frames": N, "fps": 30, "timing": "stable"|"ntsc60"|"disabled",
//!   "animate": [{"path": "barrel", "from": 0, "to": -0.4, "start": 0, "end": 1, "ease": "inout"}],
//!   "out": "dir/"
//! }
use anyhow::{Context, Result};
use crtsim_core::{config::Config, Renderer, Sequence, Timing};
use serde_json::Value;
use std::path::PathBuf;

fn merge(base: &mut Value, over: &Value) {
    match (base, over) {
        (Value::Object(b), Value::Object(o)) => {
            for (k, v) in o {
                merge(b.entry(k.clone()).or_insert(Value::Null), v);
            }
        }
        (b, o) => *b = o.clone(),
    }
}

fn set_path(v: &mut Value, path: &str, x: f64) {
    let mut cur = v;
    for part in path.split('.') {
        cur = match part.parse::<usize>() {
            Ok(i) => &mut cur[i],
            Err(_) => &mut cur[part],
        };
    }
    *cur = serde_json::json!(x);
}

fn ease(kind: &str, t: f64) -> f64 {
    let t = t.clamp(0., 1.);
    match kind {
        "in" => t * t * t,
        "out" => 1. - (1. - t).powi(3),
        "inout" => {
            if t < 0.5 {
                4. * t * t * t
            } else {
                1. - (-2. * t + 2.).powi(3) / 2.
            }
        }
        "sine" => 0.5 - 0.5 * (t * std::f64::consts::TAU).cos(),
        _ => t,
    }
}

fn main() -> Result<()> {
    let shot_path = std::env::args().nth(1).context("shot json")?;
    let shot: Value = serde_json::from_slice(&std::fs::read(&shot_path)?)?;
    let base = match shot["base"].as_str().unwrap_or("default") {
        "general" => Config::general(),
        _ => Config::default(),
    };
    let mut cfg_value = serde_json::to_value(&base)?;
    merge(&mut cfg_value, &shot["config"]);
    let frames = shot["frames"].as_u64().unwrap_or(1);
    let fps = shot["fps"].as_f64().unwrap_or(30.);
    let timing = match shot["timing"].as_str().unwrap_or("stable") {
        "ntsc60" => Timing::Ntsc60,
        "disabled" => Timing::Disabled,
        _ => Timing::Stable,
    };
    let out = PathBuf::from(shot["out"].as_str().context("out")?);
    std::fs::create_dir_all(&out)?;
    let src = &shot["source"];
    let dir_frames: Vec<PathBuf> = if let Some(d) = src["dir"].as_str() {
        let mut v: Vec<_> = std::fs::read_dir(d)?
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|e| e == "png"))
            .collect();
        v.sort();
        v
    } else {
        vec![]
    };
    let still = if let Some(p) = src["image"].as_str() {
        Some(image::open(p)?.to_rgba8())
    } else if src["test_card"].as_bool() == Some(true) {
        Some(crtsim_core::config::test_card())
    } else {
        None
    };
    let renderer = pollster::block_on(Renderer::new(wgpu::Backends::VULKAN))?;
    eprintln!("adapter {}", renderer.adapter.name);
    let mut seq = Sequence::video(timing, fps);
    let anims = shot["animate"].as_array().cloned().unwrap_or_default();
    let t0 = std::time::Instant::now();
    for i in 0..frames {
        let input = if let Some(s) = &still {
            s.clone()
        } else if !dir_frames.is_empty() {
            let start = src["dir_start"].as_u64().unwrap_or(0) as usize;
            let idx = (start + i as usize).min(dir_frames.len() - 1);
            image::open(&dir_frames[idx])?.to_rgba8()
        } else {
            let start = src["test_clip"].as_u64().unwrap_or(0);
            let step = src["step"].as_u64().unwrap_or(2);
            crtsim_core::test_clip::frame(start + i * step)
        };
        if shot["dump_source"].as_bool() == Some(true) {
            input.save(out.join(format!("{:05}.png", i)))?;
            continue;
        }
        let mut v = cfg_value.clone();
        let t = if frames > 1 { i as f64 / (frames - 1) as f64 } else { 0. };
        for a in &anims {
            let s = a["start"].as_f64().unwrap_or(0.);
            let e = a["end"].as_f64().unwrap_or(1.);
            let local = ((t - s) / (e - s).max(1e-9)).clamp(0., 1.);
            let k = ease(a["ease"].as_str().unwrap_or("inout"), local);
            let from = a["from"].as_f64().unwrap();
            let to = a["to"].as_f64().unwrap();
            set_path(&mut v, a["path"].as_str().unwrap(), from + (to - from) * k);
        }
        let mut cfg: Config = serde_json::from_value(v)?;
        if shot["mask_signal"].as_bool() == Some(true) {
            cfg.mask_repeats = crtsim_core::config::MaskRepeats::Signal;
        }
        pollster::block_on(renderer.frame(&mut seq, &input, &cfg, None, |_| {}))?;
        let img = pollster::block_on(renderer.read(&mut seq))?;
        img.save(out.join(format!("{:05}.png", i)))?;
        if i % 10 == 0 {
            eprintln!("{} frame {i}/{frames} {:.1}s", shot_path, t0.elapsed().as_secs_f64());
        }
    }
    Ok(())
}
