//! Runs the Godot port in Godot itself: a project holding the exported add-on, whose camera
//! shows each picture for its frames through a look's `CRTSimEffect`, then saves the last
//! frame drawn. Godot draws with Vulkan, which software Vulkan provides, in a window, which
//! Xvfb provides.

use anyhow::{ensure, Context, Result};
use crtsim_core::config::Config;
use crtsim_shader_ports as ports;
use image::RgbaImage;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The project's settings: Forward+ on Vulkan, nothing between the camera's picture and the
/// window but the effect, and a window of the picture's size. The editor imports the add-on
/// one file at a time: importing on worker threads, it can touch the scene tree from one and
/// crash, as Godot 4.7.2 did now and then on CI's runners.
const PROJECT: &str = r#"config_version=5

[application]
config/name="CRTSim Renderer parity"
run/main_scene="res://main.tscn"

[editor]

import/use_multiple_threads=false

[display]
window/size/viewport_width={width}
window/size/viewport_height={height}
window/size/resizable=false

[rendering]
renderer/rendering_method="forward_plus"
anti_aliasing/quality/msaa_3d=0
"#;

/// The scene: a camera whose environment draws the canvas as its background, with no
/// tonemapping, and the look's effect in its compositor; and the picture, on the canvas.
const SCENE: &str = r#"[gd_scene load_steps=5 format=3]

[ext_resource type="Script" path="res://drive.gd" id="1"]
[ext_resource type="CompositorEffect" path="res://addons/crtsim_renderer/looks/look.tres" id="2"]

[sub_resource type="Environment" id="environment"]
background_mode = 3
tonemap_mode = 0

[sub_resource type="Compositor" id="compositor"]
compositor_effects = Array[CompositorEffect]([ExtResource("2")])

[node name="Main" type="Node"]
script = ExtResource("1")

[node name="Camera" type="Camera3D" parent="."]

[node name="Environment" type="WorldEnvironment" parent="."]
environment = SubResource("environment")
compositor = SubResource("compositor")

[node name="Picture" type="TextureRect" parent="."]
texture_filter = 1
"#;

/// Shows each step's picture for its frames, then saves the last frame drawn and quits.
const DRIVE: &str = r#"extends Node

var steps := []
var frame := 0


func _ready() -> void:
	steps = JSON.parse_string(FileAccess.get_file_as_string("res://steps.json"))
	_show(0)


## Shows step `index`'s picture, raw RGBA bytes of its size.
func _show(index: int) -> void:
	var step: Dictionary = steps[index]
	var bytes := FileAccess.get_file_as_bytes("res://" + step.file)
	var image := Image.create_from_data(step.width, step.height, false, Image.FORMAT_RGBA8, bytes)
	$Picture.texture = ImageTexture.create_from_image(image)
	$Picture.size = Vector2(step.width, step.height)


func _process(_delta: float) -> void:
	frame += 1
	var shown := 0
	for index in steps.size():
		shown += int(steps[index].frames)
		if frame <= shown:
			if frame == shown - int(steps[index].frames) + 1:
				_show(index)
			break
	if frame == shown and frame >= _total():
		await RenderingServer.frame_post_draw
		get_viewport().get_texture().get_image().save_png("res://output.png")
		get_tree().quit()


func _total() -> int:
	var total := 0
	for step in steps:
		total += int(step.frames)
	return total
"#;

/// The Godot editor binary every project runs in.
pub struct Engine {
    godot: PathBuf,
}

impl Engine {
    pub fn new(godot: &Path) -> Result<Self> {
        ensure!(godot.exists(), "{} is not Godot", godot.display());
        Ok(Self {
            godot: godot.into(),
        })
    }

    /// Shows each picture for its frames, each frame lasting `frame_seconds`, through the
    /// effect set to `look`, and returns the last frame drawn. Works in `folder`, a project of
    /// its own, which must not exist yet.
    pub fn render(
        &self,
        look: &Config,
        steps: &[(RgbaImage, u32)],
        frame_seconds: f64,
        folder: &Path,
    ) -> Result<RgbaImage> {
        let (width, height) = steps.last().context("no picture to show")?.0.dimensions();
        let mut files = ports::export(ports::Host::Godot, &[("look", look)])?;
        let project = PROJECT
            .replace("{width}", &width.to_string())
            .replace("{height}", &height.to_string());
        let mut listed = vec![];
        for (index, (picture, frames)) in steps.iter().enumerate() {
            let file = format!("input-{index}.rgba");
            listed.push(format!(
                "{{\"file\": \"{file}\", \"width\": {}, \"height\": {}, \"frames\": {frames}}}",
                picture.width(),
                picture.height()
            ));
            files.push(ports::File {
                path: file,
                bytes: picture.as_raw().clone(),
            });
        }
        for (path, text) in [
            ("project.godot", project),
            ("main.tscn", SCENE.into()),
            ("drive.gd", DRIVE.into()),
            ("steps.json", format!("[{}]", listed.join(", "))),
        ] {
            files.push(ports::File {
                path: path.into(),
                bytes: text.into_bytes(),
            });
        }
        ports::write(folder, &files)?;
        // The add-on's shaders and images are imported before the project can run.
        self.run(folder, &["--headless", "--import"])?;
        let fps = (1. / frame_seconds).round().to_string();
        let output = self.run(
            folder,
            &[
                "--audio-driver",
                "Dummy",
                "--rendering-driver",
                "vulkan",
                "--fixed-fps",
                &fps,
            ],
        )?;
        let errors: Vec<_> = output
            .lines()
            .filter(|line| line.contains("ERROR") || line.contains("SCRIPT ERROR"))
            .filter(|line| !line.contains("audio") && !line.contains("ALSA"))
            .map(String::from)
            .collect();
        ensure!(errors.is_empty(), "Godot complained: {errors:#?}\n{output}");
        let picture = image::open(folder.join("output.png"))
            .with_context(|| format!("Godot drew no frame:\n{output}"))?
            .to_rgba8();
        ensure!(
            picture.dimensions() == (width, height),
            "Godot drew {:?}, not {width}x{height}",
            picture.dimensions()
        );
        Ok(picture)
    }

    /// Runs Godot on the project in `folder` with `args`, returning what it printed.
    fn run(&self, folder: &Path, args: &[&str]) -> Result<String> {
        let output = Command::new(&self.godot)
            .args(args)
            .arg("--path")
            .arg(folder)
            .output()
            .context("cannot run Godot")?;
        let printed = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        ensure!(output.status.success(), "Godot failed:\n{printed}");
        Ok(printed)
    }
}
