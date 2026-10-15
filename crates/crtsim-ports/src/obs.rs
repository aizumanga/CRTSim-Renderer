//! Runs the OBS filter headless, through OBS itself: the plugin and its test driver, built from
//! `ports/obs` with CMake (`-DCRTSIM_OBS_TEST=ON`), draw a source's frames through the filter
//! with a look's settings and read the last back. It needs an X display, which Xvfb provides.

use anyhow::{ensure, Context, Result};
use crtsim_core::{config::Config, ports};
use image::RgbaImage;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The built plugin and its driver, with the data folder the plugin reads.
pub struct Filter {
    driver: PathBuf,
    plugin: PathBuf,
    data: PathBuf,
}

impl Filter {
    /// The plugin's build in `build`, as CMake leaves it, with its data written into `work`.
    pub fn new(build: &Path, work: &Path) -> Result<Self> {
        let data = work.join("data");
        ports::write(&data, &ports::obs::plugin_data()?)?;
        let filter = Self {
            driver: build.join("crtsim-obs-render"),
            plugin: build.join("crtsim-renderer.so"),
            data,
        };
        for built in [&filter.driver, &filter.plugin] {
            ensure!(built.exists(), "{} is not built", built.display());
        }
        Ok(filter)
    }

    /// Shows each picture for its frames, each frame lasting `frame_seconds`, through the
    /// filter set to `look`, and returns the last frame drawn. Works in `folder`, which must
    /// not exist yet.
    pub fn render(
        &self,
        look: &Config,
        steps: &[(RgbaImage, u32)],
        frame_seconds: f64,
        folder: &Path,
    ) -> Result<RgbaImage> {
        // The look's settings name its colour table beside them, which the filter reads from
        // the folder it runs in.
        ports::write(folder, &ports::obs::export(&[("look", look)])?)?;
        let mut command = Command::new(&self.driver);
        command
            .current_dir(folder)
            .arg(&self.plugin)
            .arg(&self.data)
            .arg("look.json")
            .arg("output.rgba")
            .arg(frame_seconds.to_string());
        for (index, (picture, frames)) in steps.iter().enumerate() {
            let input = format!("input-{index}.rgba");
            std::fs::write(folder.join(&input), picture.as_raw())?;
            let (width, height) = picture.dimensions();
            command.args([
                width.to_string(),
                height.to_string(),
                frames.to_string(),
                input,
            ]);
        }
        let output = command.output().context("cannot run the OBS test driver")?;
        ensure!(
            output.status.success(),
            "the OBS test driver failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let errors: Vec<_> = String::from_utf8_lossy(&output.stderr)
            .lines()
            .filter(|line| line.starts_with("error:") || line.starts_with("warning:"))
            .map(String::from)
            .collect();
        ensure!(errors.is_empty(), "OBS complained: {errors:#?}");
        let (width, height) = steps.last().context("no picture to show")?.0.dimensions();
        let mut picture =
            RgbaImage::from_raw(width, height, std::fs::read(folder.join("output.rgba"))?)
                .context("the frame read back is the wrong length")?;
        // As a host shows it: opaque, whatever alpha the last pass wrote.
        for pixel in picture.pixels_mut() {
            pixel[3] = 255;
        }
        Ok(picture)
    }
}
