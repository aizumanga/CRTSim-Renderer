//! Runs the RetroArch port in RetroArch itself: its built-in image viewer shows a picture as
//! the core's frame, through a look's preset, for a number of frames, then RetroArch saves a
//! screenshot of what it drew with the shader (`video_gpu_screenshot`) and quits. It needs an
//! X display, which Xvfb provides, and draws with glcore on llvmpipe or Vulkan on lavapipe.

use anyhow::{bail, ensure, Context, Result};
use crtsim_core::config::Config;
use crtsim_shader_ports as ports;
use image::{Rgba, RgbaImage};
use std::path::{Path, PathBuf};
use std::process::Command;

/// The video driver RetroArch draws with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Driver {
    Glcore,
    Vulkan,
}

impl Driver {
    pub fn name(self) -> &'static str {
        match self {
            Self::Glcore => "glcore",
            Self::Vulkan => "vulkan",
        }
    }
}

/// Everything RetroArch would otherwise draw over the picture, scale it by, or change between
/// frames, turned off; the window the size of the screenshot, and the picture over all of it
/// (aspect ratio 24 is Full).
const CONFIG: &str = r#"video_driver = "{driver}"
video_fullscreen = "false"
video_windowed_fullscreen = "false"
video_window_custom_size_enable = "true"
video_windowed_position_width = "{width}"
video_windowed_position_height = "{height}"
video_window_show_decorations = "false"
aspect_ratio_index = "24"
video_scale_integer = "false"
video_crop_overscan = "false"
video_allow_rotate = "false"
video_rotation = "0"
video_smooth = "false"
video_threaded = "false"
video_vsync = "false"
video_frame_delay = "0"
video_black_frame_insertion = "0"
video_hdr_enable = "false"
video_shader_enable = "true"
video_shader_watch_files = "false"
video_gpu_screenshot = "true"
video_font_enable = "false"
menu_enable_widgets = "false"
menu_driver = "rgui"
input_overlay_enable = "false"
fps_show = "false"
statistics_show = "false"
builtin_imageviewer_enable = "true"
audio_driver = "null"
audio_enable = "false"
microphone_enable = "false"
midi_driver = "null"
input_joypad_driver = "null"
pause_nonactive = "false"
config_save_on_exit = "false"
auto_shaders_enable = "false"
auto_overrides_enable = "false"
auto_remaps_enable = "false"
history_list_enable = "false"
savestate_auto_load = "false"
rewind_enable = "false"
run_ahead_enabled = "false"
ui_companion_enable = "false"
screenshot_directory = "{folder}"
"#;

/// How a run of RetroArch counts the frames a preset draws, as `Frontend::new` measures it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Counting {
    /// The frames drawn for the screenshot past the core's last, with its picture: RetroArch
    /// draws the last frame again as it starts a GPU screenshot and again to read it back.
    /// Vulkan also rebuilds the passes after the first frame, as it rebuilds the swapchain, so
    /// their feedback starts again from the second, which counts here as drawn for nothing.
    pub extra: u32,
    /// `FrameCount` in the first frame whose feedback the screenshot carries.
    pub first: u32,
}

/// RetroArch, run on a look's preset in a folder of its own.
pub struct Frontend {
    retroarch: PathBuf,
    driver: Driver,
    version: (u32, u32, u32),
    counting: Counting,
}

impl Frontend {
    /// RetroArch at `retroarch`, drawing with `driver`, which it first runs on this crate's
    /// feedback fixture in `work`, a new folder, to count how it draws frames.
    pub fn new(retroarch: &Path, driver: Driver, work: &Path) -> Result<Self> {
        let output = Command::new(retroarch)
            .arg("--version")
            .output()
            .with_context(|| format!("cannot run {}", retroarch.display()))?;
        let printed = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let version = printed
            .lines()
            .find_map(|line| line.trim().strip_prefix("Version: "))
            .and_then(|version| {
                let mut numbers = version
                    .split(|c: char| !c.is_ascii_digit())
                    .map(|n| n.parse().ok());
                Some((numbers.next()??, numbers.next()??, numbers.next()??))
            })
            .with_context(|| format!("{} printed no version:\n{printed}", retroarch.display()))?;
        // From 1.20 a preset is told how long each frame lasted, as measured: a run drawing
        // as fast as it can decays the glow by its own pace, not by a still's ticks.
        ensure!(
            version < (1, 20, 0),
            "RetroArch {version:?} decays the glow by measured time; check with one before 1.20"
        );
        let mut frontend = Self {
            retroarch: retroarch.into(),
            driver,
            version,
            counting: Counting { extra: 0, first: 0 },
        };
        frontend.counting = frontend.count(work)?;
        Ok(frontend)
    }

    pub fn version(&self) -> (u32, u32, u32) {
        self.version
    }

    pub fn counting(&self) -> Counting {
        self.counting
    }

