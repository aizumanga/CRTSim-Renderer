// CC0. CRTSim Renderer's CRT as a ReShade effect: the composite signal with its artifacts and
// glow, the shadow mask, the curved glass, its bezel and their lighting, and bloom, drawn over
// the game's picture. A port of J. Kyle Pittman's CRTSim, from
// https://github.com/aizumanga/CRTSim-Renderer, whose passes it shares with the RetroArch port.
//
// The game's picture is resized to the signal, a texture whose size is a preprocessor
// definition, as a look's preset sets it. So are linear light, whose buffers are floats, and
// the colour table's image. Every other setting is a uniform.

#include "crtsim-renderer/parameters.fxh"

// The signal's size: its rows, else the game's; its columns, else as many as keep the game's
// shape.
#if CRTSIM_SIGNAL_HEIGHT > 0
#define SIGNAL_HEIGHT CRTSIM_SIGNAL_HEIGHT
#else
#define SIGNAL_HEIGHT BUFFER_HEIGHT
#endif
#if CRTSIM_SIGNAL_WIDTH > 0
#define SIGNAL_WIDTH CRTSIM_SIGNAL_WIDTH
#else
#define SIGNAL_WIDTH ((2 * BUFFER_WIDTH * SIGNAL_HEIGHT + BUFFER_HEIGHT) / (2 * BUFFER_HEIGHT))
#endif
#define RESIZED (SIGNAL_WIDTH != BUFFER_WIDTH || SIGNAL_HEIGHT != BUFFER_HEIGHT)

#if CRTSIM_LINEAR
#define SURFACE_FORMAT RGBA16F
#else
#define SURFACE_FORMAT RGBA8
#endif

uniform int crtsim_frame_count < source = "framecount"; >;
uniform float crtsim_frame_time < source = "frametime"; >;

texture tex_back_buffer : COLOR;
// The game's columns at the signal's rows, in 8-bit steps, unrounded.
texture tex_rows { Width = BUFFER_WIDTH; Height = SIGNAL_HEIGHT; Format = RGBA32F; };
texture tex_picture { Width = SIGNAL_WIDTH; Height = SIGNAL_HEIGHT; Format = RGBA8; };
texture tex_prepared { Width = SIGNAL_WIDTH; Height = SIGNAL_HEIGHT; Format = RGBA8; };
texture tex_signal { Width = SIGNAL_WIDTH; Height = SIGNAL_HEIGHT; Format = RGBA8; };
// The last frame's signal, which the glow persists from.
texture tex_previous { Width = SIGNAL_WIDTH; Height = SIGNAL_HEIGHT; Format = RGBA8; };
// With mipmaps, which ReShade makes once the surface is drawn, for the bloom's first blur to
// read down to the level below a sixteenth.
texture tex_surface {
    Width = BUFFER_WIDTH;
    Height = BUFFER_HEIGHT;
    Format = SURFACE_FORMAT;
    MipLevels = 6;
};
// A sixteenth of the picture each way, rounded down, as the renderer and the game size it.
texture tex_bloom_down {
    Width = BUFFER_WIDTH / 16;
    Height = BUFFER_HEIGHT / 16;
    Format = SURFACE_FORMAT;
};
texture tex_bloom_up { Width = BUFFER_WIDTH; Height = BUFFER_HEIGHT; Format = SURFACE_FORMAT; };

texture tex_table < source = CRTSIM_TABLE_FILE; > {
    Width = CRTSIM_TABLE_SIZE * CRTSIM_TABLE_SIZE;
    Height = CRTSIM_TABLE_SIZE;
};
texture tex_artifacts < source = "crtsim-renderer/artifacts.png"; > {
    Width = CRTSIM_ARTIFACTS_WIDTH;
    Height = CRTSIM_ARTIFACTS_HEIGHT;
};
// Filtered with mipmaps, each level the average of the one above.
texture tex_mask < source = "crtsim-renderer/mask.png"; > {
    Width = CRTSIM_MASK_WIDTH;
    Height = CRTSIM_MASK_HEIGHT;
    MipLevels = CRTSIM_MASK_LEVELS;
};
texture tex_bezel_shape < source = "crtsim-renderer/bezel-shape.png"; > {
    Width = CRTSIM_BEZEL_WIDTH;
    Height = CRTSIM_BEZEL_HEIGHT;
};
texture tex_bezel_uv < source = "crtsim-renderer/bezel-uv.png"; > {
    Width = CRTSIM_BEZEL_WIDTH;
    Height = CRTSIM_BEZEL_HEIGHT;
};
texture tex_bezel_normal < source = "crtsim-renderer/bezel-normal.png"; > {
    Width = CRTSIM_BEZEL_WIDTH;
    Height = CRTSIM_BEZEL_HEIGHT;
};
texture tex_glass < source = "crtsim-renderer/glass.png"; > {
    Width = CRTSIM_GLASS_WIDTH;
    Height = CRTSIM_GLASS_HEIGHT;
};

