// CC0. One triangle over the whole target, with uv running from 0 at its top left to 1 at its
// bottom right, for each pass of the Godot port to draw every pixel of its target.

layout(location = 0) out vec2 uv;

void main() {
    uv = vec2(float((gl_VertexIndex << 1) & 2), float(gl_VertexIndex & 2));
    gl_Position = vec4(uv * 2.0 - 1.0, 0.0, 1.0);
}
