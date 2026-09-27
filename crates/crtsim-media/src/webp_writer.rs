//! Animated WebP written here rather than by FFmpeg, for hosts without it. Each frame is a
//! still WebP, encoded losslessly here or lossily by the host, and only the rectangle that
//! changed is stored; a frame that changes nothing lengthens the one before it.
use anyhow::{bail, ensure, Context, Result};
use image::RgbaImage;

/// A still WebP's picture chunks (`ALPH`, `VP8 ` or `VP8L`), as an animation frame holds them.
pub(crate) fn picture_chunks(file: &[u8]) -> Result<Vec<u8>> {
    ensure!(
        file.len() >= 12 && &file[..4] == b"RIFF" && &file[8..12] == b"WEBP",
        "Not a WebP file"
    );
    let mut chunks = vec![];
    let mut at = 12;
    while at + 8 <= file.len() {
        let size = u32::from_le_bytes(file[at + 4..at + 8].try_into()?) as usize;
        let end = (at + 8).checked_add(size).context("Truncated WebP")?;
        ensure!(end <= file.len(), "Truncated WebP");
        if matches!(&file[at..at + 4], b"ALPH" | b"VP8 " | b"VP8L") {
            chunks.extend_from_slice(&file[at..end]);
            if size % 2 == 1 {
                chunks.push(0);
            }
        }
        at = end + size % 2;
    }
    ensure!(!chunks.is_empty(), "The WebP holds no picture");
    Ok(chunks)
}

/// `image`'s pixels as a lossless still WebP, without alpha.
pub(crate) fn lossless(image: &RgbaImage) -> Result<Vec<u8>> {
    let rgb: Vec<u8> = image
        .pixels()
        .flat_map(|pixel| [pixel[0], pixel[1], pixel[2]])
        .collect();
    let mut file = vec![];
    image_webp::WebPEncoder::new(&mut file).encode(
        &rgb,
        image.width(),
        image.height(),
        image_webp::ColorType::Rgb8,
    )?;
    Ok(file)
}

/// The part of a frame that changed since the last one, with even offsets as WebP needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    pub fn crop(&self, image: &RgbaImage) -> RgbaImage {
        image::imageops::crop_imm(image, self.x, self.y, self.width, self.height).to_image()
    }
}

/// Writes an animated WebP frame by frame.
pub(crate) struct WebpWriter {
    size: (u32, u32),
    fps: u32,
    /// The last frame, to find what the next one changes.
    last: Option<RgbaImage>,
    /// The frames so far, as `ANMF` chunks without their headers, the last one's duration
    /// still open to lengthening.
    frames: Vec<Vec<u8>>,
    alpha: bool,
    count: u64,
}

impl WebpWriter {
    pub fn new(size: (u32, u32), fps: u32) -> Result<Self> {
        ensure!(fps > 0, "An animation needs a frame rate");
        ensure!(
            size.0 <= 1 << 24 && size.1 <= 1 << 24,
            "WebP frames are too large"
        );
        Ok(Self {
            size,
            fps,
            last: None,
            frames: vec![],
            alpha: false,
            count: 0,
        })
    }

    /// How long frame `index` shows, in milliseconds, rounded so the rate averages out.
    fn duration(&self, index: u64) -> u32 {
        let at = |frame: u64| (frame * 1000 + u64::from(self.fps) / 2) / u64::from(self.fps);
        (at(index + 1) - at(index)) as u32
    }

    /// What of `frame` must be stored, or `None` when it shows the same as the last one. The
    /// caller encodes that part and hands it to `add`.
    pub fn changed(&self, frame: &RgbaImage) -> Result<Option<Rect>> {
        ensure!(
            frame.dimensions() == self.size,
            "Animation frames must all be one size"
        );
        let Some(last) = &self.last else {
            return Ok(Some(Rect {
                x: 0,
                y: 0,
                width: self.size.0,
                height: self.size.1,
            }));
        };
        let (width, height) = self.size;
        let differs = |x: u32, y: u32| frame.get_pixel(x, y) != last.get_pixel(x, y);
        let rows: Vec<u32> = (0..height)
            .filter(|&y| (0..width).any(|x| differs(x, y)))
            .collect();
        let (Some(&top), Some(&bottom)) = (rows.first(), rows.last()) else {
            return Ok(None);
        };
        let left = rows
            .iter()
            .filter_map(|&y| (0..width).find(|&x| differs(x, y)))
            .min()
            .unwrap_or(0);
        let right = rows
            .iter()
            .filter_map(|&y| (0..width).rev().find(|&x| differs(x, y)))
            .max()
            .unwrap_or(width - 1);
        let (x, y) = (left & !1, top & !1);
        Ok(Some(Rect {
            x,
            y,
            width: right + 1 - x,
            height: bottom + 1 - y,
        }))
    }

    /// Adds the next frame, which shows for one tick of the rate: `rect` of it, as `changed`
    /// gave, encoded as the still WebP `picture`; or, with no rect, nothing new.
    pub fn add(&mut self, frame: &RgbaImage, change: Option<(Rect, &[u8])>) -> Result<()> {
        let duration = self.duration(self.count);
        self.count += 1;
        let Some((rect, picture)) = change else {
            let Some(last) = self.frames.last_mut() else {
                bail!("The first frame must be stored");
            };
            let open = u32::from_le_bytes([last[12], last[13], last[14], 0]) + duration;
            ensure!(open < 1 << 24, "A frame shows for too long");
            last[12..15].copy_from_slice(&open.to_le_bytes()[..3]);
            return Ok(());
        };
        let chunks = picture_chunks(picture)?;
        self.alpha |= chunks.windows(4).any(|w| w == b"ALPH");
        let mut anmf = vec![];
        for value in [
            rect.x / 2,
            rect.y / 2,
            rect.width - 1,
            rect.height - 1,
            duration,
        ] {
            anmf.extend_from_slice(&value.to_le_bytes()[..3]);
        }
        // Not blended with what is under it, and left in place for the next frame.
        anmf.push(0b10);
        anmf.extend_from_slice(&chunks);
        self.frames.push(anmf);
        self.last = Some(frame.clone());
        Ok(())
    }

