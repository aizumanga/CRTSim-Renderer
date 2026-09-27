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

/// How many times the shadow mask repeats across the screen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MaskRepeats {
    /// As the original: a mask column for every two signal columns and a row for every signal
    /// row, so a finer signal gets a finer mask. Saved as `"signal"`.
    Signal,
    /// These columns and rows across the screen, whatever the signal. Saved as
    /// `[columns, rows]`, which is how every preset held it before it could follow the signal.
    Fixed([f32; 2]),
}

impl MaskRepeats {
    /// The columns and rows across the screen for a signal of `size`.
    pub fn resolve(self, (width, height): (u32, u32)) -> [f32; 2] {
        match self {
            Self::Signal => [width as f32 / 2., height as f32],
            Self::Fixed(repeats) => repeats,
        }
    }
}

impl Serialize for MaskRepeats {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        match self {
            Self::Signal => serializer.serialize_str("signal"),
            Self::Fixed(repeats) => repeats.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for MaskRepeats {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged, expecting = "\"signal\" or [columns, rows]")]
        enum Saved {
            Fixed([f32; 2]),
            Named(String),
        }
        match Saved::deserialize(deserializer)? {
            Saved::Fixed(repeats) => Ok(Self::Fixed(repeats)),
            Saved::Named(name) if name == "signal" => Ok(Self::Signal),
            Saved::Named(name) => Err(serde::de::Error::custom(format!(
                "unknown mask repeats {name:?}, expected \"signal\" or [columns, rows]"
            ))),
        }
    }
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
    /// How much of the LUT's colour applies, as Super Win the Game's NTSC Palette does: 1 the
    /// LUT alone, 0 none of it.
    pub lut_strength: f32,
    /// The NES palette made from its composite signal, with the game's Tint controls, in place
    /// of a LUT. See `palette`.
    pub palette: Option<crate::palette::NesPalette>,
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
    /// How far the two composite artifact patterns blend into each other, as Super Win the
    /// Game's NTSC Blending does. With alternating phase, ticks show them mixed this far and
    /// then the other way round; 0 switches cleanly between them, as the public source does,
    /// and 0.5 shows their average on every tick. Stable phase always shows the average.
    pub ntsc_blending: f32,
    pub persistence: [f32; 3],
    pub overscan: f32,
    pub barrel: f32,
    pub saturation: f32,
    pub mask_brightness: f32,
    pub mask_opacity: f32,
    pub mask_repeats: MaskRepeats,
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
    /// Samples the mask from mipmaps, each the average of the level above, and between them,
    /// as the original did: the mask stays smooth where it is drawn smaller than it is. Off
    /// samples only the full-size mask, which shimmers into moiré when shrunk.
    pub mask_antialias: bool,
    /// Interlaced scanning: each tick refreshes only every other signal row, alternating
    /// between the two fields, and the rows it skips only decay by `persistence`. Meant for
    /// 480- or 576-row signals, as an interlaced set drew them. Off draws every row every tick.
    pub interlace: bool,
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
            lut_strength: 1.,
            palette: None,
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
            ntsc_blending: 0.,
            persistence: [0.7, 0.525, 0.42],
            overscan: 1.,
            barrel: -0.115,
            saturation: 1.35,
            mask_brightness: 0.45,
            mask_opacity: 1.,
            mask_repeats: MaskRepeats::Signal,
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
            mask_antialias: true,
            interlace: false,
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
    /// The starting point for ordinary images rather than 256x224 game frames: square pixels,
    /// smooth resizing, contain fitting, neutral saturation, and the reference's mask density
    /// whatever the signal, since an image's rows are not a picture tube's. `default` stays the
    /// public reference's settings.
    pub fn general() -> Self {
        Self {
            signal: "auto".into(),
            output: "1080p".into(),
            fit: Fit::Contain,
            filter: Filter::Lanczos,
            pixel_aspect: 1.,
            saturation: 1.,
            mask_repeats: MaskRepeats::Fixed([128., 224.]),
            ..Self::default()
        }
    }

    /// The colour table prepare applies: the NES palette's when one is set, else the LUT.
    pub fn lut_in_use(&self) -> Option<std::sync::Arc<crate::workflow::Lut>> {
        match &self.palette {
            Some(palette) => Some(palette.lut()),
            None => self.lut.clone(),
        }
    }

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
        ensure!(
            self.lut.is_none() || self.palette.is_none(),
            "Use either a LUT or the NES palette, not both"
        );
        ensure!(self.version == 1, "unsupported config version");
        ensure!(self.warmup <= 240, "warmup must be <=240 ticks");
        crate::settings::validate(self)
    }
    pub fn signal_size(&self, input: (u32, u32)) -> Result<(u32, u32)> {
        validate_size(input)?;
        let input = self.source.size(input);
        let height = match self.signal.as_str() {
            "original" => return Ok((256, 224)),
            "native" => return Ok(input),
            "auto" => input.1.min(480),
            "240p" => 240,
            "288p" => 288,
            "360p" => 360,
            "480p" => 480,
            "576p" => 576,
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
    /// Checks the settings, and that they size a render of an input `input` pixels large, so
    /// settings that cannot render it are refused when they are loaded or saved, not later.
    pub fn validate_for(&self, input: (u32, u32)) -> Result<()> {
        self.validate()?;
        self.signal_size(input)?;
        self.output_size(input)?;
        Ok(())
    }
    /// The same settings with the output canvas scaled down, keeping its aspect, so that its
    /// longer side is at most `max_side`. The signal, and so the look, stays as it is.
    pub fn with_max_output_side(&self, input: (u32, u32), max_side: Option<u32>) -> Result<Self> {
        self.validate_for(input)?;
        let (w, h) = self.output_size(input)?;
        let scale = max_side.map_or(1., |m| (m as f64 / w.max(h) as f64).min(1.));
        let mut scaled = self.clone();
        scaled.output = format!(
            "{}x{}",
            (w as f64 * scale).round().max(1.) as u32,
            (h as f64 * scale).round().max(1.) as u32
        );
        Ok(scaled)
    }
    /// Whether prepare takes the source-edit route -- resampled through the crop, rotation,
    /// zoom and pan -- rather than the plain resize.
    pub fn edits_source(&self) -> bool {
        let s = &self.source;
        s.crop != [0.; 4]
            || s.rotation != 0.
            || s.zoom != 1.
            || s.position != [0.; 2]
            || s.checkerboard
    }
    /// Whether the YIQ grade does anything; neutral values leave the signal untouched.
    pub fn grades(&self) -> bool {
        self.hue != 0. || self.chroma != 1.
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
///
/// Renders run this on the GPU (see `gpu_prepare`); this is the reference that port is held to,
/// and the fallback for an input too large to prepare on the device.
pub fn prepare(input: &RgbaImage, config: &Config) -> Result<RgbaImage> {
    config.validate()?;
    let (w, h) = config.signal_size(input.dimensions())?;
    let mut resized = if config.edits_source() {
        config
            .source
            .prepare(input, (w, h), config.filter == Filter::Nearest)
    } else {
        let mut opaque = input.clone();
        flatten_alpha(&mut opaque, config.source.background);
        let filter = match config.filter {
            Filter::Nearest => imageops::FilterType::Nearest,
            Filter::Lanczos => imageops::FilterType::Lanczos3,
        };
        imageops::resize(&opaque, w, h, filter)
    };
    if let Some(lut) = config.lut_in_use() {
        lut.apply_with_strength(&mut resized, config.lut_strength);
    }
    if config.grades() {
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

/// Composites every pixel over an opaque `background` and makes it opaque, rounding to the
/// nearest 8-bit step.
pub fn flatten_alpha(image: &mut RgbaImage, background: [u8; 3]) {
    for p in image.pixels_mut() {
        let a = u16::from(p[3]);
        for i in 0..3 {
            p[i] = ((u16::from(p[i]) * a + u16::from(background[i]) * (255 - a) + 127) / 255) as u8;
        }
        p[3] = 255;
    }
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
        // A setting a preset leaves out takes the original's value; one it holds is kept.
        assert!(c.mask_antialias);
        let unfiltered = br#"{"version":1,"mask_antialias":false}"#;
        assert!(!Config::from_json_slice(unfiltered).unwrap().mask_antialias);
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
    fn a_preset_holds_the_nes_palette_as_its_three_controls() {
        let json = br#"{"palette":{"tint":5.0,"tint_i":2.0,"tint_q":0.5}}"#;
        let c = Config::from_json_slice(json).unwrap();
        let palette = c.palette.unwrap();
        assert_eq!(
            (palette.tint, palette.tint_i, palette.tint_q),
            (5., 2., 0.5)
        );
        assert!(c.lut_in_use().unwrap().name.starts_with("NES palette"));
        // Controls it leaves out take the game's defaults.
        let c = Config::from_json_slice(br#"{"palette":{}}"#).unwrap();
        assert_eq!(c.palette, Some(crate::palette::NesPalette::default()));
        let both = Config {
            lut: Some(std::sync::Arc::new(crate::nes_luts::load(0).unwrap())),
            ..c
        };
        assert!(both.validate().is_err());
        assert!(Config::from_json_slice(br#"{"palette":{"tint_i":11}}"#).is_err());
    }

    #[test]
    fn mask_repeats_follow_the_signal_or_stay_as_saved() {
        // The original's density: 128 columns and 224 rows for its 256×224 signal.
        assert_eq!(MaskRepeats::Signal.resolve((256, 224)), [128., 224.]);
        assert_eq!(MaskRepeats::Signal.resolve((427, 240)), [213.5, 240.]);
        assert_eq!(
            MaskRepeats::Fixed([90., 60.]).resolve((427, 240)),
            [90., 60.]
        );
        let load = |json: &str| Config::from_json_slice(json.as_bytes()).map(|c| c.mask_repeats);
        // Every preset saved before the mask could follow the signal holds columns and rows.
        assert_eq!(
            load(r#"{"mask_repeats":[128,224]}"#).unwrap(),
            MaskRepeats::Fixed([128., 224.])
        );
        assert_eq!(
            load(r#"{"mask_repeats":"signal"}"#).unwrap(),
            MaskRepeats::Signal
        );
        assert_eq!(load("{}").unwrap(), MaskRepeats::Signal);
        let wrong = load(r#"{"mask_repeats":"dense"}"#).unwrap_err().to_string();
        assert!(
            wrong.contains(r#"expected "signal" or [columns, rows]"#),
            "{wrong}"
        );
        assert!(load(r#"{"mask_repeats":[0,224]}"#).is_err());
        for c in [Config::default(), Config::general()] {
            let saved = serde_json::to_vec(&c).unwrap();
            assert_eq!(Config::from_json_slice(&saved).unwrap(), c);
        }
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
        c.signal = "576p".into();
        assert_eq!(c.signal_size((1920, 1080)).unwrap(), (1024, 576));
        c.signal = "288p".into();
        assert_eq!(c.signal_size((640, 480)).unwrap(), (384, 288));
        assert!(dimensions("0x10").is_err());
        assert!(dimensions("16384x16384").is_err());
        c.persistence[0] = 1.;
        assert!(c.validate().is_err());
    }
    #[test]
    fn a_smaller_canvas_keeps_its_aspect_and_the_signal() {
        let mut c = Config::general();
        c.output = "4k".into();
        let p = c.with_max_output_side((1216, 832), Some(1280)).unwrap();
        assert_eq!(p.output_size((1216, 832)).unwrap(), (1280, 720));
        assert_eq!(
            p.signal_size((1216, 832)).unwrap(),
            c.signal_size((1216, 832)).unwrap()
        );
        assert_eq!(c.output, "4k");
        c.output = "600x1200".into();
        assert_eq!(
            c.with_max_output_side((1, 1), Some(800)).unwrap().output,
            "400x800"
        );
        c.output = "0x100".into();
        assert!(c.with_max_output_side((1, 1), Some(800)).is_err());
        assert!(c.validate_for((1, 1)).is_err());
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
