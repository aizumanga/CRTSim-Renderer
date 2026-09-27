//! GIFs written here rather than by FFmpeg, for hosts without it: 255 colors chosen for the
//! whole animation from every frame, as FFmpeg's `palettegen` chooses them, and the frames
//! mapped to them with the same dithering `paletteuse` offers.
//!
//! The colors are counted in a first pass over the rendered frames and written in a second,
//! so no frame is held in between. Each frame stores only the rectangle that changed, with
//! unchanged pixels inside it left transparent, and a frame that changes nothing lengthens the
//! one before it instead.
use crate::Dither;
use anyhow::{ensure, Context, Result};
use image::RgbaImage;
use std::borrow::Cow;

/// Palette entries for colors. The 256th is kept for "unchanged since the last frame".
const COLORS: usize = 255;
const TRANSPARENT: u8 = 255;
/// Colors are counted at 5 bits a channel; each box's color is the exact mean of its pixels.
const BITS: u32 = 5;

/// How often each color appears across an animation's frames.
pub(crate) struct Histogram {
    /// Per 5-bit color: the pixels in it and the sum of their exact channels.
    counts: Vec<u64>,
    sums: Vec<[u64; 3]>,
}

impl Default for Histogram {
    fn default() -> Self {
        let bins = 1 << (3 * BITS);
        Self {
            counts: vec![0; bins],
            sums: vec![[0; 3]; bins],
        }
    }
}

fn bin([r, g, b]: [u8; 3]) -> usize {
    let shift = 8 - BITS;
    (usize::from(r >> shift) << (2 * BITS))
        | (usize::from(g >> shift) << BITS)
        | usize::from(b >> shift)
}

impl Histogram {
    pub fn add(&mut self, frame: &RgbaImage) {
        for pixel in frame.pixels() {
            let [r, g, b, _] = pixel.0;
            let bin = bin([r, g, b]);
            self.counts[bin] += 1;
            let sum = &mut self.sums[bin];
            sum[0] += u64::from(r);
            sum[1] += u64::from(g);
            sum[2] += u64::from(b);
        }
    }

    /// Up to 255 colors by median cut: the box with the most pixels spread along one channel
    /// is split at the median of that channel, until there are enough boxes.
    pub fn palette(&self) -> Vec<[u8; 3]> {
        let colors: Vec<(u64, [f64; 3])> = self
            .counts
            .iter()
            .zip(&self.sums)
            .filter(|(&count, _)| count > 0)
            .map(|(&count, sum)| (count, sum.map(|s| s as f64 / count as f64)))
            .collect();
        if colors.is_empty() {
            return vec![[0; 3]];
        }
        let mut boxes = vec![colors];
        while boxes.len() < COLORS {
            // The box whose widest channel, weighted by its pixels, is widest.
            let widest = boxes
                .iter()
                .enumerate()
                .filter(|(_, colors)| colors.len() > 1)
                .map(|(index, colors)| {
                    let (channel, range) = widest_channel(colors);
                    let pixels: u64 = colors.iter().map(|(count, _)| count).sum();
                    (index, channel, range * pixels as f64)
                })
                .max_by(|a, b| a.2.total_cmp(&b.2));
            let Some((index, channel, _)) = widest else {
                break;
            };
            let mut colors = boxes.swap_remove(index);
            colors.sort_by(|a, b| a.1[channel].total_cmp(&b.1[channel]));
            let total: u64 = colors.iter().map(|(count, _)| count).sum();
            let mut seen = 0;
            let median = colors
                .iter()
                .position(|(count, _)| {
                    seen += count;
                    seen * 2 >= total
                })
                .unwrap_or(0);
            // Both halves keep at least one color.
            let split = (median + 1).clamp(1, colors.len() - 1);
            let upper = colors.split_off(split);
            boxes.push(colors);
            boxes.push(upper);
        }
        boxes
            .iter()
            .map(|colors| {
                let total: f64 = colors.iter().map(|(count, _)| *count as f64).sum();
                let mut mean = [0.; 3];
                for (count, color) in colors {
                    for channel in 0..3 {
                        mean[channel] += color[channel] * *count as f64 / total;
                    }
                }
                mean.map(|c| c.round().clamp(0., 255.) as u8)
            })
            .collect()
    }
}

