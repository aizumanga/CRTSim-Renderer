@tool
class_name CRTSimEffect
extends CompositorEffect
## J. Kyle Pittman's CRTSim, from CRTSim Renderer: the composite signal and its glow, the
## shadow mask, the curved glass and its bezel, and bloom, drawn over a camera's picture.
##
## Add it to the Compositor of a WorldEnvironment or a Camera3D. Each frame it resizes the
## picture to the signal, composites it over the last frame's signal for the glow, draws it on
## the tube and blooms it, as the passes every port of CRTSim Renderer shares do, and gives it
## back to Godot before tonemapping. For the look the app shows, tonemap Linear and turn
## Godot's own glow off. A 2D game draws through it with the environment's background set to
## Canvas. CC0.

# BEGIN PARAMETERS: written by CRTSim Renderer's exporter from its settings table; edit that,
# not this. `cargo test -p crtsim-shader-ports godot` says when it is out of date, and
# CRTSIM_WRITE_GODOT_SCRIPT=1 rewrites it.
## Signal columns (0 keeps the picture's shape)
@export_range(0.0, 4096.0, 1.0) var signal_width: float = 0.0
## Signal rows (0 for the picture's own)
@export_range(0.0, 4096.0, 1.0) var signal_height: float = 480.0
## Resize to the signal
@export_enum("nearest", "Lanczos") var resize: int = 1
## Colour table
@export_enum("off", "on") var table: int = 0
## Colour table red and green
@export_enum("blended", "nearest") var table_nearest: int = 0
## Linear light
@export_enum("off", "on") var linear: int = 0
## Phase
@export_enum("stable", "A", "B", "alternating") var phase: int = 0
## Artifact pattern
@export_enum("as the public source", "flipped") var flip_artifacts: int = 0
## Glow floor, as 8-bit history holds it
@export_enum("off", "on") var glow_floor: int = 0
## Glow fades
@export_enum("by the time a frame lasts", "once a frame") var glow_per_frame: int = 0
## Interlaced fields
@export_enum("off", "on", "auto") var interlace: int = 2
## Fit on the tube
@export_enum("original", "contain", "cover", "stretch") var fit: int = 1
## Screen only, without the bezel
@export_enum("off", "on") var screen_only: int = 0
## Bezel reflection
@export_enum("as the public source", "as the screen") var reflection_as_screen: int = 0
## Glass
@export_enum("the sphere", "the mesh's facets") var faceted_glass: int = 0
## Bloom reads
@export_enum("the picture itself", "its mipmaps") var bloom_mipmaps: int = 0
## FXAA
@export_enum("off", "low", "medium", "high", "ultra") var fxaa: int = 0
## Filter mask when shrinking
@export_enum("off", "on") var mask_antialias: int = 1
## Mask columns (0 follows the signal)
@export_range(0.0, 4096.0, 1.0) var mask_columns: float = 128.0
## Mask rows (0 follows the signal)
@export_range(0.0, 4096.0, 1.0) var mask_rows: float = 224.0
## Pixel aspect
@export_range(0.1, 10.0, 0.1) var pixel_aspect: float = 1.0
## LUT strength
@export_range(0.0, 1.0, 0.01) var lut_strength: float = 1.0
## Hue (degrees)
@export_range(-180.0, 180.0, 4.0) var hue: float = 0.0
## Chroma
@export_range(0.0, 2.0, 0.02) var chroma: float = 1.0
## Saturation
@export_range(0.0, 4.0, 0.04) var saturation: float = 1.0
## Sharpness / ringing
@export_range(0.0, 4.0, 0.04) var sharpness: float = 0.8
## Color bleed
@export_range(0.0, 2.0, 0.02) var bleed: float = 0.5
## Composite artifacts
@export_range(0.0, 2.0, 0.02) var artifacts: float = 0.5
## NTSC blending
@export_range(0.0, 2.0, 0.02) var ntsc_blending: float = 0.0
## Barrel distortion
@export_range(-2.0, 2.0, 0.04) var barrel: float = -0.115
## Overscan
@export_range(0.1, 3.0, 0.03) var overscan: float = 1.0
## Tube aspect (width ÷ height)
@export_range(0.75, 2.0, 0.01) var tube_aspect: float = 1.3333334
## Edge dimming
@export_range(0.0, 2.0, 0.02) var dimming: float = 0.5
## Camera field of view
@export_range(0.05, 90.0, 0.9) var fov: float = 15.0
## Mask opacity
@export_range(0.0, 2.0, 0.02) var mask_opacity: float = 1.0
## Mask brightness
@export_range(0.0, 2.0, 0.02) var mask_brightness: float = 0.45
## Bloom amount
@export_range(0.0, 2.0, 0.02) var bloom: float = 0.25
## Bloom power
@export_range(0.1, 8.0, 0.08) var bloom_power: float = 2.0
## Bloom spread, first blur
@export_range(0.0, 0.2, 0.002) var bloom_spread: float = 0.025
## Bloom spread, second blur
@export_range(0.0, 0.2, 0.002) var bloom_spread_up: float = 0.025
## Diffuse light
@export_range(0.0, 2.0, 0.02) var diffuse: float = 0.5
## Specular light
@export_range(0.0, 5.0, 0.05) var specular: float = 0.35
## Specular power
@export_range(1.0, 200.0, 2.0) var specular_power: float = 50.0
## Rim light
@export_range(0.0, 10.0, 0.1) var rim: float = 1.0
## Edge reflection
@export_range(0.0, 2.0, 0.02) var reflection: float = 0.3
## Bezel red
@export_range(0.0, 1.0, 0.01) var bezel_r: float = 0.06
## Bezel green
@export_range(0.0, 1.0, 0.01) var bezel_g: float = 0.06
## Bezel blue
@export_range(0.0, 1.0, 0.01) var bezel_b: float = 0.06
## Backdrop red
@export_range(0.0, 1.0, 0.01) var backdrop_r: float = 0.0
## Backdrop green
@export_range(0.0, 1.0, 0.01) var backdrop_g: float = 0.0
## Backdrop blue
@export_range(0.0, 1.0, 0.01) var backdrop_b: float = 0.0
## Light X
@export_range(-1000.0, 1000.0, 20.0) var light_x: float = -10.0
## Light Y
@export_range(-1000.0, 1000.0, 20.0) var light_y: float = -5.0
## Light Z
@export_range(-1000.0, 1000.0, 20.0) var light_z: float = 10.0
## Red persistence
@export_range(0.0, 0.999, 0.01) var persistence_r: float = 0.7
## Green persistence
@export_range(0.0, 0.999, 0.01) var persistence_g: float = 0.525
## Blue persistence
@export_range(0.0, 0.999, 0.01) var persistence_b: float = 0.42
## Brightness
@export_range(-1.0, 1.0, 0.02) var brightness: float = 0.0
## TV static
@export_range(0.0, 1.0, 0.01) var static_amount: float = 0.0
## Channel cycle, seconds (0 holds the static)
@export_range(0.0, 60.0, 0.6) var static_cycle: float = 0.0

