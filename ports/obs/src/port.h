/* CC0. Written by CRTSim Renderer's exporter from its settings table; edit that, not
 * this. `cargo test -p crtsim-core obs` says when it is out of date, and
 * CRTSIM_WRITE_OBS_HEADER=1 rewrites it. */

#pragma once

#include <stdbool.h>
#include <stddef.h>

/* A setting: a number in a range, or a choice of the items, numbered from 0. */
struct crtsim_parameter {
	const char *name;
	const char *label;
	float min, max, step;
	float value;
	const char *const *items;
};

static const char *const CRTSIM_RESIZE_ITEMS[] = {"nearest", "Lanczos", NULL};
static const char *const CRTSIM_TABLE_ITEMS[] = {"off", "on", NULL};
static const char *const CRTSIM_TABLE_NEAREST_ITEMS[] = {"blended", "nearest", NULL};
static const char *const CRTSIM_LINEAR_ITEMS[] = {"off", "on", NULL};
static const char *const CRTSIM_PHASE_ITEMS[] = {"stable", "A", "B", "alternating", NULL};
static const char *const CRTSIM_FLIP_ARTIFACTS_ITEMS[] = {"as the public source", "flipped", NULL};
static const char *const CRTSIM_INTERLACE_ITEMS[] = {"off", "on", "auto", NULL};
static const char *const CRTSIM_FIT_ITEMS[] = {"original", "contain", "cover", "stretch", NULL};
static const char *const CRTSIM_SCREEN_ONLY_ITEMS[] = {"off", "on", NULL};
static const char *const CRTSIM_REFLECTION_AS_SCREEN_ITEMS[] = {"as the public source", "as the screen", NULL};
static const char *const CRTSIM_MASK_ANTIALIAS_ITEMS[] = {"off", "on", NULL};

enum crtsim_parameter_index {
	CRTSIM_SIGNAL_WIDTH_INDEX,
	CRTSIM_SIGNAL_HEIGHT_INDEX,
	CRTSIM_RESIZE_INDEX,
	CRTSIM_TABLE_INDEX,
	CRTSIM_TABLE_NEAREST_INDEX,
	CRTSIM_LINEAR_INDEX,
	CRTSIM_PHASE_INDEX,
	CRTSIM_FLIP_ARTIFACTS_INDEX,
	CRTSIM_INTERLACE_INDEX,
	CRTSIM_FIT_INDEX,
	CRTSIM_SCREEN_ONLY_INDEX,
	CRTSIM_REFLECTION_AS_SCREEN_INDEX,
	CRTSIM_MASK_ANTIALIAS_INDEX,
	CRTSIM_MASK_COLUMNS_INDEX,
	CRTSIM_MASK_ROWS_INDEX,
	CRTSIM_PIXEL_ASPECT_INDEX,
	CRTSIM_LUT_STRENGTH_INDEX,
	CRTSIM_HUE_INDEX,
	CRTSIM_CHROMA_INDEX,
	CRTSIM_SATURATION_INDEX,
	CRTSIM_SHARPNESS_INDEX,
	CRTSIM_BLEED_INDEX,
	CRTSIM_ARTIFACTS_INDEX,
	CRTSIM_NTSC_BLENDING_INDEX,
	CRTSIM_BARREL_INDEX,
	CRTSIM_OVERSCAN_INDEX,
	CRTSIM_DIMMING_INDEX,
	CRTSIM_FOV_INDEX,
	CRTSIM_MASK_OPACITY_INDEX,
	CRTSIM_MASK_BRIGHTNESS_INDEX,
	CRTSIM_BLOOM_INDEX,
	CRTSIM_BLOOM_POWER_INDEX,
	CRTSIM_BLOOM_SPREAD_INDEX,
	CRTSIM_DIFFUSE_INDEX,
	CRTSIM_SPECULAR_INDEX,
	CRTSIM_SPECULAR_POWER_INDEX,
	CRTSIM_RIM_INDEX,
	CRTSIM_REFLECTION_INDEX,
	CRTSIM_BEZEL_R_INDEX,
	CRTSIM_BEZEL_G_INDEX,
	CRTSIM_BEZEL_B_INDEX,
	CRTSIM_BACKDROP_R_INDEX,
	CRTSIM_BACKDROP_G_INDEX,
	CRTSIM_BACKDROP_B_INDEX,
	CRTSIM_LIGHT_X_INDEX,
	CRTSIM_LIGHT_Y_INDEX,
	CRTSIM_LIGHT_Z_INDEX,
	CRTSIM_PERSISTENCE_R_INDEX,
	CRTSIM_PERSISTENCE_G_INDEX,
	CRTSIM_PERSISTENCE_B_INDEX,
	CRTSIM_PARAMETER_COUNT,
};

