//! What a request asks the service to render, read from its query or its JSON body and checked
//! against the service's limits before it is queued, so a request that cannot be rendered is
//! refused at once, saying why.
//!
//! A request's body is the picture, animation or video itself, its options in the query; or a
//! JSON object holding it in base64 as `input` beside the same options, and a look's whole
//! settings as `settings`.

use crate::http::{Refused, Request};
use base64::Engine as _;
use crtsim_core::{color::Space, config::Config, input::Profiles, presets};
use crtsim_media::{AnimationOptions, Audio, Quality, Timing};
use serde_json::Value;

/// What the service takes, set when it starts.
#[derive(Clone, Debug)]
pub struct Limits {
    /// The largest upload, in bytes.
    pub upload: u64,
    /// The most pixels a still may have.
    pub input_pixels: u64,
    /// The most pixels a still's CRT canvas may have.
    pub output_pixels: u64,
    /// The longest video, or stretch of an animation, in seconds.
    pub seconds: f64,
    /// The longest side of an animation's or a video's CRT canvas.
    pub moving_side: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            upload: 64 * 1024 * 1024,
            input_pixels: 4096 * 4096,
            output_pixels: 3840 * 2160,
            seconds: 30.,
            moving_side: 1280,
        }
    }
}

/// What a job writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// A still, or an animated PNG of a moving input.
    Png,
    Gif,
    Webp,
    Mp4,
    Mkv,
    Webm,
}

impl Format {
    const ALL: [(Self, &'static str); 6] = [
        (Self::Png, "png"),
        (Self::Gif, "gif"),
        (Self::Webp, "webp"),
        (Self::Mp4, "mp4"),
        (Self::Mkv, "mkv"),
        (Self::Webm, "webm"),
    ];

    pub fn extension(self) -> &'static str {
        Self::ALL
            .iter()
            .find(|(f, _)| *f == self)
            .map(|(_, e)| *e)
            .expect("every format is listed")
    }

    pub fn content_type(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Gif => "image/gif",
            Self::Webp => "image/webp",
            Self::Mp4 => "video/mp4",
            Self::Mkv => "video/x-matroska",
            Self::Webm => "video/webm",
        }
    }

    /// Whether it is a video, rather than an animation or a still.
    pub fn video(self) -> bool {
        matches!(self, Self::Mp4 | Self::Mkv | Self::Webm)
    }
}

/// What was uploaded.
#[derive(Debug)]
pub enum Input {
    Still(Vec<u8>),
    /// An animated GIF or WebP, or a video, kept as a file of this extension for FFmpeg.
    Moving {
        bytes: Vec<u8>,
        extension: &'static str,
    },
}

/// A job, ready to queue.
pub struct Spec {
    pub input: Input,
    pub config: Config,
    pub format: Format,
    pub space: Space,
    pub profiles: Profiles,
    pub animation: AnimationOptions,
    pub video: crtsim_media::Options,
    /// The longest a video may be, in seconds; an animation renders only `animation`'s stretch.
    pub seconds: f64,
}

fn bad(message: impl Into<String>) -> Refused {
    Refused {
        status: 400,
        message: message.into(),
    }
}

fn unprocessable(message: impl Into<String>) -> Refused {
    Refused {
        status: 422,
        message: message.into(),
    }
}

/// The options a request may give, by name, each with what it is for.
pub const OPTIONS: &[(&str, &str)] = &[
    (
        "preset",
        "an included look by name; GET /presets lists them",
    ),
    ("settings", "a look's whole settings, as a preset's JSON"),
    (
        "size",
        "the CRT canvas: 720p, 1080p, 1440p, 4k, reference, match-input or WxH",
    ),
    (
        "signal",
        "original, auto, native, 240p, 288p, 360p, 480p, 576p or WxH",
    ),
    (
        "format",
        "png, gif, webp, mp4, mkv or webm; by default the input's own kind",
    ),
    (
        "color_space",
        "a still's: srgb, display-p3, adobe-rgb, rec2020 or prophoto-rgb",
    ),
    (
        "color_profile",
        "a still's colour space as an RGB ICC profile, in base64 (JSON only)",
    ),
    (
        "ignore_profile",
        "true to read the input's numbers as sRGB, whatever profile it carries",
    ),
    ("timing", "stable, ntsc60 (videos only) or off"),
    ("fps", "an animation's frames a second"),
    (
        "max_side",
        "an animation's or video's canvas, longest side in pixels",
    ),
    ("start", "where in a moving input to start, in seconds"),
    ("seconds", "how many seconds of a moving input to render"),
    (
        "quality",
        "a video's: draft, balanced, high, archival or lossless (mkv)",
    ),
    ("lossless", "true for a lossless WebP animation"),
];

