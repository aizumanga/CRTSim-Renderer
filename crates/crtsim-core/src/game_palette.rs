//! Super Win the Game's NTSC palette as the game itself builds it: [`colors`] is its
//! `PaletteGen::MakePalette` and [`lut`] its `PaletteGen::MakePaletteLUT`, read from the
//! shipped executable and checked against what the running game generates. See
//! `docs/research/swtg-crt`.
//!
//! The game works in 32-bit floats on the x87 unit: values it stores are rounded to `f32` here
//! where it stores them, and its intermediates, which the x87 keeps wider, are carried in
//! `f64`.

use crate::workflow::{Lut, Sampling};

/// Each luma row's low and high signal level: grey at the low level is colour 0x?D, grey at the
/// high level colour 0x?0, and the hues sit half-way with the difference as their chroma.
const LOW: [f32; 4] = [-0.117, 0., 0.308, 0.715];
const HIGH: [f32; 4] = [0.397, 0.681, 1., 1.];

/// The twelve hues' turns past Tint, as the game holds them: 30 degrees apart, the last a
/// unit in the last place off `f32` 330 degrees.
const HUE_STEPS: [u32; 12] = [
    0,
    0x3f06_0a92,
    0x3f86_0a92,
    0x3fc9_0fdb,
    0x4006_0a92,
    0x4027_8d36,
    0x4049_0fdb,
    0x406a_927f,
    0x4086_0a92,
    0x4096_cbe4,
    0x40a7_8d36,
    0x40b8_4e89,
];

/// The colours of the NES palette the game's art is drawn in, which the LUT takes from: the
/// widely used palette beginning `7C7C7C 0000FC 0000BC`, indexed by NES colour number.
pub const SOURCE: [[u8; 3]; 64] = [
    [0x7C, 0x7C, 0x7C],
    [0x00, 0x00, 0xFC],
    [0x00, 0x00, 0xBC],
    [0x44, 0x28, 0xBC],
    [0x94, 0x00, 0x84],
    [0xA8, 0x00, 0x20],
    [0xA8, 0x10, 0x00],
    [0x88, 0x14, 0x00],
    [0x50, 0x30, 0x00],
    [0x00, 0x78, 0x00],
    [0x00, 0x68, 0x00],
    [0x00, 0x58, 0x00],
    [0x00, 0x40, 0x58],
    [0x00, 0x00, 0x00],
    [0x00, 0x00, 0x00],
    [0x00, 0x00, 0x00],
    [0xBC, 0xBC, 0xBC],
    [0x00, 0x78, 0xF8],
    [0x00, 0x58, 0xF8],
    [0x68, 0x44, 0xFC],
    [0xD8, 0x00, 0xCC],
    [0xE4, 0x00, 0x58],
    [0xF8, 0x38, 0x00],
    [0xE4, 0x5C, 0x10],
    [0xAC, 0x7C, 0x00],
    [0x00, 0xB8, 0x00],
    [0x00, 0xA8, 0x00],
    [0x00, 0xA8, 0x44],
    [0x00, 0x88, 0x88],
    [0x08, 0x08, 0x08],
    [0x00, 0x00, 0x00],
    [0x00, 0x00, 0x00],
    [0xF8, 0xF8, 0xF8],
    [0x3C, 0xBC, 0xFC],
    [0x68, 0x88, 0xFC],
    [0x98, 0x78, 0xF8],
    [0xF8, 0x78, 0xF8],
    [0xF8, 0x58, 0x98],
    [0xF8, 0x78, 0x58],
    [0xFC, 0xA0, 0x44],
    [0xF8, 0xB8, 0x00],
    [0xB8, 0xF8, 0x18],
    [0x58, 0xD8, 0x54],
    [0x58, 0xF8, 0x98],
    [0x00, 0xE8, 0xD8],
    [0x78, 0x78, 0x78],
    [0x00, 0x00, 0x00],
    [0x00, 0x00, 0x00],
    [0xFC, 0xFC, 0xFC],
    [0xA4, 0xE4, 0xFC],
    [0xB8, 0xB8, 0xF8],
    [0xD8, 0xB8, 0xF8],
    [0xF8, 0xB8, 0xF8],
    [0xF8, 0xA4, 0xC0],
    [0xF0, 0xD0, 0xB0],
    [0xFC, 0xE0, 0xA8],
    [0xF8, 0xD8, 0x78],
    [0xD8, 0xF8, 0x78],
    [0xB8, 0xF8, 0xB8],
    [0xB8, 0xF8, 0xD8],
    [0x00, 0xFC, 0xFC],
    [0xD8, 0xD8, 0xD8],
    [0x00, 0x00, 0x00],
    [0x00, 0x00, 0x00],
];

