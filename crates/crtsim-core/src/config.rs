use anyhow::{bail, ensure, Result};
use image::{imageops, Rgba, RgbaImage};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

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

/// Super Win the Game's FXAA, run over the glass and bezel before the bloom is added: off, or
/// its Low, Medium, High or Ultra level, which find edges more readily and follow them further.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Fxaa {
    #[default]
    Off,
    Low,
    Medium,
    High,
    Ultra,
}

/// Super Win the Game's tate: the finished picture turned in quarter turns, for a screen on its
/// side. An odd number of quarters draws the tube across the output's height.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Tate {
    #[default]
    Off,
    Quarter,
    Half,
    ThreeQuarters,
}

impl Tate {
    /// Quarter turns.
    pub fn quarters(self) -> u32 {
        self as u32
    }
    /// The size a picture of `size` is turned into.
    pub fn turned(self, (width, height): (u32, u32)) -> (u32, u32) {
        if self.quarters() % 2 == 1 {
            (height, width)
        } else {
            (width, height)
        }
    }
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

/// The signal a picture is prepared at, as a look names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Signal {
    /// 256x224, the original's.
    Original,
    /// The source's own size.
    Native,
    /// The source's rows, at most 480, keeping its shape.
    Auto,
    /// These rows, keeping the source's shape.
    Rows(u32),
    /// This size, whatever the source's shape.
    Size(u32, u32),
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ColorMode {
    #[default]
    Reference,
    LinearLight,
}

/// The version settings are saved as. Version 2 measures NTSC blending in Super Win the Game's
/// units, twice version 1's; version 3 gives the bloom's second blur a spread of its own. Settings
/// saved by an earlier version, or with no version, are migrated as they load.
pub const VERSION: u32 = 3;

/// Settings saved with no version are the first version's.
fn unversioned() -> u32 {
    1
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(remote = "Self", default, deny_unknown_fields)]
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
    #[serde(default = "unversioned")]
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
    /// How far the two composite artifact patterns blend into each other, in the units of
    /// Super Win the Game's NTSC Blending. With alternating phase, ticks show half this much of
    /// the other pattern and then the other way round: 0 switches cleanly between them, as the
    /// public source does, the game's 0.35 mixes them 17.5/82.5, and 1 shows their average on
    /// every tick. Up to 2, past the average, holds what version 1 settings could. Stable phase
    /// always shows the average.
    pub ntsc_blending: f32,
    /// The composite artifact pattern upside down against the picture, each tick blending the
    /// row above rather than the one below, as Super Win the Game's Linux build draws it. Off
    /// is the public source's (Direct3D) orientation, which the game's Windows build draws.
    pub flip_artifacts: bool,
    pub persistence: [f32; 3],
    /// The glow decays once a frame, whatever the frame's length, as Super Win the Game's does
    /// on any display: faster above 60 frames a second, slower below. Off, the glow decays by
    /// how long each frame lasts, as a 60 Hz tube's would, which is the same at 60. For videos
    /// and hosts; a still's and an editing preview's ticks are each a sixtieth either way.
    pub glow_per_frame: bool,
    /// The glow never fades below the level 8-bit history holds it at once a pixel has been lit:
    /// each channel at the brightest level its persistence rounds back to itself, 1 of 255 in
    /// red and green at the defaults. Super Win the Game's title screen lights every pixel with
    /// static, so its blacks sit there from then on. Off fades to black, as a tube never lit.
    pub glow_floor: bool,
    pub overscan: f32,
    pub barrel: f32,
    /// The picture tube's width over its height: the original's 4:3, or wider, as later sets
    /// were, to 16:9, or narrower, to a portrait 3:4. The glass is the original's stretched
    /// across; the bezel keeps its corners and sides, its top and bottom running between them.
    pub tube_aspect: f32,
    pub saturation: f32,
    pub mask_brightness: f32,
    pub mask_opacity: f32,
    pub mask_repeats: MaskRepeats,
    pub dimming: f32,
    pub reflection: f32,
    /// The bezel reflects the picture with the screen's overscan and mask density, as Super
    /// Win the Game draws it. Off, as the public source draws it, the reflection takes the
    /// overscan unreciprocated and half the mask's rows.
    pub reflection_as_screen: bool,
    pub diffuse: f32,
    pub specular: f32,
    pub specular_power: f32,
    pub rim: f32,
    pub light_position: [f32; 3],
    pub frame_color: [f32; 3],
    /// What shows where neither the glass nor the bezel is, past the bezel's edges in an output
    /// wider than 4:3. Super Win the Game draws a backdrop there, grey at 1/16.
    pub backdrop_color: [f32; 3],
    pub fov: f32,
    pub bloom: f32,
    pub bloom_power: f32,
    pub bloom_spread: f32,
    /// The bloom's first blur reads the picture through its mipmaps, each level the average of
    /// 2×2 texels of the one above, at the level GL picks for a target a sixteenth its size, as
    /// Super Win the Game draws it: each tap averages 16×16 of the picture's pixels. Off, as
    /// the public source draws it, each tap reads the picture itself, where the mask can alias
    /// into the glow.
    pub bloom_from_mipmaps: bool,
    /// The second blur's spread, which Super Win the Game sets apart from the first's
    /// (`bloom_spread`); 0.025 both.
    pub bloom_spread_up: f32,
    /// Super Win the Game's Video Brightness: the finished picture's luma raised to 2^-brightness,
    /// its hue kept, from -1, darker, to 1, brighter; 0 leaves it as it is.
    pub brightness: f32,
    /// Super Win the Game's FXAA over the glass and bezel.
    pub fxaa: Fxaa,
    /// Super Win the Game's tate: the finished picture turned for a screen on its side.
    pub tate: Tate,
    /// The CRT off, as Super Win the Game draws without its CRT simulation: the picture flat,
    /// pixel for pixel in its pixel aspect inside the output, over black, with no colour table,
    /// glow or bloom; brightness, FXAA and tate still apply.
    pub crt_off: bool,
    /// A still of an alternating look ends on a tick showing both artifact patterns averaged,
    /// as Super Win the Game draws the frame it saves as a screenshot, after warm-up ticks that
    /// alternate as the frames before it did. Off, a still shows its last tick's one pattern.
    pub screenshot_stills: bool,
    /// TV static over the signal, from 0 to 1: noise rolling under a dark band, the picture
    /// wavering beneath it, as a set between channels shows it. Each tick draws new noise.
    pub static_amount: f32,
    /// Seconds of a channel-changing cycle, as Super Win the Game's title screen runs one: static,
    /// fading to the picture, the picture, and back, over and over at `static_amount`'s
    /// strength. 0 holds the static at `static_amount`.
    pub static_cycle: f32,
    pub color_mode: ColorMode,
    /// Samples the mask from mipmaps, each the average of the level above, and between them,
    /// as the original did: the mask stays smooth where it is drawn smaller than it is. Off
    /// samples only the full-size mask, which shimmers into moiré when shrunk.
    pub mask_antialias: bool,
    /// Interlaced scanning: each tick refreshes only every other signal row, alternating
    /// between the two fields, and the rows it skips only decay by `persistence`. Meant for
    /// 480- or 576-row signals, as an interlaced set drew them. Off draws every row every tick.
    pub interlace: bool,
    /// Optional YIQ hue rotation, in degrees. Not the game's NES palette (`palette`).
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
            version: VERSION,
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
            flip_artifacts: false,
            reflection_as_screen: false,
            persistence: [0.7, 0.525, 0.42],
            glow_per_frame: false,
            glow_floor: false,
            overscan: 1.,
            barrel: -0.115,
            tube_aspect: 4. / 3.,
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
            backdrop_color: [0.; 3],
            fov: 15.,
            bloom: 0.25,
            bloom_power: 2.,
            bloom_spread: 0.025,
            bloom_from_mipmaps: false,
            bloom_spread_up: 0.025,
            brightness: 0.,
            fxaa: Fxaa::Off,
            tate: Tate::Off,
            crt_off: false,
            screenshot_stills: false,
            static_amount: 0.,
            static_cycle: 0.,
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
// Every route that reads settings, from a preset to a project's batch queue, deserializes a
// `Config`, so migrating here reaches them all.
impl<'de> Deserialize<'de> for Config {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut config = Config::deserialize(deserializer)?;
        config.migrate();
        Ok(config)
    }
}

