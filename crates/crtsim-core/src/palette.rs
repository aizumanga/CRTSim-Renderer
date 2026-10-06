//! The NES palette with Super Win the Game's NTSC palette controls -- Tint, Tint I and Tint Q --
//! turned into a colour LUT, made one of two ways ([`Model`]):
//!
//! - **The game's**, exactly as Super Win the Game makes it (`game_palette`), taking the NES
//!   palette its art is drawn in onto the colours its controls give.
//! - **From the composite signal**: the NES palette decoded the documented way, from the PPU's
//!   measured voltage levels, sampled at the twelve phases of its colour wave and demodulated
//!   against a colour burst at hue 8. A set's colour control sizes chroma by the burst, which
//!   the NES sends larger than broadcast; the resulting 60% and an 8 degree phase offset are
//!   fitted to FirebrandX's Composite Direct capture as the bundled LUT holds it, and land
//!   within 23 steps of it on average. Tint turns every hue and the other two scale the I and
//!   Q axes, about the game's defaults as the standard decode. The input side is MAME's
//!   palette, which the bundled NES LUTs also expect, so a frame from MAME -- or art drawn in
//!   its colours -- maps exactly, and colours between its entries follow smoothly rather than
//!   being forced onto one.

use crate::workflow::Lut;
use serde::{Deserialize, Serialize};
use std::{
    f64::consts::PI,
    sync::{Arc, Mutex},
};

/// The game's NTSC palette controls, and how the palette is made from them.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NesPalette {
    /// Turns every hue, in radians.
    pub tint: f32,
    /// Scales the I axis: orange against blue.
    pub tint_i: f32,
    /// Scales the Q axis: purple against green.
    pub tint_q: f32,
    /// Palettes saved before the game's own existed decoded the signal, and keep doing so.
    #[serde(default = "Model::saved_without_one")]
    pub model: Model,
}

/// How the palette is made from the controls.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Model {
    /// Exactly as Super Win the Game makes it, for art in the NES palette the game's is drawn
    /// in. I and Q only steer each hue there, never its saturation.
    #[default]
    Game,
    /// Decoded from the NES's composite signal, for art in MAME's NES palette.
    Signal,
}

impl Model {
    fn saved_without_one() -> Self {
        Self::Signal
    }

    /// The palette's name after "NES palette", as the app and a generated LUT show it.
    pub fn name(self) -> &'static str {
        match self {
            Model::Game => "Super Win the Game's",
            Model::Signal => "from the composite signal",
        }
    }
}

impl Default for NesPalette {
    /// The game's defaults.
    fn default() -> Self {
        Self {
            tint: 5.183186,
            tint_i: 1.75,
            tint_q: 1.,
            model: Model::Game,
        }
    }
}

/// The controls at which the signal's decode is the standard one, which its Tint turns from
/// and its I and Q scale from.
const SIGNAL_NEUTRAL: (f32, f32, f32) = (5.18, 1.75, 1.);

/// Samples along each side of the signal model's LUT, as in the bundled NES LUTs.
const LUT_SIZE: usize = 64;

impl NesPalette {
    /// The 64 colours of the palette, in 0-1 RGB, indexed by NES colour number.
    pub fn colors(&self) -> [[f64; 3]; 64] {
        match self.model {
            Model::Game => crate::game_palette::colors(self.tint, self.tint_i, self.tint_q)
                .map(|color| color.map(|c| f64::from(c) / 255.)),
            Model::Signal => self.signal_colors(),
        }
    }

    /// The 64 colours the signal decodes to.
    fn signal_colors(&self) -> [[f64; 3]; 64] {
        let (tint, tint_i, tint_q) = SIGNAL_NEUTRAL;
        let turn = f64::from(self.tint - tint);
        let gain_i = f64::from(self.tint_i / tint_i);
        let gain_q = f64::from(self.tint_q / tint_q);
        std::array::from_fn(|index| {
            let (y, u, v) = decode(index);
            // Tint turns the hue; the gains act on I and Q, which sit 33 degrees round from
            // U and V.
            let (sin, cos) = turn.sin_cos();
            let (u, v) = (u * cos - v * sin, u * sin + v * cos);
            let (sin33, cos33) = (33f64).to_radians().sin_cos();
            let i = (v * cos33 - u * sin33) * gain_i;
            let q = (v * sin33 + u * cos33) * gain_q;
            let (u, v) = (q * cos33 - i * sin33, q * sin33 + i * cos33);
            let (b, r) = (y + u / 0.492, y + v / 0.877);
            let g = (y - 0.299 * r - 0.114 * b) / 0.587;
            [r, g, b].map(|c| c.clamp(0., 1.))
        })
    }