/// Samples along each side of the game's LUT.
const LUT_SIZE: usize = 32;

/// The 64 NES colours at Tint, I and Q, as the game makes them. Colours 0x?E and 0x?F are
/// black. Tint turns every hue, in radians; I and Q set only the direction each hue takes in
/// the IQ plane, never its saturation, which comes from the row's signal levels.
pub fn colors(tint: f32, scale_i: f32, scale_q: f32) -> [[u8; 3]; 64] {
    let mut colors = [[0; 3]; 64];
    for row in 0..4 {
        let (low, high) = (LOW[row], HIGH[row]);
        let chroma = high - low;
        let luma = (high + low) * 0.5;
        colors[row * 16] = from_yiq(high, 0., 0.);
        for (hue, step) in HUE_STEPS.iter().enumerate() {
            let turn = f64::from(tint) + f64::from(f32::from_bits(*step));
            let i = (f64::from(scale_i) * turn.sin()) as f32;
            let q = (f64::from(scale_q) * turn.cos()) as f32;
            let (i, q) = (f64::from(i), f64::from(q));
            let length = (i * i + q * q).sqrt();
            // The game cannot reach this, its sliders stopping above zero; with neither axis
            // there is no hue, so the colour is grey.
            let (i, q) = if length > 0. {
                (
                    i / length * f64::from(chroma),
                    q / length * f64::from(chroma),
                )
            } else {
                (0., 0.)
            };
            colors[row * 16 + 1 + hue] = from_yiq(luma, i as f32, q as f32);
        }
        colors[row * 16 + 13] = from_yiq(low, 0., 0.);
    }
    colors
}

/// One colour from Y, I and Q, as the game's `PaletteGen::FromYIQ`: a channel above 1 pulls
/// the chroma in until it fits, then the channels are clipped to 0-1 and scaled together to
/// keep the luma, and each is truncated from 256 steps.
fn from_yiq(y: f32, i: f32, q: f32) -> [u8; 3] {
    let (y, i, q) = (f64::from(y), f64::from(i), f64::from(q));
    let k = |v: f32| f64::from(v);
    // The parts of each channel's chroma the game keeps as floats, which it sums again when it
    // pulls the chroma in.
    let (ri, rq) = (f64::from((k(0.9563) * i) as f32), k(0.621) * q);
    let (gi, gq) = (f64::from((k(-0.2721) * i) as f32), k(-0.6474) * q);
    let (bi, bq) = (k(-1.107) * i, k(1.7046) * q);
    let chroma = [
        ri + f64::from(rq as f32),
        gi + f64::from(gq as f32),
        f64::from(bq as f32) + bi,
    ];
    let mut rgb = if y < 0. {
        [y; 3]
    } else {
        [(ri + y) + rq, (gi + y) + gq, (y + bi) + bq]
    };
    let chroma = if y < 0. { [0.; 3] } else { chroma };
    let over = k(1.00001);
    while rgb.iter().any(|&c| c > over) {
        // Each channel above 1 asks for the chroma that brings it to 1; the game applies the
        // product of what they ask.
        let ask: [f64; 3] = std::array::from_fn(|c| {
            if rgb[c] > 1. {
                (1. - y) / chroma[c]
            } else {
                1.
            }
        });
        let pull = ask[2] * (ask[1] * ask[0]);
        rgb = std::array::from_fn(|c| y + chroma[c] * pull);
    }
    let clipped = rgb.map(|c| c.clamp(0., 1.));
    let luma = (k(0.299) * clipped[0] + k(0.587) * clipped[1]) + k(0.114) * clipped[2];
    clipped.map(|c| {
        // Black where the luma cannot be kept: the game's integer conversion of the NaN or
        // infinity it divides into lands below zero.
        let steps = c * (y / luma) * 256.;
        if !steps.is_finite() || steps <= 0. {
            0
        } else {
            steps.min(255.) as u8
        }
    })
}