    /// The whole animation.
    pub fn finish(self) -> Result<Vec<u8>> {
        ensure!(!self.frames.is_empty(), "No frames to write");
        let mut body = b"WEBP".to_vec();
        let mut vp8x = vec![0b10 | if self.alpha { 0b1_0000 } else { 0 }, 0, 0, 0];
        vp8x.extend_from_slice(&(self.size.0 - 1).to_le_bytes()[..3]);
        vp8x.extend_from_slice(&(self.size.1 - 1).to_le_bytes()[..3]);
        chunk(&mut body, b"VP8X", &vp8x);
        // A black background, looping forever.
        chunk(&mut body, b"ANIM", &[0, 0, 0, 255, 0, 0]);
        for frame in &self.frames {
            chunk(&mut body, b"ANMF", frame);
        }
        let mut file = b"RIFF".to_vec();
        file.extend_from_slice(&u32::try_from(body.len())?.to_le_bytes());
        file.extend_from_slice(&body);
        Ok(file)
    }
}

fn chunk(into: &mut Vec<u8>, name: &[u8; 4], data: &[u8]) {
    into.extend_from_slice(name);
    into.extend_from_slice(&(data.len() as u32).to_le_bytes());
    into.extend_from_slice(data);
    if data.len() % 2 == 1 {
        into.push(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    /// Frames of the animation as a browser shows them, with durations in milliseconds.
    fn decoded(bytes: &[u8]) -> Vec<(Vec<u8>, u32)> {
        let mut decoder = image_webp::WebPDecoder::new(std::io::Cursor::new(bytes)).unwrap();
        assert!(decoder.is_animated());
        let mut buffer = vec![0; decoder.output_buffer_size().unwrap()];
        let mut frames = vec![];
        while let Ok(duration) = decoder.read_frame(&mut buffer) {
            frames.push((buffer.clone(), duration));
        }
        frames
    }

    fn written(frames: &[&RgbaImage], fps: u32) -> Vec<u8> {
        let mut writer = WebpWriter::new(frames[0].dimensions(), fps).unwrap();
        for frame in frames {
            let change = writer.changed(frame).unwrap();
            let picture = change.map(|rect| lossless(&rect.crop(frame)).unwrap());
            writer.add(frame, change.zip(picture.as_deref())).unwrap();
        }
        writer.finish().unwrap()
    }

    #[test]
    fn lossless_frames_come_back_exactly_with_their_timing() {
        let first =
            RgbaImage::from_fn(16, 8, |x, y| Rgba([(x * 16) as u8, (y * 32) as u8, 7, 255]));
        let mut second = first.clone();
        second.put_pixel(5, 3, Rgba([255, 255, 255, 255]));
        let bytes = written(&[&first, &first, &second, &second, &second], 24);
        let frames = decoded(&bytes);
        assert_eq!(frames.len(), 2);
        // 24 per second in milliseconds: 42, 41, 42, 42, 41.
        assert_eq!((frames[0].1, frames[1].1), (83, 125));
        let rgb = |image: &RgbaImage| -> Vec<u8> {
            image.pixels().flat_map(|p| [p[0], p[1], p[2]]).collect()
        };
        // The decoder writes RGB for a file without alpha.
        assert_eq!(frames[0].0, rgb(&first));
        assert_eq!(frames[1].0, rgb(&second));
    }

    #[test]
    fn only_the_changed_rectangle_is_stored_at_an_even_offset() {
        let first = RgbaImage::from_pixel(40, 20, Rgba([10, 20, 30, 255]));
        let mut second = first.clone();
        second.put_pixel(7, 5, Rgba([200, 0, 0, 255]));
        second.put_pixel(9, 6, Rgba([200, 0, 0, 255]));
        let mut writer = WebpWriter::new((40, 20), 10).unwrap();
        writer
            .add(
                &first,
                Some((
                    writer.changed(&first).unwrap().unwrap(),
                    &lossless(&first).unwrap(),
                )),
            )
            .unwrap();
        assert_eq!(
            writer.changed(&second).unwrap(),
            Some(Rect {
                x: 6,
                y: 4,
                width: 4,
                height: 3
            })
        );
        assert_eq!(writer.changed(&first).unwrap(), None);
    }

    #[test]
    fn a_hosts_lossy_still_is_taken_apart_into_its_picture() {
        // A still with an extended header and alpha, as some encoders write, keeps both
        // picture chunks and drops the rest.
        let mut file = b"RIFF\0\0\0\0WEBP".to_vec();
        chunk(&mut file, b"VP8X", &[0x10, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        chunk(&mut file, b"ALPH", &[1, 2, 3]);
        chunk(&mut file, b"VP8 ", &[4, 5]);
        let chunks = picture_chunks(&file).unwrap();
        assert_eq!(&chunks[..4], b"ALPH");
        assert_eq!(chunks.len(), 8 + 4 + 8 + 2);
        assert!(picture_chunks(b"\x89PNG\r\n\x1a\n").is_err());
    }
}