// Texels are read through point samplers at their centres, which every ReShade reads alike.
#define TEXELS(name) sampler name##_texels { \
    Texture = name; MinFilter = POINT; MagFilter = POINT; MipFilter = POINT; \
    AddressU = CLAMP; AddressV = CLAMP; }
TEXELS(tex_back_buffer);
TEXELS(tex_rows);
TEXELS(tex_picture);
TEXELS(tex_prepared);
TEXELS(tex_signal);
TEXELS(tex_previous);
TEXELS(tex_table);
TEXELS(tex_artifacts);
TEXELS(tex_bezel_shape);
TEXELS(tex_bezel_uv);
TEXELS(tex_bezel_normal);
TEXELS(tex_glass);
#define FILTERED(name, address) sampler name##_filtered { \
    Texture = name; MinFilter = LINEAR; MagFilter = LINEAR; MipFilter = LINEAR; \
    AddressU = address; AddressV = address; }
FILTERED(tex_mask, REPEAT);
FILTERED(tex_bezel_shape, CLAMP);
FILTERED(tex_surface, CLAMP);
FILTERED(tex_bloom_down, CLAMP);
FILTERED(tex_bloom_up, CLAMP);

#define FETCH(name, xy, size) tex2Dlod(name##_texels, float4((float2(xy) + 0.5) / (size), 0.0, 0.0))
#define SAMPLE(name, uv, lod) tex2Dlod(name##_filtered, float4(uv, 0.0, lod))

#define P(name) name
#define HOST_LINEAR (CRTSIM_LINEAR != 0)
#define HOST_FRAME_COUNT crtsim_frame_count
#define HOST_FRAME_TICKS (crtsim_frame_time * 0.06)
#define HOST_PICTURE_ASPECT (float(SIGNAL_WIDTH) / float(SIGNAL_HEIGHT) * CRTSIM_PIXEL_ASPECT)
#define HOST_SIGNAL_SIZE float2(SIGNAL_WIDTH, SIGNAL_HEIGHT)
#define HOST_OUTPUT_SIZE float2(BUFFER_WIDTH, BUFFER_HEIGHT)
#define HOST_ORIGINAL_SIZE float2(BUFFER_WIDTH, BUFFER_HEIGHT)
#define HOST_RESIZE_NEAREST (CRTSIM_RESIZE == 0)
#define HOST_FETCH_ORIGINAL(xy) FETCH(tex_back_buffer, xy, HOST_ORIGINAL_SIZE)
#define HOST_FETCH_ROWS(xy) FETCH(tex_rows, xy, float2(BUFFER_WIDTH, SIGNAL_HEIGHT))
#if RESIZED
#define HOST_FETCH_PICTURE(xy) FETCH(tex_picture, xy, HOST_SIGNAL_SIZE)
#else
#define HOST_FETCH_PICTURE(xy) FETCH(tex_back_buffer, xy, HOST_SIGNAL_SIZE)
#endif
#define HOST_TABLE_SIZE float(CRTSIM_TABLE_SIZE)
#define HOST_FETCH_TABLE(xy) \
    FETCH(tex_table, xy, float2(CRTSIM_TABLE_SIZE * CRTSIM_TABLE_SIZE, CRTSIM_TABLE_SIZE))
#define HOST_FETCH_PREPARED(xy) FETCH(tex_prepared, xy, HOST_SIGNAL_SIZE)
// A texture ReShade has just made holds no signal: every one this effect writes is opaque.
#define HOST_HAS_PREVIOUS true
#define HOST_FETCH_PREVIOUS(xy) previous_texel(xy)
#define HOST_ARTIFACT_SIZE float2(CRTSIM_ARTIFACTS_WIDTH, CRTSIM_ARTIFACTS_HEIGHT)
#define HOST_FETCH_ARTIFACTS(xy) FETCH(tex_artifacts, xy, HOST_ARTIFACT_SIZE)
#define HOST_FETCH_SIGNAL(xy) FETCH(tex_signal, xy, HOST_SIGNAL_SIZE)
#define HOST_MASK_SIZE float2(CRTSIM_MASK_WIDTH, CRTSIM_MASK_HEIGHT)
#define HOST_SAMPLE_MASK(uv, lod) SAMPLE(tex_mask, uv, lod)
#define HOST_BEZEL_SIZE float2(CRTSIM_BEZEL_WIDTH, CRTSIM_BEZEL_HEIGHT)
#define HOST_FETCH_BEZEL_SHAPE(xy) FETCH(tex_bezel_shape, xy, HOST_BEZEL_SIZE)
#define HOST_SAMPLE_BEZEL_SHAPE(uv) SAMPLE(tex_bezel_shape, uv, 0.0)
#define HOST_FETCH_BEZEL_UV(xy) FETCH(tex_bezel_uv, xy, HOST_BEZEL_SIZE)
#define HOST_FETCH_BEZEL_NORMAL(xy) FETCH(tex_bezel_normal, xy, HOST_BEZEL_SIZE)
#define HOST_GLASS_SIZE float2(CRTSIM_GLASS_WIDTH, CRTSIM_GLASS_HEIGHT)
#define HOST_FETCH_GLASS(xy) FETCH(tex_glass, xy, HOST_GLASS_SIZE)