/// The game's colour table: for each of its 32 levels per channel, the colour [`colors`] gives
/// the nearest of the [`SOURCE`] palette's colours 0x00-0x?D, the first of equals winning. It
/// is read as the game reads it, nearest in red and green and blended in blue.
pub fn lut(colors: &[[u8; 3]; 64], name: String) -> Lut {
    let points: Vec<([f64; 3], usize)> = (0..14)
        .flat_map(|hue| (0..4).map(move |row| hue + 16 * row))
        .map(|index| (SOURCE[index].map(f64::from), index))
        .collect();
    let level = |k: usize| f64::from((k as f64 / (LUT_SIZE - 1) as f64 * 255.) as u8);
    let values = (0..LUT_SIZE.pow(3))
        .map(|n| {
            let at = [
                n % LUT_SIZE,
                n / LUT_SIZE % LUT_SIZE,
                n / LUT_SIZE / LUT_SIZE,
            ]
            .map(level);
            let distance = |p: &[f64; 3]| {
                (at[2] - p[2]).powi(2) + ((at[1] - p[1]).powi(2) + (at[0] - p[0]).powi(2))
            };
            let (_, index) = points.iter().fold((f64::INFINITY, 0), |best, (p, index)| {
                let d = distance(p);
                if d < best.0 {
                    (d, *index)
                } else {
                    best
                }
            });
            colors[index].map(|c| f32::from(c) / 255.)
        })
        .collect();
    Lut {
        name,
        size: LUT_SIZE,
        domain_min: [0.; 3],
        domain_max: [1.; 3],
        values,
        sampling: Sampling::NearestRedGreen,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// FNV-1a over `bytes`: enough to tell whether two runs of bytes are the same, without
    /// keeping the game's output in the repository.
    fn digest(bytes: impl IntoIterator<Item = u8>) -> u64 {
        bytes.into_iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
        })
    }

    /// Tint, I and Q as `f32` bits: the game's defaults, the ends of its sliders and random
    /// points between.
    const SETTINGS: [[u32; 3]; 16] = [
        [0x40a5dca9, 0x3fe00000, 0x3f800000],
        [0x00000000, 0x3f800000, 0x3f800000],
        [0x40400000, 0x3e800000, 0x40800000],
        [0x40c90fda, 0x40800000, 0x3e800000],
        [0x3f9df3b6, 0x40200000, 0x3fc00000],
        [0x40b00000, 0x3fe00000, 0x3f800000],
        [0x409ccccd, 0x3f800000, 0x40000000],
        [0x40c25127, 0x3ff38d39, 0x3e8e6235],
        [0x40b7299f, 0x40716cb0, 0x401bbc10],
        [0x4087069e, 0x3f1094a9, 0x4047f497],
        [0x3fbe7426, 0x3ebb29b5, 0x404d4e32],
        [0x400b2bae, 0x4025966a, 0x4023cbb7],
        [0x3f6ef30f, 0x3f6fc45c, 0x3f2dd622],
        [0x3dbc1d2a, 0x4004d204, 0x4077938d],
        [0x3ecfb23e, 0x4011dc75, 0x3fffa19d],
        [0x4071dce0, 0x3f155f26, 0x401af5ed],
    ];

    #[test]
    fn the_palettes_are_the_games_to_the_bit() {
        // The digest of what the game's own `MakePalette` gives for these settings, run in the
        // game (docs/research/swtg-crt/scripts). It holds every colour of all sixteen.
        let made = SETTINGS.iter().flat_map(|setting| {
            let [tint, i, q] = setting.map(f32::from_bits);
            colors(tint, i, q).into_iter().flatten()
        });
        assert_eq!(digest(made), 0x381f_eb76_f48e_f0bd);
    }

    #[test]
    fn the_lut_is_the_games_to_the_bit() {
        // The digest of the 32x32x32 table the running game uploads at its defaults, in this
        // LUT's red-fastest order.
        let [tint, i, q] = SETTINGS[0].map(f32::from_bits);
        let lut = lut(&colors(tint, i, q), String::new());
        lut.validate().unwrap();
        let made = lut
            .values
            .iter()
            .flat_map(|value| value.map(|c| (c * 255.).round() as u8));
        assert_eq!(digest(made), 0xaeee_7375_97c8_f8a6);
    }

    #[test]
    fn colours_keep_their_luma_and_stay_in_range() {
        let colors = colors(5.183186, 1.75, 1.);
        // Greys at each row's levels, black past the twelve hues, and the row below black.
        assert_eq!(colors[0x0D], [0, 0, 0]);
        assert_eq!(colors[0x0E], [0, 0, 0]);
        assert_eq!(colors[0x20], [255, 255, 255]);
        let [r, g, b] = colors[0x00];
        assert!(r == g && g == b);
        // Neither axis: every hue is the grey between its row's levels.
        let grey = super::colors(1., 0., 0.);
        for hue in 1..13 {
            let [r, g, b] = grey[0x10 + hue];
            assert!(r == g && g == b, "{hue}: {r} {g} {b}");
        }
    }
}