fn widest_channel(colors: &[(u64, [f64; 3])]) -> (usize, f64) {
    (0..3)
        .map(|channel| {
            let (low, high) = colors.iter().fold((f64::MAX, f64::MIN), |(low, high), c| {
                (low.min(c.1[channel]), high.max(c.1[channel]))
            });
            (channel, high - low)
        })
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .unwrap_or((0, 0.))
}

/// The nearest palette entry to each color, worked out once per 6-bit color when first needed.
struct Nearest {
    palette: Vec<[u8; 3]>,
    cache: Vec<u8>,
}

const UNKNOWN: u8 = TRANSPARENT;

impl Nearest {
    fn new(palette: Vec<[u8; 3]>) -> Self {
        Self {
            palette,
            cache: vec![UNKNOWN; 1 << 18],
        }
    }

    fn index(&mut self, [r, g, b]: [u8; 3]) -> u8 {
        let key = (usize::from(r >> 2) << 12) | (usize::from(g >> 2) << 6) | usize::from(b >> 2);
        if self.cache[key] == UNKNOWN {
            let centre = [r | 2, g | 2, b | 2].map(i32::from);
            let distance = |color: &[u8; 3]| -> i32 {
                (0..3)
                    .map(|c| (i32::from(color[c]) - centre[c]).pow(2))
                    .sum()
            };
            self.cache[key] = (0..self.palette.len())
                .min_by_key(|&i| distance(&self.palette[i]))
                .unwrap_or(0) as u8;
        }
        self.cache[key]
    }
}

/// FFmpeg's `paletteuse` 8x8 Bayer offsets at `bayer_scale=3`, from -4 to 3.
fn bayer(x: u32, y: u32) -> i32 {
    let p = ((y & 7) << 3 | (x & 7)) as i32;
    let q = p ^ (p >> 3);
    let value =
        (p & 1) << 5 | (q & 1) << 4 | (p & 2) << 2 | (q & 2) << 1 | (p & 4) >> 1 | (q & 4) >> 2;
    (value >> 3) - 4
}

/// Writes a GIF frame by frame, once its palette is known.
pub(crate) struct GifWriter {
    encoder: gif::Encoder<Vec<u8>>,
    size: (u32, u32),
    nearest: Nearest,
    dither: Dither,
    /// What the viewer shows after the frames written so far, as palette indices.
    canvas: Vec<u8>,
    /// The last frame, held back so a frame that changes nothing can lengthen it.
    pending: Option<gif::Frame<'static>>,
    /// Frames written or pending, to time the next one from.
    frames: u64,
    fps: u32,
}

impl GifWriter {
    pub fn new(size: (u32, u32), fps: u32, palette: Vec<[u8; 3]>, dither: Dither) -> Result<Self> {
        ensure!(fps > 0, "A GIF needs a frame rate");
        let width = u16::try_from(size.0).context("GIFs are at most 65535 pixels wide")?;
        let height = u16::try_from(size.1).context("GIFs are at most 65535 pixels high")?;
        let mut entries: Vec<u8> = palette.iter().flatten().copied().collect();
        entries.resize(256 * 3, 0);
        let mut encoder = gif::Encoder::new(Vec::new(), width, height, &entries)?;
        encoder.set_repeat(gif::Repeat::Infinite)?;
        Ok(Self {
            encoder,
            size,
            nearest: Nearest::new(palette),
            dither,
            canvas: vec![],
            pending: None,
            frames: 0,
            fps,
        })
    }

    /// How long frame `index` shows, in hundredths: the rate's frame times, rounded, so that
    /// rates that do not divide 100 alternate between two delays averaging to it.
    fn delay(&self, index: u64) -> u16 {
        let at = |frame: u64| (frame * 100 + u64::from(self.fps) / 2) / u64::from(self.fps);
        (at(index + 1) - at(index)) as u16
    }