/// Reads `request` into a job, checking it against `limits`; `ffmpeg` says whether FFmpeg is
/// there for animations and videos.
pub fn read(request: &Request, limits: &Limits, ffmpeg: bool) -> Result<Spec, Refused> {
    let json = request
        .header("content-type")
        .is_some_and(|t| t.trim().starts_with("application/json"));
    let (bytes, options) = if json {
        let Value::Object(mut fields) = serde_json::from_slice(&request.body)
            .map_err(|e| bad(format!("The body is not JSON: {e}")))?
        else {
            return Err(bad("The body must be a JSON object"));
        };
        let input = fields
            .remove("input")
            .and_then(|v| v.as_str().map(String::from))
            .ok_or_else(|| bad("Give the picture as \"input\", in base64"))?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(input.trim())
            .map_err(|e| bad(format!("\"input\" is not base64: {e}")))?;
        (bytes, fields.into_iter().collect::<Vec<_>>())
    } else {
        let options = request
            .query
            .iter()
            .map(|(k, v)| (k.clone(), Value::String(v.clone())))
            .collect();
        (request.body.clone(), options)
    };
    for (name, _) in &options {
        if !OPTIONS.iter().any(|(known, _)| known == name) {
            let known: Vec<_> = OPTIONS.iter().map(|(n, _)| *n).collect();
            return Err(bad(format!(
                "Unknown option {name:?}; the options are {}",
                known.join(", ")
            )));
        }
    }
    let option = |name: &str| options.iter().find(|(n, _)| n == name).map(|(_, v)| v);
    let text = |name: &str| -> Result<Option<String>, Refused> {
        match option(name) {
            None => Ok(None),
            Some(Value::String(s)) => Ok(Some(s.clone())),
            Some(Value::Number(n)) => Ok(Some(n.to_string())),
            Some(Value::Bool(b)) => Ok(Some(b.to_string())),
            Some(_) => Err(bad(format!(
                "{name} must be text, a number or true or false"
            ))),
        }
    };
    let number = |name: &str| -> Result<Option<f64>, Refused> {
        text(name)?
            .map(|t| {
                t.parse::<f64>()
                    .ok()
                    .filter(|v| v.is_finite())
                    .ok_or_else(|| bad(format!("{name} must be a number, not {t:?}")))
            })
            .transpose()
    };
    let flag = |name: &str| -> Result<bool, Refused> {
        match text(name)?.as_deref() {
            None | Some("false") | Some("0") => Ok(false),
            Some("true") | Some("1") | Some("") => Ok(true),
            Some(other) => Err(bad(format!("{name} must be true or false, not {other:?}"))),
        }
    };

    let mut config = match (text("preset")?, option("settings")) {
        (Some(_), Some(_)) => return Err(bad("Give a preset or settings, not both")),
        (Some(name), None) => presets::named(&name).map(|p| p.config).ok_or_else(|| {
            bad(format!(
                "No included preset is called {name:?}; GET /presets"
            ))
        })?,
        (None, Some(settings)) => {
            let json = match settings {
                Value::String(text) => text.clone().into_bytes(),
                other => serde_json::to_vec(other).expect("JSON from a value"),
            };
            Config::from_json_slice(&json).map_err(|e| bad(format!("Invalid settings: {e:#}")))?
        }
        (None, None) => Config::default(),
    };
    if let Some(size) = text("size")? {
        config.output = size;
    }
    if let Some(signal) = text("signal")? {
        config.signal = signal;
    }
    config
        .validate()
        .map_err(|e| bad(format!("Invalid settings: {e:#}")))?;

    let input = sniff(bytes)?;
    let format = match text("format")? {
        Some(name) => Format::ALL
            .iter()
            .find(|(_, e)| e.eq_ignore_ascii_case(&name))
            .map(|(f, _)| *f)
            .ok_or_else(|| bad(format!("Unknown format {name:?}")))?,
        None => match &input {
            Input::Still(_) => Format::Png,
            Input::Moving { extension, .. } => match *extension {
                "gif" => Format::Gif,
                "webp" => Format::Webp,
                _ => Format::Mp4,
            },
        },
    };

    let space = match (text("color_space")?, text("color_profile")?) {
        (Some(_), Some(_)) => return Err(bad("Give color_space or color_profile, not both")),
        (Some(name), None) => Space::NAMED
            .iter()
            .find(|(slug, _)| *slug == name)
            .map(|(_, s)| s.clone())
            .ok_or_else(|| bad(format!("Unknown colour space {name:?}")))?,
        (None, Some(profile)) => {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(profile.trim())
                .map_err(|e| bad(format!("color_profile is not base64: {e}")))?;
            Space::from_icc(bytes).map_err(|e| unprocessable(format!("{e:#}")))?
        }
        (None, None) => Space::Srgb,
    };
    let profiles = if flag("ignore_profile")? {
        Profiles::Ignore
    } else {
        Profiles::Convert
    };

    match &input {
        Input::Still(bytes) => {
            if format != Format::Png {
                return Err(unprocessable(format!(
                    "A still renders to PNG, not {}",
                    format.extension()
                )));
            }
            let (width, height) = image::io::Reader::new(std::io::Cursor::new(bytes))
                .with_guessed_format()
                .map_err(|e| unprocessable(e.to_string()))?
                .into_dimensions()
                .map_err(|e| unprocessable(format!("Cannot read the picture: {e}")))?;
            if u64::from(width) * u64::from(height) > limits.input_pixels {
                return Err(unprocessable(format!(
                    "The picture is {width}x{height}; this service takes at most {} pixels",
                    limits.input_pixels
                )));
            }
            let (w, h) = config
                .output_size((width, height))
                .map_err(|e| bad(format!("Invalid size: {e:#}")))?;
            if u64::from(w) * u64::from(h) > limits.output_pixels {
                return Err(unprocessable(format!(
                    "A {w}x{h} canvas is larger than this service draws, {} pixels",
                    limits.output_pixels
                )));
            }
            config
                .signal_size((width, height))
                .map_err(|e| bad(format!("Invalid signal: {e:#}")))?;
        }
        Input::Moving { .. } => {
            if !ffmpeg {
                return Err(Refused {
                    status: 501,
                    message: "This service renders stills only: it has no FFmpeg".into(),
                });
            }
            if space != Space::Srgb {
                return Err(unprocessable("Animations and videos are sRGB"));
            }
        }
    }

    let timing = match text("timing")?.as_deref() {
        None | Some("stable") => Timing::Stable,
        Some("ntsc60") => Timing::Ntsc60,
        Some("off") => Timing::Disabled,
        Some(other) => return Err(bad(format!("Unknown timing {other:?}"))),
    };
    let max_side = match number("max_side")? {
        Some(side) if (16. ..=f64::from(limits.moving_side)).contains(&side) => side as u32,
        Some(side) => {
            return Err(unprocessable(format!(
                "max_side must be 16 to {}, not {side}",
                limits.moving_side
            )))
        }
        None => 640.min(limits.moving_side),
    };
    let start = number("start")?.unwrap_or(0.);
    if start < 0. {
        return Err(bad("start must not be negative"));
    }
    let seconds = number("seconds")?.unwrap_or(limits.seconds.min(10.));
    if !(seconds > 0. && seconds <= limits.seconds) {
        return Err(unprocessable(format!(
            "seconds must be above 0 and at most {}",
            limits.seconds
        )));
    }
    let defaults = AnimationOptions::default();
    let animation = AnimationOptions {
        max_side,
        fps: match number("fps")? {
            Some(fps) if (1. ..=60.).contains(&fps) => fps as u32,
            Some(fps) => return Err(unprocessable(format!("fps must be 1 to 60, not {fps}"))),
            None => defaults.fps,
        },
        timing,
        start,
        max_seconds: Some(seconds),
        lossless: flag("lossless")?,
        ..defaults
    };
    let video = crtsim_media::Options {
        timing,
        audio: Audio::Auto,
        quality: match text("quality")?.as_deref() {
            None | Some("balanced") => Quality::Balanced,
            Some("draft") => Quality::Draft,
            Some("high") => Quality::High,
            Some("archival") => Quality::Archival,
            Some("lossless") => Quality::Lossless,
            Some(other) => return Err(bad(format!("Unknown quality {other:?}"))),
        },
        ..Default::default()
    };
    if format.video() {
        video
            .validate()
            .map_err(|e| unprocessable(format!("{e:#}")))?;
        if video.quality == Quality::Lossless && format != Format::Mkv {
            return Err(unprocessable("Lossless video is MKV"));
        }
    }
    Ok(Spec {
        input,
        config,
        format,
        space,
        profiles,
        animation,
        video,
        seconds: limits.seconds,
    })
}