## The parameters in the order parameters.inc declares them.
const _PARAMETERS: Array[StringName] = [
	&"signal_width",
	&"signal_height",
	&"resize",
	&"table",
	&"table_nearest",
	&"linear",
	&"phase",
	&"flip_artifacts",
	&"glow_floor",
	&"glow_per_frame",
	&"interlace",
	&"fit",
	&"screen_only",
	&"reflection_as_screen",
	&"faceted_glass",
	&"bloom_mipmaps",
	&"fxaa",
	&"mask_antialias",
	&"mask_columns",
	&"mask_rows",
	&"pixel_aspect",
	&"lut_strength",
	&"hue",
	&"chroma",
	&"saturation",
	&"sharpness",
	&"bleed",
	&"artifacts",
	&"ntsc_blending",
	&"barrel",
	&"overscan",
	&"tube_aspect",
	&"dimming",
	&"fov",
	&"mask_opacity",
	&"mask_brightness",
	&"bloom",
	&"bloom_power",
	&"bloom_spread",
	&"bloom_spread_up",
	&"diffuse",
	&"specular",
	&"specular_power",
	&"rim",
	&"reflection",
	&"bezel_r",
	&"bezel_g",
	&"bezel_b",
	&"backdrop_r",
	&"backdrop_g",
	&"backdrop_b",
	&"light_x",
	&"light_y",
	&"light_z",
	&"persistence_r",
	&"persistence_g",
	&"persistence_b",
	&"brightness",
	&"static_amount",
	&"static_cycle",
]
# END PARAMETERS

