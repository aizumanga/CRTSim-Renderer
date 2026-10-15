/* CC0. CRTSim Renderer's CRT as an OBS filter: the passes every port shares, run over a
 * source's picture once a frame, keeping the signal from one frame to the next for the glow,
 * which OBS's own shader filters cannot. A port of J. Kyle Pittman's CRTSim, from
 * https://github.com/aizumanga/CRTSim-Renderer.
 *
 * Each frame the source is drawn into a texture, resized to the signal when the signal's size
 * differs, then prepared, composited over the last frame's signal, drawn on the glass and the
 * bezel, bloomed and presented, each pass a technique of crtsim.effect. The result is kept, so
 * a source shown in several places in one frame advances its glow only once. */

#include <obs-module.h>
#include <graphics/image-file.h>
#include <graphics/vec2.h>
#include <util/bmem.h>
#include <util/dstr.h>
#include <util/platform.h>
#include <math.h>
#include <string.h>

#include "port.h"

OBS_DECLARE_MODULE()

MODULE_EXPORT const char *obs_module_description(void)
{
	return "A CRT: the composite signal and its glow, the shadow mask, the curved glass and its "
	       "bezel, and bloom.";
}

/* The setting holding a look's colour table, an image of its lattice's slices side by side. */
#define TABLE_FILE "table_file"
#define LOOK_FILE "look_file"

struct crtsim {
	obs_source_t *context;
	gs_effect_t *effect;
	gs_eparam_t *parameters[CRTSIM_PARAMETER_COUNT];
	float values[CRTSIM_PARAMETER_COUNT];
	gs_texture_t *images[CRTSIM_IMAGE_COUNT];
	/* The look's colour table, or NULL for none. */
	gs_texture_t *table;
	struct dstr table_path;

	gs_texrender_t *input;
	gs_texrender_t *rows;
	gs_texrender_t *picture;
	gs_texrender_t *prepared;
	/* This frame's signal and the last one's, in turn. */
	gs_texrender_t *signals[2];
	gs_texrender_t *surface;
	gs_texrender_t *bloom_down;
	gs_texrender_t *bloom_up;
	gs_texrender_t *output;
	/* Whether the surface, the bloom and the output keep floats, for linear light. */
	bool linear;

	int current;
	/* Whether signals[!current] holds the last frame's signal at this size. */
	bool has_previous;
	/* The source's picture, which is also the output's size, and the signal's. */
	uint32_t cx, cy;
	uint32_t signal_cx, signal_cy;
	int frame_count;
	float frame_seconds;
	/* Whether a frame has begun since the last simulated one. */
	bool ticked;
};

static const char *crtsim_name(void *unused)
{
	UNUSED_PARAMETER(unused);
	return "CRTSim Renderer";
}

/* `image`, w×h texels of 4 bytes, at half size, each texel the rounded average of the 2×2 it
 * replaces, as the renderer builds the mask's mipmaps. A side already one texel stays one. */
static uint8_t *halve(const uint8_t *image, uint32_t w, uint32_t h)
{
	uint32_t hw = w > 1 ? w / 2 : 1, hh = h > 1 ? h / 2 : 1;
	uint8_t *half = bmalloc((size_t)hw * hh * 4);
	for (uint32_t y = 0; y < hh; y++) {
		for (uint32_t x = 0; x < hw; x++) {
			uint32_t left = 2 * x, top = 2 * y;
			uint32_t right = left + 1 < w ? left + 1 : w - 1;
			uint32_t bottom = top + 1 < h ? top + 1 : h - 1;
			for (int c = 0; c < 4; c++) {
				uint32_t sum = image[(top * w + left) * 4 + c] +
					       image[(top * w + right) * 4 + c] +
					       image[(bottom * w + left) * 4 + c] +
					       image[(bottom * w + right) * 4 + c];
				half[(y * hw + x) * 4 + c] = (uint8_t)((sum + 2) / 4);
			}
		}
	}
	return half;
}

