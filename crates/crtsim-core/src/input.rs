//! Reading a source image from a file, shared by the CLI and the desktop.
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
    let mut reader = image::io::Reader::open(path)?.with_guessed_format()?;
    let mut limits = image::io::Limits::default();
    limits.max_alloc = Some(512 * 1024 * 1024);
    reader.limits(limits);
    Ok(reader
        .decode()
        .context("Cannot decode image (PNG, JPEG, WebP, BMP or GIF expected)")?
        .to_rgba8())
}