## A look's colour table: its lattice's slices side by side, as the exporter writes it. None
## draws the look without one.
@export var colour_table: Texture2D

const _PASSES: Array[StringName] = [
	&"capture", &"rows", &"columns", &"prepare", &"composite", &"surface", &"halve",
	&"bloom_down", &"bloom_up", &"present",
]
## The surface's mip levels the bloom may read below it: down to the one below a sixteenth.
const _BLOOM_LEVELS := 5
const _DIR := "res://addons/crtsim_renderer/"

var _rd: RenderingDevice
var _shaders := {}
## Each pass's pipeline, by pass and the format of the framebuffer it draws into.
var _pipelines := {}
var _parameters: RID
var _point: RID
var _linear: RID
var _repeat: RID
var _images := {}
## What each view of each camera's buffers keeps from frame to frame, by the buffers' id and
## the view.
var _views := {}


## What one view keeps: its targets, its signal and the last frame's, and its frame count.
class View:
	var size := Vector2i.ZERO
	var signal_size := Vector2i.ZERO
	var linear := false
	var targets := {}
	var signals: Array[RID] = []
	var current := 0
	var has_previous := false
	var frame_count := 0
	## The two of the surface's levels the bloom's first blur reads between, and the first's
	## number.
	var bloom_level := RID()
	var bloom_below := RID()
	var bloom_level_index := 0
	## The frame it was last drawn in, so a camera that has gone is forgotten.
	var drawn := 0

	func free_targets(rd: RenderingDevice) -> void:
		for target: RID in targets.values():
			rd.free_rid(target)
		targets.clear()
		bloom_level = RID()
		bloom_below = RID()

	func free_signals(rd: RenderingDevice) -> void:
		for target in signals:
			rd.free_rid(target)
		signals.clear()


func _init() -> void:
	effect_callback_type = EFFECT_CALLBACK_TYPE_POST_TRANSPARENT
	RenderingServer.call_on_render_thread(_start)


func _notification(what: int) -> void:
	if what == NOTIFICATION_PREDELETE and _rd:
		for view: View in _views.values():
			view.free_targets(_rd)
			view.free_signals(_rd)
		for rid: RID in _pipelines.values() + _shaders.values():
			_rd.free_rid(rid)
		for rid in [_parameters, _point, _linear, _repeat]:
			if rid.is_valid():
				_rd.free_rid(rid)


func _start() -> void:
	_rd = RenderingServer.get_rendering_device()
	if not _rd:
		return
	for pass_name in _PASSES:
		var file: RDShaderFile = load(_DIR + "shaders/%s.glsl" % pass_name)
		var spirv := file.get_spirv()
		if spirv.compile_error_vertex or spirv.compile_error_fragment:
			push_error("CRTSim: %s: %s%s" % [
				pass_name, spirv.compile_error_vertex, spirv.compile_error_fragment])
			_rd = null
			return
		_shaders[pass_name] = _rd.shader_create_from_spirv(spirv, "CRTSim " + pass_name)
	_parameters = _rd.uniform_buffer_create(_parameter_bytes().size())
	_point = _sampler(RenderingDevice.SAMPLER_FILTER_NEAREST, false)
	_linear = _sampler(RenderingDevice.SAMPLER_FILTER_LINEAR, false)
	_repeat = _sampler(RenderingDevice.SAMPLER_FILTER_LINEAR, true)
	for image in [&"artifacts", &"mask", &"no-table", &"bezel-shape", &"bezel-uv",
			&"bezel-normal", &"glass"]:
		_images[image] = load(_DIR + "images/%s.png" % image)


