//! Colour management. The CRT works in sRGB, as the sets it simulates were close to: a picture
//! is read into sRGB from the colour space its file says it is in, and a still can be written
//! out in another space, with that space's profile embedded so other programs read it right.
//!
//! A file says what its colours are with an ICC profile (PNG's iCCP, JPEG's APP2 segments,
//! WebP's ICCP chunk), with PNG's sRGB chunk, with PNG's cICP code points, or with PNG's
//! older gAMA and cHRM. A file that says nothing is taken to be sRGB, as browsers take it.
//! A profile that converts every 8-bit value to within a step of itself is sRGB in effect,
//! and is left alone, so a palette's exact values survive a program tagging its files with its
//! own copy of sRGB.

use anyhow::{ensure, Context, Result};
use image::RgbaImage;
use moxcms::Transform8BitExecutor;
use moxcms::{
    ColorPrimaries, ColorProfile, DataColorSpace, Layout, ProfileText, RenderingIntent,
    ToneReprCurve, TransformOptions, XyY,
};
use std::io::Cursor;
use std::sync::{Arc, OnceLock};

/// What a file says about its colours.
#[derive(Debug, PartialEq)]
pub(crate) enum Tag {
    /// Nothing, or that it is sRGB.
    Srgb,
    /// An ICC profile.
    Icc(Vec<u8>),
    /// PNG's gAMA, the power its values were encoded with, and cHRM's white point and red,
    /// green and blue primaries as x, y pairs, when it has one.
    Gamma {
        gamma: f64,
        chromaticities: Option<[(f64, f64); 4]>,
    },
    /// PNG's cICP: H.273's colour primaries and transfer characteristics.
    Cicp { primaries: u8, transfer: u8 },
}

/// What `bytes`, a PNG, JPEG or WebP file, says about its colours. PNG's chunks rank as its
/// specification ranks them: cICP, then iCCP, then sRGB, then gAMA and cHRM.
pub(crate) fn tag(bytes: &[u8]) -> Tag {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        png_tag(bytes).unwrap_or(Tag::Srgb)
    } else if bytes.starts_with(&[0xFF, 0xD8]) {
        let mut decoder = jpeg_decoder::Decoder::new(Cursor::new(bytes));
        match decoder.read_info() {
            Ok(()) => decoder.icc_profile().map_or(Tag::Srgb, Tag::Icc),
            Err(_) => Tag::Srgb,
        }
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        webp_icc(bytes).map_or(Tag::Srgb, Tag::Icc)
    } else {
        Tag::Srgb
    }
}

fn png_tag(bytes: &[u8]) -> Option<Tag> {
    let reader = png::Decoder::new(Cursor::new(bytes)).read_info().ok()?;
    let info = reader.info();
    if let Some(cicp) = &info.coding_independent_code_points {
        return Some(Tag::Cicp {
            primaries: cicp.color_primaries,
            transfer: cicp.transfer_function,
        });
    }
    if let Some(icc) = &info.icc_profile {
        return Some(Tag::Icc(icc.to_vec()));
    }
    if info.srgb.is_some() {
        return Some(Tag::Srgb);
    }
    let gamma = info.gama_chunk?.into_value();
    let chromaticities = info
        .chrm_chunk
        .map(|c| [c.white, c.red, c.green, c.blue].map(|(x, y)| (x.into_value(), y.into_value())));
    Some(Tag::Gamma {
        gamma: f64::from(gamma),
        chromaticities: chromaticities
            .map(|c| c.map(|(x, y): (f32, f32)| (f64::from(x), f64::from(y)))),
    })
}

/// The ICC profile in a WebP file's ICCP chunk.
fn webp_icc(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let name = &bytes[at..at + 4];
        let size = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().ok()?) as usize;
        let data = bytes.get(at + 8..at + 8 + size)?;
        if name == b"ICCP" {
            return Some(data.to_vec());
        }
        // Chunks are padded to an even size.
        at += 8 + size + (size & 1);
    }
    None
}

/// A colour space a picture is read from.
pub(crate) struct Source {
    profile: ColorProfile,
    pub name: String,
}