    /// The palette as a LUT from the NES colours its model expects to these, shared while the
    /// settings stay the same, so a renderer can keep it on the GPU. The few palettes used last are kept, so
    /// a preview, an export and a gallery thumbnail with different settings do not keep
    /// replacing each other's.
    pub fn lut(&self) -> Arc<Lut> {
        const KEPT: usize = 4;
        static RECENT: Mutex<Vec<(NesPalette, Arc<Lut>)>> = Mutex::new(Vec::new());
        let mut recent = RECENT
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(at) = recent.iter().position(|(settings, _)| settings == self) {
            let entry = recent.remove(at);
            let lut = entry.1.clone();
            recent.push(entry);
            return lut;
        }
        let lut = Arc::new(self.build_lut());
        if recent.len() == KEPT {
            recent.remove(0);
        }
        recent.push((*self, lut.clone()));
        lut
    }

    fn build_lut(&self) -> Lut {
        let name = format!(
            "NES palette, {} (tint {:.2}, I {:.2}, Q {:.2})",
            self.model.name(),
            self.tint,
            self.tint_i,
            self.tint_q
        );
        match self.model {
            Model::Game => crate::game_palette::lut(
                &crate::game_palette::colors(self.tint, self.tint_i, self.tint_q),
                name,
            ),
            Model::Signal => self.signal_lut(name),
        }
    }

    fn signal_lut(&self, name: String) -> Lut {
        // Entries MAME draws alike (its blacks, its two whites) share one target: the average.
        let mut points: Vec<([f64; 3], [f64; 3], f64)> = vec![];
        for (from, to) in mame_colors().into_iter().zip(self.colors()) {
            match points.iter_mut().find(|(at, ..)| *at == from) {
                Some((_, total, count)) => {
                    for c in 0..3 {
                        total[c] += to[c];
                    }
                    *count += 1.;
                }
                None => points.push((from, to, 1.)),
            }
        }
        let points: Vec<([f64; 3], [f64; 3])> = points
            .into_iter()
            .map(|(from, total, count)| {
                let to = total.map(|c| c / count);
                (from, std::array::from_fn(|c| to[c] - from[c]))
            })
            .collect();
        let step = 1. / (LUT_SIZE - 1) as f64;
        let values = (0..LUT_SIZE.pow(3))
            .map(|n| {
                let at = [
                    n % LUT_SIZE,
                    n / LUT_SIZE % LUT_SIZE,
                    n / LUT_SIZE / LUT_SIZE,
                ]
                .map(|k| k as f64 * step);
                shift(&points, at).map(|c| c as f32)
            })
            .collect();
        Lut {
            name,
            size: LUT_SIZE,
            domain_min: [0.; 3],
            domain_max: [1.; 3],
            values,
            sampling: Default::default(),
        }
    }
}

/// Where `at` goes: moved by each palette entry's correction, weighted by closeness, so every
/// entry lands exactly on its target and the colours between follow smoothly.
fn shift(points: &[([f64; 3], [f64; 3])], at: [f64; 3]) -> [f64; 3] {
    let mut total = [0.; 3];
    let mut weights = 0.;
    for (from, delta) in points {
        let distance: f64 = (0..3).map(|c| (at[c] - from[c]).powi(2)).sum();
        if distance < 1e-12 {
            return std::array::from_fn(|c| (at[c] + delta[c]).clamp(0., 1.));
        }
        // Inverse distance to the fourth power: flat around each entry, so trilinear lookups
        // near one land on its target too.
        let weight = 1. / (distance * distance);
        weights += weight;
        for c in 0..3 {
            total[c] += weight * delta[c];
        }
    }
    std::array::from_fn(|c| (at[c] + total[c] / weights).clamp(0., 1.))
}

