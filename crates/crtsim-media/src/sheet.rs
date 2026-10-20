//! A sprite sheet with its JSON, as Aseprite and TexturePacker write them, opened as an
//! animation: each frame cut from the sheet onto its canvas, with how long it shows. The
//! picture is read whole as a still is, colour profile and all, and every pixel kept.
use anyhow::{bail, ensure, Context, Result};
use crtsim_core::{config, input};
use image::{GenericImageView, RgbaImage};
use serde::{de, Deserialize};
use std::{fmt, path::Path};

/// The largest JSON read as a sprite sheet's.
const MOST_JSON: u64 = 64 * 1024 * 1024;
/// How long a frame shows when the JSON does not say, as TexturePacker's does not.
const UNTIMED_MS: u32 = 100;

/// A sprite sheet's picture and its frames, in order.
pub struct Sheet {
    picture: RgbaImage,
    frames: Vec<Frame>,
    /// How long each frame shows, in milliseconds.
    delays: Vec<u32>,
    /// The canvas every frame is drawn onto.
    pub size: (u32, u32),
}

/// Where a frame is in the sheet and on its canvas.
struct Frame {
    /// x, y, width and height in the sheet.
    cut: [u32; 4],
    /// Where its top left corner goes on the canvas: trimmed frames are placed within it.
    at: (u32, u32),
}

impl fmt::Debug for Sheet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Sheet")
            .field("size", &self.size)
            .field("frames", &self.frames.len())
            .finish()
    }
}

/// The JSON, as both writers' "array" and "hash" layouts have it.
#[derive(Deserialize)]
struct Json {
    frames: Entries,
    meta: Meta,
}

#[derive(Deserialize)]
struct Meta {
    /// The picture's file, beside the JSON.
    image: String,
}

#[derive(Deserialize)]
struct Entry {
    frame: Rect,
    #[serde(default)]
    rotated: bool,
    #[serde(rename = "spriteSourceSize")]
    placed: Option<Rect>,
    #[serde(rename = "sourceSize")]
    canvas: Option<Size>,
    duration: Option<u32>,
}

#[derive(Clone, Copy, Deserialize)]
struct Rect {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
}

#[derive(Clone, Copy, Deserialize, PartialEq, Eq)]
struct Size {
    w: u32,
    h: u32,
}

/// The frames as a list, or as an object of named frames, kept in the order they are written.
struct Entries(Vec<Entry>);

impl<'de> Deserialize<'de> for Entries {
    fn deserialize<D: de::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> de::Visitor<'de> for Visitor {
            type Value = Entries;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a list or an object of frames")
            }

            fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<Entries, A::Error> {
                let mut entries = vec![];
                while let Some(entry) = seq.next_element()? {
                    entries.push(entry);
                }
                Ok(Entries(entries))
            }

            fn visit_map<A: de::MapAccess<'de>>(self, mut map: A) -> Result<Entries, A::Error> {
                let mut entries = vec![];
                while let Some((_, entry)) = map.next_entry::<de::IgnoredAny, Entry>()? {
                    entries.push(entry);
                }
                Ok(Entries(entries))
            }
        }
        deserializer.deserialize_any(Visitor)
    }
}

fn parse(path: &Path) -> Result<Json> {
    let extension = path.extension().and_then(|e| e.to_str());
    ensure!(
        extension.is_some_and(|e| e.eq_ignore_ascii_case("json")),
        "Not a sprite sheet's JSON"
    );
    let size = std::fs::metadata(path)
        .context("Cannot open sprite sheet")?
        .len();
    ensure!(size <= MOST_JSON, "The sprite sheet's JSON is too large");
    let bytes = std::fs::read(path).context("Cannot open sprite sheet")?;
    let json: Json = serde_json::from_slice(&bytes).context("Not a sprite sheet's JSON")?;
    ensure!(!json.frames.0.is_empty(), "The sprite sheet has no frames");
    Ok(json)
}

/// Whether `path` is a sprite sheet's JSON. Reads the JSON, not the picture.
pub(crate) fn detect(path: &Path) -> bool {
    parse(path).is_ok()
}

impl Sheet {
    /// Reads the sprite sheet whose JSON is at `path`, and its picture.
    pub fn read(path: &Path) -> Result<Self> {
        let json = parse(path)?;
        let picture_path = path
            .parent()
            .unwrap_or(Path::new(""))
            .join(&json.meta.image);
        let picture = input::read(&picture_path, input::Profiles::Convert)
            .with_context(|| format!("Cannot read the sprite sheet {}", picture_path.display()))?
            .image;
        let first = &json.frames.0[0];
        let canvas = first.canvas.unwrap_or(Size {
            w: first.frame.w,
            h: first.frame.h,
        });
        let size = (canvas.w, canvas.h);
        config::validate_size(size)?;
        let (mut frames, mut delays) = (vec![], vec![]);
        for (n, entry) in json.frames.0.iter().enumerate() {
            let number = n + 1;
            if entry.rotated {
                bail!("Frame {number} is rotated in the sheet, which is not supported");
            }
            if entry.canvas.is_some_and(|c| c != canvas) {
                bail!("Frame {number} is not the size of the first");
            }
            let Rect { x, y, w, h } = entry.frame;
            let (right, bottom) = (x.checked_add(w), y.checked_add(h));
            ensure!(
                right.is_some_and(|r| r <= picture.width())
                    && bottom.is_some_and(|b| b <= picture.height()),
                "Frame {number} lies outside the sheet"
            );
            let placed = entry.placed.unwrap_or(Rect { x: 0, y: 0, w, h });
            ensure!(
                placed.w == w
                    && placed.h == h
                    && u64::from(placed.x) + u64::from(w) <= u64::from(canvas.w)
                    && u64::from(placed.y) + u64::from(h) <= u64::from(canvas.h),
                "Frame {number} lies outside its canvas"
            );
            frames.push(Frame {
                cut: [x, y, w, h],
                at: (placed.x, placed.y),
            });
            delays.push(entry.duration.unwrap_or(UNTIMED_MS).max(1));
        }
        Ok(Self {
            picture,
            frames,
            delays,
            size,
        })
    }