/// What the colour space `tag` names, read: `None` where it is sRGB in effect, and an error
/// saying why where it cannot be used.
pub(crate) fn source(tag: Tag) -> Result<Option<Source>> {
    let profile = match tag {
        Tag::Srgb => return Ok(None),
        Tag::Icc(bytes) => {
            ColorProfile::new_from_slice(&bytes).context("its ICC profile is malformed")?
        }
        Tag::Gamma {
            gamma,
            chromaticities,
        } => {
            // Many programs write a gAMA of 1/2.2 to say sRGB, and nothing else; reading it
            // as a pure power would darken every shadow of the files they save.
            let srgb_primaries = chromaticities.is_none_or(|c| {
                let srgb = [(0.3127, 0.329), (0.64, 0.33), (0.3, 0.6), (0.15, 0.06)];
                c.iter()
                    .zip(srgb)
                    .all(|(a, b)| (a.0 - b.0).abs() < 0.005 && (a.1 - b.1).abs() < 0.005)
            });
            if srgb_primaries && (gamma - 1. / 2.2).abs() < 0.01 {
                return Ok(None);
            }
            ensure!(
                gamma > 0. && gamma.is_finite(),
                "its gAMA chunk is malformed"
            );
            let mut profile = ColorProfile::new_srgb();
            if let Some([white, red, green, blue]) = chromaticities {
                let xy = |(x, y): (f64, f64)| moxcms::Chromaticity::new(x as f32, y as f32);
                profile.update_rgb_colorimetry(
                    XyY {
                        x: white.0,
                        y: white.1,
                        yb: 1.,
                    },
                    ColorPrimaries {
                        red: xy(red),
                        green: xy(green),
                        blue: xy(blue),
                    },
                );
            }
            // gAMA holds the exponent values were encoded with; decoding raises them to its
            // reciprocal.
            let curve = ToneReprCurve::Parametric(vec![(1. / gamma) as f32]);
            profile.red_trc = Some(curve.clone());
            profile.green_trc = Some(curve.clone());
            profile.blue_trc = Some(curve);
            profile.description = None;
            profile.cicp = None;
            profile
        }
        Tag::Cicp {
            primaries,
            transfer,
        } => {
            let cicp = moxcms::CicpProfile {
                color_primaries: primaries
                    .try_into()
                    .context("its cICP primaries are unknown")?,
                transfer_characteristics: transfer
                    .try_into()
                    .context("its cICP transfer is unknown")?,
                matrix_coefficients: moxcms::MatrixCoefficients::Identity,
                full_range: true,
            };
            ColorProfile::new_from_cicp(cicp)
        }
    };
    ensure!(
        matches!(
            profile.color_space,
            DataColorSpace::Rgb | DataColorSpace::Gray | DataColorSpace::Cmyk
        ),
        "its profile is for {:?} colours, not RGB, grey or CMYK",
        profile.color_space
    );
    let name = name(&profile);
    let source = Source { profile, name };
    Ok((!source.is_srgb()).then_some(source))
}

/// What a profile calls itself.
fn name(profile: &ColorProfile) -> String {
    let text = match &profile.description {
        Some(ProfileText::PlainString(text)) => text.clone(),
        Some(ProfileText::Localizable(strings)) => strings
            .iter()
            .find(|s| s.language == "en")
            .or(strings.first())
            .map(|s| s.value.clone())
            .unwrap_or_default(),
        Some(ProfileText::Description(description)) => description.ascii_string.clone(),
        None => String::new(),
    };
    let text = text.trim_matches(|c: char| c.is_whitespace() || c == '\0');
    if text.is_empty() {
        "an unnamed profile".into()
    } else {
        text.into()
    }
}

/// Perceptual, interpolating a profile's tables between four points rather than eight, as
/// lcms does, with finer weights. Checked against lcms in docs/VALIDATION.md: RGB profiles
/// within two 8-bit steps everywhere, a CMYK one wherever its colour lies inside sRGB.
fn options() -> TransformOptions {
    TransformOptions {
        rendering_intent: RenderingIntent::Perceptual,
        interpolation_method: moxcms::InterpolationMethod::Tetrahedral,
        barycentric_weight_scale: moxcms::BarycentricWeightScale::High,
        ..Default::default()
    }
}

