//! Reading a source image from a file or from memory, shared by the CLI and the app, with its
//! colours read into sRGB from the profile its file carries (see `color`).
use crate::color::{Pixels, Reading};
use anyhow::{Context, Result};
use image::{ColorType, DynamicImage, RgbaImage};
use std::path::Path;

pub use crate::color::Profiles;

/// The formats `load_image` decodes, as file extensions: the `image` features this crate enables.
pub const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp", "bmp", "gif"];

/// A picture read into sRGB.
pub struct Decoded {
    pub image: RgbaImage,
    /// What became of the colour profile its file carries, in a sentence, when it carries one
    /// other than sRGB.
    pub note: Option<String>,
}

/// Decodes an image into sRGB after checking its dimensions against the renderer's limits,
/// and caps the decoder's allocation so a malformed file cannot exhaust memory.
pub fn load_image(path: &Path) -> Result<RgbaImage> {
    Ok(read(path, Profiles::Convert)?.image)
}

/// `load_image` for an image already in memory, such as a file picked in a browser.
pub fn decode_image(bytes: &[u8]) -> Result<RgbaImage> {
    Ok(read_bytes(bytes, Profiles::Convert)?.image)
}

/// `load_image`, using its colour profile as `profiles` says, and saying what became of it.
pub fn read(path: &Path, profiles: Profiles) -> Result<Decoded> {
    crate::config::validate_size(
        image::image_dimensions(path).context("Cannot read image dimensions")?,
    )?;
    let bytes = std::fs::read(path).with_context(|| format!("Cannot read {}", path.display()))?;
    read_bytes(&bytes, profiles)
}

/// `read` for an image already in memory.
pub fn read_bytes(bytes: &[u8], profiles: Profiles) -> Result<Decoded> {
    crate::config::validate_size(
        reader(bytes)?
            .into_dimensions()
            .context("Cannot read image dimensions")?,
    )?;
    let reading = Reading::of_file(bytes, profiles);
    if reading.source.is_none() {
        return Ok(Decoded {
            image: decode(bytes)?.to_rgba8(),
            note: reading.note().map(String::from),
        });
    }
    match convert(bytes, &reading) {
        Ok(image) => Ok(Decoded {
            image,
            note: reading.note().map(String::from),
        }),
        // A profile that turns out not to fit the picture leaves it read as sRGB, as it would
        // be without one, and says so.
        Err(why) => Ok(Decoded {
            image: decode(bytes)?.to_rgba8(),
            note: Reading::unusable(why).note().map(String::from),
        }),
    }
}

/// The picture in `bytes` converted into sRGB by `reading`, which converts.
fn convert(bytes: &[u8], reading: &Reading) -> Result<RgbaImage> {
    let source = reading.source.as_ref().context("nothing to convert")?;
    if source.is_cmyk() {
        return cmyk_jpeg(bytes, source);
    }
    let image = decode(bytes)?;
    let (width, height) = (image.width(), image.height());
    let gray = matches!(
        image.color(),
        ColorType::L8 | ColorType::La8 | ColorType::L16 | ColorType::La16
    );
    let deep = matches!(
        image.color(),
        ColorType::L16
            | ColorType::La16
            | ColorType::Rgb16
            | ColorType::Rgba16
            | ColorType::Rgb32F
            | ColorType::Rgba32F
    );
    anyhow::ensure!(
        !source.is_gray() || gray,
        "it is for grey, and the picture is in colour"
    );
    let rgba = match (source.is_gray(), deep) {
        (true, false) => source.to_srgb_8(Pixels::GrayAlpha, image.to_luma_alpha8().as_raw())?,
        (false, false) => source.to_srgb_8(Pixels::Rgba, image.to_rgba8().as_raw())?,
        (gray, true) => {
            // Converted at 16 bits and rounded once, so a deep picture loses no more than an
            // 8-bit one does.
            let wide = if gray {
                source.to_srgb_16(Pixels::GrayAlpha, image.to_luma_alpha16().as_raw())?
            } else {
                source.to_srgb_16(Pixels::Rgba, image.to_rgba16().as_raw())?
            };
            wide.iter()
                .map(|&v| ((u32::from(v) * 255 + 32767) / 65535) as u8)
                .collect()
        }
    };
    RgbaImage::from_raw(width, height, rgba).context("the conversion lost pixels")
}

/// A CMYK JPEG's inks converted into sRGB by its profile. The image crate turns inks into RGB
/// by arithmetic alone, which is how a CMYK JPEG without a profile is read.
fn cmyk_jpeg(bytes: &[u8], source: &crate::color::Source) -> Result<RgbaImage> {
    let mut decoder = jpeg_decoder::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_max_decoding_buffer_size(512 * 1024 * 1024);
    let inks = decoder
        .decode()
        .context("it is for CMYK, and the picture is not")?;
    let info = decoder
        .info()
        .context("it is for CMYK, and the picture is not")?;
    anyhow::ensure!(
        info.pixel_format == jpeg_decoder::PixelFormat::CMYK32,
        "it is for CMYK, and the picture is not"
    );
    let rgba = source.to_srgb_8(Pixels::Cmyk, &inks)?;
    RgbaImage::from_raw(u32::from(info.width), u32::from(info.height), rgba)
        .context("the conversion lost pixels")
}

