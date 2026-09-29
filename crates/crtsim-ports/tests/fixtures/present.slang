#version 450

// CC0 1.0: written for this repository's tests and dedicated to the public domain.
//
// Copies the previous pass to the viewport. Where RetroArch has them, blue in the bottom half
// shows OriginalAspect / 4 on the left and FrameTimeDelta / 100000 on the right.

layout(std140, set = 0, binding = 0) uniform UBO
{
	mat4 MVP;
#if defined(_HAS_ORIGINALASPECT_UNIFORMS) && defined(_HAS_FRAMETIME_UNIFORMS)
	float OriginalAspect;
	uint FrameTimeDelta;
#endif
} global;

#pragma stage vertex
layout(location = 0) in vec4 Position;
layout(location = 1) in vec2 TexCoord;
layout(location = 0) out vec2 vTexCoord;

void main()
{
	gl_Position = global.MVP * Position;
	vTexCoord = TexCoord;
}

#pragma stage fragment
layout(location = 0) in vec2 vTexCoord;
layout(location = 0) out vec4 FragColor;
layout(set = 0, binding = 2) uniform sampler2D Source;

void main()
{
	FragColor = texture(Source, vTexCoord);
#if defined(_HAS_ORIGINALASPECT_UNIFORMS) && defined(_HAS_FRAMETIME_UNIFORMS)
	if (vTexCoord.y > 0.5)
		FragColor.b = vTexCoord.x < 0.5
			? global.OriginalAspect / 4.0
			: float(global.FrameTimeDelta) / 100000.0;
#endif
}