impl Source {
    /// Whether it converts every 8-bit value to within a step of itself.
    fn is_srgb(&self) -> bool {
        let srgb = ColorProfile::new_srgb();
        match self.profile.color_space {
            DataColorSpace::Rgb => {
                let Ok(transform) =
                    self.profile
                        .create_transform_8bit(Layout::Rgb, &srgb, Layout::Rgb, options())
                else {
                    return false;
                };
                let steps: Vec<u8> = (0..=255).step_by(15).collect();
                let mut probe = Vec::new();
                for &r in &steps {
                    for &g in &steps {
                        for &b in &steps {
                            probe.extend([r, g, b]);
                        }
                    }
                }
                probe.extend((0..=255).flat_map(|v| [v, v, v]));
                let mut out = vec![0; probe.len()];
                transform.transform(&probe, &mut out).is_ok()
                    && probe.iter().zip(&out).all(|(a, b)| a.abs_diff(*b) <= 1)
            }
            DataColorSpace::Gray => {
                let Ok(transform) =
                    self.profile
                        .create_transform_8bit(Layout::Gray, &srgb, Layout::Rgb, options())
                else {
                    return false;
                };
                let probe: Vec<u8> = (0..=255).collect();
                let mut out = vec![0; probe.len() * 3];
                transform.transform(&probe, &mut out).is_ok()
                    && probe
                        .iter()
                        .zip(out.chunks_exact(3))
                        .all(|(v, rgb)| rgb.iter().all(|c| c.abs_diff(*v) <= 1))
            }
            _ => false,
        }
    }

    pub(crate) fn is_cmyk(&self) -> bool {
        self.profile.color_space == DataColorSpace::Cmyk
    }

    pub(crate) fn is_gray(&self) -> bool {
        self.profile.color_space == DataColorSpace::Gray
    }

    /// Converts `pixels`, laid out as `pixels` says, into sRGB RGBA at 8 bits.
    pub(crate) fn to_srgb_8(&self, kind: Pixels, pixels: &[u8]) -> Result<Vec<u8>> {
        let transform = self
            .profile
            .create_transform_8bit(
                kind.layout(),
                &ColorProfile::new_srgb(),
                Layout::Rgba,
                options(),
            )
            .context("it cannot convert to sRGB")?;
        let mut out = vec![0; pixels.len() / kind.channels() * 4];
        transform
            .transform(pixels, &mut out)
            .context("it cannot convert to sRGB")?;
        kind.keep_alpha(pixels, &mut out, 255);
        Ok(out)
    }

    /// Converts `pixels`, RGBA or grey and alpha at 16 bits, into sRGB RGBA at 16 bits.
    pub(crate) fn to_srgb_16(&self, kind: Pixels, pixels: &[u16]) -> Result<Vec<u16>> {
        let transform = self
            .profile
            .create_transform_16bit(
                kind.layout(),
                &ColorProfile::new_srgb(),
                Layout::Rgba,
                options(),
            )
            .context("it cannot convert to sRGB")?;
        let mut out = vec![0; pixels.len() / kind.channels() * 4];
        transform
            .transform(pixels, &mut out)
            .context("it cannot convert to sRGB")?;
        kind.keep_alpha(pixels, &mut out, u16::MAX);
        Ok(out)
    }
}

/// How the pixels a profile converts are laid out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Pixels {
    Rgba,
    GrayAlpha,
    /// Ink amounts, 0 for none, with no alpha.
    Cmyk,
}

impl Pixels {
    fn layout(self) -> Layout {
        match self {
            // moxcms reads four 8-bit inks as it reads RGBA.
            Self::Rgba | Self::Cmyk => Layout::Rgba,
            Self::GrayAlpha => Layout::GrayAlpha,
        }
    }

    fn channels(self) -> usize {
        match self {
            Self::Rgba | Self::Cmyk => 4,
            Self::GrayAlpha => 2,
        }
    }

    /// Puts the input's alpha on each converted pixel, as it was: only colour converts.
    fn keep_alpha<T: Copy>(self, input: &[T], out: &mut [T], opaque: T) {
        for (o, i) in out
            .chunks_exact_mut(4)
            .zip(input.chunks_exact(self.channels()))
        {
            o[3] = match self {
                Self::Rgba => i[3],
                Self::GrayAlpha => i[1],
                Self::Cmyk => opaque,
            };
        }
    }
}