const UNREADABLE: &str = "Cannot decode image (PNG, JPEG, WebP, BMP or GIF expected)";

fn reader(bytes: &[u8]) -> Result<image::io::Reader<std::io::Cursor<&[u8]>>> {
    Ok(image::io::Reader::new(std::io::Cursor::new(bytes)).with_guessed_format()?)
}

fn decode(bytes: &[u8]) -> Result<DynamicImage> {
    let mut reader = reader(bytes)?;
    let mut limits = image::io::Limits::default();
    limits.max_alloc = Some(512 * 1024 * 1024);
    reader.limits(limits);
    reader.decode().context(UNREADABLE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use moxcms::ColorProfile;

    fn png(image: &DynamicImage, icc: Option<Vec<u8>>) -> Vec<u8> {
        let (color, depth, data): (png::ColorType, png::BitDepth, Vec<u8>) = match image {
            DynamicImage::ImageRgba8(i) => (
                png::ColorType::Rgba,
                png::BitDepth::Eight,
                i.as_raw().clone(),
            ),
            DynamicImage::ImageRgba16(i) => (
                png::ColorType::Rgba,
                png::BitDepth::Sixteen,
                i.as_raw().iter().flat_map(|v| v.to_be_bytes()).collect(),
            ),
            DynamicImage::ImageLuma8(i) => (
                png::ColorType::Grayscale,
                png::BitDepth::Eight,
                i.as_raw().clone(),
            ),
            _ => unreachable!(),
        };
        let mut info = png::Info::with_size(image.width(), image.height());
        info.color_type = color;
        info.bit_depth = depth;
        info.icc_profile = icc.map(Into::into);
        let mut bytes = Vec::new();
        let mut writer = png::Encoder::with_info(&mut bytes, info)
            .unwrap()
            .write_header()
            .unwrap();
        writer.write_image_data(&data).unwrap();
        writer.finish().unwrap();
        bytes
    }

    #[test]
    fn a_profile_is_converted_ignored_or_left_alone_and_said() {
        let picture = RgbaImage::from_fn(8, 8, |x, y| {
            image::Rgba([(x * 32) as u8, (y * 32) as u8, 100, 255])
        });
        let p3 = ColorProfile::new_display_p3().encode().unwrap();
        let tagged = png(&DynamicImage::ImageRgba8(picture.clone()), Some(p3));
        let converted = read_bytes(&tagged, Profiles::Convert).unwrap();
        assert_ne!(converted.image, picture);
        assert!(converted.note.unwrap().contains("Display P3"));
        let ignored = read_bytes(&tagged, Profiles::Ignore).unwrap();
        assert_eq!(
            ignored.image, picture,
            "an ignored profile keeps every value"
        );
        assert!(ignored.note.unwrap().contains("ignored"));

        // A program's own copy of sRGB keeps a palette's exact values, and says nothing.
        let srgb = ColorProfile::new_srgb().encode().unwrap();
        let plain = read_bytes(
            &png(&DynamicImage::ImageRgba8(picture.clone()), Some(srgb)),
            Profiles::Convert,
        )
        .unwrap();
        assert_eq!((plain.image, plain.note), (picture.clone(), None));

        let broken = read_bytes(
            &png(
                &DynamicImage::ImageRgba8(picture.clone()),
                Some(b"junk".to_vec()),
            ),
            Profiles::Convert,
        )
        .unwrap();
        assert_eq!(broken.image, picture);
        assert!(broken.note.unwrap().contains("cannot be used"));
    }

    #[test]
    fn deep_and_grey_pictures_convert_through_their_own_layouts() {
        let p3 = ColorProfile::new_display_p3().encode().unwrap();
        let deep = image::ImageBuffer::from_fn(4, 4, |x, _| {
            image::Rgba([(x * 16000) as u16, 30000, 50000, 65535])
        });
        let eight = DynamicImage::ImageRgba16(deep.clone()).to_rgba8();
        let from_deep = read_bytes(
            &png(&DynamicImage::ImageRgba16(deep), Some(p3.clone())),
            Profiles::Convert,
        )
        .unwrap()
        .image;
        let from_eight = read_bytes(
            &png(&DynamicImage::ImageRgba8(eight), Some(p3)),
            Profiles::Convert,
        )
        .unwrap()
        .image;
        let most = from_deep
            .pixels()
            .zip(from_eight.pixels())
            .flat_map(|(a, b)| (0..4).map(move |i| a[i].abs_diff(b[i])))
            .max();
        assert!(most <= Some(1), "{most:?}");

        // A grey picture with a grey profile of gamma 1.8 lightens its middle greys in sRGB, whose
        // curve is nearer 2.2.
        let gray = image::GrayImage::from_fn(4, 1, |x, _| image::Luma([(x * 80) as u8]));
        let profile = ColorProfile::new_gray_with_gamma(1.8).encode().unwrap();
        let out = read_bytes(
            &png(&DynamicImage::ImageLuma8(gray), Some(profile)),
            Profiles::Convert,
        )
        .unwrap();
        let p = out.image.get_pixel(1, 0);
        assert!(p[0] == p[1] && p[1] == p[2] && p[0] > 90, "{p:?}");
        assert_eq!(out.image.get_pixel(0, 0).0, [0, 0, 0, 255]);
    }
}