static const struct crtsim_parameter crtsim_parameters[CRTSIM_PARAMETER_COUNT] = {
	{"CRTSIM_SIGNAL_WIDTH", "Signal columns (0 keeps the picture's shape)", 0.0f, 4096.0f, 1.0f, 0.0f, NULL},
	{"CRTSIM_SIGNAL_HEIGHT", "Signal rows (0 for the picture's own)", 0.0f, 4096.0f, 1.0f, 480.0f, NULL},
	{"CRTSIM_RESIZE", "Resize to the signal", 0.0f, 1.0f, 1.0f, 1.0f, CRTSIM_RESIZE_ITEMS},
	{"CRTSIM_TABLE", "Colour table", 0.0f, 1.0f, 1.0f, 0.0f, CRTSIM_TABLE_ITEMS},
	{"CRTSIM_TABLE_NEAREST", "Colour table red and green", 0.0f, 1.0f, 1.0f, 0.0f, CRTSIM_TABLE_NEAREST_ITEMS},
	{"CRTSIM_LINEAR", "Linear light", 0.0f, 1.0f, 1.0f, 0.0f, CRTSIM_LINEAR_ITEMS},
	{"CRTSIM_PHASE", "Phase", 0.0f, 3.0f, 1.0f, 0.0f, CRTSIM_PHASE_ITEMS},
	{"CRTSIM_FLIP_ARTIFACTS", "Artifact pattern", 0.0f, 1.0f, 1.0f, 0.0f, CRTSIM_FLIP_ARTIFACTS_ITEMS},
	{"CRTSIM_INTERLACE", "Interlaced fields", 0.0f, 2.0f, 1.0f, 2.0f, CRTSIM_INTERLACE_ITEMS},
	{"CRTSIM_FIT", "Fit on 4:3 tube", 0.0f, 3.0f, 1.0f, 1.0f, CRTSIM_FIT_ITEMS},
	{"CRTSIM_SCREEN_ONLY", "Screen only, without the bezel", 0.0f, 1.0f, 1.0f, 0.0f, CRTSIM_SCREEN_ONLY_ITEMS},
	{"CRTSIM_REFLECTION_AS_SCREEN", "Bezel reflection", 0.0f, 1.0f, 1.0f, 0.0f, CRTSIM_REFLECTION_AS_SCREEN_ITEMS},
	{"CRTSIM_MASK_ANTIALIAS", "Filter mask when shrinking", 0.0f, 1.0f, 1.0f, 1.0f, CRTSIM_MASK_ANTIALIAS_ITEMS},
	{"CRTSIM_MASK_COLUMNS", "Mask columns (0 follows the signal)", 0.0f, 4096.0f, 1.0f, 128.0f, NULL},
	{"CRTSIM_MASK_ROWS", "Mask rows (0 follows the signal)", 0.0f, 4096.0f, 1.0f, 224.0f, NULL},
	{"CRTSIM_PIXEL_ASPECT", "Pixel aspect", 0.1f, 10.0f, 0.1f, 1.0f, NULL},
	{"CRTSIM_LUT_STRENGTH", "LUT strength", 0.0f, 1.0f, 0.01f, 1.0f, NULL},
	{"CRTSIM_HUE", "Hue (degrees)", -180.0f, 180.0f, 4.0f, 0.0f, NULL},
	{"CRTSIM_CHROMA", "Chroma", 0.0f, 2.0f, 0.02f, 1.0f, NULL},
	{"CRTSIM_SATURATION", "Saturation", 0.0f, 3.0f, 0.03f, 1.0f, NULL},
	{"CRTSIM_SHARPNESS", "Sharpness / ringing", 0.0f, 3.0f, 0.03f, 0.8f, NULL},
	{"CRTSIM_BLEED", "Color bleed", 0.0f, 2.0f, 0.02f, 0.5f, NULL},
	{"CRTSIM_ARTIFACTS", "Composite artifacts", 0.0f, 2.0f, 0.02f, 0.5f, NULL},
	{"CRTSIM_NTSC_BLENDING", "NTSC blending", 0.0f, 2.0f, 0.02f, 0.0f, NULL},
	{"CRTSIM_BARREL", "Barrel distortion", -2.0f, 2.0f, 0.04f, -0.115f, NULL},
	{"CRTSIM_OVERSCAN", "Overscan", 0.1f, 3.0f, 0.03f, 1.0f, NULL},
	{"CRTSIM_DIMMING", "Edge dimming", 0.0f, 1.0f, 0.01f, 0.5f, NULL},
	{"CRTSIM_FOV", "Camera field of view", 5.0f, 90.0f, 0.9f, 15.0f, NULL},
	{"CRTSIM_MASK_OPACITY", "Mask opacity", 0.0f, 1.0f, 0.01f, 1.0f, NULL},
	{"CRTSIM_MASK_BRIGHTNESS", "Mask brightness", 0.0f, 2.0f, 0.02f, 0.45f, NULL},
	{"CRTSIM_BLOOM", "Bloom amount", 0.0f, 2.0f, 0.02f, 0.25f, NULL},
	{"CRTSIM_BLOOM_POWER", "Bloom power", 0.1f, 8.0f, 0.08f, 2.0f, NULL},
	{"CRTSIM_BLOOM_SPREAD", "Bloom spread", 0.0f, 0.2f, 0.002f, 0.025f, NULL},
	{"CRTSIM_DIFFUSE", "Diffuse light", 0.0f, 2.0f, 0.02f, 0.5f, NULL},
	{"CRTSIM_SPECULAR", "Specular light", 0.0f, 2.0f, 0.02f, 0.35f, NULL},
	{"CRTSIM_SPECULAR_POWER", "Specular power", 1.0f, 200.0f, 2.0f, 50.0f, NULL},
	{"CRTSIM_RIM", "Rim light", 0.0f, 2.0f, 0.02f, 1.0f, NULL},
	{"CRTSIM_REFLECTION", "Edge reflection", 0.0f, 2.0f, 0.02f, 0.3f, NULL},
	{"CRTSIM_BEZEL_R", "Bezel red", 0.0f, 1.0f, 0.01f, 0.06f, NULL},
	{"CRTSIM_BEZEL_G", "Bezel green", 0.0f, 1.0f, 0.01f, 0.06f, NULL},
	{"CRTSIM_BEZEL_B", "Bezel blue", 0.0f, 1.0f, 0.01f, 0.06f, NULL},
	{"CRTSIM_BACKDROP_R", "Backdrop red", 0.0f, 1.0f, 0.01f, 0.0f, NULL},
	{"CRTSIM_BACKDROP_G", "Backdrop green", 0.0f, 1.0f, 0.01f, 0.0f, NULL},
	{"CRTSIM_BACKDROP_B", "Backdrop blue", 0.0f, 1.0f, 0.01f, 0.0f, NULL},
	{"CRTSIM_LIGHT_X", "Light X", -1000.0f, 1000.0f, 20.0f, -10.0f, NULL},
	{"CRTSIM_LIGHT_Y", "Light Y", -1000.0f, 1000.0f, 20.0f, -5.0f, NULL},
	{"CRTSIM_LIGHT_Z", "Light Z", -1000.0f, 1000.0f, 20.0f, 10.0f, NULL},
	{"CRTSIM_PERSISTENCE_R", "Red persistence", 0.0f, 0.999f, 0.01f, 0.7f, NULL},
	{"CRTSIM_PERSISTENCE_G", "Green persistence", 0.0f, 0.999f, 0.01f, 0.525f, NULL},
	{"CRTSIM_PERSISTENCE_B", "Blue persistence", 0.0f, 0.999f, 0.01f, 0.42f, NULL},
};