/// How one file's pictures are read into sRGB, decided once from what the file says: every
/// frame of an animation is read alike.
pub struct Reading {
    pub(crate) source: Option<Source>,
    note: Option<String>,
    /// The conversion of RGBA frames, made with the first and kept for the rest.
    frames: OnceLock<Option<Arc<Transform8BitExecutor>>>,
}

impl Reading {
    fn of_tag(tag: Tag, profiles: Profiles) -> Self {
        match source(tag) {
            Err(why) => Self::unusable(why),
            Ok(None) => Self::as_srgb(None),
            Ok(Some(source)) if profiles == Profiles::Ignore => Self::as_srgb(Some(format!(
                "Its colour profile, {}, is ignored: its colours are read as sRGB.",
                source.name
            ))),
            Ok(Some(source)) => Self {
                note: Some(format!(
                    "Its colours are converted to sRGB from its profile, {}.",
                    source.name
                )),
                source: Some(source),
                frames: OnceLock::new(),
            },
        }
    }

    fn as_srgb(note: Option<String>) -> Self {
        Self {
            source: None,
            note,
            frames: OnceLock::new(),
        }
    }

    /// Reads as sRGB, saying why its profile is not used.
    pub(crate) fn unusable(why: anyhow::Error) -> Self {
        Self::as_srgb(Some(format!(
            "Its colour profile cannot be used ({why:#}), so its colours are read as sRGB."
        )))
    }

    /// How a PNG, JPEG or WebP file, all of `bytes`, is read, as `profiles` asks.
    pub fn of_file(bytes: &[u8], profiles: Profiles) -> Self {
        Self::of_tag(tag(bytes), profiles)
    }

    /// How pictures carrying `icc`, or no profile, are read, as `profiles` asks.
    pub fn of_icc(icc: Option<Vec<u8>>, profiles: Profiles) -> Self {
        Self::of_tag(icc.map_or(Tag::Srgb, Tag::Icc), profiles)
    }

    /// What became of the file's profile, in a sentence, when it has one other than sRGB.
    pub fn note(&self) -> Option<&str> {
        self.note.as_deref()
    }

    /// Converts a decoded RGBA picture into sRGB.
    pub fn apply(&self, image: &mut RgbaImage) -> Result<()> {
        let Some(source) = &self.source else {
            return Ok(());
        };
        if source.is_gray() {
            // A grey picture decodes with equal channels; its grey is any one of them.
            let gray: Vec<u8> = image.pixels().flat_map(|p| [p[0], p[3]]).collect();
            let converted = source.to_srgb_8(Pixels::GrayAlpha, &gray)?;
            image.copy_from_slice(&converted);
            return Ok(());
        }
        ensure!(!source.is_cmyk(), "it is for CMYK, and the picture is not");
        let transform = self
            .frames
            .get_or_init(|| {
                source
                    .profile
                    .create_transform_8bit(
                        Layout::Rgba,
                        &ColorProfile::new_srgb(),
                        Layout::Rgba,
                        options(),
                    )
                    .ok()
            })
            .as_ref()
            .context("it cannot convert to sRGB")?;
        let mut converted = vec![0; image.as_raw().len()];
        transform
            .transform(image.as_raw(), &mut converted)
            .context("it cannot convert to sRGB")?;
        Pixels::Rgba.keep_alpha(image.as_raw(), &mut converted, 255);
        image.copy_from_slice(&converted);
        Ok(())
    }
}

/// A colour space a still is written in, with its profile embedded.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Space {
    /// The CRT's own, written as rendered, at 8 bits, with PNG's sRGB chunk.
    #[default]
    Srgb,
    DisplayP3,
    AdobeRgb,
    Rec2020,
    ProPhotoRgb,
    /// An RGB ICC profile from a file, such as a display's or a print service's.
    Icc(Arc<IccProfile>),
}

