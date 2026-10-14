//! Animated PNG written here rather than by FFmpeg, for hosts without it. Each frame is
//! compressed as it comes, as a still PNG whose image data is kept, and the file is put
//! together once the number of frames is known, which APNG's header states first. Every pixel
//! is kept exactly, as in the PNG stills the app exports.
use anyhow::{ensure, Context, Result};
use image::RgbaImage;

/// The PNG file signature.
const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'];

pub(crate) struct ApngWriter {
    size: (u32, u32),
    /// How long each frame shows: `delay.0 / delay.1` seconds.
    delay: (u16, u16),
    /// The still PNG's header chunk's data, from the first frame.
    header: Option<Vec<u8>>,
    /// Each frame's compressed image data.
    frames: Vec<Vec<u8>>,
}

impl ApngWriter {
    pub fn new(size: (u32, u32), fps: u32) -> Result<Self> {
        ensure!(size.0 > 0 && size.1 > 0, "An animation needs a picture");
        let fps = u16::try_from(fps)
            .ok()
            .filter(|&fps| fps > 0)
            .context("An APNG's frame rate must be 1–65535 per second")?;
        Ok(Self {
            size,
            delay: (1, fps),
            header: None,
            frames: vec![],
        })
    }

    /// Compresses `frame`, the next one, without alpha: the CRT is opaque.
    pub fn frame(&mut self, frame: &RgbaImage) -> Result<()> {
        ensure!(
            frame.dimensions() == self.size,
            "Every frame of an animation must be the same size"
        );
        let rgb: Vec<u8> = frame
            .pixels()
            .flat_map(|pixel| [pixel[0], pixel[1], pixel[2]])
            .collect();
        let mut still = vec![];
        let mut encoder = png::Encoder::new(&mut still, self.size.0, self.size.1);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Best);
        encoder.set_adaptive_filter(png::AdaptiveFilterType::Adaptive);
        let mut writer = encoder.write_header()?;
        writer.write_image_data(&rgb)?;
        writer.finish()?;
        let mut data = vec![];
        for (kind, body) in chunks(&still)? {
            match kind {
                b"IHDR" if self.header.is_none() => self.header = Some(body.to_vec()),
                b"IDAT" => data.extend_from_slice(body),
                _ => {}
            }
        }
        self.frames.push(data);
        Ok(())
    }

    /// The whole file, looping forever.
    pub fn finish(self) -> Result<Vec<u8>> {
        let header = self.header.context("No frames were written")?;
        let mut file = SIGNATURE.to_vec();
        chunk(&mut file, b"IHDR", &header);
        let count = u32::try_from(self.frames.len()).context("Too many frames for an APNG")?;
        chunk(
            &mut file,
            b"acTL",
            &[count.to_be_bytes(), 0u32.to_be_bytes()].concat(),
        );
        // Each frame's control and data chunks share one sequence, counted from 0.
        let mut sequence = 0u32;
        for (n, data) in self.frames.iter().enumerate() {
            let control = [
                &sequence.to_be_bytes()[..],
                &self.size.0.to_be_bytes(),
                &self.size.1.to_be_bytes(),
                &0u32.to_be_bytes(),
                &0u32.to_be_bytes(),
                &self.delay.0.to_be_bytes(),
                &self.delay.1.to_be_bytes(),
                // Each frame replaces the whole picture: no disposal, no blending.
                &[0, 0],
            ]
            .concat();
            chunk(&mut file, b"fcTL", &control);
            sequence += 1;
            if n == 0 {
                // The first frame is the still a viewer without APNG support shows.
                chunk(&mut file, b"IDAT", data);
            } else {
                chunk(
                    &mut file,
                    b"fdAT",
                    &[&sequence.to_be_bytes()[..], data].concat(),
                );
                sequence += 1;
            }
        }
        chunk(&mut file, b"IEND", &[]);
        Ok(file)
    }
}

/// The chunks of a PNG file: each one's type and data.
fn chunks(file: &[u8]) -> Result<Vec<(&[u8; 4], &[u8])>> {
    ensure!(file.starts_with(&SIGNATURE), "Not a PNG file");
    let mut found = vec![];
    let mut at = SIGNATURE.len();
    while at + 12 <= file.len() {
        let length = u32::from_be_bytes(file[at..at + 4].try_into()?) as usize;
        let kind: &[u8; 4] = file[at + 4..at + 8].try_into()?;
        let end = (at + 8).checked_add(length).context("Truncated PNG")?;
        ensure!(end + 4 <= file.len(), "Truncated PNG");
        found.push((kind, &file[at + 8..end]));
        at = end + 4;
    }
    Ok(found)
}

/// Appends a chunk of `kind` holding `data`, with its length and checksum.
fn chunk(file: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    file.extend_from_slice(&(data.len() as u32).to_be_bytes());
    file.extend_from_slice(kind);
    file.extend_from_slice(data);
    let mut crc = crc32fast::Hasher::new();
    crc.update(kind);
    crc.update(data);
    file.extend_from_slice(&crc.finalize().to_be_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{codecs::png::PngDecoder, AnimationDecoder, Rgba};

    fn frame(shade: u8) -> RgbaImage {
        RgbaImage::from_fn(6, 4, |x, y| {
            Rgba([shade, (x * 40) as u8, (y * 60) as u8, 255])
        })
    }

    #[test]
    fn frames_read_back_exactly_with_their_timing() {
        let mut writer = ApngWriter::new((6, 4), 25).unwrap();
        let frames = [frame(10), frame(120), frame(250)];
        for f in &frames {
            writer.frame(f).unwrap();
        }
        let file = writer.finish().unwrap();
        let decoder = PngDecoder::new(std::io::Cursor::new(&file)).unwrap();
        assert!(decoder.is_apng());
        let read: Vec<_> = decoder
            .apng()
            .into_frames()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(read.len(), 3);
        for (written, read) in frames.iter().zip(&read) {
            assert_eq!(read.buffer(), written);
            assert_eq!(
                std::time::Duration::from(read.delay()),
                std::time::Duration::from_millis(40)
            );
        }
        // A viewer that does not animate PNGs shows the first frame.
        let still = image::load_from_memory(&file).unwrap().to_rgba8();
        assert_eq!(&still, &frames[0]);
    }

    #[test]
    fn frames_of_another_size_or_none_at_all_are_refused() {
        let mut writer = ApngWriter::new((6, 4), 25).unwrap();
        assert!(writer.frame(&RgbaImage::new(5, 4)).is_err());
        assert!(writer.finish().is_err());
        assert!(ApngWriter::new((6, 4), 0).is_err());
    }
}
