//! Every setting a preset holds, once, in the order the desktop shows them: its key in preset
//! JSON, its name, and for the numeric ones the values a preset may hold and how the desktop
//! edits them. Validation, the desktop's sliders and its lists of how two presets differ all
//! read this table, so a setting's name and limits cannot drift apart between them.

use crate::config::Config;
use anyhow::{ensure, Result};
use std::ops::{
    Bound::{self, Excluded, Included},
    RangeBounds, RangeInclusive,
};

/// A group of controls in the desktop's settings panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Image,
    Grade,
    Signal,
    Glass,
    Bloom,
    Lighting,
    Persistence,
}

pub struct Setting {
    /// The key in preset JSON; a nested setting joins its keys with dots.
    pub key: &'static str,
    /// The name of the setting as a whole.
    pub label: &'static str,
    /// For a numeric setting, its values' limits and controls. The others -- sizes, choices,
    /// switches, source framing and the LUT -- are validated and drawn by hand.
    pub numbers: Option<Numbers>,
}

pub struct Numbers {
    pub section: Section,
    /// One name per value of an array, each for its own slider; empty for a single value.
    pub names: &'static [&'static str],
    pub control: Control,
    /// What a preset may hold.
    pub valid: (Bound<f32>, Bound<f32>),
    pub access: Access,
}

pub enum Control {
    /// A slider spanning this range. A typed value may go past it, as far as the setting allows.
    Slider {
        span: RangeInclusive<f32>,
        logarithmic: bool,
    },
    /// A color picker over three values.
    Color,
}

/// Reaches a setting's numbers as a slice, so a single value and an array read alike.
pub struct Access {
    pub get: fn(&Config) -> &[f32],
    pub get_mut: fn(&mut Config) -> &mut [f32],
}

trait Values {
    fn values(&self) -> &[f32];
    fn values_mut(&mut self) -> &mut [f32];
}
impl Values for f32 {
    fn values(&self) -> &[f32] {
        std::slice::from_ref(self)
    }
    fn values_mut(&mut self) -> &mut [f32] {
        std::slice::from_mut(self)
    }
}
impl<const N: usize> Values for [f32; N] {
    fn values(&self) -> &[f32] {
        self
    }
    fn values_mut(&mut self) -> &mut [f32] {
        self
    }
}

macro_rules! access {
    ($field:ident) => {
        Access {
            get: |c| c.$field.values(),
            get_mut: |c| c.$field.values_mut(),
        }
    };
}

impl Setting {
    const fn other(key: &'static str, label: &'static str) -> Self {
        Self {
            key,
            label,
            numbers: None,
        }
    }
    const fn numbers(key: &'static str, label: &'static str, numbers: Numbers) -> Self {
        Self {
            key,
            label,
            numbers: Some(numbers),
        }
    }
    /// The name of one of its values: its own name within an array, else the setting's.
    pub fn value_name(&self, index: usize) -> &'static str {
        match &self.numbers {
            Some(numbers) if index < numbers.names.len() => numbers.names[index],
            _ => self.label,
        }
    }
}

impl Numbers {
    /// A slider whose span is also what a preset may hold.
    const fn slider(section: Section, access: Access, span: RangeInclusive<f32>) -> Self {
        Self {
            section,
            names: &[],
            valid: (Included(*span.start()), Included(*span.end())),
            control: Control::Slider {
                span,
                logarithmic: false,
            },
            access,
        }
    }
    const fn color(section: Section, access: Access) -> Self {
        Self {
            section,
            names: &[],
            control: Control::Color,
            valid: (Included(0.), Included(1.)),
            access,
        }
    }
    const fn named(self, names: &'static [&'static str]) -> Self {
        Self { names, ..self }
    }
    const fn logarithmic(self) -> Self {
        let Control::Slider { span, .. } = self.control else {
            panic!("only a slider is logarithmic")
        };
        Self {
            control: Control::Slider {
                span,
                logarithmic: true,
            },
            ..self
        }
    }
    /// What a preset may hold, where that is wider than the slider.
    const fn accepting(self, valid: (Bound<f32>, Bound<f32>)) -> Self {
        Self { valid, ..self }
    }
}