func _sampler(filter: RenderingDevice.SamplerFilter, repeat: bool) -> RID:
	var state := RDSamplerState.new()
	state.min_filter = filter
	state.mag_filter = filter
	state.mip_filter = filter
	var mode := RenderingDevice.SAMPLER_REPEAT_MODE_REPEAT if repeat \
			else RenderingDevice.SAMPLER_REPEAT_MODE_CLAMP_TO_EDGE
	state.repeat_u = mode
	state.repeat_v = mode
	return _rd.sampler_create(state)


func _render_callback(_type: int, render_data: RenderData) -> void:
	if not _rd:
		return
	var buffers := render_data.get_render_scene_buffers() as RenderSceneBuffersRD
	if not buffers:
		return
	var size := buffers.get_internal_size()
	if size.x <= 0 or size.y <= 0:
		return
	# Godot renders on the main thread unless set otherwise, where this is the frame's own
	# length; on a rendering thread of its own, it is the frame before's.
	var seconds := (Engine.get_main_loop() as SceneTree).root.get_process_delta_time()
	var now := Engine.get_frames_drawn()
	for index in buffers.get_view_count():
		var key := [buffers.get_instance_id(), index]
		if not _views.has(key):
			_views[key] = View.new()
		_views[key].drawn = now
		_draw(buffers.get_color_layer(index), size, _views[key], seconds)
	# A view not drawn for a while belongs to buffers Godot has replaced or a camera that has
	# gone; every view in use is drawn each frame.
	for key in _views.keys():
		var view: View = _views[key]
		if now - view.drawn > 2:
			view.free_targets(_rd)
			view.free_signals(_rd)
			_views.erase(key)


## The signal's size for a picture of `size`: its rows, else the picture's; its columns, else
## as many as keep the picture's shape.
func _signal_size(size: Vector2i) -> Vector2i:
	var rows := int(signal_height) if signal_height >= 1.0 else size.y
	var columns := int(signal_width) if signal_width >= 1.0 \
			else maxi(1, roundi(float(size.x) * rows / size.y))
	return Vector2i(columns, rows)


## A target of `size` and `format` for a pass to draw into and the next to read.
func _target(size: Vector2i, format: RenderingDevice.DataFormat) -> RID:
	var spec := RDTextureFormat.new()
	spec.width = size.x
	spec.height = size.y
	spec.format = format
	spec.usage_bits = RenderingDevice.TEXTURE_USAGE_SAMPLING_BIT \
			| RenderingDevice.TEXTURE_USAGE_COLOR_ATTACHMENT_BIT
	return _rd.texture_create(spec, RDTextureView.new())


## Makes `view`'s targets for a picture of `size`, keeping its signals, and so the glow, while
## the signal's size stays.
func _shape(view: View, size: Vector2i) -> void:
	var signal_size := _signal_size(size)
	var in_linear := linear > 0.5
	if signal_size != view.signal_size:
		view.free_signals(_rd)
		var byte := RenderingDevice.DATA_FORMAT_R8G8B8A8_UNORM
		view.signals = [_target(signal_size, byte), _target(signal_size, byte)]
		view.signal_size = signal_size
		view.has_previous = false
	if size == view.size and in_linear == view.linear and not view.targets.is_empty():
		return
	view.free_targets(_rd)
	var byte := RenderingDevice.DATA_FORMAT_R8G8B8A8_UNORM
	# In linear light the surface and what follows it keep floats, as the renderer does.
	var surface := RenderingDevice.DATA_FORMAT_R16G16B16A16_SFLOAT if in_linear else byte
	var bloom := _bloom_size(size)
	view.targets = {
		&"input": _target(size, byte),
		# The picture's columns at the signal's rows, in 8-bit steps, unrounded.
		&"rows": _target(Vector2i(size.x, signal_size.y), RenderingDevice.DATA_FORMAT_R32G32B32A32_SFLOAT),
		&"picture": _target(signal_size, byte),
		&"prepared": _target(signal_size, byte),
		&"surface": _target(size, surface),
		&"bloom_down": _target(bloom, surface),
		&"bloom_up": _target(size, surface),
	}
	# The surface's mip levels below it, each half the one above, for the bloom from mipmaps.
	var level := size
	for number in range(1, _BLOOM_LEVELS + 1):
		level = Vector2i(maxi(1, level.x / 2), maxi(1, level.y / 2))
		view.targets[_level(number)] = _target(level, surface)
	view.size = size
	view.linear = in_linear