/// The composite signal of NES colour `index` decoded to Y, U and V, before any tint: black 0,
/// white 1, and hue 8 on the colour burst at 180 degrees.
fn decode(index: usize) -> (f64, f64, f64) {
    // The PPU's output levels in volts, for luma levels 0-3: the wave's low and high halves.
    const LOW: [f64; 4] = [0.350, 0.518, 0.962, 1.550];
    const HIGH: [f64; 4] = [1.094, 1.506, 1.962, 1.962];
    const BLACK: f64 = 0.518;
    const WHITE: f64 = 1.962;
    let hue = index % 16;
    let level = if hue > 13 { 1 } else { index / 16 };
    let (mut low, mut high) = (LOW[level], HIGH[level]);
    if hue == 0 {
        low = high;
    }
    if hue > 12 {
        high = low;
    }
    let (mut y, mut re, mut im) = (0., 0., 0.);
    for phase in 0..12 {
        let volts = if (hue + phase) % 12 < 6 { high } else { low };
        let signal = (volts - BLACK) / (WHITE - BLACK);
        let angle = PI * phase as f64 / 6.;
        y += signal / 12.;
        re += signal * angle.cos() * 2. / 12.;
        im -= signal * angle.sin() * 2. / 12.;
    }
    // Hue 8's wave sits at 165 degrees here; the decoder turns it onto the burst, give or take
    // its phase offset, and sizes chroma by the burst.
    const PHASE_OFFSET: f64 = -8.;
    const CHROMA: f64 = 0.6;
    let turn = PI - PI / 6. * 5.5 + PHASE_OFFSET.to_radians();
    let (sin, cos) = turn.sin_cos();
    (
        y,
        (re * cos - im * sin) * CHROMA,
        (re * sin + im * cos) * CHROMA,
    )
}