/// An RGB ICC profile from a file, checked to convert from sRGB.
#[derive(Debug, PartialEq)]
pub struct IccProfile {
    /// What it calls itself.
    pub name: String,
    pub bytes: Vec<u8>,
}

impl Space {
    /// The spaces built in, by the names the command line knows them by.
    pub const NAMED: [(&'static str, Space); 5] = [
        ("srgb", Space::Srgb),
        ("display-p3", Space::DisplayP3),
        ("adobe-rgb", Space::AdobeRgb),
        ("rec2020", Space::Rec2020),
        ("prophoto-rgb", Space::ProPhotoRgb),
    ];

    /// The space built in by this name, or the profile in the file it names.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn parse(text: &str) -> Result<Self> {
        if let Some((_, space)) = Self::NAMED.iter().find(|(name, _)| *name == text) {
            return Ok(space.clone());
        }
        let path = std::path::Path::new(text);
        ensure!(
            path.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("icc") || e.eq_ignore_ascii_case("icm")),
            "A colour space is one of {}, or an .icc or .icm profile",
            Self::NAMED.map(|(name, _)| name).join(", ")
        );
        let bytes = std::fs::read(path).with_context(|| format!("Cannot read {text}"))?;
        Self::from_icc(bytes)
    }

    /// The name of a built-in space on the command line; none for a profile from a file.
    pub fn slug(&self) -> Option<&'static str> {
        Self::NAMED
            .iter()
            .find(|(_, space)| space == self)
            .map(|(name, _)| *name)
    }

    /// An ICC profile's space, if it is an RGB profile that converts from sRGB.
    pub fn from_icc(bytes: Vec<u8>) -> Result<Self> {
        ensure!(bytes.len() <= 16 * 1024 * 1024, "The profile exceeds 16 MB");
        let profile =
            ColorProfile::new_from_slice(&bytes).context("The ICC profile is malformed")?;
        ensure!(
            profile.color_space == DataColorSpace::Rgb,
            "A PNG holds RGB: choose an RGB profile, not a {:?} one",
            profile.color_space
        );
        ColorProfile::new_srgb()
            .create_transform_16bit(Layout::Rgba, &profile, Layout::Rgba, options())
            .context("The profile cannot convert from sRGB")?;
        Ok(Self::Icc(Arc::new(IccProfile {
            name: name(&profile),
            bytes,
        })))
    }

    /// What it is called.
    pub fn name(&self) -> String {
        match self {
            Self::Srgb => "sRGB".into(),
            Self::DisplayP3 => "Display P3".into(),
            Self::AdobeRgb => "Adobe RGB (1998)".into(),
            Self::Rec2020 => "Rec. 2020".into(),
            Self::ProPhotoRgb => "ProPhoto RGB".into(),
            Self::Icc(profile) => profile.name.clone(),
        }
    }

    fn profile(&self) -> Result<ColorProfile> {
        Ok(match self {
            Self::Srgb => ColorProfile::new_srgb(),
            Self::DisplayP3 => ColorProfile::new_display_p3(),
            Self::AdobeRgb => ColorProfile::new_adobe_rgb(),
            Self::Rec2020 => ColorProfile::new_bt2020(),
            Self::ProPhotoRgb => ColorProfile::new_pro_photo_rgb(),
            Self::Icc(profile) => ColorProfile::new_from_slice(&profile.bytes)
                .context("The ICC profile is malformed")?,
        })
    }

    /// `image`, rendered in sRGB, as a PNG in this space. sRGB stays as rendered, at 8 bits,
    /// with PNG's sRGB chunk. Another space is converted at 16 bits, so the steps between
    /// colours stay finer than sRGB's in a wider gamut, and its profile is embedded.
    pub fn png(&self, image: &RgbaImage) -> Result<Vec<u8>> {
        let (width, height) = image.dimensions();
        let mut bytes = Vec::new();
        if *self == Self::Srgb {
            let mut encoder = png::Encoder::new(&mut bytes, width, height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
            // As the image crate writes a PNG, which the app's exports were written with.
            encoder.set_filter(png::FilterType::Sub);
            encoder.set_adaptive_filter(png::AdaptiveFilterType::Adaptive);
            let mut writer = encoder.write_header()?;
            writer.write_image_data(image.as_raw())?;
            writer.finish()?;
            return Ok(bytes);
        }
        let destination = self.profile()?;
        let icc = match self {
            Self::Icc(profile) => profile.bytes.clone(),
            _ => destination
                .encode()
                .context("Cannot write the colour space's profile")?,
        };
        let transform = ColorProfile::new_srgb()
            .create_transform_16bit(Layout::Rgba, &destination, Layout::Rgba, options())
            .context("Cannot convert to the colour space")?;
        let wide: Vec<u16> = image.as_raw().iter().map(|&v| u16::from(v) * 257).collect();
        let mut converted = vec![0; wide.len()];
        transform
            .transform(&wide, &mut converted)
            .context("Cannot convert to the colour space")?;
        let mut data = Vec::with_capacity(converted.len() * 2);
        for (i, value) in converted.iter().enumerate() {
            // The renderer's output is opaque; alpha is carried over rather than converted.
            let value = if i % 4 == 3 { wide[i] } else { *value };
            data.extend(value.to_be_bytes());
        }
        let mut info = png::Info::with_size(width, height);
        info.color_type = png::ColorType::Rgba;
        info.bit_depth = png::BitDepth::Sixteen;
        info.icc_profile = Some(icc.into());
        let mut encoder = png::Encoder::with_info(&mut bytes, info)?;
        encoder.set_filter(png::FilterType::Sub);
        encoder.set_adaptive_filter(png::AdaptiveFilterType::Adaptive);
        let mut writer = encoder.write_header()?;
        writer.write_image_data(&data)?;
        writer.finish()?;
        Ok(bytes)
    }
}