#define MAX_LEVELS 16

/* An image as it is stored, its alpha straight: the bezel's and glass's carry data in it.
 * Mipmapped, it has every level down to one texel. */
static gs_texture_t *load_image(const char *path, bool mipmapped)
{
	enum gs_color_format format;
	uint32_t cx, cy;
	uint8_t *levels[MAX_LEVELS] = {
		gs_create_texture_file_data2(path, GS_IMAGE_ALPHA_STRAIGHT, &format, &cx, &cy)};
	if (!levels[0] || (format != GS_RGBA && format != GS_BGRA)) {
		blog(LOG_WARNING, "[crtsim-renderer] cannot read %s as 8-bit RGBA", path);
		bfree(levels[0]);
		return NULL;
	}
	uint32_t count = 1;
	for (uint32_t w = cx, h = cy; mipmapped && (w > 1 || h > 1) && count < MAX_LEVELS;
	     count++) {
		levels[count] = halve(levels[count - 1], w, h);
		w = w > 1 ? w / 2 : 1;
		h = h > 1 ? h / 2 : 1;
	}
	gs_texture_t *texture =
		gs_texture_create(cx, cy, format, count, (const uint8_t **)levels, 0);
	for (uint32_t i = 0; i < count; i++)
		bfree(levels[i]);
	return texture;
}

static void free_targets(struct crtsim *f)
{
	gs_texrender_t **targets[] = {&f->input,      &f->rows,       &f->picture,
				      &f->prepared,   &f->signals[0], &f->signals[1],
				      &f->surface,    &f->bloom_down, &f->bloom_up,
				      &f->output};
	for (size_t i = 0; i < sizeof(targets) / sizeof(targets[0]); i++) {
		gs_texrender_destroy(*targets[i]);
		*targets[i] = NULL;
	}
}

/* The textures the passes draw into. In linear light the surface and what follows it keep
 * floats, as the renderer does; the signal is always 8-bit. */
static void make_targets(struct crtsim *f, bool linear)
{
	free_targets(f);
	enum gs_color_format surface = linear ? GS_RGBA16F : GS_RGBA;
	f->input = gs_texrender_create(GS_RGBA, GS_ZS_NONE);
	/* The picture's columns at the signal's rows, in 8-bit steps, unrounded. */
	f->rows = gs_texrender_create(GS_RGBA32F, GS_ZS_NONE);
	f->picture = gs_texrender_create(GS_RGBA, GS_ZS_NONE);
	f->prepared = gs_texrender_create(GS_RGBA, GS_ZS_NONE);
	f->signals[0] = gs_texrender_create(GS_RGBA, GS_ZS_NONE);
	f->signals[1] = gs_texrender_create(GS_RGBA, GS_ZS_NONE);
	f->surface = gs_texrender_create(surface, GS_ZS_NONE);
	f->bloom_down = gs_texrender_create(surface, GS_ZS_NONE);
	f->bloom_up = gs_texrender_create(surface, GS_ZS_NONE);
	f->output = gs_texrender_create(GS_RGBA, GS_ZS_NONE);
	f->linear = linear;
	f->has_previous = false;
}

/* Loads the look's colour table, a strip of N slices of N×N. Must run in the graphics
 * context. */
static void load_table(struct crtsim *f, const char *path)
{
	gs_texture_destroy(f->table);
	f->table = NULL;
	dstr_copy(&f->table_path, path);
	if (!path || !*path)
		return;
	gs_texture_t *table = load_image(path, false);
	if (table && gs_texture_get_width(table) ==
			     gs_texture_get_height(table) * gs_texture_get_height(table)) {
		f->table = table;
		return;
	}
	blog(LOG_WARNING, "[crtsim-renderer] %s is not a colour table's slices side by side",
	     path);
	gs_texture_destroy(table);
}