use Section::*;

pub static SETTINGS: &[Setting] = &[
    Setting::other("signal", "Signal"),
    Setting::other("output", "Export size"),
    Setting::other("fit", "Fit on 4:3 tube"),
    Setting::other("filter", "Resize filter"),
    Setting::numbers(
        "pixel_aspect",
        "Pixel aspect",
        Numbers::slider(Image, access!(pixel_aspect), 0.1..=10.),
    ),
    Setting::other("screen_only", "Screen only"),
    Setting::other("source.crop", "Crop (left, top, right, bottom)"),
    Setting::other("source.rotation", "Rotation °"),
    Setting::other("source.zoom", "Source zoom"),
    Setting::other("source.position", "Pan (X, Y)"),
    Setting::other("source.background", "Transparency background"),
    Setting::other("source.checkerboard", "Checker background"),
    Setting::other("lut", "LUT"),
    Setting::other("color_mode", "Color processing"),
    Setting::numbers(
        "hue",
        "Hue (degrees)",
        Numbers::slider(Grade, access!(hue), -180.0..=180.),
    ),
    Setting::numbers(
        "chroma",
        "Chroma",
        Numbers::slider(Grade, access!(chroma), 0.0..=2.),
    ),
    Setting::other("mask_antialias", "Filter mask when shrinking"),
    Setting::numbers(
        "saturation",
        "Saturation",
        Numbers::slider(Signal, access!(saturation), 0.0..=3.),
    ),
    Setting::numbers(
        "sharpness",
        "Sharpness / ringing",
        Numbers::slider(Signal, access!(sharpness), 0.0..=3.),
    ),
    Setting::numbers(
        "bleed",
        "Color bleed",
        Numbers::slider(Signal, access!(bleed), 0.0..=2.),
    ),
    Setting::numbers(
        "artifacts",
        "Composite artifacts",
        Numbers::slider(Signal, access!(artifacts), 0.0..=2.),
    ),
    Setting::numbers(
        "barrel",
        "Barrel distortion",
        Numbers::slider(Glass, access!(barrel), -2.0..=2.),
    ),
    Setting::numbers(
        "overscan",
        "Overscan",
        Numbers::slider(Glass, access!(overscan), 0.1..=3.),
    ),
    Setting::numbers(
        "mask_opacity",
        "Mask opacity",
        Numbers::slider(Glass, access!(mask_opacity), 0.0..=1.),
    ),
    Setting::numbers(
        "mask_brightness",
        "Mask brightness",
        Numbers::slider(Glass, access!(mask_brightness), 0.0..=2.),
    ),
    Setting::numbers(
        "mask_repeats",
        "Mask columns, rows",
        Numbers::slider(Glass, access!(mask_repeats), 1.0..=16384.)
            .logarithmic()
            .named(&["Mask columns", "Mask rows"])
            .accepting((Excluded(0.), Included(16384.))),
    ),
    Setting::numbers(
        "dimming",
        "Edge dimming",
        Numbers::slider(Glass, access!(dimming), 0.0..=1.),
    ),
    Setting::numbers(
        "fov",
        "Camera field of view",
        Numbers::slider(Glass, access!(fov), 5.0..=90.),
    ),
    Setting::numbers(
        "bloom",
        "Bloom amount",
        Numbers::slider(Bloom, access!(bloom), 0.0..=2.),
    ),
    Setting::numbers(
        "bloom_power",
        "Bloom power",
        Numbers::slider(Bloom, access!(bloom_power), 0.1..=8.),
    ),
    Setting::numbers(
        "bloom_spread",
        "Bloom spread",
        Numbers::slider(Bloom, access!(bloom_spread), 0.0..=0.2),
    ),
    Setting::numbers(
        "reflection",
        "Edge reflection",
        Numbers::slider(Bloom, access!(reflection), 0.0..=2.),
    ),
    Setting::numbers(
        "frame_color",
        "Frame color",
        Numbers::color(Lighting, access!(frame_color)),
    ),
    Setting::numbers(
        "diffuse",
        "Diffuse light",
        Numbers::slider(Lighting, access!(diffuse), 0.0..=2.),
    ),
    Setting::numbers(
        "specular",
        "Specular light",
        Numbers::slider(Lighting, access!(specular), 0.0..=2.),
    ),
    Setting::numbers(
        "specular_power",
        "Specular power",
        Numbers::slider(Lighting, access!(specular_power), 1.0..=200.).logarithmic(),
    ),
    Setting::numbers(
        "rim",
        "Rim light",
        Numbers::slider(Lighting, access!(rim), 0.0..=2.),
    ),
    Setting::numbers(
        "light_position",
        "Light position (X, Y, Z)",
        Numbers::slider(Lighting, access!(light_position), -1000.0..=1000.)
            .named(&["Light X", "Light Y", "Light Z"]),
    ),
    Setting::numbers(
        "persistence",
        "Persistence (R, G, B)",
        Numbers::slider(Persistence, access!(persistence), 0.0..=0.999)
            .named(&["Red persistence", "Green persistence", "Blue persistence"])
            .accepting((Included(0.), Excluded(1.))),
    ),
    Setting::other("warmup", "Warm-up ticks"),
    Setting::other("phase", "Phase"),
    Setting::other("interlace", "Interlaced fields"),
];