/// How a picture's colour profile is used.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Profiles {
    /// Its colours converted into sRGB, the CRT's own, from the profile it carries.
    #[default]
    Convert,
    /// Its numbers read as sRGB whatever it carries: for art drawn in exact palette values
    /// that a program tagged with a profile of its own, which the NES palette must match.
    Ignore,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_with(chunks: impl FnOnce(&mut png::Info)) -> Vec<u8> {
        let mut info = png::Info::with_size(2, 1);
        info.color_type = png::ColorType::Rgba;
        info.bit_depth = png::BitDepth::Eight;
        chunks(&mut info);
        let mut bytes = Vec::new();
        let mut writer = png::Encoder::with_info(&mut bytes, info)
            .unwrap()
            .write_header()
            .unwrap();
        writer
            .write_image_data(&[255, 0, 0, 255, 128, 128, 128, 255])
            .unwrap();
        writer.finish().unwrap();
        bytes
    }

    #[test]
    fn a_files_tag_is_read_in_pngs_order_and_from_webp() {
        assert_eq!(tag(&png_with(|_| {})), Tag::Srgb);
        let p3 = ColorProfile::new_display_p3().encode().unwrap();
        let icc = p3.clone();
        assert_eq!(
            tag(&png_with(|info| info.icc_profile = Some(icc.into()))),
            Tag::Icc(p3.clone())
        );
        let gamma = tag(&png_with(|info| {
            info.source_gamma = Some(png::ScaledFloat::new(1. / 1.8))
        }));
        assert!(
            matches!(gamma, Tag::Gamma { gamma, chromaticities: None } if (gamma - 1. / 1.8).abs() < 1e-4)
        );
        // A WebP's ICCP chunk, after a VP8X.
        let mut webp = b"RIFF\0\0\0\0WEBPVP8X".to_vec();
        webp.extend(10_u32.to_le_bytes());
        webp.extend([0x20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        webp.extend(b"ICCP");
        webp.extend((p3.len() as u32).to_le_bytes());
        webp.extend(&p3);
        assert_eq!(tag(&webp), Tag::Icc(p3));
        assert_eq!(tag(b"BM not a tagged format"), Tag::Srgb);
    }

    #[test]
    fn srgb_in_effect_is_left_alone_and_other_spaces_are_named() {
        let srgb = ColorProfile::new_srgb().encode().unwrap();
        assert!(source(Tag::Icc(srgb)).unwrap().is_none());
        // A gAMA of 1/2.2 is how many programs say sRGB.
        let said = Tag::Gamma {
            gamma: 1. / 2.2,
            chromaticities: None,
        };
        assert!(source(said).unwrap().is_none());
        let mac = Tag::Gamma {
            gamma: 1. / 1.8,
            chromaticities: None,
        };
        assert!(source(mac).unwrap().is_some());
        let p3 = ColorProfile::new_display_p3().encode().unwrap();
        assert_eq!(source(Tag::Icc(p3)).unwrap().unwrap().name, "Display P3");
        assert!(source(Tag::Icc(b"not a profile".to_vec())).is_err());
        // cICP's BT.709 primaries with sRGB's transfer are sRGB.
        let cicp = Tag::Cicp {
            primaries: 1,
            transfer: 13,
        };
        assert!(source(cicp).unwrap().is_none());
        let p3 = Tag::Cicp {
            primaries: 12,
            transfer: 13,
        };
        assert!(source(p3).unwrap().is_some());
    }

    #[test]
    fn display_p3_converts_into_srgb_as_its_primaries_say() {
        let p3 = source(Tag::Icc(ColorProfile::new_display_p3().encode().unwrap()))
            .unwrap()
            .unwrap();
        let pixels = [255, 0, 0, 255, 128, 128, 128, 200, 0, 255, 0, 255];
        let out = p3.to_srgb_8(Pixels::Rgba, &pixels).unwrap();
        // P3's red lies outside sRGB, and clips to its red; grey stays grey; alpha is kept.
        assert_eq!(&out[..4], &[255, 0, 0, 255]);
        assert!(out[4..7].iter().all(|v| v.abs_diff(128) <= 1));
        assert_eq!(out[7], 200);
        let wide = p3
            .to_srgb_16(Pixels::Rgba, &[32768, 32768, 32768, 65535])
            .unwrap();
        assert!(wide[..3].iter().all(|v| v.abs_diff(32768) <= 64));
        // A less saturated P3 green is a fully saturated sRGB one's neighbour, not itself.
        let green = p3.to_srgb_8(Pixels::Rgba, &[0, 200, 0, 255]).unwrap();
        assert!(
            green[1] > 200 && green[0] < 60 && green[2] < 60,
            "{green:?}"
        );
    }

    #[test]
    fn a_still_is_written_in_its_space_with_the_profile_embedded() {
        let image = RgbaImage::from_fn(4, 2, |x, y| {
            image::Rgba([(x * 60) as u8, (y * 100) as u8, 200, 255])
        });
        let srgb = Space::Srgb.png(&image).unwrap();
        let decoded = image::load_from_memory(&srgb).unwrap().to_rgba8();
        assert_eq!(decoded, image, "sRGB is written as rendered");
        let reader = png::Decoder::new(Cursor::new(&srgb)).read_info().unwrap();
        assert!(reader.info().srgb.is_some());

        let p3 = Space::DisplayP3.png(&image).unwrap();
        let reader = png::Decoder::new(Cursor::new(&p3)).read_info().unwrap();
        assert_eq!(reader.info().bit_depth, png::BitDepth::Sixteen);
        let icc = reader.info().icc_profile.as_ref().unwrap().to_vec();
        assert_eq!(
            name(&ColorProfile::new_from_slice(&icc).unwrap()),
            "Display P3"
        );
        // Read back through its profile, it is the picture that was rendered.
        let back = crate::input::decode_image(&p3).unwrap();
        let most = back
            .pixels()
            .zip(image.pixels())
            .flat_map(|(a, b)| (0..4).map(move |i| a[i].abs_diff(b[i])))
            .max();
        assert!(most <= Some(1), "{most:?}");

        let adobe = ColorProfile::new_adobe_rgb().encode().unwrap();
        let space = Space::from_icc(adobe).unwrap();
        assert_eq!(
            space.name(),
            "Adobe RGB 1998",
            "as the profile names itself"
        );
        assert!(space.png(&image).is_ok());
        let gray = ColorProfile::new_gray_with_gamma(2.2).encode().unwrap();
        assert!(Space::from_icc(gray).is_err());
        assert!(Space::from_icc(b"nonsense".to_vec()).is_err());
        assert_eq!(Space::DisplayP3.slug(), Some("display-p3"));
    }
}