static void crtsim_update(void *data, obs_data_t *settings)
{
	struct crtsim *f = data;
	for (int i = 0; i < CRTSIM_PARAMETER_COUNT; i++)
		f->values[i] = (float)obs_data_get_double(settings, crtsim_parameters[i].name);
	const char *table = obs_data_get_string(settings, TABLE_FILE);
	if (strcmp(table, f->table_path.array ? f->table_path.array : "") != 0) {
		obs_enter_graphics();
		load_table(f, table);
		obs_leave_graphics();
	}
}

static void crtsim_destroy(void *data)
{
	struct crtsim *f = data;
	obs_enter_graphics();
	free_targets(f);
	for (int i = 0; i < CRTSIM_IMAGE_COUNT; i++)
		gs_texture_destroy(f->images[i]);
	gs_texture_destroy(f->table);
	gs_effect_destroy(f->effect);
	obs_leave_graphics();
	dstr_free(&f->table_path);
	bfree(f);
}

static void *crtsim_create(obs_data_t *settings, obs_source_t *context)
{
	struct crtsim *f = bzalloc(sizeof(struct crtsim));
	f->context = context;
	char *effect_path = obs_module_file("crtsim.effect");
	obs_enter_graphics();
	char *errors = NULL;
	f->effect = effect_path ? gs_effect_create_from_file(effect_path, &errors) : NULL;
	if (!f->effect)
		blog(LOG_ERROR, "[crtsim-renderer] cannot load %s: %s",
		     effect_path ? effect_path : "crtsim.effect", errors ? errors : "not found");
	bfree(errors);
	for (int i = 0; i < CRTSIM_IMAGE_COUNT; i++) {
		char *path = obs_module_file(crtsim_images[i].file);
		f->images[i] = path ? load_image(path, crtsim_images[i].mipmapped) : NULL;
		bfree(path);
	}
	if (f->effect)
		for (int i = 0; i < CRTSIM_PARAMETER_COUNT; i++)
			f->parameters[i] =
				gs_effect_get_param_by_name(f->effect, crtsim_parameters[i].name);
	make_targets(f, false);
	obs_leave_graphics();
	bfree(effect_path);
	crtsim_update(f, settings);
	return f;
}

static void crtsim_defaults(obs_data_t *settings)
{
	for (int i = 0; i < CRTSIM_PARAMETER_COUNT; i++)
		obs_data_set_default_double(settings, crtsim_parameters[i].name,
					    crtsim_parameters[i].value);
	obs_data_set_default_string(settings, TABLE_FILE, "");
}

/* Applies the look file the properties name: its settings, and its colour table, a path
 * relative to it. */
static bool apply_look(obs_properties_t *props, obs_property_t *property, void *data)
{
	UNUSED_PARAMETER(props);
	UNUSED_PARAMETER(property);
	struct crtsim *f = data;
	obs_data_t *settings = obs_source_get_settings(f->context);
	const char *path = obs_data_get_string(settings, LOOK_FILE);
	obs_data_t *look = path && *path ? obs_data_create_from_json_file(path) : NULL;
	if (!look) {
		blog(LOG_WARNING, "[crtsim-renderer] cannot read the look %s", path ? path : "");
		obs_data_release(settings);
		return false;
	}
	for (int i = 0; i < CRTSIM_PARAMETER_COUNT; i++) {
		const char *name = crtsim_parameters[i].name;
		if (obs_data_has_user_value(look, name))
			obs_data_set_double(settings, name, obs_data_get_double(look, name));
	}
	const char *table = obs_data_get_string(look, TABLE_FILE);
	struct dstr table_path = {0};
	if (table && *table) {
		const char *slash = strrchr(path, '/');
		const char *backslash = strrchr(path, '\\');
		if (backslash > slash)
			slash = backslash;
		if (slash)
			dstr_ncopy(&table_path, path, slash - path + 1);
		dstr_cat(&table_path, table);
	}
	obs_data_set_string(settings, TABLE_FILE, table_path.array ? table_path.array : "");
	dstr_free(&table_path);
	obs_source_update(f->context, settings);
	obs_data_release(look);
	obs_data_release(settings);
	return true;
}

