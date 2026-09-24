use anyhow::{ensure, Context, Result};
use crtsim_core::config::Config;
use image::{DynamicImage, ImageOutputFormat, RgbaImage};
use std::{
    io::{Cursor, Read, Write},
    path::Path,
};

const PRESET_KEYWORD: &[u8] = b"CRTSim-Renderer-Preset";

/// Everything Open File accepts as a source: images, then videos.
pub fn media_extensions() -> Vec<&'static str> {
    [
        crtsim_core::input::IMAGE_EXTENSIONS,
        crtsim_media::VIDEO_EXTENSIONS,
    ]
    .concat()
}

pub fn load_preset(path: &Path, input: (u32, u32)) -> Result<Config> {
    ensure!(
        path.metadata()?.len() <= 32 * 1024 * 1024,
        "Preset exceeds 32 MB"
    );
    let c = Config::from_json_slice(&std::fs::read(path)?)?;
    c.signal_size(input)?;
    c.output_size(input)?;
    Ok(c)
}
/// Publish only a complete file, atomically replacing a destination approved by the save dialog.
pub fn save_atomic(
    path: &Path,
    write: impl FnOnce(&mut std::fs::File) -> Result<()>,
) -> Result<()> {
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
pub fn save_png(path: &Path, image: RgbaImage, preset: Option<&Config>) -> Result<()> {
    ensure!(
        path.extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("png")),
        "Export filename must end in .png"
    );
    save_atomic(path, |file| {
        let mut encoded = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(image).write_to(&mut encoded, ImageOutputFormat::Png)?;
        let mut bytes = encoded.into_inner();
        if let Some(config) = preset {
            let json = serde_json::to_vec(config)?;
            ensure!(
                json.len() <= 32 * 1024 * 1024,
                "Preset metadata exceeds 32 MB"
            );
            add_text_chunk(&mut bytes, PRESET_KEYWORD, &json)?;
        }
        file.write_all(&bytes)?;
        Ok(())
    })
}

/// Read the embedded settings from a PNG produced by this application.
pub fn load_preset_from_image(path: &Path, input: (u32, u32)) -> Result<Config> {
    let file = std::fs::File::open(path).context("Cannot read image file")?;
    ensure!(
        file.metadata()?.len() <= 512 * 1024 * 1024,
        "Image file exceeds 512 MB"
    );
    let mut bytes = Vec::new();
    file.take(512 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= 512 * 1024 * 1024,
        "Image file exceeds 512 MB"
    );
    let json = find_text_chunk(&bytes, PRESET_KEYWORD)
        .context("This image does not contain a CRTSim-Renderer preset")?;
    ensure!(
        json.len() <= 32 * 1024 * 1024,
        "Preset metadata exceeds 32 MB"
    );
    let c = Config::from_json_slice(json).context("Embedded preset metadata is invalid")?;
    c.signal_size(input)?;
    c.output_size(input)?;
    Ok(c)
}

fn add_text_chunk(bytes: &mut Vec<u8>, keyword: &[u8], text: &[u8]) -> Result<()> {
    ensure!(
        keyword.len() <= 79 && !keyword.contains(&0),
        "Invalid PNG metadata keyword"
    );
    ensure!(
        !text.contains(&0),
        "Preset metadata contains an unsupported NUL byte"
    );
    let mut data = Vec::with_capacity(keyword.len() + 1 + text.len());
    data.extend_from_slice(keyword);
    data.push(0);
    data.extend_from_slice(text);
    let mut chunk = Vec::with_capacity(data.len() + 12);
    chunk.extend_from_slice(&(data.len() as u32).to_be_bytes());
    chunk.extend_from_slice(b"tEXt");
    chunk.extend_from_slice(&data);
    chunk.extend_from_slice(&crc32(&[b"tEXt".as_slice(), data.as_slice()].concat()).to_be_bytes());
    ensure!(
        bytes.starts_with(b"\x89PNG\r\n\x1a\n") && bytes.len() >= 12,
        "Invalid PNG output"
    );
    let insert_at = bytes.len() - 12; // IEND is always the final chunk.
    bytes.splice(insert_at..insert_at, chunk);
    Ok(())
}

fn find_text_chunk<'a>(bytes: &'a [u8], keyword: &[u8]) -> Option<&'a [u8]> {
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return None;
    }
    let mut offset: usize = 8;
    while offset.checked_add(12)? <= bytes.len() {
        let length = u32::from_be_bytes(bytes[offset..offset + 4].try_into().ok()?) as usize;
        let data_start = offset + 8;
        let data_end = data_start.checked_add(length)?;
        if data_end.checked_add(4)? > bytes.len() {
            return None;
        }
        if &bytes[offset + 4..offset + 8] == b"tEXt" {
            if let Some(nul) = bytes[data_start..data_end].iter().position(|&b| b == 0) {
                if &bytes[data_start..data_start + nul] == keyword {
                    let expected =
                        u32::from_be_bytes(bytes[data_end..data_end + 4].try_into().ok()?);
                    if crc32(&bytes[offset + 4..data_end]) != expected {
                        return None;
                    }
                    return Some(&bytes[data_start + nul + 1..data_end]);
                }
            }
        }
        offset = data_end + 4;
    }
    None
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffff;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
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
    fn included_lut_survives_preset_and_png_metadata_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = Config::general();
        c.lut = Some(std::sync::Arc::new(crtsim_core::nes_luts::load(0).unwrap()));
        assert_eq!(c.lut.as_ref().unwrap().size, 64);
        let path = dir.path().join("nes-preset.json");
        save_preset(&path, &c).unwrap();
        assert_eq!(load_preset(&path, (1, 1)).unwrap(), c);
        let png = dir.path().join("nes-image.png");
        let source = RgbaImage::from_pixel(1, 1, image::Rgba([32, 64, 128, 255]));
        save_png(&png, source.clone(), Some(&c)).unwrap();
        assert_eq!(crtsim_core::input::load_image(&png).unwrap(), source);
        assert_eq!(load_preset_from_image(&png, (1, 1)).unwrap(), c);
    }

    #[test]
    fn preset_and_png_saves_atomically_replace_existing_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("preset.json");
        let c = Config::general();
        save_preset(&path, &c).unwrap();
        assert_eq!(load_preset(&path, (1216, 832)).unwrap(), c);
        save_preset(&path, &Config::default()).unwrap();
        assert_eq!(load_preset(&path, (1216, 832)).unwrap(), Config::default());
        let png = dir.path().join("image.png");
        let src = crtsim_core::config::test_card();
        save_png(&png, src.clone(), Some(&c)).unwrap();
        assert_eq!(crtsim_core::input::load_image(&png).unwrap(), src);
        assert_eq!(load_preset_from_image(&png, (1216, 832)).unwrap(), c);
        let mut damaged = std::fs::read(&png).unwrap();
        let index = damaged
            .windows(PRESET_KEYWORD.len())
            .position(|w| w == PRESET_KEYWORD)
            .unwrap();
        damaged[index + PRESET_KEYWORD.len() + 2] ^= 1;
        assert!(find_text_chunk(&damaged, PRESET_KEYWORD).is_none());
        save_png(&png, RgbaImage::new(1, 1), None).unwrap();
        assert!(load_preset_from_image(&png, (1, 1)).is_err());
        let original = std::fs::read(&png).unwrap();
        assert!(save_atomic(&png, |f| {
            f.write_all(b"incomplete")?;
            anyhow::bail!("encoding failed")
        })
        .is_err());
        assert_eq!(std::fs::read(&png).unwrap(), original);
    }
}
