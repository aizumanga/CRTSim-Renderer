//! Decoded frames of a video, one at a time: single previews and continuous playback.
use crate::{check_cancel, command, process::Process, render_config, Options, Rate, Video};
use anyhow::{ensure, Context, Result};
use crtsim_core::{config::Config, Renderer, Sequence};
use image::RgbaImage;
use std::{
    io::Read,
    process::Command,
    sync::{atomic::AtomicBool, Arc},
};

pub(crate) fn decode_command(
    video: &Video,
    fps: Option<&str>,
    time: f64,
    frame: Option<u64>,
) -> Command {
    let mut cmd = command("ffmpeg");
    // Seeking is input-relative and preview-only; full exports always start at zero.
    if time > 0. {
        cmd.args(["-ss", &time.to_string()]);
    }
    cmd.arg("-i").arg(&video.path).args([
        "-map",
        &format!("0:{}", video.stream),
        "-an",
        "-sn",
        "-dn",
    ]);
    let mut filters = vec!["setpts=PTS-STARTPTS".to_string()];
    if let Some(frame) = frame {
        // Select by decoded frame ordinal, including VFR sources. Decode from the start
        // to avoid timestamp rounding and keyframe seeks skipping or repeating frames.
        filters.push(format!("select=eq(n\\,{frame})"));
    }
    if let Some(fps) = fps {
        filters.push(format!("fps={fps}:start_time=0:round=near"));
    }
    if video.hdr {
        filters.push(
            "zscale=t=linear:npl=100,format=gbrpf32le,zscale=p=bt709,\
             tonemap=tonemap=mobius:desat=0,zscale=t=bt709:m=bt709:r=full"
                .into(),
        );
    }
    filters.push(format!(
        "scale={}:{}:flags=lanczos,setsar=1",
        video.size.0, video.size.1
    ));
    cmd.args(["-vf", &filters.join(",")]);
    if fps.is_none() {
        cmd.args(["-frames:v", "1"]);
    }
    cmd.args(["-pix_fmt", "rgba", "-f", "rawvideo", "pipe:1"]);
    cmd
}

/// The frame shown at `time` seconds.
pub fn preview(video: &Video, time: f64, cancel: &Arc<AtomicBool>) -> Result<RgbaImage> {
    ensure!(
        time.is_finite() && time >= 0. && time < video.duration,
        "Preview time is outside the video"
    );
    let mut decode = decode_command(video, None, time, None);
    decode_one(
        video,
        &mut decode,
        cancel,
        "FFmpeg did not return a complete preview frame",
    )
}

/// The frame numbered `frame`, counting from 0 in decoding order.
pub fn preview_frame(video: &Video, frame: u64, cancel: &Arc<AtomicBool>) -> Result<RgbaImage> {
    let mut decode = decode_command(video, None, 0., Some(frame));
    decode_one(
        video,
        &mut decode,
        cancel,
        "FFmpeg did not return the requested frame",
    )
}

/// Runs a decode that stops after one frame and returns it. `missing` is the error when it
/// ends before a whole frame.
fn decode_one(
    video: &Video,
    decode: &mut Command,
    cancel: &Arc<AtomicBool>,
    missing: &str,
) -> Result<RgbaImage> {
    let mut child = Process::spawn(decode, cancel)?;
    drop(child.stdin());
    let mut bytes = vec![0; video.size.0 as usize * video.size.1 as usize * 4];
    let read = child.stdout().read_exact(&mut bytes);
    child.wait()?;
    read.context(missing.to_owned())?;
    RgbaImage::from_raw(video.size.0, video.size.1, bytes).context("Invalid preview pixels")
}

/// Stream a CFR preview, retaining a short history before the requested media time.
/// The callback provides backpressure; cancellation also interrupts decoder reads.
pub fn playback(
    video: &Video,
    start: f64,
    config: &Config,
    options: &Options,
    renderer: &Renderer,
    cancel: &Arc<AtomicBool>,
    mut frame_ready: impl FnMut(f64, RgbaImage, RgbaImage) -> Result<()>,
) -> Result<()> {
    let rate = Rate::of(video, options);
    let preroll = (start - 0.2).max(0.);
    let mut decoder = Process::spawn(
        &mut decode_command(video, Some(&rate.text), preroll, None),
        cancel,
    )?;
    drop(decoder.stdin());
    let mut output = decoder.stdout();
    let mut input = RgbaImage::new(video.size.0, video.size.1);
    let c = render_config(config, options, rate.fps);
    let mut sequence = Sequence::default();
    let mut index = 0u64;
    loop {
        check_cancel(cancel)?;
        let bytes = input.as_mut();
        if output.read(&mut bytes[..1])? == 0 {
            break;
        }
        output
            .read_exact(&mut bytes[1..])
            .context("Truncated playback frame")?;
        let rendered = renderer.render_frame(&input, &c, &mut sequence, Some(cancel), |_| {})?;
        let time = preroll + index as f64 / rate.fps;
        index += 1;
        if time + 0.00001 >= start {
            frame_ready(time, input.clone(), rendered)?;
        }
    }
    decoder.wait()
}