/// What `bytes` are, by their first bytes: a still, an animation or a video.
fn sniff(bytes: Vec<u8>) -> Result<Input, Refused> {
    let starts = |magic: &[u8]| bytes.starts_with(magic);
    let riff = |kind: &[u8]| bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == kind;
    let animated = |extension: &'static str, bytes: Vec<u8>| {
        let contents = crtsim_media::Contents(bytes.clone().into());
        let name = std::path::PathBuf::from(format!("input.{extension}"));
        if crtsim_media::detect_bytes(&name, &contents).is_some() {
            Input::Moving { bytes, extension }
        } else {
            Input::Still(bytes)
        }
    };
    Ok(if starts(b"GIF8") {
        animated("gif", bytes)
    } else if riff(b"WEBP") {
        animated("webp", bytes)
    } else if starts(b"\x89PNG\r\n\x1a\n") || starts(&[0xFF, 0xD8, 0xFF]) || starts(b"BM") {
        Input::Still(bytes)
    } else if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" {
        Input::Moving {
            bytes,
            extension: "mp4",
        }
    } else if starts(&[0x1A, 0x45, 0xDF, 0xA3]) {
        Input::Moving {
            bytes,
            extension: "mkv",
        }
    } else if riff(b"AVI ") {
        Input::Moving {
            bytes,
            extension: "avi",
        }
    } else if bytes.is_empty() {
        return Err(bad("The request has no picture: send it as the body"));
    } else {
        return Err(Refused {
            status: 415,
            message: "Send a PNG, JPEG, WebP, BMP or GIF picture, or an MP4, MOV, MKV, WebM or \
                      AVI video"
                .into(),
        });
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = std::io::Cursor::new(vec![]);
        image::RgbaImage::new(width, height)
            .write_to(&mut bytes, image::ImageOutputFormat::Png)
            .unwrap();
        bytes.into_inner()
    }

    fn request(query: &[(&str, &str)], content_type: Option<&str>, body: Vec<u8>) -> Request {
        let mut raw = format!(
            "POST /render?{} HTTP/1.1\r\nContent-Length: {}\r\n",
            form_urlencoded::Serializer::new(String::new())
                .extend_pairs(query)
                .finish(),
            body.len()
        );
        if let Some(t) = content_type {
            raw.push_str(&format!("Content-Type: {t}\r\n"));
        }
        raw.push_str("\r\n");
        let mut raw = raw.into_bytes();
        raw.extend(body);
        crate::http::tests_support::parse(&raw)
    }

    #[test]
    fn a_still_takes_its_options_from_the_query_or_from_json() {
        let limits = Limits::default();
        let spec = read(
            &request(
                &[("preset", "NTSC 240p"), ("size", "720p")],
                None,
                png(64, 48),
            ),
            &limits,
            false,
        )
        .ok()
        .unwrap();
        assert!(matches!(spec.input, Input::Still(_)));
        assert_eq!(
            (spec.format, spec.config.output.as_str()),
            (Format::Png, "720p")
        );
        assert_eq!(spec.config.signal, "240p");

        let body = serde_json::json!({
            "input": base64::engine::general_purpose::STANDARD.encode(png(64, 48)),
            "settings": { "bloom": 0.5, "output": "320x240" },
            "color_space": "display-p3",
            "ignore_profile": true,
        });
        let spec = read(
            &request(
                &[],
                Some("application/json"),
                serde_json::to_vec(&body).unwrap(),
            ),
            &limits,
            false,
        )
        .ok()
        .unwrap();
        assert_eq!((spec.config.bloom, spec.space), (0.5, Space::DisplayP3));
        assert_eq!(spec.profiles, Profiles::Ignore);
    }

    #[test]
    fn a_request_past_the_limits_or_unclear_is_refused_saying_why() {
        let limits = Limits {
            input_pixels: 100 * 100,
            output_pixels: 640 * 480,
            ..Limits::default()
        };
        let status = |query: &[(&str, &str)], body: Vec<u8>| {
            read(&request(query, None, body), &limits, false)
                .err()
                .map(|r| (r.status, r.message))
        };
        let (code, message) = status(&[], png(200, 200)).unwrap();
        assert_eq!(code, 422, "{message}");
        assert_eq!(status(&[("size", "1080p")], png(64, 48)).unwrap().0, 422);
        assert_eq!(status(&[("bloom", "1")], png(64, 48)).unwrap().0, 400);
        assert_eq!(
            status(&[("preset", "No such look")], png(64, 48))
                .unwrap()
                .0,
            400
        );
        assert_eq!(status(&[("format", "gif")], png(64, 48)).unwrap().0, 422);
        assert_eq!(status(&[], b"not a picture".to_vec()).unwrap().0, 415);
        assert_eq!(status(&[], vec![]).unwrap().0, 400);
        // A video, with no FFmpeg to read it.
        let mut mp4 = vec![0, 0, 0, 24];
        mp4.extend(b"ftypisom");
        mp4.extend([0; 12]);
        assert_eq!(status(&[], mp4).unwrap().0, 501);
        assert_eq!(
            status(&[("color_space", "cmyk")], png(64, 48)).unwrap().0,
            400
        );
    }
}
