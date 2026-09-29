#version 450

// CC0 1.0: written for this repository's tests and dedicated to the public domain.
//
// Inverts red, adds STEP to green on top of what this pass drew last frame, and writes the
// frame count to blue. Every value is a whole 8-bit step, so the result is exact. Alpha is
// zero, which hosts ignore.

#pragma parameter STEP "Green added each frame, in 8-bit steps" 16.0 0.0 255.0 1.0

layout(push_constant) uniform Push
{
	float STEP;
} params;

layout(std140, set = 0, binding = 0) uniform UBO
{
	mat4 MVP;
	uint FrameCount;
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
layout(set = 0, binding = 3) uniform sampler2D PassFeedback0;

void main()
{
	float red = texture(Source, vTexCoord).r;
	float green = texture(PassFeedback0, vTexCoord).g;
	FragColor = vec4(1.0 - red, green + params.STEP / 255.0, float(global.FrameCount) / 255.0, 0.0);
}
