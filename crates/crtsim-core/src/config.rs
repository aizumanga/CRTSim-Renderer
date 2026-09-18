use anyhow::{bail, ensure, Result};
use image::{imageops, Rgba, RgbaImage};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum Phase {
    Stable,
    A,
    B,
    Alternating,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum Fit {
    Reference,
    Contain,
    Cover,
    Stretch,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum Filter {
    Nearest,
    Lanczos,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ColorMode {
    #[default]
    Reference,
    LinearLight,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub source: crate::workflow::SourceEdit,
    pub screen_only: bool,
    pub lut: Option<std::sync::Arc<crate::workflow::Lut>>,
    pub version: u32,
    pub signal: String,
    pub output: String,
    pub fit: Fit,
    pub filter: Filter,
    pub pixel_aspect: f32,
    pub phase: Phase,
    /// Number of additional ticks before the saved tick; total = warmup + 1.
    pub warmup: u32,
    pub sharpness: f32,
    pub bleed: f32,
    pub artifacts: f32,
    pub persistence: [f32; 3],
    pub overscan: f32,
    pub barrel: f32,
    pub saturation: f32,
    pub mask_brightness: f32,
    pub mask_opacity: f32,
    /// Physical mask repeats across the screen (not tied to signal dimensions).
    pub mask_repeats: [f32; 2],
    pub dimming: f32,
    pub reflection: f32,
    pub diffuse: f32,
    pub specular: f32,
    pub specular_power: f32,
    pub rim: f32,
    pub light_position: [f32; 3],
    pub frame_color: [f32; 3],
    pub fov: f32,
    pub bloom: f32,
    pub bloom_power: f32,
    pub bloom_spread: f32,
    pub color_mode: ColorMode,
    pub mask_antialias: bool,
    /// Optional YIQ hue rotation, in degrees. Not the game's unpublished NES palette LUT.
    pub hue: f32,
    pub chroma: f32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            source: Default::default(),
            screen_only: false,
            lut: None,
            version: 1,
            signal: "original".into(),
            output: "reference".into(),
            fit: Fit::Reference,
            filter: Filter::Nearest,
            pixel_aspect: 8. / 7.,
            phase: Phase::Stable,
            warmup: 16,
            sharpness: 0.8,
            bleed: 0.5,
            artifacts: 0.5,
            persistence: [0.7, 0.525, 0.42],
            overscan: 1.,
            barrel: -0.115,
            saturation: 1.35,
            mask_brightness: 0.45,
            mask_opacity: 1.,
            mask_repeats: [128., 224.],
            dimming: 0.5,
            reflection: 0.3,
            diffuse: 0.5,
            specular: 0.35,
            specular_power: 50.,
            rim: 1.,
            light_position: [-10., -5., 10.],
            frame_color: [0.06; 3],
            fov: 15.,
            bloom: 0.25,
            bloom_power: 2.,
            bloom_spread: 0.025,
            color_mode: ColorMode::Reference,
            mask_antialias: false,
            hue: 0.,
            chroma: 1.,
        }
    }
}

pub fn dimensions(text: &str) -> Result<(u32, u32)> {
    let (w, h) = text
        .split_once('x')
        .ok_or_else(|| anyhow::anyhow!("expected WIDTHxHEIGHT, got {text}"))?;
    let pair = (w.parse()?, h.parse()?);
    validate_size(pair)?;
    Ok(pair)
}
pub fn validate_size((w, h): (u32, u32)) -> Result<()> {
    ensure!(
        w > 0 && h > 0 && w <= 16384 && h <= 16384 && u64::from(w) * u64::from(h) <= 64_000_000,
        "dimensions must be positive, <=16384 per side, and <=64 megapixels"
    );
    Ok(())
}
impl Config {
    /// One version-aware entry point for presets. Future migrations belong here instead of
    /// being duplicated across the CLI, desktop JSON loader and embedded metadata readers.
    pub fn from_json_slice(bytes: &[u8]) -> Result<Self> {
        let value: serde_json::Value = serde_json::from_slice(bytes)?;
        let version = value.get("version").and_then(serde_json::Value::as_u64);
        if let Some(version) = version {
            ensure!(version == 1, "unsupported config version {version}");
        }
        let config: Self = serde_json::from_value(value)?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        self.source.validate()?;
        if let Some(lut) = &self.lut {
            lut.validate()?;
        }
        ensure!(self.version == 1, "unsupported config version");
        ensure!(self.warmup <= 240, "warmup must be <=240 ticks");
        let ranges = [
            (self.pixel_aspect, 0.1, 10.),
            (self.sharpness, 0., 3.),
            (self.bleed, 0., 2.),
            (self.artifacts, 0., 2.),
            (self.overscan, 0.1, 3.),
            (self.barrel, -2., 2.),
            (self.saturation, 0., 3.),
            (self.mask_brightness, 0., 2.),
            (self.mask_opacity, 0., 1.),
            (self.dimming, 0., 1.),
            (self.reflection, 0., 2.),
            (self.diffuse, 0., 2.),
            (self.specular, 0., 2.),
            (self.rim, 0., 2.),
            (self.specular_power, 1., 200.),
            (self.fov, 5., 90.),
            (self.bloom, 0., 2.),
            (self.bloom_power, 0.1, 8.),
            (self.bloom_spread, 0., 0.2),
            (self.hue, -180., 180.),
            (self.chroma, 0., 2.),
        ];
        for (v, min, max) in ranges {
            ensure!(
                v.is_finite() && v >= min && v <= max,
                "parameter {v} outside {min}..{max}"
            );
        }
        for v in self.persistence {
            ensure!(
                v.is_finite() && (0.0..1.0).contains(&v),
                "persistence must be >=0 and <1"
            );
        }
        for v in self.frame_color {
            ensure!(
                v.is_finite() && (0.0..=1.0).contains(&v),
                "invalid frame color"
            );
        }
        for v in self.light_position {
            ensure!(v.is_finite() && v.abs() <= 1000., "invalid light position");
        }
        for v in self.mask_repeats {
            ensure!(
                v.is_finite() && v > 0. && v <= 16384.,
                "invalid mask density"
            );
        }
        Ok(())
    }
    pub fn signal_size(&self, input: (u32, u32)) -> Result<(u32, u32)> {
        validate_size(input)?;
        let input = self.source.size(input);
        let height = match self.signal.as_str() {
            "original" => return Ok((256, 224)),
            "native" => return Ok(input),
            "auto" => input.1.min(480),
            "240p" => 240,
            "360p" => 360,
            "480p" => 480,
            v if v.contains('x') => return dimensions(v),
            v => bail!("unknown signal preset: {v}"),
        };
        let w =
            ((f64::from(input.0) * f64::from(height) / f64::from(input.1)).round() as u32).max(1);
        validate_size((w, height))?;
        Ok((w, height))
    }
    pub fn output_size(&self, input: (u32, u32)) -> Result<(u32, u32)> {
        let d = match self.output.as_str() {
            "reference" => (1600, 900),
            "720p" => (1280, 720),
            "1080p" => (1920, 1080),
            "1440p" => (2560, 1440),
            "4k" => (3840, 2160),
            "match-input" => input,
            s => dimensions(s)?,
        };
        validate_size(d)?;
        Ok(d)
    }
    pub fn uv_scale(&self, signal: (u32, u32)) -> [f32; 2] {
        let ratio = signal.0 as f32 / signal.1 as f32 * self.pixel_aspect / (4. / 3.);
        match self.fit {
            Fit::Reference => [1., ratio],
            Fit::Stretch => [1., 1.],
            Fit::Contain if ratio >= 1. => [1., ratio],
            Fit::Contain => [1. / ratio, 1.],
            Fit::Cover if ratio >= 1. => [1. / ratio, 1.],
            Fit::Cover => [1., ratio],
        }
    }
}

/// Produces the logical signal; alpha is composited onto the selected background before filtering.
/// Explicit custom sizes/original may change aspect: the caller must opt into them.
pub fn prepare(input: &RgbaImage, config: &Config) -> Result<RgbaImage> {
    config.validate()?;
    let (w, h) = config.signal_size(input.dimensions())?;
    let transformed = config.source.crop != [0.; 4]
        || config.source.rotation != 0.
        || config.source.zoom != 1.
        || config.source.position != [0.; 2]
        || config.source.checkerboard;
    let mut resized = if transformed {
        config
            .source
            .prepare(input, (w, h), config.filter == Filter::Nearest)
    } else {
        let mut opaque = input.clone();
        for p in opaque.pixels_mut() {
            let a = u16::from(p[3]);
            for i in 0..3 {
                p[i] = ((u16::from(p[i]) * a
                    + u16::from(config.source.background[i]) * (255 - a)
                    + 127)
                    / 255) as u8;
            }
            p[3] = 255;
        }
        let filter = match config.filter {
            Filter::Nearest => imageops::FilterType::Nearest,
            Filter::Lanczos => imageops::FilterType::Lanczos3,
        };
        imageops::resize(&opaque, w, h, filter)
    };
    if let Some(lut) = &config.lut {
        lut.apply(&mut resized);
    }
    if config.hue != 0. || config.chroma != 1. {
        let (sin, cos) = config.hue.to_radians().sin_cos();
        for p in resized.pixels_mut() {
            let [r, g, b] = [p[0] as f32 / 255., p[1] as f32 / 255., p[2] as f32 / 255.];
            let y = 0.299 * r + 0.587 * g + 0.114 * b;
            let i = 0.596 * r - 0.274 * g - 0.322 * b;
            let q = 0.211 * r - 0.523 * g + 0.312 * b;
            let ii = (i * cos - q * sin) * config.chroma;
            let qq = (i * sin + q * cos) * config.chroma;
            for (channel, value) in [
                y + 0.956 * ii + 0.621 * qq,
                y - 0.272 * ii - 0.647 * qq,
                y - 1.106 * ii + 1.703 * qq,
            ]
            .iter()
            .enumerate()
            {
                p[channel] = (value.clamp(0., 1.) * 255.).round() as u8;
            }
        }
    }
    Ok(resized)
}

/// Original test card, not a screenshot from a commercial game.
pub fn test_card() -> RgbaImage {
    RgbaImage::from_fn(256, 224, |x, y| {
        let bars = [
            [255, 255, 255],
            [255, 255, 0],
            [0, 255, 255],
            [0, 255, 0],
            [255, 0, 255],
            [255, 0, 0],
            [0, 0, 255],
            [0, 0, 0],
        ];
        let c = if y < 96 {
            bars[(x / 32) as usize]
        } else if y < 144 {
            [x as u8; 3]
        } else if y < 184 {
            [if (x / 4 + y / 4) % 2 == 0 { 240 } else { 16 }; 3]
        } else {
            [if x % 16 < 2 { 255 } else { 0 }; 3]
        };
        Rgba([c[0], c[1], c[2], 255])
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn old_presets_keep_reference_behavior_and_neutral_grade_is_exact() {
        let c = Config::from_json_slice(b"{\"version\":1,\"signal\":\"native\"}").unwrap();
        assert_eq!(c.color_mode, ColorMode::Reference);
        assert!(!c.mask_antialias);
        let src = test_card();
        assert_eq!(prepare(&src, &c).unwrap(), src);
        let gray = prepare(
            &src,
            &Config {
                chroma: 0.,
                ..c.clone()
            },
        )
        .unwrap();
        assert!(gray.pixels().all(|p| p[0] == p[1] && p[1] == p[2]));
        assert_ne!(prepare(&src, &Config { hue: 30., ..c }).unwrap(), src);
    }

    #[test]
    fn versioned_loader_accepts_legacy_defaults_and_rejects_future_configs() {
        assert_eq!(
            Config::from_json_slice(b"{\"signal\":\"native\"}")
                .unwrap()
                .version,
            1
        );
        let error = Config::from_json_slice(b"{\"version\":2}").unwrap_err();
        assert!(error.to_string().contains("unsupported config version 2"));
    }
    #[test]
    fn presets_and_limits() {
        let mut c = Config::default();
        assert_eq!(c.signal_size((1920, 1080)).unwrap(), (256, 224));
        c.signal = "auto".into();
        assert_eq!(c.signal_size((1920, 1080)).unwrap(), (853, 480));
        assert_eq!(c.signal_size((320, 200)).unwrap(), (320, 200));
        assert!(dimensions("0x10").is_err());
        assert!(dimensions("16384x16384").is_err());
        c.persistence[0] = 1.;
        assert!(c.validate().is_err());
    }
    #[test]
    fn fit_and_reference_are_distinct() {
        let mut c = Config::default();
        assert!((c.uv_scale((256, 224))[1] - 0.9795918).abs() < 0.0001);
        c.pixel_aspect = 1.;
        c.fit = Fit::Contain;
        assert!((c.uv_scale((1920, 1080))[1] - 4. / 3.).abs() < 0.00001);
    }
    #[test]
    fn alpha_composites_onto_black() {
        let c = Config {
            signal: "native".into(),
            ..Config::default()
        };
        let src = RgbaImage::from_pixel(1, 1, Rgba([255, 0, 0, 128]));
        assert_eq!(
            prepare(&src, &c).unwrap().get_pixel(0, 0).0,
            [128, 0, 0, 255]
        );
    }
}
