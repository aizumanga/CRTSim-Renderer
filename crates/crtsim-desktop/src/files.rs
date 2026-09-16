use anyhow::{ensure, Context, Result};
use crtsim_core::config::{self, Config};
use image::{DynamicImage, ImageOutputFormat, RgbaImage};
use std::{io::Write, path::Path};

pub fn load_image(path: &Path) -> Result<RgbaImage> {
    config::validate_size(image::image_dimensions(path).context("Cannot read image dimensions")?)?;
    let mut reader = image::io::Reader::open(path)?.with_guessed_format()?;
    let mut limits = image::io::Limits::default();
    limits.max_alloc = Some(512 * 1024 * 1024);
    reader.limits(limits);
    Ok(reader
        .decode()
        .context("Cannot decode image (PNG, JPEG, WebP or BMP expected)")?
        .to_rgba8())
}
pub fn load_preset(path: &Path, input: (u32, u32)) -> Result<Config> {
    ensure!(path.metadata()?.len() <= 1024 * 1024, "Preset exceeds 1 MB");
    let c: Config = serde_json::from_slice(&std::fs::read(path)?)?;
    c.validate()?;
    c.signal_size(input)?;
    c.output_size(input)?;
    Ok(c)
}
/// Publish only a complete file, atomically replacing a destination approved by the save dialog.
fn save_atomic(path: &Path, write: impl FnOnce(&mut std::fs::File) -> Result<()>) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    write(temporary.as_file_mut())?;
    temporary.as_file_mut().sync_all()?;
    temporary
        .persist(path)
        .map_err(|e| anyhow::anyhow!("Cannot save {}: {}", path.display(), e.error))?;
    Ok(())
}
pub fn save_png(path: &Path, image: RgbaImage) -> Result<()> {
    ensure!(
        path.extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("png")),
        "Export filename must end in .png"
    );
    save_atomic(path, |file| {
        DynamicImage::ImageRgba8(image).write_to(file, ImageOutputFormat::Png)?;
        Ok(())
    })
}
pub fn save_preset(path: &Path, c: &Config) -> Result<()> {
    c.validate()?;
    save_atomic(path, |f| {
        f.write_all(serde_json::to_string_pretty(c)?.as_bytes())?;
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preset_and_png_saves_atomically_replace_existing_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("preset.json");
        let c = crate::model::general();
        save_preset(&path, &c).unwrap();
        assert_eq!(load_preset(&path, (1216, 832)).unwrap(), c);
        save_preset(&path, &Config::default()).unwrap();
        assert_eq!(load_preset(&path, (1216, 832)).unwrap(), Config::default());
        let png = dir.path().join("image.png");
        let src = config::test_card();
        save_png(&png, src.clone()).unwrap();
        save_png(&png, RgbaImage::new(1, 1)).unwrap();
        assert_eq!(load_image(&png).unwrap(), RgbaImage::new(1, 1));
    }
}
