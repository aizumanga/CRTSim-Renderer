/* CC0. Runs the filter headless in libobs, for the parity test against the renderer
 * (crates/crtsim-ports/tests/obs_parity.rs): a source of raw RGBA frames, the filter over it
 * with a look's settings, and the frames it draws read back as raw RGBA.
 *
 *   render PLUGIN DATA LOOK.json OUT.rgba FRAME_SECONDS [W H FRAMES IN.rgba]...
 *
 * Each step shows IN.rgba, W×H, for FRAMES frames, each lasting FRAME_SECONDS; OUT.rgba is the
 * last frame drawn, at the last step's size. It needs an X display, which Xvfb provides, and
 * draws with OpenGL. The video thread never ticks a source while it runs: the test ticks the
 * filter itself, so each frame lasts exactly FRAME_SECONDS. */

#include <obs.h>
#include <obs-module.h>
#include <obs-nix-platform.h>
#include <util/platform.h>
#include <X11/Xlib.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

/* A source showing one picture, which the test replaces from step to step. */
struct picture {
	gs_texture_t *texture;
	uint32_t cx, cy;
};

static struct picture shown;

static const char *picture_name(void *unused)
{
	(void)unused;
	return "Test picture";
}

static void *picture_create(obs_data_t *settings, obs_source_t *source)
{
	(void)settings;
	(void)source;
	return &shown;
}

static void picture_destroy(void *data) { (void)data; }
static uint32_t picture_width(void *data) { return ((struct picture *)data)->cx; }
static uint32_t picture_height(void *data) { return ((struct picture *)data)->cy; }

static void picture_render(void *data, gs_effect_t *effect)
{
	struct picture *p = data;
	(void)effect;
	gs_effect_t *draw = obs_get_base_effect(OBS_EFFECT_DEFAULT);
	gs_effect_set_texture(gs_effect_get_param_by_name(draw, "image"), p->texture);
	while (gs_effect_loop(draw, "Draw"))
		gs_draw_sprite(p->texture, 0, p->cx, p->cy);
}

static struct obs_source_info picture_info = {
	.id = "crtsim_test_picture",
	.type = OBS_SOURCE_TYPE_INPUT,
	.output_flags = OBS_SOURCE_VIDEO | OBS_SOURCE_CUSTOM_DRAW,
	.get_name = picture_name,
	.create = picture_create,
	.destroy = picture_destroy,
	.get_width = picture_width,
	.get_height = picture_height,
	.video_render = picture_render,
};

/* Set once the video thread has ticked, which it does as it starts and then not for a
 * thousand seconds. */
static volatile bool video_ticked;

static void first_tick(void *param, float seconds)
{
	(void)param;
	(void)seconds;
	video_ticked = true;
}

static uint8_t *read_file(const char *path, size_t size)
{
	FILE *file = fopen(path, "rb");
	uint8_t *bytes = malloc(size);
	if (!file || fread(bytes, 1, size, file) != size) {
		fprintf(stderr, "cannot read %zu bytes of %s\n", size, path);
		exit(1);
	}
	fclose(file);
	return bytes;
}

