use anyhow::{ensure, Context, Result};
use crtsim_core::config::Config;
use image::{DynamicImage, ImageOutputFormat, RgbaImage};
use std::{
    io::{Cursor, Write},
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

/// A preset from its JSON, checked against a source of size `input`.
pub fn preset_from_json(bytes: &[u8], input: (u32, u32)) -> Result<Config> {
    ensure!(bytes.len() <= 32 * 1024 * 1024, "Preset exceeds 32 MB");
    let c = Config::from_json_slice(bytes)?;
    c.validate_for(input)?;
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
    let bytes = png_bytes(image, preset)?;
    save_atomic(path, |file| Ok(file.write_all(&bytes)?))
}

/// `image` as a PNG, carrying `preset` when there is one.
pub fn png_bytes(image: RgbaImage, preset: Option<&Config>) -> Result<Vec<u8>> {
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
    Ok(bytes)
}

/// The settings a PNG this application made carries, checked against a source of size `input`.
pub fn preset_from_png(bytes: &[u8], input: (u32, u32)) -> Result<Config> {
    ensure!(
        bytes.len() <= 512 * 1024 * 1024,
        "Image file exceeds 512 MB"
    );
    let json = find_text_chunk(bytes, PRESET_KEYWORD)
        .context("This image does not contain a CRTSim-Renderer preset")?;
    ensure!(
        json.len() <= 32 * 1024 * 1024,
        "Preset metadata exceeds 32 MB"
    );
    let c = Config::from_json_slice(json).context("Embedded preset metadata is invalid")?;
    c.validate_for(input)?;
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
/// `c` as a RetroArch shader preset, zipped in a folder of its own with the port's shaders.
/// The look is named after the file, less any `-retroarch`.
pub fn save_retroarch(path: &Path, c: &Config) -> Result<()> {
    let stem = crate::file_stem(path);
    let stem = stem.strip_suffix("-retroarch").unwrap_or(&stem);
    let name: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let bytes = zip(
        &format!("{name}-retroarch"),
        &crtsim_core::retroarch::export(&[(&name, c)])?,
    );
    #[cfg(not(target_arch = "wasm32"))]
    return save_atomic(path, |f| Ok(f.write_all(&bytes)?));
    // A browser saves by downloading, under the name the path gives.
    #[cfg(target_arch = "wasm32")]
    return crate::web::download(&crate::file_name(path), &bytes, "application/zip");
}

/// `files` in a zip archive under `folder`, stored uncompressed: the PNGs are compressed
/// already and the shaders are small, so storing keeps this short and free of dependencies.
fn zip(folder: &str, files: &[crtsim_core::retroarch::File]) -> Vec<u8> {
    let mut archive = Vec::new();
    let mut directory = Vec::new();
    for file in files {
        let name = format!("{folder}/{}", file.path);
        let (crc, size) = (crc32(&file.bytes), file.bytes.len() as u32);
        let offset = archive.len() as u32;
        // Version 2.0, no flags, stored, a zero time and date; the checksum, sizes and name.
        let fields = |out: &mut Vec<u8>| {
            for field in [20_u16, 0, 0, 0, 0] {
                out.extend_from_slice(&field.to_le_bytes());
            }
            for field in [crc, size, size] {
                out.extend_from_slice(&field.to_le_bytes());
            }
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend_from_slice(&0_u16.to_le_bytes());
        };
        archive.extend_from_slice(&0x0403_4b50_u32.to_le_bytes());
        fields(&mut archive);
        archive.extend_from_slice(name.as_bytes());
        archive.extend_from_slice(&file.bytes);
        directory.extend_from_slice(&0x0201_4b50_u32.to_le_bytes());
        directory.extend_from_slice(&20_u16.to_le_bytes());
        fields(&mut directory);
        // No comment, the first disk, no attributes, then where the file's header starts.
        for field in [0_u16, 0, 0] {
            directory.extend_from_slice(&field.to_le_bytes());
        }
        directory.extend_from_slice(&0_u32.to_le_bytes());
        directory.extend_from_slice(&offset.to_le_bytes());
        directory.extend_from_slice(name.as_bytes());
    }
    let (start, count) = (archive.len() as u32, files.len() as u16);
    archive.extend_from_slice(&directory);
    archive.extend_from_slice(&0x0605_4b50_u32.to_le_bytes());
    for field in [0_u16, 0, count, count] {
        archive.extend_from_slice(&field.to_le_bytes());
    }
    archive.extend_from_slice(&(directory.len() as u32).to_le_bytes());
    archive.extend_from_slice(&start.to_le_bytes());
    archive.extend_from_slice(&0_u16.to_le_bytes());
    archive
}

pub fn save_preset(path: &Path, c: &Config) -> Result<()> {
    let bytes = preset_json(c)?;
    #[cfg(not(target_arch = "wasm32"))]
    return save_atomic(path, |f| Ok(f.write_all(&bytes)?));
    // A browser saves by downloading, under the name the path gives.
    #[cfg(target_arch = "wasm32")]
    return crate::web::download(&crate::file_name(path), &bytes, "application/json");
}

/// `c` as a preset file's JSON.
pub fn preset_json(c: &Config) -> Result<Vec<u8>> {
    c.validate()?;
    Ok(serde_json::to_string_pretty(c)?.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_retroarch_export_is_a_zip_of_one_folder_named_for_its_look() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Warm Tube-retroarch.zip");
        let mut c = Config::general();
        c.lut = Some(std::sync::Arc::new(crtsim_core::nes_luts::load(0).unwrap()));
        save_retroarch(&path, &c).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        // The end record: how many files, and where the directory listing them starts.
        let end = bytes.len() - 22;
        assert_eq!(&bytes[end..end + 4], b"PK\x05\x06");
        let count = u16::from_le_bytes([bytes[end + 10], bytes[end + 11]]) as usize;
        let mut at = u32::from_le_bytes(bytes[end + 16..end + 20].try_into().unwrap()) as usize;
        let mut names = vec![];
        for _ in 0..count {
            assert_eq!(&bytes[at..at + 4], b"PK\x01\x02");
            let field =
                |offset: usize| u16::from_le_bytes([bytes[at + offset], bytes[at + offset + 1]]);
            let (length, local) = (
                field(28) as usize,
                u32::from_le_bytes(bytes[at + 42..at + 46].try_into().unwrap()),
            );
            let name = String::from_utf8(bytes[at + 46..at + 46 + length].to_vec()).unwrap();
            // Each entry's own header is where the directory says, with the same checksum.
            let local = local as usize;
            assert_eq!(&bytes[local..local + 4], b"PK\x03\x04");
            assert_eq!(bytes[local + 14..local + 18], bytes[at + 16..at + 20]);
            names.push(name);
            at += 46 + length;
        }
        assert!(
            names.iter().all(|n| n.starts_with("Warm_Tube-retroarch/")),
            "{names:?}"
        );
        for expected in ["Warm_Tube.slangp", "Warm_Tube-table.png", "README.md"] {
            assert!(
                names.contains(&format!("Warm_Tube-retroarch/{expected}")),
                "{names:?}"
            );
        }
    }

    #[test]
    fn included_lut_survives_preset_and_png_metadata_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = Config::general();
        c.lut = Some(std::sync::Arc::new(crtsim_core::nes_luts::load(0).unwrap()));
        assert_eq!(c.lut.as_ref().unwrap().size, 64);
        let path = dir.path().join("nes-preset.json");
        save_preset(&path, &c).unwrap();
        assert_eq!(
            preset_from_json(&std::fs::read(&path).unwrap(), (1, 1)).unwrap(),
            c
        );
        let png = dir.path().join("nes-image.png");
        let source = RgbaImage::from_pixel(1, 1, image::Rgba([32, 64, 128, 255]));
        save_png(&png, source.clone(), Some(&c)).unwrap();
        assert_eq!(crtsim_core::input::load_image(&png).unwrap(), source);
        assert_eq!(
            preset_from_png(&std::fs::read(&png).unwrap(), (1, 1)).unwrap(),
            c
        );
    }

    #[test]
    fn preset_and_png_saves_atomically_replace_existing_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("preset.json");
        let c = Config::general();
        save_preset(&path, &c).unwrap();
        assert_eq!(
            preset_from_json(&std::fs::read(&path).unwrap(), (1216, 832)).unwrap(),
            c
        );
        save_preset(&path, &Config::default()).unwrap();
        assert_eq!(
            preset_from_json(&std::fs::read(&path).unwrap(), (1216, 832)).unwrap(),
            Config::default()
        );
        let png = dir.path().join("image.png");
        let src = crtsim_core::config::test_card();
        save_png(&png, src.clone(), Some(&c)).unwrap();
        assert_eq!(crtsim_core::input::load_image(&png).unwrap(), src);
        assert_eq!(
            preset_from_png(&std::fs::read(&png).unwrap(), (1216, 832)).unwrap(),
            c
        );
        let mut damaged = std::fs::read(&png).unwrap();
        let index = damaged
            .windows(PRESET_KEYWORD.len())
            .position(|w| w == PRESET_KEYWORD)
            .unwrap();
        damaged[index + PRESET_KEYWORD.len() + 2] ^= 1;
        assert!(find_text_chunk(&damaged, PRESET_KEYWORD).is_none());
        save_png(&png, RgbaImage::new(1, 1), None).unwrap();
        assert!(preset_from_png(&std::fs::read(&png).unwrap(), (1, 1)).is_err());
        let original = std::fs::read(&png).unwrap();
        assert!(save_atomic(&png, |f| {
            f.write_all(b"incomplete")?;
            anyhow::bail!("encoding failed")
        })
        .is_err());
        assert_eq!(std::fs::read(&png).unwrap(), original);
    }
}
