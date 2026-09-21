//! Bundled NES LUT gallery. Original PNG bytes are embedded for offline use.
//! See assets/nes-luts and THIRD_PARTY_NOTICES.md for provenance and credits.

use crate::workflow::Lut;
use anyhow::{ensure, Context, Result};

/// A bundled look. Decoding is deferred until it is previewed or selected.
pub struct NesLutEntry {
    pub name: &'static str,
    png: &'static [u8],
}

pub static ENTRIES: &[NesLutEntry] = &[
    NesLutEntry {
        name: "00 - NES SMPTE-2025",
        png: include_bytes!("../../../assets/nes-luts/00 - NES SMPTE-2025.png"),
    },
    NesLutEntry {
        name: "01 - NES NTSC SAT_x1",
        png: include_bytes!("../../../assets/nes-luts/01 - NES NTSC SAT_x1.png"),
    },
    NesLutEntry {
        name: "01 - NES NTSC SAT_x2",
        png: include_bytes!("../../../assets/nes-luts/01 - NES NTSC SAT_x2.png"),
    },
    NesLutEntry {
        name: "02 - NES Classic (FBX)",
        png: include_bytes!("../../../assets/nes-luts/02 - NES Classic (FBX).png"),
    },
    NesLutEntry {
        name: "03 - NES Composite Direct (FBX)",
        png: include_bytes!("../../../assets/nes-luts/03 - NES Composite Direct (FBX).png"),
    },
    NesLutEntry {
        name: "04 - NES PC-10",
        png: include_bytes!("../../../assets/nes-luts/04 - NES PC-10.png"),
    },
    NesLutEntry {
        name: "05 - NES PVM Style D93 (FBX)",
        png: include_bytes!("../../../assets/nes-luts/05 - NES PVM Style D93 (FBX).png"),
    },
    NesLutEntry {
        name: "06 - NES Smooth (FBX)",
        png: include_bytes!("../../../assets/nes-luts/06 - NES Smooth (FBX).png"),
    },
    NesLutEntry {
        name: "07 - NES Sony CXA",
        png: include_bytes!("../../../assets/nes-luts/07 - NES Sony CXA.png"),
    },
    NesLutEntry {
        name: "08 - NES Wavebeam",
        png: include_bytes!("../../../assets/nes-luts/08 - NES Wavebeam.png"),
    },
    NesLutEntry {
        name: "09 - NES ASQ_realityA",
        png: include_bytes!("../../../assets/nes-luts/09 - NES ASQ_realityA.png"),
    },
    NesLutEntry {
        name: "10 - NES ASQ_realityB",
        png: include_bytes!("../../../assets/nes-luts/10 - NES ASQ_realityB.png"),
    },
    NesLutEntry {
        name: "11 - NES BMF_final2",
        png: include_bytes!("../../../assets/nes-luts/11 - NES BMF_final2.png"),
    },
    NesLutEntry {
        name: "12 - NES BMF_final3",
        png: include_bytes!("../../../assets/nes-luts/12 - NES BMF_final3.png"),
    },
    NesLutEntry {
        name: "13 - NES FCEU-13-default_nitsuja",
        png: include_bytes!("../../../assets/nes-luts/13 - NES FCEU-13-default_nitsuja.png"),
    },
    NesLutEntry {
        name: "14 - NES FCEU-15-nitsuja_new",
        png: include_bytes!("../../../assets/nes-luts/14 - NES FCEU-15-nitsuja_new.png"),
    },
    NesLutEntry {
        name: "15 - NES FCEUX",
        png: include_bytes!("../../../assets/nes-luts/15 - NES FCEUX.png"),
    },
    NesLutEntry {
        name: "16 - NES nestopia_rgb",
        png: include_bytes!("../../../assets/nes-luts/16 - NES nestopia_rgb.png"),
    },
    NesLutEntry {
        name: "17 - NES nestopia_yuv",
        png: include_bytes!("../../../assets/nes-luts/17 - NES nestopia_yuv.png"),
    },
    NesLutEntry {
        name: "18 - NES RP2C03",
        png: include_bytes!("../../../assets/nes-luts/18 - NES RP2C03.png"),
    },
    NesLutEntry {
        name: "19 - NES SONY_CXA2025AS_US",
        png: include_bytes!("../../../assets/nes-luts/19 - NES SONY_CXA2025AS_US.png"),
    },
    NesLutEntry {
        name: "20 - NES Unsaturated-V6",
        png: include_bytes!("../../../assets/nes-luts/20 - NES Unsaturated-V6.png"),
    },
    NesLutEntry {
        name: "21 - NES YUV",
        png: include_bytes!("../../../assets/nes-luts/21 - NES YUV.png"),
    },
    NesLutEntry {
        name: "22 - NES RGB",
        png: include_bytes!("../../../assets/nes-luts/22 - NES RGB.png"),
    },
    NesLutEntry {
        name: "23 - NES Nintendulator NTSC",
        png: include_bytes!("../../../assets/nes-luts/23 - NES Nintendulator NTSC.png"),
    },
    NesLutEntry {
        name: "24 - NES PAL",
        png: include_bytes!("../../../assets/nes-luts/24 - NES PAL.png"),
    },
    NesLutEntry {
        name: "25 - NES Rockman 9 - 21 to 2C",
        png: include_bytes!("../../../assets/nes-luts/25 - NES Rockman 9 - 21 to 2C.png"),
    },
    NesLutEntry {
        name: "26 - NES Rockman 9",
        png: include_bytes!("../../../assets/nes-luts/26 - NES Rockman 9.png"),
    },
    NesLutEntry {
        name: "27 - NES Kizul's Definitive NES Palette",
        png: include_bytes!("../../../assets/nes-luts/27 - NES Kizul's Definitive NES Palette.png"),
    },
    NesLutEntry {
        name: "28 - NES NES Classic Edition",
        png: include_bytes!("../../../assets/nes-luts/28 - NES NES Classic Edition.png"),
    },
    NesLutEntry {
        name: "29 - NES 3DS VC",
        png: include_bytes!("../../../assets/nes-luts/29 - NES 3DS VC.png"),
    },
    NesLutEntry {
        name: "30 - NES HYBRID",
        png: include_bytes!("../../../assets/nes-luts/30 - NES HYBRID.png"),
    },
    NesLutEntry {
        name: "31 - NES NESCAP",
        png: include_bytes!("../../../assets/nes-luts/31 - NES NESCAP.png"),
    },
    NesLutEntry {
        name: "32 - NES NESCLASSIC",
        png: include_bytes!("../../../assets/nes-luts/32 - NES NESCLASSIC.png"),
    },
    NesLutEntry {
        name: "33 - NES RP2C04_0001",
        png: include_bytes!("../../../assets/nes-luts/33 - NES RP2C04_0001.png"),
    },
    NesLutEntry {
        name: "34 - NES RP2C04_0002",
        png: include_bytes!("../../../assets/nes-luts/34 - NES RP2C04_0002.png"),
    },
    NesLutEntry {
        name: "35 - NES RP2C04_0003",
        png: include_bytes!("../../../assets/nes-luts/35 - NES RP2C04_0003.png"),
    },
    NesLutEntry {
        name: "36 - NES RP2C04_0004",
        png: include_bytes!("../../../assets/nes-luts/36 - NES RP2C04_0004.png"),
    },
];