## A sixteenth of the picture each way, rounded down, as the renderer and the game size the
## bloom's first blur.
func _bloom_size(size: Vector2i) -> Vector2i:
	return Vector2i(maxi(1, size.x / 16), maxi(1, size.y / 16))


## The surface's mip level `number` below it, as `view.targets` holds it.
func _level(number: int) -> StringName:
	return StringName("level_%d" % number)


## The first of the two of the surface's mip levels the bloom's first blur reads between, for a
## picture of `size`: with the bloom from mipmaps, the one GL picks for its target; else the
## surface itself.
func _bloom_level_index(size: Vector2i) -> int:
	if bloom_mipmaps == 0:
		return 0
	var bloom := _bloom_size(size)
	var ratio := maxf(float(size.x) / bloom.x, float(size.y) / bloom.y)
	var first := 0
	while first + 1 < _BLOOM_LEVELS and ratio >= float(1 << (first + 1)):
		first += 1
	return first


## The two of the surface's mip levels the bloom's first blur reads between: the surface halved
## level by level down to them, as GL makes mipmaps, with the bloom from mipmaps; else the
## surface.
func _halve(view: View) -> void:
	var above: RID = view.targets[&"surface"]
	view.bloom_level = above
	view.bloom_below = above
	if bloom_mipmaps == 0:
		return
	for number in range(1, view.bloom_level_index + 2):
		var level: RID = view.targets[_level(number)]
		_run(&"halve", level, above, view)
		above = level
		if number == view.bloom_level_index:
			view.bloom_level = above
	view.bloom_below = above


## Runs one frame of the CRT over `color`, the camera's picture, of `size`, and draws it there.
func _draw(color: RID, size: Vector2i, view: View, seconds: float) -> void:
	_shape(view, size)
	view.bloom_level_index = _bloom_level_index(size)
	var t := view.targets
	var bytes := _parameter_bytes(size, view, seconds)
	_rd.buffer_update(_parameters, 0, bytes.size(), bytes)
	_run(&"capture", t[&"input"], color, view)
	var picture: RID = t[&"input"]
	if view.signal_size != size:
		_run(&"rows", t[&"rows"], picture, view)
		_run(&"columns", t[&"picture"], t[&"rows"], view)
		picture = t[&"picture"]
	_run(&"prepare", t[&"prepared"], picture, view)
	var signal_now: RID = view.signals[view.current]
	_run(&"composite", signal_now, t[&"prepared"], view)
	_run(&"surface", t[&"surface"], signal_now, view)
	_halve(view)
	_run(&"bloom_down", t[&"bloom_down"], t[&"surface"], view)
	_run(&"bloom_up", t[&"bloom_up"], t[&"bloom_down"], view)
	_run(&"present", color, t[&"bloom_up"], view)
	view.current = 1 - view.current
	view.has_previous = true
	view.frame_count += 1


