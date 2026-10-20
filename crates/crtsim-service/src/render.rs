//! A job rendered: a still as a PNG in its colour space, or an animation or a video through the
//! media exports, which FFmpeg encodes.

use crate::jobs::Failure;
use crate::spec::{Format, Input, Spec};
use crtsim_core::{Renderer, Sequence};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

fn unprocessable(error: anyhow::Error) -> Failure {
    Failure {
        status: 422,
        message: format!("{error:#}"),
    }
}

fn failed(error: anyhow::Error) -> Failure {
    Failure {
        status: 500,
        message: format!("{error:#}"),
    }
}

/// Renders `spec` on `renderer` into a file in `folder` named for job `id`, reporting how far
/// it has got and stopping once `cancel` is set.
pub fn run(
    renderer: &Renderer,
    spec: Spec,
    folder: &Path,
    id: &str,
    cancel: &Arc<AtomicBool>,
    mut report: impl FnMut(f32, String),
) -> Result<PathBuf, Failure> {
    let output = folder.join(format!("{id}.{}", spec.format.extension()));
    match spec.input {
        Input::Still(ref bytes) => {
            report(0.05, "Reading the picture".into());
            let decoded =
                crtsim_core::input::read_bytes(bytes, spec.profiles).map_err(unprocessable)?;
            let mut sequence = Sequence::still();
            pollster::block_on(renderer.frame(
                &mut sequence,
                &decoded.image,
                &spec.config,
                Some(cancel),
                |progress| report(0.1 + progress.fraction * 0.75, "Rendering".into()),
            ))
            .map_err(failed)?;
            let image = pollster::block_on(renderer.read(&mut sequence)).map_err(failed)?;
            report(0.9, "Encoding the PNG".into());
            let png = spec.space.png(&image).map_err(failed)?;
            std::fs::write(&output, png).map_err(|e| failed(e.into()))?;
            Ok(output)
        }
        Input::Moving {
            ref bytes,
            extension,
        } => {
            let input = folder.join(format!("{id}-input.{extension}"));
            std::fs::write(&input, bytes).map_err(|e| failed(e.into()))?;
            let rendered = moving(renderer, &spec, &input, &output, cancel, &mut report);
            let _ = std::fs::remove_file(&input);
            rendered.map(|()| output)
        }
    }
}

fn moving(
    renderer: &Renderer,
    spec: &Spec,
    input: &Path,
    output: &Path,
    cancel: &Arc<AtomicBool>,
    report: &mut impl FnMut(f32, String),
) -> Result<(), Failure> {
    report(0.02, "Reading the video".into());
    let video = crtsim_media::probe(input, cancel).map_err(unprocessable)?;
    let progress = |p: crtsim_media::Progress| report(p.fraction, p.stage);
    if spec.format.video() {
        if video.duration > spec.seconds {
            return Err(unprocessable(anyhow::anyhow!(
                "The video is {:.1} seconds; this service renders at most {}. Trim it, or ask \
                 for an animation of part of it",
                video.duration,
                spec.seconds
            )));
        }
        let config = even_canvas(&spec.config, video.size, spec.animation.max_side)
            .map_err(unprocessable)?;
        crtsim_media::export(
            &video,
            output,
            &config,
            &spec.video,
            renderer,
            cancel,
            progress,
        )
        .map_err(failed)
    } else {
        debug_assert!(matches!(
            spec.format,
            Format::Gif | Format::Webp | Format::Png
        ));
        crtsim_media::export_animation(
            &video,
            output,
            &spec.config,
            &spec.animation,
            renderer,
            cancel,
            progress,
        )
        .map_err(failed)
    }
}

/// `config` with its canvas at most `side` across and of even size, which video needs.
fn even_canvas(
    config: &crtsim_core::config::Config,
    input: (u32, u32),
    side: u32,
) -> anyhow::Result<crtsim_core::config::Config> {
    let mut config = config.with_max_output_side(input, Some(side))?;
    let (width, height) = config.output_size(input)?;
    config.output = format!("{}x{}", (width & !1).max(2), (height & !1).max(2));
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_video_canvas_is_even_and_no_wider_than_asked() {
        let config = crtsim_core::config::Config {
            output: "1920x1080".into(),
            ..Default::default()
        };
        let even = even_canvas(&config, (640, 360), 641).unwrap();
        assert_eq!(even.output, "640x360");
        let odd = crtsim_core::config::Config {
            output: "333x201".into(),
            ..Default::default()
        };
        assert_eq!(even_canvas(&odd, (1, 1), 1000).unwrap().output, "332x200");
    }
}
