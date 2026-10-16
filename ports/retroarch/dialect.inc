// CC0. HLSL's names for GLSL's types and functions, so RetroArch reads the passes every port of
// CRTSim Renderer shares, in passes/, as they are written; and P, how they read a parameter.

#define float2 vec2
#define float3 vec3
#define float4 vec4
#define int2 ivec2
#define int3 ivec3
#define int4 ivec4
#define lerp mix
#define frac fract
#define ddx dFdx
#define ddy dFdy
#define P(name) global.name