    /// How long each frame shows, in milliseconds.
    pub fn delays(&self) -> &[u32] {
        &self.delays
    }

    /// Frame `n`, counting from 0, on its canvas; transparent where the sheet trimmed it.
    pub fn frame(&self, n: usize) -> RgbaImage {
        let mut canvas = RgbaImage::new(self.size.0, self.size.1);
        if let Some(frame) = self.frames.get(n) {
            let [x, y, w, h] = frame.cut;
            let cut = self.picture.view(x, y, w, h);
            for (cx, cy, pixel) in cut.pixels() {
                canvas.put_pixel(frame.at.0 + cx, frame.at.1 + cy, pixel);
            }
        }
        canvas
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sheet of three 4x2 frames in a row, each a different red, and its JSON as Aseprite
    /// writes it in `layout`, with the last frame trimmed to its right half.
    fn sheet(directory: &Path, layout: &str) -> std::path::PathBuf {
        let mut picture = RgbaImage::new(10, 2);
        for x in 0..10 {
            for y in 0..2 {
                picture.put_pixel(x, y, image::Rgba([(x / 4 * 100) as u8, y as u8, 0, 255]));
            }
        }
        picture.save(directory.join("walk.png")).unwrap();
        let entry = |x: u32, w: u32, placed_x: u32, ms: u32| {
            format!(
                r#""frame": {{ "x": {x}, "y": 0, "w": {w}, "h": 2 }}, "rotated": false,
                "trimmed": {}, "spriteSourceSize": {{ "x": {placed_x}, "y": 0, "w": {w}, "h": 2 }},
                "sourceSize": {{ "w": 4, "h": 2 }}, "duration": {ms}"#,
                w < 4
            )
        };
        let entries = [entry(0, 4, 0, 100), entry(4, 4, 0, 150), entry(8, 2, 2, 50)];
        let frames = match layout {
            "array" => format!(
                "[{}]",
                entries
                    .iter()
                    .enumerate()
                    .map(|(n, e)| format!(r#"{{ "filename": "walk {n}.aseprite", {e} }}"#))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            _ => format!(
                "{{{}}}",
                // Named so that sorting the names would put them out of order.
                ["walk 9", "walk 10", "walk 11"]
                    .iter()
                    .zip(&entries)
                    .map(|(name, e)| format!(r#""{name}.aseprite": {{ {e} }}"#))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
        };
        let json = format!(
            r#"{{ "frames": {frames}, "meta": {{ "app": "https://www.aseprite.org/",
            "image": "walk.png", "format": "RGBA8888", "size": {{ "w": 10, "h": 2 }}, "scale": "1" }} }}"#
        );
        let path = directory.join(format!("walk-{layout}.json"));
        std::fs::write(&path, json).unwrap();
        path
    }

    #[test]
    fn frames_are_cut_from_the_sheet_in_order_with_their_durations() {
        let directory = tempfile::tempdir().unwrap();
        for layout in ["array", "hash"] {
            let path = sheet(directory.path(), layout);
            assert!(detect(&path), "{layout}");
            let sheet = Sheet::read(&path).unwrap();
            assert_eq!(sheet.size, (4, 2));
            assert_eq!(sheet.delays(), [100, 150, 50], "{layout}");
            assert_eq!(sheet.frame(1).get_pixel(0, 1).0, [100, 1, 0, 255]);
            // The trimmed frame sits on the right of a transparent canvas.
            let last = sheet.frame(2);
            assert_eq!(last.get_pixel(0, 0).0, [0; 4]);
            assert_eq!(last.get_pixel(3, 0).0, [200, 0, 0, 255]);
        }
    }

    #[test]
    fn other_json_and_broken_sheets_are_refused() {
        let directory = tempfile::tempdir().unwrap();
        let preset = directory.path().join("look.json");
        std::fs::write(
            &preset,
            serde_json::to_vec(&crtsim_core::config::Config::general()).unwrap(),
        )
        .unwrap();
        assert!(!detect(&preset));
        let path = sheet(directory.path(), "array");
        let outside = std::fs::read_to_string(&path)
            .unwrap()
            .replace(r#""x": 8"#, r#""x": 9"#);
        std::fs::write(&path, outside).unwrap();
        assert!(detect(&path), "the JSON alone looks right");
        let error = Sheet::read(&path).unwrap_err().to_string();
        assert!(error.contains("Frame 3 lies outside the sheet"), "{error}");
    }
}