    fn indices(&mut self, frame: &RgbaImage) -> Vec<u8> {
        let (width, height) = self.size;
        let mut indices = Vec::with_capacity((width * height) as usize);
        // Error diffusion carries each pixel's error right and down: Sierra-2-4A's
        // 2/4 to the right, 1/4 down-left and 1/4 down.
        let mut errors = vec![[0i32; 3]; (width as usize + 2) * 2];
        let row = width as usize + 2;
        for y in 0..height {
            let (current, next) = errors.split_at_mut(row);
            if self.dither == Dither::Diffusion {
                current.swap_with_slice(next);
                next.fill([0; 3]);
            }
            for x in 0..width {
                let [r, g, b, _] = frame.get_pixel(x, y).0;
                let wanted = [r, g, b].map(i32::from);
                let adjusted = match self.dither {
                    Dither::None => wanted,
                    Dither::Bayer => wanted.map(|c| c + bayer(x, y)),
                    Dither::Diffusion => {
                        let carried = current[x as usize + 1];
                        [0, 1, 2].map(|c| wanted[c] + carried[c] / 4)
                    }
                };
                let color = adjusted.map(|c| c.clamp(0, 255) as u8);
                let index = self.nearest.index(color);
                if self.dither == Dither::Diffusion {
                    let chosen = self.nearest.palette[usize::from(index)].map(i32::from);
                    let error = [0, 1, 2].map(|c| i32::from(color[c]) - chosen[c]);
                    let at = x as usize + 1;
                    for c in 0..3 {
                        current[at + 1][c] += error[c] * 2;
                        next[at - 1][c] += error[c];
                        next[at][c] += error[c];
                    }
                }
                indices.push(index);
            }
        }
        indices
    }

    /// Adds the next frame, which shows for one tick of the rate.
    pub fn frame(&mut self, frame: &RgbaImage) -> Result<()> {
        ensure!(
            frame.dimensions() == self.size,
            "GIF frames must all be one size"
        );
        let indices = self.indices(frame);
        let delay = self.delay(self.frames);
        self.frames += 1;
        let width = self.size.0 as usize;
        if self.canvas.is_empty() {
            self.pending = Some(gif::Frame {
                width: self.size.0 as u16,
                height: self.size.1 as u16,
                delay,
                dispose: gif::DisposalMethod::Keep,
                buffer: Cow::Owned(indices.clone()),
                ..gif::Frame::default()
            });
            self.canvas = indices;
            return Ok(());
        }
        // The rectangle that changed.
        let changed = |i: usize| indices[i] != self.canvas[i];
        let rows: Vec<usize> = (0..self.size.1 as usize)
            .filter(|&y| (0..width).any(|x| changed(y * width + x)))
            .collect();
        let (Some(&top), Some(&bottom)) = (rows.first(), rows.last()) else {
            if let Some(pending) = &mut self.pending {
                pending.delay = pending.delay.saturating_add(delay);
            }
            return Ok(());
        };
        let left = rows
            .iter()
            .filter_map(|&y| (0..width).find(|&x| changed(y * width + x)))
            .min()
            .unwrap_or(0);
        let right = rows
            .iter()
            .filter_map(|&y| (0..width).rev().find(|&x| changed(y * width + x)))
            .max()
            .unwrap_or(width - 1);
        let mut buffer = Vec::with_capacity((right - left + 1) * (bottom - top + 1));
        for y in top..=bottom {
            for x in left..=right {
                let i = y * width + x;
                buffer.push(if changed(i) { indices[i] } else { TRANSPARENT });
            }
        }
        for y in top..=bottom {
            let row = y * width;
            self.canvas[row + left..=row + right]
                .copy_from_slice(&indices[row + left..=row + right]);
        }
        self.flush()?;
        self.pending = Some(gif::Frame {
            left: left as u16,
            top: top as u16,
            width: (right - left + 1) as u16,
            height: (bottom - top + 1) as u16,
            delay,
            dispose: gif::DisposalMethod::Keep,
            transparent: Some(TRANSPARENT),
            buffer: Cow::Owned(buffer),
            ..gif::Frame::default()
        });
        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        if let Some(frame) = self.pending.take() {
            self.encoder.write_frame(&frame)?;
        }
        Ok(())
    }