    /// Runs the feedback fixture, which adds 16 to green each frame on top of its last and
    /// writes `FrameCount` to blue, for one core frame.
    fn count(&self, work: &Path) -> Result<Counting> {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        std::fs::create_dir(work)
            .with_context(|| format!("{} must be a new folder", work.display()))?;
        for file in ["feedback.slangp", "accumulate.slang", "present.slang"] {
            std::fs::copy(fixture.join(file), work.join(file))?;
        }
        let picture = RgbaImage::from_pixel(16, 8, Rgba([0, 0, 0, 255]));
        let drawn = self.run(work, "feedback.slangp", &picture, 1, (16, 8))?;
        let pixel = drawn.get_pixel(0, 0);
        let (green, last) = (u32::from(pixel[1]), u32::from(pixel[2]));
        ensure!(
            green % 16 == 0 && green >= 16 && last + 1 >= green / 16,
            "RetroArch drew the feedback fixture as {pixel:?}"
        );
        Ok(Counting {
            extra: green / 16 - 1,
            first: last + 1 - green / 16,
        })
    }

    /// Shows `picture` through `look`'s preset until the preset has drawn `frames` frames, in
    /// a window of `size`, and returns the last as RetroArch saved it. RetroArch before 1.20
    /// has no `OriginalAspect`, so the preset tells the port the picture's shape, `aspect`.
    /// Works in `folder`, which must not exist yet.
    pub fn render(
        &self,
        look: &Config,
        picture: &RgbaImage,
        frames: u32,
        size: (u32, u32),
        aspect: f32,
        folder: &Path,
    ) -> Result<RgbaImage> {
        ensure!(
            frames > self.counting.extra,
            "RetroArch's {} draws {} frames for a screenshot, more than {frames}",
            self.driver.name(),
            self.counting.extra + 1
        );
        let mut files = ports::retroarch::export(&[("look", look)])?;
        let preset = files
            .iter_mut()
            .find(|file| file.path == "look.slangp")
            .context("the export has no look.slangp")?;
        preset.bytes = with_aspect(&String::from_utf8(preset.bytes.clone())?, aspect)?.into();
        ports::write(folder, &files)?;
        self.run(
            folder,
            "look.slangp",
            picture,
            frames - self.counting.extra,
            size,
        )
    }

    /// Runs RetroArch in `folder` on `preset` there, showing `picture` for `core_frames`
    /// frames in a window of `size`, and returns its screenshot.
    fn run(
        &self,
        folder: &Path,
        preset: &str,
        picture: &RgbaImage,
        core_frames: u32,
        (width, height): (u32, u32),
    ) -> Result<RgbaImage> {
        picture.save(folder.join("picture.png"))?;
        let home = folder.join("home");
        std::fs::create_dir(&home)?;
        let config = CONFIG
            .replace("{driver}", self.driver.name())
            .replace("{width}", &width.to_string())
            .replace("{height}", &height.to_string())
            .replace("{folder}", &folder.display().to_string());
        std::fs::write(folder.join("retroarch.cfg"), config)?;
        let screenshot = folder.join("screenshot.png");
        let output = Command::new(&self.retroarch)
            .current_dir(folder)
            .env("HOME", &home)
            .env("XDG_CONFIG_HOME", home.join(".config"))
            .arg("--verbose")
            .arg("--config")
            .arg(folder.join("retroarch.cfg"))
            .arg("--set-shader")
            .arg(folder.join(preset))
            .arg(format!("--max-frames={core_frames}"))
            .arg("--max-frames-ss")
            .arg(format!("--max-frames-ss-path={}", screenshot.display()))
            .arg(folder.join("picture.png"))
            .output()
            .context("cannot run RetroArch")?;
        let log = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        std::fs::write(folder.join("retroarch.log"), &log)?;
        ensure!(output.status.success(), "RetroArch failed:\n{log}");
        // RetroArch draws without the preset when it cannot load it, and says so only in its
        // log: every pass must have compiled, and nothing gone wrong since.
        let errors: Vec<_> = log
            .lines()
            .filter(|line| line.contains("[ERROR]"))
            .filter(|line| !line.contains("[MIDI]") && !line.contains("[Wayland]"))
            .collect();
        ensure!(errors.is_empty(), "RetroArch complained: {errors:#?}");
        let passes = passes(&std::fs::read_to_string(folder.join(preset))?)?;
        let compiled = log
            .lines()
            .filter(|line| line.contains("[slang]: Compiling shader"))
            .count();
        ensure!(
            compiled >= passes,
            "RetroArch compiled {compiled} of the preset's {passes} shaders:\n{log}"
        );
        let picture = image::open(&screenshot)
            .with_context(|| format!("RetroArch saved no screenshot:\n{log}"))?
            .to_rgba8();
        ensure!(
            picture.dimensions() == (width, height),
            "RetroArch drew {:?}, not {width}x{height}",
            picture.dimensions()
        );
        Ok(picture)
    }
}

/// How many passes `preset` has.
fn passes(preset: &str) -> Result<usize> {
    preset
        .lines()
        .find_map(|line| line.strip_prefix("shaders = ")?.trim().parse().ok())
        .context("the preset says no number of shaders")
}

/// `preset` with its picture aspect set to `aspect`.
fn with_aspect(preset: &str, aspect: f32) -> Result<String> {
    let mut found = false;
    let lines: Vec<String> = preset
        .lines()
        .map(|line| {
            if line.starts_with("CRTSIM_ASPECT = ") {
                found = true;
                format!("CRTSIM_ASPECT = {aspect}")
            } else {
                line.into()
            }
        })
        .collect();
    if !found {
        bail!("the preset sets no CRTSIM_ASPECT:\n{preset}");
    }
    Ok(lines.join("\n") + "\n")
}