int main(int argc, char **argv)
{
	if (argc < 10 || (argc - 6) % 4 != 0) {
		fprintf(stderr, "usage: render PLUGIN DATA LOOK.json OUT.rgba FRAME_SECONDS "
				"[W H FRAMES IN.rgba]...\n");
		return 2;
	}
	Display *display = XOpenDisplay(NULL);
	if (!display) {
		fprintf(stderr, "no X display\n");
		return 1;
	}
	obs_set_nix_platform(OBS_NIX_PLATFORM_X11_EGL);
	obs_set_nix_platform_display(display);
	if (!obs_startup("en-US", NULL, NULL))
		return 1;
	/* The video thread ticks every source; at one frame in 1000 seconds it never does while
	 * the test runs, so the test alone says how long each frame lasts. */
	struct obs_video_info video = {
		.graphics_module = "libobs-opengl",
		.fps_num = 1,
		.fps_den = 1000,
		.base_width = 64,
		.base_height = 64,
		.output_width = 64,
		.output_height = 64,
		.output_format = VIDEO_FORMAT_RGBA,
		.colorspace = VIDEO_CS_709,
		.range = VIDEO_RANGE_FULL,
		.gpu_conversion = true,
		.scale_type = OBS_SCALE_BICUBIC,
	};
	obs_add_tick_callback(first_tick, NULL);
	if (obs_reset_video(&video) != OBS_VIDEO_SUCCESS)
		return 1;
	/* Its first tick would reach a filter made before it, at a moment of its own choosing. */
	while (!video_ticked)
		os_sleep_ms(1);
	obs_remove_tick_callback(first_tick, NULL);
	obs_module_t *module = NULL;
	if (obs_open_module(&module, argv[1], argv[2]) != MODULE_SUCCESS ||
	    !obs_init_module(module)) {
		fprintf(stderr, "cannot load %s\n", argv[1]);
		return 1;
	}
	/* libobs ticks sources only from its video thread, at its own pace; the test ticks the
	 * filter itself, through the plugin's own description of it. */
	const struct obs_source_info *crt = os_dlsym(obs_get_module_lib(module), "crtsim_filter");
	if (!crt) {
		fprintf(stderr, "%s describes no crtsim_filter\n", argv[1]);
		return 1;
	}
	obs_register_source(&picture_info);
	obs_data_t *look = obs_data_create_from_json_file(argv[3]);
	if (!look) {
		fprintf(stderr, "cannot read %s\n", argv[3]);
		return 1;
	}
	obs_source_t *source = obs_source_create_private("crtsim_test_picture", "picture", NULL);
	obs_source_t *filter =
		obs_source_create_private("crtsim_renderer_filter", "crt", look);
	if (!filter) {
		fprintf(stderr, "no filter\n");
		return 1;
	}
	obs_source_filter_add(source, filter);
	float seconds = (float)atof(argv[5]);
	for (int arg = 6; arg < argc; arg += 4) {
		uint32_t cx = (uint32_t)atoi(argv[arg]), cy = (uint32_t)atoi(argv[arg + 1]);
		int frames = atoi(argv[arg + 2]);
		uint8_t *pixels = read_file(argv[arg + 3], (size_t)cx * cy * 4);
		obs_enter_graphics();
		gs_texture_destroy(shown.texture);
		const uint8_t *levels[] = {pixels};
		shown.texture = gs_texture_create(cx, cy, GS_RGBA, 1, levels, 0);
		shown.cx = cx;
		shown.cy = cy;
		obs_leave_graphics();
		free(pixels);
		for (int frame = 0; frame < frames; frame++) {
			crt->video_tick(obs_obj_get_data(filter), seconds);
			obs_enter_graphics();
			gs_texrender_t *target = gs_texrender_create(GS_RGBA, GS_ZS_NONE);
			if (gs_texrender_begin(target, cx, cy)) {
				struct vec4 clear;
				vec4_zero(&clear);
				gs_clear(GS_CLEAR_COLOR, &clear, 0.0f, 0);
				gs_ortho(0.0f, (float)cx, 0.0f, (float)cy, -100.0f, 100.0f);
				obs_source_video_render(source);
				gs_texrender_end(target);
			}
			bool last = arg + 4 >= argc && frame == frames - 1;
			if (last) {
				gs_stagesurf_t *stage = gs_stagesurface_create(cx, cy, GS_RGBA);
				gs_stage_texture(stage, gs_texrender_get_texture(target));
				uint8_t *data;
				uint32_t linesize;
				FILE *out = fopen(argv[4], "wb");
				if (!out || !gs_stagesurface_map(stage, &data, &linesize)) {
					fprintf(stderr, "cannot read the frame back\n");
					return 1;
				}
				for (uint32_t y = 0; y < cy; y++)
					fwrite(data + (size_t)y * linesize, 1, (size_t)cx * 4, out);
				fclose(out);
				gs_stagesurface_unmap(stage);
				gs_stagesurface_destroy(stage);
			}
			gs_texrender_destroy(target);
			obs_leave_graphics();
		}
	}
	/* Shutting down waits for the video thread's next frame, a thousand seconds away. */
	fflush(NULL);
	_Exit(0);
}