static obs_properties_t *crtsim_properties(void *data)
{
	UNUSED_PARAMETER(data);
	obs_properties_t *props = obs_properties_create();
	char *looks = obs_module_file("looks");
	obs_properties_add_path(props, LOOK_FILE, "Look", OBS_PATH_FILE, "Looks (*.json)", looks);
	bfree(looks);
	obs_properties_add_button(props, "apply_look", "Apply look", apply_look);
	obs_properties_add_path(props, TABLE_FILE, "Colour table image", OBS_PATH_FILE,
				"Images (*.png)", NULL);
	for (int i = 0; i < CRTSIM_PARAMETER_COUNT; i++) {
		const struct crtsim_parameter *p = &crtsim_parameters[i];
		if (p->items) {
			obs_property_t *list = obs_properties_add_list(
				props, p->name, p->label, OBS_COMBO_TYPE_LIST, OBS_COMBO_FORMAT_FLOAT);
			for (int item = 0; p->items[item]; item++)
				obs_property_list_add_float(list, p->items[item], item);
		} else {
			obs_properties_add_float_slider(props, p->name, p->label, p->min, p->max,
							p->step);
		}
	}
	return props;
}

static void crtsim_tick(void *data, float seconds)
{
	struct crtsim *f = data;
	f->frame_seconds = seconds;
	f->ticked = true;
}

static void set_texture(struct crtsim *f, const char *name, gs_texture_t *texture)
{
	gs_eparam_t *param = gs_effect_get_param_by_name(f->effect, name);
	if (param)
		gs_effect_set_texture(param, texture);
}

static void set_size(struct crtsim *f, const char *name, float cx, float cy)
{
	gs_eparam_t *param = gs_effect_get_param_by_name(f->effect, name);
	struct vec2 size;
	vec2_set(&size, cx, cy);
	if (param)
		gs_effect_set_vec2(param, &size);
}

static void set_texture_size(struct crtsim *f, const char *name, gs_texture_t *texture)
{
	set_size(f, name, (float)gs_texture_get_width(texture),
		 (float)gs_texture_get_height(texture));
}

/* Sets what every pass reads but its input. OBS forgets an effect's values once a technique
 * ends, so this runs before each pass. */
static void set_values(struct crtsim *f)
{
	for (int i = 0; i < CRTSIM_PARAMETER_COUNT; i++)
		if (f->parameters[i])
			gs_effect_set_float(f->parameters[i], f->values[i]);
	/* A look whose table did not load is drawn without one. */
	if (!f->table && f->parameters[CRTSIM_TABLE_INDEX])
		gs_effect_set_float(f->parameters[CRTSIM_TABLE_INDEX], 0.0f);
	gs_texture_t *table_image = f->table ? f->table : f->images[CRTSIM_IMAGE_NO_TABLE];
	gs_effect_set_int(gs_effect_get_param_by_name(f->effect, "crtsim_frame_count"),
			  f->frame_count);
	/* In sixtieths of a second, worked in doubles, so a frame of 1/60 s, as a float holds
	 * it, lasts exactly one. */
	gs_effect_set_float(gs_effect_get_param_by_name(f->effect, "crtsim_frame_ticks"),
			    (float)(f->frame_seconds * 60.0));
	gs_effect_set_float(gs_effect_get_param_by_name(f->effect, "crtsim_has_previous"),
			    f->has_previous ? 1.0f : 0.0f);
	gs_effect_set_float(gs_effect_get_param_by_name(f->effect, "crtsim_table_size"),
			    (float)gs_texture_get_height(table_image));
	set_size(f, "crtsim_original_size", (float)f->cx, (float)f->cy);
	set_size(f, "crtsim_signal_size", (float)f->signal_cx, (float)f->signal_cy);
	set_size(f, "crtsim_output_size", (float)f->cx, (float)f->cy);
	set_texture_size(f, "crtsim_artifacts_size", f->images[CRTSIM_IMAGE_ARTIFACTS]);
	set_texture_size(f, "crtsim_mask_size", f->images[CRTSIM_IMAGE_MASK]);
	set_texture_size(f, "crtsim_bezel_size", f->images[CRTSIM_IMAGE_BEZEL_SHAPE]);
	set_texture_size(f, "crtsim_glass_size", f->images[CRTSIM_IMAGE_GLASS]);
	set_texture(f, "table_image", table_image);
	for (int i = 0; i < CRTSIM_IMAGE_COUNT; i++)
		if (crtsim_images[i].param)
			set_texture(f, crtsim_images[i].param, f->images[i]);
	set_texture(f, "previous_signal", gs_texrender_get_texture(f->signals[!f->current]));
	set_texture(f, "surface_image", gs_texrender_get_texture(f->surface));
}

