//! Reading a source image from a file or from memory, shared by the CLI and the app.
use anyhow::{Context, Result};
use image::RgbaImage;
use std::path::Path;

/// The formats `load_image` decodes, as file extensions: the `image` features this crate enables.
pub const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp", "bmp", "gif"];

/// Decodes an image after checking its dimensions against the renderer's limits, and caps the
/// decoder's allocation so a malformed file cannot exhaust memory.
pub fn load_image(path: &Path) -> Result<RgbaImage> {
    crate::config::validate_size(
        image::image_dimensions(path).context("Cannot read image dimensions")?,
    )?;
    decode(image::io::Reader::open(path)?.with_guessed_format()?)
}

/// `load_image` for an image already in memory, such as a file picked in a browser.
pub fn decode_image(bytes: &[u8]) -> Result<RgbaImage> {
    let reader = || image::io::Reader::new(std::io::Cursor::new(bytes)).with_guessed_format();
    crate::config::validate_size(
        reader()?
            .into_dimensions()
            .context("Cannot read image dimensions")?,
    )?;
    decode(reader()?)
}

const UNREADABLE: &str = "Cannot decode image (PNG, JPEG, WebP, BMP or GIF expected)";

fn decode<R: std::io::BufRead + std::io::Seek>(
    mut reader: image::io::Reader<R>,
) -> Result<RgbaImage> {
    let mut limits = image::io::Limits::default();
    limits.max_alloc = Some(512 * 1024 * 1024);
    reader.limits(limits);
    Ok(reader.decode().context(UNREADABLE)?.to_rgba8())
}