/* An image the passes read, from the data folder, and the effect's texture for it. */
struct crtsim_image {
	const char *file;
	const char *param;
	bool mipmapped;
};

enum crtsim_image_index {
	CRTSIM_IMAGE_ARTIFACTS,
	CRTSIM_IMAGE_MASK,
	CRTSIM_IMAGE_NO_TABLE,
	CRTSIM_IMAGE_BEZEL_SHAPE,
	CRTSIM_IMAGE_BEZEL_SHAPE_FILTERED,
	CRTSIM_IMAGE_BEZEL_UV,
	CRTSIM_IMAGE_BEZEL_NORMAL,
	CRTSIM_IMAGE_GLASS,
	CRTSIM_IMAGE_COUNT,
};

static const struct crtsim_image crtsim_images[CRTSIM_IMAGE_COUNT] = {
	{"images/artifacts.png", "artifacts_image", false},
	{"images/mask.png", "mask_image", true},
	{"images/no-table.png", NULL, false},
	{"images/bezel-shape.png", "bezel_shape_image", false},
	{"images/bezel-shape.png", "bezel_shape_filtered", false},
	{"images/bezel-uv.png", "bezel_uv_image", false},
	{"images/bezel-normal.png", "bezel_normal_image", false},
	{"images/glass.png", "glass_image", false},
};