## The block every pass reads: the sizes, the frame, and every parameter in the order
## parameters.inc declares them, padded to whole vectors.
func _parameter_bytes(size := Vector2i.ONE, view: View = null, seconds := 0.0) -> PackedByteArray:
	var signal_size := view.signal_size if view else Vector2i.ONE
	var values := PackedFloat32Array([
		size.x, size.y, signal_size.x, signal_size.y, size.x, size.y,
		view.frame_count if view else 0, seconds * 60.0,
		1.0 if view and view.has_previous else 0.0,
		view.bloom_level_index if view else 0,
	])
	for parameter in _PARAMETERS:
		values.append(get(parameter))
	# A look whose table is missing is drawn without one.
	if not colour_table:
		values[10 + _PARAMETERS.find(&"table")] = 0.0
	while values.size() % 4:
		values.append(0.0)
	return values.to_byte_array()


func _run(pass_name: StringName, target: RID, input: RID, view: View) -> void:
	var framebuffer := FramebufferCacheRD.get_cache_multipass([target], [], 1)
	var format := _rd.framebuffer_get_format(framebuffer)
	var key := [pass_name, format]
	if not _pipelines.has(key):
		var blend := RDPipelineColorBlendState.new()
		blend.attachments.append(RDPipelineColorBlendStateAttachment.new())
		_pipelines[key] = _rd.render_pipeline_create(_shaders[pass_name], format,
				RenderingDevice.INVALID_FORMAT_ID, RenderingDevice.RENDER_PRIMITIVE_TRIANGLES,
				RDPipelineRasterizationState.new(), RDPipelineMultisampleState.new(),
				RDPipelineDepthStencilState.new(), blend)
	var table: Texture2D = colour_table if colour_table else _images[&"no-table"]
	var previous: RID = view.signals[1 - view.current]
	var uniforms: Array[RDUniform] = [_buffer(0, _parameters)]
	for binding: Array in [
		[1, _linear, input],
		[2, _linear, view.targets[&"surface"]],
		[3, _point, previous],
		[4, _point, _rd_texture(table)],
		[5, _point, _rd_texture(_images[&"artifacts"])],
		[6, _repeat, _rd_texture(_images[&"mask"])],
		[7, _point, _rd_texture(_images[&"bezel-shape"])],
		[8, _linear, _rd_texture(_images[&"bezel-shape"])],
		[9, _point, _rd_texture(_images[&"bezel-uv"])],
		[10, _point, _rd_texture(_images[&"bezel-normal"])],
		[11, _point, _rd_texture(_images[&"glass"])],
		[12, _linear, view.bloom_level if view.bloom_level.is_valid() else view.targets[&"surface"]],
		[13, _linear, view.bloom_below if view.bloom_below.is_valid() else view.targets[&"surface"]],
	]:
		# A pass cannot read the target it draws into, which it never needs: the surface pass
		# reads no surface.
		var texture: RID = binding[2]
		if texture == target:
			texture = _rd_texture(_images[&"no-table"])
		uniforms.append(_sampled(binding[0], binding[1], texture))
	var set := UniformSetCacheRD.get_cache(_shaders[pass_name], 0, uniforms)
	# Every pass draws every pixel of its target.
	var list := _rd.draw_list_begin(framebuffer, RenderingDevice.DRAW_IGNORE_COLOR_ALL)
	_rd.draw_list_bind_render_pipeline(list, _pipelines[key])
	_rd.draw_list_bind_uniform_set(list, set, 0)
	_rd.draw_list_draw(list, false, 1, 3)
	_rd.draw_list_end()


func _rd_texture(texture: Texture2D) -> RID:
	return RenderingServer.texture_get_rd_texture(texture.get_rid())


func _buffer(binding: int, buffer: RID) -> RDUniform:
	var uniform := RDUniform.new()
	uniform.uniform_type = RenderingDevice.UNIFORM_TYPE_UNIFORM_BUFFER
	uniform.binding = binding
	uniform.add_id(buffer)
	return uniform


func _sampled(binding: int, sampler: RID, texture: RID) -> RDUniform:
	var uniform := RDUniform.new()
	uniform.uniform_type = RenderingDevice.UNIFORM_TYPE_SAMPLER_WITH_TEXTURE
	uniform.binding = binding
	uniform.add_id(sampler)
	uniform.add_id(texture)
	return uniform