/// Checks every numeric setting against what a preset may hold, naming the first that is not.
pub(crate) fn validate(c: &Config) -> Result<()> {
    for setting in SETTINGS {
        let Some(numbers) = &setting.numbers else {
            continue;
        };
        for (index, value) in (numbers.access.get)(c).iter().enumerate() {
            ensure!(
                numbers.valid.contains(value),
                "{} must be {}, not {value}",
                setting.value_name(index),
                describe(&numbers.valid)
            );
        }
    }
    Ok(())
}

fn describe((low, high): &(Bound<f32>, Bound<f32>)) -> String {
    let low = match low {
        Included(v) => format!("at least {v}"),
        Excluded(v) => format!("above {v}"),
        Bound::Unbounded => "any value".into(),
    };
    match high {
        Included(v) => format!("{low} and at most {v}"),
        Excluded(v) => format!("{low} and below {v}"),
        Bound::Unbounded => low,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(prefix: &str, value: &serde_json::Value, out: &mut Vec<String>) {
        match value {
            serde_json::Value::Object(map) => {
                for (key, value) in map {
                    let key = if prefix.is_empty() {
                        key.clone()
                    } else {
                        format!("{prefix}.{key}")
                    };
                    keys(&key, value, out);
                }
            }
            _ => out.push(prefix.to_owned()),
        }
    }

    #[test]
    fn every_preset_key_has_one_setting() {
        let mut found = vec![];
        keys(
            "",
            &serde_json::to_value(Config::default()).unwrap(),
            &mut found,
        );
        found.retain(|key| key != "version");
        let mut listed: Vec<_> = SETTINGS.iter().map(|s| s.key.to_owned()).collect();
        found.sort();
        listed.sort();
        assert_eq!(found, listed);
    }

    #[test]
    fn numbers_are_checked_against_their_limits_by_name() {
        assert!(validate(&Config::default()).is_ok());
        assert!(validate(&Config::general()).is_ok());
        let c = Config {
            bloom: 2.5,
            ..Config::default()
        };
        assert_eq!(
            validate(&c).unwrap_err().to_string(),
            "Bloom amount must be at least 0 and at most 2, not 2.5"
        );
        let mut c = Config::default();
        c.persistence[1] = 1.;
        assert_eq!(
            validate(&c).unwrap_err().to_string(),
            "Green persistence must be at least 0 and below 1, not 1"
        );
        // The slider stops short of what a preset may hold.
        c.persistence[1] = 0.9995;
        c.mask_repeats[0] = 0.5;
        assert!(validate(&c).is_ok());
        c.mask_repeats[0] = 0.;
        assert!(validate(&c).is_err());
        c.mask_repeats[0] = 128.;
        c.frame_color[2] = f32::NAN;
        assert!(validate(&c).is_err());
    }
}