/* Draws `technique` over all of `target`, at cx×cy, reading `input`. */
static void run(struct crtsim *f, gs_texrender_t *target, uint32_t cx, uint32_t cy,
		const char *technique, gs_texture_t *input)
{
	gs_texrender_reset(target);
	if (!gs_texrender_begin(target, cx, cy))
		return;
	gs_ortho(0.0f, (float)cx, 0.0f, (float)cy, -100.0f, 100.0f);
	set_values(f);
	set_texture(f, "pass_input", input);
	while (gs_effect_loop(f->effect, technique))
		gs_draw_sprite(NULL, 0, cx, cy);
	gs_texrender_end(target);
}

/* The source's picture, drawn over black into f->input, as OBS draws a filter's input: the
 * source itself by its default drawing when this is its first filter and it draws no way of
 * its own, else whatever this filter follows. */
static bool capture(struct crtsim *f, uint32_t cx, uint32_t cy)
{
	obs_source_t *target = obs_filter_get_target(f->context);
	obs_source_t *parent = obs_filter_get_parent(f->context);
	if (!target || !parent)
		return false;
	gs_texrender_reset(f->input);
	if (!gs_texrender_begin(f->input, cx, cy))
		return false;
	struct vec4 black;
	vec4_zero(&black);
	gs_clear(GS_CLEAR_COLOR, &black, 0.0f, 0);
	gs_ortho(0.0f, (float)cx, 0.0f, (float)cy, -100.0f, 100.0f);
	uint32_t flags = obs_source_get_output_flags(parent);
	if (target == parent && !(flags & (OBS_SOURCE_CUSTOM_DRAW | OBS_SOURCE_ASYNC)))
		obs_source_default_render(target);
	else
		obs_source_video_render(target);
	gs_texrender_end(f->input);
	return true;
}

/* The signal's size for a picture of cx×cy: its rows, else the picture's; its columns, else as
 * many as keep the picture's shape. */
static void signal_size(const struct crtsim *f, uint32_t cx, uint32_t cy, uint32_t *signal_cx,
			uint32_t *signal_cy)
{
	float rows = f->values[CRTSIM_SIGNAL_HEIGHT_INDEX];
	float columns = f->values[CRTSIM_SIGNAL_WIDTH_INDEX];
	*signal_cy = rows >= 1.0f ? (uint32_t)rows : cy;
	*signal_cx = columns >= 1.0f ? (uint32_t)columns
				     : (uint32_t)fmax(1.0, round((double)cx * *signal_cy / cy));
}