/// Load a bundled 64³ LUT using the same values/order as a 3D .cube file.
pub fn load(index: usize) -> Result<Lut> {
    let entry = ENTRIES
        .get(index)
        .context("Unknown NES LUT gallery entry")?;
    decode_strip(entry.name, entry.png)
}

fn decode_strip(name: &str, png: &[u8]) -> Result<Lut> {
    let image = image::load_from_memory_with_format(png, image::ImageFormat::Png)
        .context("Could not decode bundled NES LUT")?
        .to_rgb8();
    ensure!(
        image.dimensions() == (4096, 64),
        "NES LUT must be a 4096 × 64 strip"
    );
    let size = 64;
    let mut values = Vec::with_capacity(size * size * size);
    // MAME hlsl/color.fx apply_lut: x = r + 64*b, y = g.
    // https://github.com/mamedev/mame/blob/master/hlsl/color.fx
    // Reorder rows into .cube order (red fastest, then green, then blue).
    // LUT pixels are numeric data; do not apply gamma/color-profile transforms.
    for b in 0..size {
        for g in 0..size {
            for r in 0..size {
                values.push(
                    image
                        .get_pixel((b * size + r) as u32, g as u32)
                        .0
                        .map(|v| v as f32 / 255.),
                );
            }
        }
    }
    let lut = Lut {
        name: name.to_owned(),
        size,
        domain_min: [0.; 3],
        domain_max: [1.; 3],
        values,
    };
    lut.validate()?;
    Ok(lut)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_bundled_lut_loads_and_has_a_unique_name() {
        let mut names = std::collections::HashSet::new();
        assert_eq!(ENTRIES.len(), 38);
        for (index, entry) in ENTRIES.iter().enumerate() {
            assert!(names.insert(entry.name));
            let lut = load(index).unwrap();
            assert_eq!(lut.name, entry.name);
            assert_eq!(lut.size, 64);
            lut.validate().unwrap();
        }
        assert!(load(ENTRIES.len()).is_err());
    }

    #[test]
    fn strip_axes_match_mame_and_preserve_alpha() {
        // Independent, asymmetric known samples from the original SMPTE PNG.
        // These detect RGB swaps, flipped green, and incorrect strip ordering.
        let lut = load(0).unwrap();
        for (rgb, expected) in [
            ([0, 0, 0], [17, 18, 18]),
            ([63, 0, 0], [230, 0, 0]),
            ([0, 63, 0], [0, 219, 0]),
            ([0, 0, 63], [12, 3, 224]),
            ([63, 63, 63], [249, 251, 252]),
            ([30, 20, 10], [123, 72, 0]),
        ] {
            let [r, g, b] = rgb;
            assert_eq!(
                lut.values[r + 64 * g + 4096 * b].map(|v| (v * 255.).round() as u8),
                expected
            );
        }
        let mut image = image::RgbaImage::from_pixel(1, 1, image::Rgba([255, 0, 0, 77]));
        lut.apply(&mut image);
        assert_eq!(image.get_pixel(0, 0).0, [230, 0, 0, 77]);
    }
}