#define HOST_SAMPLE_BLOOM_INPUT(uv, second, lod) bloom_input(uv, second, lod)
#define HOST_SAMPLE_SURFACE(uv) SAMPLE(tex_surface, uv, 0.0)
#define HOST_SAMPLE_BLOOM(uv) SAMPLE(tex_bloom_up, uv, 0.0)

float4 bloom_input(float2 uv, bool second, float lod) {
    if (second) return SAMPLE(tex_bloom_down, uv, 0.0);
    return SAMPLE(tex_surface, uv, lod);
}

float4 previous_texel(int2 xy) {
    float4 texel = FETCH(tex_previous, xy, HOST_SIGNAL_SIZE);
    return texel * texel.a;
}

#include "crtsim-renderer/passes/common.inc"
#include "crtsim-renderer/passes/resize.inc"
#include "crtsim-renderer/passes/prepare.inc"
#include "crtsim-renderer/passes/composite.inc"
#include "crtsim-renderer/passes/surface.inc"
#include "crtsim-renderer/passes/bloom.inc"
#include "crtsim-renderer/passes/present.inc"

void full_screen(uint id : SV_VertexID, out float4 position : SV_Position,
    out float2 uv : TEXCOORD)
{
    uv.x = id == 2 ? 2.0 : 0.0;
    uv.y = id == 1 ? 2.0 : 0.0;
    position = float4(uv * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0);
}

float4 rows_ps(float4 position : SV_Position) : SV_Target { return crtsim_resize_rows(int2(position.xy)); }
float4 columns_ps(float4 position : SV_Position) : SV_Target { return crtsim_resize_columns(int2(position.xy)); }
float4 prepare_ps(float4 position : SV_Position) : SV_Target { return crtsim_prepare(int2(position.xy)); }
float4 composite_ps(float4 position : SV_Position) : SV_Target { return crtsim_composite(int2(position.xy)); }
float4 keep_ps(float4 position : SV_Position) : SV_Target { return HOST_FETCH_SIGNAL(int2(position.xy)); }
float4 surface_ps(float4 position : SV_Position, float2 uv : TEXCOORD) : SV_Target { return crtsim_surface(uv); }
float4 bloom_down_ps(float4 position : SV_Position, float2 uv : TEXCOORD) : SV_Target {
    return crtsim_bloom(uv, false);
}
float4 bloom_up_ps(float4 position : SV_Position, float2 uv : TEXCOORD) : SV_Target {
    return crtsim_bloom(uv, true);
}
float4 present_ps(float4 position : SV_Position, float2 uv : TEXCOORD) : SV_Target { return crtsim_present(uv); }

technique CRTSimRenderer <
    ui_label = "CRTSim Renderer";
    ui_tooltip = "A CRT: the composite signal and its glow, the shadow mask, the curved glass "
        "and its bezel, and bloom. Load one of its presets for a look.";
>
{
#if RESIZED
    pass rows { VertexShader = full_screen; PixelShader = rows_ps; RenderTarget = tex_rows; }
    pass columns { VertexShader = full_screen; PixelShader = columns_ps; RenderTarget = tex_picture; }
#endif
    pass prepare { VertexShader = full_screen; PixelShader = prepare_ps; RenderTarget = tex_prepared; }
    pass composite { VertexShader = full_screen; PixelShader = composite_ps; RenderTarget = tex_signal; }
    pass keep { VertexShader = full_screen; PixelShader = keep_ps; RenderTarget = tex_previous; }
    pass surface { VertexShader = full_screen; PixelShader = surface_ps; RenderTarget = tex_surface; }
    pass bloom_down { VertexShader = full_screen; PixelShader = bloom_down_ps; RenderTarget = tex_bloom_down; }
    pass bloom_up { VertexShader = full_screen; PixelShader = bloom_up_ps; RenderTarget = tex_bloom_up; }
    pass present { VertexShader = full_screen; PixelShader = present_ps; }
}