/* Runs one frame of the CRT over the source's picture, into f->output. */
static bool simulate(struct crtsim *f, uint32_t cx, uint32_t cy)
{
	bool linear = f->values[CRTSIM_LINEAR_INDEX] > 0.5f;
	if (linear != f->linear)
		make_targets(f, linear);
	if (!capture(f, cx, cy))
		return false;
	/* The passes overwrite every pixel. */
	gs_enable_blending(false);
	uint32_t scx, scy;
	signal_size(f, cx, cy, &scx, &scy);
	if (scx != f->signal_cx || scy != f->signal_cy) {
		f->signal_cx = scx;
		f->signal_cy = scy;
		f->has_previous = false;
	}
	f->cx = cx;
	f->cy = cy;

	gs_texture_t *picture = gs_texrender_get_texture(f->input);
	if (scx != cx || scy != cy) {
		run(f, f->rows, cx, scy, "Rows", picture);
		run(f, f->picture, scx, scy, "Columns", gs_texrender_get_texture(f->rows));
		picture = gs_texrender_get_texture(f->picture);
	}
	run(f, f->prepared, scx, scy, "Prepare", picture);
	gs_texrender_t *signal = f->signals[f->current];
	run(f, signal, scx, scy, "Composite", gs_texrender_get_texture(f->prepared));
	run(f, f->surface, cx, cy, "Surface", gs_texrender_get_texture(signal));
	uint32_t bloom_cx = (cx + 8) / 16, bloom_cy = (cy + 8) / 16;
	run(f, f->bloom_down, bloom_cx ? bloom_cx : 1, bloom_cy ? bloom_cy : 1, "BloomDown",
	    gs_texrender_get_texture(f->surface));
	run(f, f->bloom_up, cx, cy, "BloomUp", gs_texrender_get_texture(f->bloom_down));
	run(f, f->output, cx, cy, "Present", gs_texrender_get_texture(f->bloom_up));

	f->current = !f->current;
	f->has_previous = true;
	f->frame_count++;
	return true;
}

static void crtsim_render(void *data, gs_effect_t *unused)
{
	UNUSED_PARAMETER(unused);
	struct crtsim *f = data;
	obs_source_t *target = obs_filter_get_target(f->context);
	uint32_t cx = target ? obs_source_get_base_width(target) : 0;
	uint32_t cy = target ? obs_source_get_base_height(target) : 0;
	if (!f->effect || !cx || !cy) {
		obs_source_skip_video_filter(f->context);
		return;
	}
	bool sized = f->cx == cx && f->cy == cy;
	if (f->ticked || !sized) {
		gs_blend_state_push();
		gs_reset_blend_state();
		bool srgb = gs_framebuffer_srgb_enabled();
		gs_enable_framebuffer_srgb(false);
		bool simulated = simulate(f, cx, cy);
		gs_enable_framebuffer_srgb(srgb);
		gs_blend_state_pop();
		if (!simulated) {
			obs_source_skip_video_filter(f->context);
			return;
		}
		f->ticked = false;
	}
	gs_effect_t *draw = obs_get_base_effect(OBS_EFFECT_DEFAULT);
	gs_texture_t *output = gs_texrender_get_texture(f->output);
	gs_effect_set_texture(gs_effect_get_param_by_name(draw, "image"), output);
	while (gs_effect_loop(draw, "Draw"))
		gs_draw_sprite(output, 0, cx, cy);
}

/* Not static: the headless test (test/render.c) ticks the filter through it. */
struct obs_source_info crtsim_filter = {
	.id = "crtsim_renderer_filter",
	.type = OBS_SOURCE_TYPE_FILTER,
	.output_flags = OBS_SOURCE_VIDEO,
	.get_name = crtsim_name,
	.create = crtsim_create,
	.destroy = crtsim_destroy,
	.update = crtsim_update,
	.get_defaults = crtsim_defaults,
	.get_properties = crtsim_properties,
	.video_tick = crtsim_tick,
	.video_render = crtsim_render,
};

bool obs_module_load(void)
{
	obs_register_source(&crtsim_filter);
	return true;
}