    /// The whole GIF.
    pub fn finish(mut self) -> Result<Vec<u8>> {
        ensure!(self.frames > 0, "No frames to write");
        self.flush()?;
        Ok(self.encoder.into_inner()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{AnimationDecoder, Rgba};

    fn gradient(shift: u8) -> RgbaImage {
        RgbaImage::from_fn(64, 32, |x, y| {
            Rgba([(x * 4) as u8, (y * 8) as u8, shift, 255])
        })
    }

    /// The frames a decoder shows, with their delays in milliseconds.
    fn decoded(bytes: &[u8]) -> Vec<(RgbaImage, u32)> {
        let decoder = image::codecs::gif::GifDecoder::new(std::io::Cursor::new(bytes)).unwrap();
        decoder
            .into_frames()
            .map(|frame| {
                let frame = frame.unwrap();
                let (numerator, denominator) = frame.delay().numer_denom_ms();
                (frame.into_buffer(), numerator / denominator)
            })
            .collect()
    }

    fn largest_error(a: &RgbaImage, b: &RgbaImage) -> i32 {
        a.pixels()
            .zip(b.pixels())
            .flat_map(|(a, b)| (0..3).map(move |c| (i32::from(a[c]) - i32::from(b[c])).abs()))
            .max()
            .unwrap()
    }

    #[test]
    fn a_few_colors_come_back_exactly() {
        let colors = [[255, 0, 0], [0, 255, 0], [0, 0, 255], [30, 60, 90]];
        let frame = RgbaImage::from_fn(8, 8, |x, _| {
            let [r, g, b] = colors[(x / 2) as usize];
            Rgba([r, g, b, 255])
        });
        let mut histogram = Histogram::default();
        histogram.add(&frame);
        let palette = histogram.palette();
        assert_eq!(palette.len(), 4);
        for color in colors {
            assert!(palette.contains(&color), "{color:?} in {palette:?}");
        }
        let mut writer = GifWriter::new((8, 8), 10, palette, Dither::Bayer).unwrap();
        writer.frame(&frame).unwrap();
        let frames = decoded(&writer.finish().unwrap());
        // Bayer offsets of at most 4 never reach another of these colors.
        assert_eq!(frames[0].0, frame);
    }

    #[test]
    fn frames_keep_their_timing_and_only_changes_are_stored() {
        let (still, moved) = (gradient(0), gradient(200));
        let mut histogram = Histogram::default();
        histogram.add(&still);
        histogram.add(&moved);
        for dither in [Dither::None, Dither::Bayer, Dither::Diffusion] {
            let mut writer = GifWriter::new((64, 32), 24, histogram.palette(), dither).unwrap();
            // Two identical frames become one twice as long; then a change.
            for frame in [&still, &still, &moved] {
                writer.frame(frame).unwrap();
            }
            let bytes = writer.finish().unwrap();
            let frames = decoded(&bytes);
            assert_eq!(frames.len(), 2, "{dither:?}");
            // 24 per second in hundredths is 4, 4, 5: the first two merge.
            assert_eq!((frames[0].1, frames[1].1), (80, 50), "{dither:?}");
            assert!(largest_error(&frames[0].0, &still) < 40, "{dither:?}");
            assert!(largest_error(&frames[1].0, &moved) < 40, "{dither:?}");
        }
        // A frame that changes one pixel stores a one-pixel rectangle.
        let mut small = still.clone();
        small.put_pixel(10, 5, Rgba([255, 255, 255, 255]));
        let mut histogram = Histogram::default();
        histogram.add(&still);
        histogram.add(&small);
        let mut writer = GifWriter::new((64, 32), 10, histogram.palette(), Dither::None).unwrap();
        writer.frame(&still).unwrap();
        let one = writer.finish().unwrap().len();
        let mut writer = GifWriter::new((64, 32), 10, histogram.palette(), Dither::None).unwrap();
        writer.frame(&still).unwrap();
        writer.frame(&small).unwrap();
        let bytes = writer.finish().unwrap();
        assert!(bytes.len() < one + 40, "{} after {one}", bytes.len());
        let frames = decoded(&bytes);
        assert_eq!(frames[1].0.get_pixel(10, 5).0, [255, 255, 255, 255]);
        assert_eq!(frames[1].0.get_pixel(11, 5), frames[0].0.get_pixel(11, 5));
    }

    #[test]
    fn delays_alternate_to_average_the_rate() {
        let writer = GifWriter::new((1, 1), 24, vec![[0; 3]], Dither::None).unwrap();
        let delays: Vec<u16> = (0..6).map(|i| writer.delay(i)).collect();
        assert_eq!(delays.iter().map(|&d| u32::from(d)).sum::<u32>(), 25);
        assert!(delays.iter().all(|&d| d == 4 || d == 5), "{delays:?}");
        let thirty = GifWriter::new((1, 1), 30, vec![[0; 3]], Dither::None).unwrap();
        assert_eq!((0..3).map(|i| thirty.delay(i)).sum::<u16>(), 10);
    }
}
