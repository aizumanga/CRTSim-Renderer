//! Source-space edits shared by stills, playback and video exports.
use anyhow::{bail, ensure, Context, Result};
use image::{Rgba, RgbaImage};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SourceEdit {
    /// Fractions removed from left, top, right, bottom.
    pub crop: [f32; 4],
    pub rotation: f32,
    pub zoom: f32,
    /// Translation in canvas widths/heights, positive right/down.
    pub position: [f32; 2],
    pub background: [u8; 3],
    pub checkerboard: bool,
}
impl Default for SourceEdit {
    fn default() -> Self {
        Self {
            crop: [0.; 4],
            rotation: 0.,
            zoom: 1.,
            position: [0.; 2],
            background: [0; 3],
            checkerboard: false,
        }
    }
}
impl SourceEdit {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.crop
                .iter()
                .all(|v| v.is_finite() && (0.0..1.0).contains(v)),
            "Crop must be between 0 and 1"
        );
        ensure!(
            self.crop[0] + self.crop[2] < 0.99 && self.crop[1] + self.crop[3] < 0.99,
            "Crop must retain at least 1% of each dimension"
        );
        ensure!(
            self.rotation.is_finite() && self.rotation.abs() <= 360.,
            "Rotation must be within ±360 degrees"
        );
        ensure!(
            self.zoom.is_finite() && (0.05..=20.).contains(&self.zoom),
            "Source zoom must be 0.05–20"
        );
        ensure!(
            self.position.iter().all(|v| v.is_finite() && v.abs() <= 4.),
            "Source position must be within ±4 canvas sizes"
        );
        Ok(())
    }
    pub fn size(&self, (w, h): (u32, u32)) -> (u32, u32) {
        (
            (w as f32 * (1. - self.crop[0] - self.crop[2]))
                .round()
                .max(1.) as u32,
            (h as f32 * (1. - self.crop[1] - self.crop[3]))
                .round()
                .max(1.) as u32,
        )
    }
    pub fn background_at(&self, x: u32, y: u32) -> [u8; 3] {
        if self.checkerboard {
            [if (x / 16 + y / 16).is_multiple_of(2) {
                192
            } else {
                128
            }; 3]
        } else {
            self.background
        }
    }
    pub fn prepare(&self, input: &RgbaImage, size: (u32, u32), nearest: bool) -> RgbaImage {
        let (cw, ch) = self.size(input.dimensions());
        let left = input.width() as f32 * self.crop[0];
        let top = input.height() as f32 * self.crop[1];
        let (sin, cos) = self.rotation.to_radians().sin_cos();
        RgbaImage::from_fn(size.0, size.1, |x, y| {
            let px =
                ((x as f32 + 0.5) / size.0 as f32 - 0.5 - self.position[0]) * cw as f32 / self.zoom;
            let py =
                ((y as f32 + 0.5) / size.1 as f32 - 0.5 - self.position[1]) * ch as f32 / self.zoom;
            let sx = cos * px + sin * py + cw as f32 * 0.5;
            let sy = -sin * px + cos * py + ch as f32 * 0.5;
            let bg = self.background_at(x, y);
            let sample = |ix: i64, iy: i64| -> [f32; 3] {
                let fx = ix as f32 + 0.5;
                let fy = iy as f32 + 0.5;
                if fx < left
                    || fy < top
                    || fx >= left + cw as f32
                    || fy >= top + ch as f32
                    || ix < 0
                    || iy < 0
                    || ix >= input.width() as i64
                    || iy >= input.height() as i64
                {
                    return bg.map(f32::from);
                }
                let p = input.get_pixel(ix as u32, iy as u32);
                let a = p[3] as f32 / 255.;
                std::array::from_fn(|i| p[i] as f32 * a + bg[i] as f32 * (1. - a))
            };
            let color = if nearest {
                sample((left + sx).floor() as i64, (top + sy).floor() as i64)
            } else {
                let fx = left + sx - 0.5;
                let fy = top + sy - 0.5;
                let ix = fx.floor() as i64;
                let iy = fy.floor() as i64;
                let dx = fx - fx.floor();
                let dy = fy - fy.floor();
                let a = sample(ix, iy);
                let b = sample(ix + 1, iy);
                let c = sample(ix, iy + 1);
                let d = sample(ix + 1, iy + 1);
                std::array::from_fn(|i| {
                    (a[i] * (1. - dx) + b[i] * dx) * (1. - dy) + (c[i] * (1. - dx) + d[i] * dx) * dy
                })
            };
            Rgba([
                color[0].round() as u8,
                color[1].round() as u8,
                color[2].round() as u8,
                255,
            ])
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lut {
    pub name: String,
    pub size: usize,
    pub domain_min: [f32; 3],
    pub domain_max: [f32; 3],
    pub values: Vec<[f32; 3]>,
}
impl Lut {
    pub fn parse_cube(name: String, text: &str) -> Result<Self> {
        ensure!(text.len() <= 16 * 1024 * 1024, "LUT exceeds 16 MB");
        let mut lut = Self {
            name,
            size: 0,
            domain_min: [0.; 3],
            domain_max: [1.; 3],
            values: vec![],
        };
        for line in text.lines() {
            let line = line.split('#').next().unwrap_or("").trim();
            if line.is_empty() || line.starts_with("TITLE ") {
                continue;
            }
            let words: Vec<_> = line.split_whitespace().collect();
            match words[0] {
                "LUT_3D_SIZE" => {
                    ensure!(
                        words.len() == 2 && lut.size == 0,
                        "Invalid or duplicate LUT_3D_SIZE"
                    );
                    lut.size = words[1].parse()?;
                    ensure!((2..=65).contains(&lut.size), "3D LUT size must be 2–65");
                }
                "DOMAIN_MIN" | "DOMAIN_MAX" => {
                    ensure!(words.len() == 4, "Invalid LUT domain");
                    let v = [words[1].parse()?, words[2].parse()?, words[3].parse()?];
                    if words[0] == "DOMAIN_MIN" {
                        lut.domain_min = v;
                    } else {
                        lut.domain_max = v;
                    }
                }
                "LUT_1D_SIZE" => bail!("Use a 3D .cube LUT; 1D LUTs are not supported"),
                _ => {
                    ensure!(
                        words.len() == 3 && lut.values.len() < 65 * 65 * 65,
                        "Invalid LUT row"
                    );
                    lut.values.push([
                        words[0].parse().context("Unknown .cube directive")?,
                        words[1].parse()?,
                        words[2].parse()?,
                    ]);
                }
            }
        }
        lut.validate()?;
        Ok(lut)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            (2..=65).contains(&self.size) && self.values.len() == self.size.pow(3),
            "LUT table size does not match LUT_3D_SIZE"
        );
        ensure!(self.name.len() <= 1024, "LUT name too long");
        for i in 0..3 {
            ensure!(
                self.domain_min[i].is_finite()
                    && self.domain_max[i].is_finite()
                    && self.domain_max[i] > self.domain_min[i],
                "Invalid LUT domain"
            );
        }
        ensure!(
            self.values
                .iter()
                .flatten()
                .all(|v| v.is_finite() && v.abs() <= 100.),
            "Invalid LUT value"
        );
        Ok(())
    }
    pub fn apply(&self, image: &mut RgbaImage) {
        for p in image.pixels_mut() {
            let xyz: [f32; 3] = std::array::from_fn(|i| {
                ((p[i] as f32 / 255. - self.domain_min[i])
                    / (self.domain_max[i] - self.domain_min[i]))
                    .clamp(0., 1.)
                    * (self.size - 1) as f32
            });
            let lo = xyz.map(|v| v.floor() as usize);
            let hi = lo.map(|v| (v + 1).min(self.size - 1));
            let f: [f32; 3] = std::array::from_fn(|i| xyz[i] - lo[i] as f32);
            let mut out = [0.; 3];
            for z in 0..2 {
                for y in 0..2 {
                    for x in 0..2 {
                        let index = (if x == 0 { lo[0] } else { hi[0] })
                            + self.size * (if y == 0 { lo[1] } else { hi[1] })
                            + self.size * self.size * (if z == 0 { lo[2] } else { hi[2] });
                        let weight = (if x == 0 { 1. - f[0] } else { f[0] })
                            * (if y == 0 { 1. - f[1] } else { f[1] })
                            * (if z == 0 { 1. - f[2] } else { f[2] });
                        for (i, v) in out.iter_mut().enumerate() {
                            *v += self.values[index][i] * weight;
                        }
                    }
                }
            }
            for i in 0..3 {
                p[i] = (out[i].clamp(0., 1.) * 255.).round() as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cube_identity_and_invalid_tables() {
        let text = "LUT_3D_SIZE 2\n0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1";
        let lut = Lut::parse_cube("identity".into(), text).unwrap();
        let mut im = crate::config::test_card();
        let original = im.clone();
        lut.apply(&mut im);
        assert_eq!(im, original);
        assert!(Lut::parse_cube("bad".into(), "LUT_3D_SIZE 65\n0 0 0").is_err());
        assert!(Lut::parse_cube("bad".into(), &text.replace("1 1 1", "NaN 1 1")).is_err());
    }
    #[test]
    fn edits_preserve_identity_and_composite_alpha() {
        let im = crate::config::test_card();
        let mut edit = SourceEdit::default();
        assert_eq!(edit.prepare(&im, im.dimensions(), true), im);
        edit.background = [255; 3];
        let transparent = RgbaImage::new(2, 2);
        assert_eq!(
            edit.prepare(&transparent, (2, 2), false).get_pixel(0, 0).0,
            [255; 4]
        );
        edit.crop = [0.6, 0., 0.5, 0.];
        assert!(edit.validate().is_err());
    }
    #[test]
    fn crop_pan_rotation_and_lut_interpolation_have_expected_pixels() {
        let im = RgbaImage::from_fn(4, 4, |x, y| Rgba([(x + 4 * y) as u8, 0, 0, 255]));
        let crop = SourceEdit {
            crop: [0.25, 0.25, 0.25, 0.25],
            ..SourceEdit::default()
        };
        let cropped = crop.prepare(&im, (2, 2), true);
        assert_eq!(
            cropped.pixels().map(|p| p[0]).collect::<Vec<_>>(),
            [5, 6, 9, 10]
        );
        let rotate = SourceEdit {
            rotation: 90.,
            ..SourceEdit::default()
        };
        let rotated = rotate.prepare(&im, (4, 4), true);
        assert_eq!(rotated.get_pixel(0, 0)[0], 12);
        assert_eq!(rotated.get_pixel(3, 0)[0], 0);
        let pan = SourceEdit {
            position: [0.5, 0.],
            background: [250; 3],
            ..SourceEdit::default()
        };
        let moved = pan.prepare(&im, (4, 4), true);
        assert_eq!(moved.get_pixel(0, 0)[0], 250);
        assert_eq!(moved.get_pixel(2, 0)[0], 0);
        let lut = Lut::parse_cube(
            "swap red/blue".into(),
            "LUT_3D_SIZE 2\n0 0 0\n0 0 1\n0 1 0\n0 1 1\n1 0 0\n1 0 1\n1 1 0\n1 1 1",
        )
        .unwrap();
        let mut color = RgbaImage::from_pixel(1, 1, Rgba([32, 96, 192, 255]));
        lut.apply(&mut color);
        assert_eq!(color.get_pixel(0, 0).0, [192, 96, 32, 255]);
    }
}