/// MAME's NES palette (`ppu2c0x_device::nespal_to_RGB`, BSD-3-Clause, no colour emphasis),
/// in 0-1 RGB after its rounding to whole steps.
pub fn mame_colors() -> [[f64; 3]; 64] {
    const TINT: f64 = 0.22;
    const HUE: f64 = 287.;
    const KR: f64 = 0.2989;
    const KB: f64 = 0.1145;
    const KU: f64 = 2.029;
    const KV: f64 = 1.140;
    const BRIGHTNESS: [[f64; 4]; 3] = [
        [0.50, 0.75, 1.0, 1.0],
        [0.29, 0.45, 0.73, 0.9],
        [0., 0.24, 0.47, 0.77],
    ];
    std::array::from_fn(|index| {
        let (intensity, number) = (index / 16, index % 16);
        let (saturation, radians, y) = match number {
            0 => (0., 0., BRIGHTNESS[0][intensity]),
            13 => (0., 0., BRIGHTNESS[2][intensity]),
            14 | 15 => (0., 0., 0.),
            _ => (
                TINT,
                (number as f64 * 30. + HUE).to_radians(),
                BRIGHTNESS[1][intensity],
            ),
        };
        let (u, v) = (saturation * radians.cos(), saturation * radians.sin());
        let r = (y + KV * v) * 255.;
        let g = (y - (KB * KU * u + KR * KV * v) / (1. - KB - KR)) * 255.;
        let b = (y + KU * u) * 255.;
        [r, g, b].map(|c| (c.clamp(0., 255.) + 0.5).floor() / 255.)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn steps(color: [f64; 3]) -> [i32; 3] {
        color.map(|c| (c * 255.).round() as i32)
    }

    /// The signal model at the game's controls.
    fn signal() -> NesPalette {
        NesPalette {
            model: Model::Signal,
            ..NesPalette::default()
        }
    }

    #[test]
    fn the_signal_decodes_to_the_nes_colours_it_is_known_by() {
        let colors = signal().colors();
        // Greys: black, the four levels of hue 0, and hue 13 at level 0 below black.
        assert_eq!(steps(colors[0x0F]), [0, 0, 0]);
        assert_eq!(steps(colors[0x0D]), [0, 0, 0]);
        assert_eq!(steps(colors[0x20]), [255, 255, 255]);
        assert_eq!(steps(colors[0x30]), [255, 255, 255]);
        let [r, g, b] = colors[0x00];
        assert!(r == g && g == b && r > 0.3 && r < 0.5);
        // Hues round the wheel: 2 blue, 6 red, 8 yellow, 10 green, 12 cyan.
        let hue = |index: usize| {
            let [r, g, b] = colors[index];
            let (max, min) = (r.max(g).max(b), r.min(g).min(b));
            let degrees = if max == r {
                60. * ((g - b) / (max - min))
            } else if max == g {
                60. * ((b - r) / (max - min) + 2.)
            } else {
                60. * ((r - g) / (max - min) + 4.)
            };
            degrees.rem_euclid(360.)
        };
        assert!((200. ..260.).contains(&hue(0x12)), "blue {}", hue(0x12));
        assert!(hue(0x16) < 20. || hue(0x16) > 340., "red {}", hue(0x16));
        assert!((40. ..70.).contains(&hue(0x28)), "yellow {}", hue(0x28));
        assert!((90. ..150.).contains(&hue(0x2A)), "green {}", hue(0x2A));
        assert!((170. ..200.).contains(&hue(0x2C)), "cyan {}", hue(0x2C));
    }

    #[test]
    fn tint_turns_hues_and_the_gains_scale_chroma() {
        let neutral = signal().colors();
        let turned = NesPalette {
            tint: 5.18 + std::f32::consts::PI,
            ..signal()
        }
        .colors();
        // Half a turn swaps a hue for its opposite: blue for yellow-ish.
        assert!(turned[0x12][0] > turned[0x12][2]);
        for c in 0..3 {
            assert!(
                (turned[0x20][c] - neutral[0x20][c]).abs() < 1e-9,
                "grey stays grey"
            );
        }
        let grey = NesPalette {
            tint_i: 0.,
            tint_q: 0.,
            ..signal()
        }
        .colors();
        for color in grey {
            assert!((color[0] - color[1]).abs() < 1e-9 && (color[1] - color[2]).abs() < 1e-9);
        }
    }

    #[test]
    fn the_lut_takes_mames_palette_onto_the_generated_one() {
        let settings = NesPalette {
            tint: 5.5,
            ..signal()
        };
        let lut = settings.lut();
        lut.validate().unwrap();
        assert!(
            Arc::ptr_eq(&lut, &settings.lut()),
            "the same settings share one LUT"
        );
        let targets = settings.colors();
        let mut image = image::RgbaImage::new(64, 1);
        for (index, color) in mame_colors().iter().enumerate() {
            let [r, g, b] = steps(*color).map(|c| c as u8);
            image.put_pixel(index as u32, 0, image::Rgba([r, g, b, 255]));
        }
        lut.apply(&mut image);
        // MAME draws several entries alike; each distinct one lands on its own target.
        for index in [0x00, 0x01, 0x06, 0x12, 0x16, 0x21, 0x27, 0x2A, 0x31, 0x3C] {
            let got = image.get_pixel(index as u32, 0);
            let want = steps(targets[index]);
            for c in 0..3 {
                assert!(
                    (i32::from(got[c]) - want[c]).abs() <= 3,
                    "{index:#04x}: {got:?} for {want:?}"
                );
            }
        }
    }
}

#[cfg(test)]
mod model_tests {
    use super::*;

    #[test]
    fn palettes_saved_without_a_model_keep_decoding_the_signal() {
        let saved: NesPalette = serde_json::from_str(r#"{"tint":5.18}"#).unwrap();
        assert_eq!(saved.model, Model::Signal);
        let new: NesPalette = serde_json::from_str(r#"{"model":"game"}"#).unwrap();
        assert_eq!(new, NesPalette::default());
        assert_eq!(NesPalette::default().model, Model::Game);
    }

    #[test]
    fn the_games_lut_takes_its_art_palette_onto_its_colours() {
        let settings = NesPalette::default();
        let lut = settings.lut();
        lut.validate().unwrap();
        assert_eq!(lut.size, 32);
        let colors = crate::game_palette::colors(settings.tint, settings.tint_i, settings.tint_q);
        // At a table sample, the colour the nearest art colour turns into: grey 0x10 sits on
        // level 23 of 31 (188.9), and 0x30 near white on level 31.
        let at = |r: usize, g: usize, b: usize| lut.values[r + 32 * (g + 32 * b)];
        let expect = |index: usize| colors[index].map(|c| f32::from(c) / 255.);
        assert_eq!(at(23, 23, 23), expect(0x10));
        assert_eq!(at(31, 31, 31), expect(0x20));
        assert_eq!(at(0, 0, 0), expect(0x0D));
    }
}