impl Serialize for Config {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        Config::serialize(self, serializer)
    }
}

impl Config {
    /// Brings settings saved by an earlier version to this one's meaning, keeping their picture.
    fn migrate(&mut self) {
        if self.version == 1 {
            // Version 1's blending was half the game's: the same picture is twice the value.
            self.ntsc_blending *= 2.;
            self.version = 2;
        }
        if self.version == 2 {
            // Version 2's one spread was both blurs'.
            self.bloom_spread_up = self.bloom_spread;
            self.version = 3;
        }
    }

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

    /// Super Win the Game's own CRT options: the public reference with a 30° field of view,
    /// alternating phase with NTSC blending at 0.35 and its NTSC palette made as the game makes
    /// it, with the artifact pattern as its Windows build draws it, the bezel reflecting the
    /// picture as the screen shows it, a grey backdrop past the bezel, the bloom read through
    /// the picture's mipmaps, the glow held at its 8-bit floor, as after the title screen, and
    /// decaying once a frame, and stills drawn as its screenshots.
    pub fn super_win_the_game() -> Self {
        Self {
            fov: 30.,
            phase: Phase::Alternating,
            ntsc_blending: 0.35,
            reflection_as_screen: true,
            // The game's engine draws a backdrop at 1/16 grey, seen past the bezel.
            backdrop_color: [0.0625; 3],
            bloom_from_mipmaps: true,
            glow_floor: true,
            glow_per_frame: true,
            screenshot_stills: true,
            palette: Some(Default::default()),
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

    /// Maps colours through `lut`, which replaces the NES palette: a look uses one colour
    /// table at most.
    pub fn set_lut(&mut self, lut: Option<std::sync::Arc<crate::workflow::Lut>>) {
        if lut.is_some() {
            self.palette = None;
        }
        self.lut = lut;
    }

    /// Makes the NES palette from its composite signal, which replaces any LUT.
    pub fn set_palette(&mut self, palette: Option<crate::palette::NesPalette>) {
        if palette.is_some() {
            self.lut = None;
        }
        self.palette = palette;
    }

    /// One version-aware entry point for presets. Future migrations belong here instead of
    /// being duplicated across the CLI, desktop JSON loader and embedded metadata readers.
    pub fn from_json_slice(bytes: &[u8]) -> Result<Self> {
        let value: serde_json::Value = serde_json::from_slice(bytes)?;
        let version = value.get("version").and_then(serde_json::Value::as_u64);
        if let Some(version) = version {
            ensure!(
                (1..=u64::from(VERSION)).contains(&version),
                "unsupported config version {version}"
            );
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
        ensure!(self.version == VERSION, "unsupported config version");
        ensure!(self.warmup <= 240, "warmup must be <=240 ticks");
        crate::settings::validate(self)
    }
    /// The shape a sequence of these settings has, for an input of size `input`: two settings
    /// of one shape can follow each other in one sequence.
    pub fn sequence_shape(&self, input: (u32, u32)) -> Result<crate::Shape> {
        Ok(crate::Shape {
            signal: self.signal_size(input)?,
            output: self.output_size(input)?,
            color_mode: self.color_mode,
        })
    }
    /// What the signal is, as its saved name says.
    pub fn signal(&self) -> Result<Signal> {
        Ok(match self.signal.as_str() {
            "original" => Signal::Original,
            "native" => Signal::Native,
            "auto" => Signal::Auto,
            "240p" => Signal::Rows(240),
            "288p" => Signal::Rows(288),
            "360p" => Signal::Rows(360),
            "480p" => Signal::Rows(480),
            "576p" => Signal::Rows(576),
            v if v.contains('x') => {
                let (width, height) = dimensions(v)?;
                Signal::Size(width, height)
            }
            v => bail!("unknown signal preset: {v}"),
        })
    }
    pub fn signal_size(&self, input: (u32, u32)) -> Result<(u32, u32)> {
        validate_size(input)?;
        let input = self.source.size(input);
        let height = match self.signal()? {
            Signal::Original => return Ok((256, 224)),
            Signal::Native => return Ok(input),
            Signal::Auto => input.1.min(480),
            Signal::Rows(rows) => rows,
            Signal::Size(width, height) => return Ok((width, height)),
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
    /// How the picture's coordinates scale across the tube, as `fit` places it there.
    pub fn uv_scale(&self, signal: (u32, u32)) -> [f32; 2] {
        let ratio = signal.0 as f32 / signal.1 as f32 * self.pixel_aspect / self.tube_aspect;
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
    fn a_sequence_shape_changes_with_its_sizes_and_color_mode_only() {
        let input = (1216, 832);
        let base = Config::general();
        let shape = base.sequence_shape(input).unwrap();
        assert_eq!(
            (shape.signal, shape.output),
            (
                base.signal_size(input).unwrap(),
                base.output_size(input).unwrap()
            )
        );
        let edited = Config {
            bloom: 0.,
            mask_opacity: 0.2,
            ..base.clone()
        };
        assert_eq!(
            edited.sequence_shape(input).unwrap(),
            shape,
            "other settings keep it"
        );
        for other in [
            Config {
                signal: "240p".into(),
                ..base.clone()
            },
            Config {
                output: "720p".into(),
                ..base.clone()
            },
            Config {
                color_mode: ColorMode::LinearLight,
                ..base.clone()
            },
        ] {
            assert_ne!(other.sequence_shape(input).unwrap(), shape);
        }
        let unplayable = Config {
            output: "not a size".into(),
            ..base
        };
        assert!(unplayable.sequence_shape(input).is_err());
    }

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
    fn a_lut_and_the_nes_palette_replace_each_other() {
        let lut = std::sync::Arc::new(crate::nes_luts::load(0).unwrap());
        let mut c = Config::default();
        c.set_palette(Some(Default::default()));
        c.set_lut(Some(lut.clone()));
        assert!(c.palette.is_none() && c.validate().is_ok());
        c.set_palette(Some(Default::default()));
        assert!(c.lut.is_none() && c.validate().is_ok());
        c.set_lut(None);
        assert!(c.palette.is_some(), "removing no LUT keeps the palette");
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
        // Controls it leaves out take the game's defaults; without a model, as palettes were
        // saved before the game's own existed, it decodes the signal.
        let c = Config::from_json_slice(br#"{"palette":{}}"#).unwrap();
        assert_eq!(
            c.palette,
            Some(crate::palette::NesPalette {
                model: crate::palette::Model::Signal,
                ..Default::default()
            })
        );
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
            VERSION
        );
        let error = Config::from_json_slice(b"{\"version\":4}").unwrap_err();
        assert!(error.to_string().contains("unsupported config version 4"));
    }

    #[test]
    fn version_1_blending_is_doubled_to_keep_its_picture() {
        let blending = |json: &str| {
            Config::from_json_slice(json.as_bytes())
                .unwrap()
                .ntsc_blending
        };
        assert_eq!(blending(r#"{"version":1,"ntsc_blending":0.35}"#), 0.7);
        assert_eq!(blending(r#"{"ntsc_blending":0.35}"#), 0.7);
        // Past the average, which version 1 allowed, stays where it was.
        assert_eq!(blending(r#"{"version":1,"ntsc_blending":1.0}"#), 2.);
        assert_eq!(blending(r#"{"version":2,"ntsc_blending":0.35}"#), 0.35);
        assert_eq!(blending(r#"{"version":3,"ntsc_blending":0.35}"#), 0.35);
        // Settings nested in other files, such as a project's, migrate too.
        let nested: Vec<Config> =
            serde_json::from_str(r#"[{"version":1,"ntsc_blending":0.25}]"#).unwrap();
        assert_eq!((nested[0].version, nested[0].ntsc_blending), (VERSION, 0.5));
        // Saved again, they are the current version and load unchanged.
        let saved = serde_json::to_vec(&nested[0]).unwrap();
        assert_eq!(Config::from_json_slice(&saved).unwrap(), nested[0]);
    }
    #[test]
    fn version_2_spread_is_both_blurs() {
        let spreads = |json: &str| {
            let c = Config::from_json_slice(json.as_bytes()).unwrap();
            (c.bloom_spread, c.bloom_spread_up)
        };
        assert_eq!(
            spreads(r#"{"version":2,"bloom_spread":0.05}"#),
            (0.05, 0.05)
        );
        assert_eq!(spreads(r#"{"bloom_spread":0.04}"#), (0.04, 0.04));
        assert_eq!(
            spreads(r#"{"version":3,"bloom_spread":0.05}"#),
            (0.05, 0.025)
        );
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