#[cfg(test)]
mod sampling_tests {
    use super::*;

    /// The game's NTSC pass, `ntsc.fx`'s `DoPost` at full strength, written out as its shader
    /// computes it: two texture coordinates into the 1024x32 table, point-sampled, and the
    /// two blue slices around the colour blended.
    fn as_the_game_reads(table: &Lut, colour: [f32; 3]) -> [f32; 3] {
        let res = 32_f32;
        let [r, g, b] = colour;
        let (b_lo, b_hi) = ((b * (res - 1.)).floor(), (b * (res - 1.)).ceil());
        let alpha = b * (res - 1.) - b_lo;
        let uv = |slice: f32| {
            [
                slice / res + r * (res - 1.) / (res * res) + 0.5 / (res * res),
                g * (res - 1.) / res + 0.5 / res,
            ]
        };
        // Point sampling picks the texel the coordinate falls in.
        let texel = |[u, v]: [f32; 2]| {
            let (x, y) = ((u * 1024.).floor() as usize, (v * 32.).floor() as usize);
            table.values[x % 32 + 32 * (y + 32 * (x / 32))]
        };
        let (low, high) = (texel(uv(b_lo)), texel(uv(b_hi)));
        std::array::from_fn(|c| low[c] + (high[c] - low[c]) * alpha)
    }

    #[test]
    fn the_table_is_read_as_the_game_reads_it() {
        let table = lut(&colors(5.183186, 1.75, 1.), String::new());
        assert_eq!(table.sampling, Sampling::NearestRedGreen);
        // Every 8-bit level along each axis, the others at a spread of levels.
        for level in 0..=255_u8 {
            for other in [0_u8, 37, 128, 200, 255] {
                for axis in 0..3 {
                    let mut colour = [f32::from(other) / 255.; 3];
                    colour[axis] = f32::from(level) / 255.;
                    let (ours, game) = (table.sample(colour), as_the_game_reads(&table, colour));
                    for c in 0..3 {
                        assert!(
                            (ours[c] - game[c]).abs() < 1e-5,
                            "{colour:?}: {ours:?} for {game:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn the_games_art_colours_come_out_as_its_palette() {
        let colors = colors(5.183186, 1.75, 1.);
        let mut image = image::RgbaImage::from_fn(14, 4, |x, y| {
            let [r, g, b] = SOURCE[x as usize + 16 * y as usize];
            image::Rgba([r, g, b, 255])
        });
        lut(&colors, String::new()).apply(&mut image);
        let missed: Vec<usize> = (0..56)
            .map(|n| n % 14 + 16 * (n / 14))
            .filter(|&index| {
                let p = image.get_pixel((index % 16) as u32, (index / 16) as u32);
                [p[0], p[1], p[2]] != colors[index]
            })
            .collect();
        // All but grey 0x2D (120): its blue falls between slices 14 and 15, and the game blends
        // in slice 15, whose cell is grey 0x00's (124).
        assert_eq!(missed, [0x2D]);
    }
}
