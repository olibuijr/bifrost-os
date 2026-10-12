#version 450
// Composition quad: a triangle strip of four corners (top-left, top-right, bottom-left, bottom-right) given in output
// pixels, with the source rectangle (normalised texture coordinates: x, y, w, h). The block must match
// `Push` in wm/src/render_server/compose.rs (128 bytes, std430 offsets in its doc comment).
layout(push_constant) uniform Push {
    vec2 p0;
    vec2 p1;
    vec2 p2;
    vec2 p3;
    vec4 src;
    vec4 color;
    vec4 clip;
    float clip_radius;
    float opacity;
    uint flags;
    float saturation;
    vec2 out_size;
    float shadow_sigma;
    float shadow_radius;
    vec4 shadow_rect;
} pc;

layout(location = 0) out vec2 v_px;
layout(location = 1) out vec2 v_uv;

void main() {
    vec2 corners[4] = vec2[4](pc.p0, pc.p1, pc.p2, pc.p3);
    vec2 frac = vec2(float(gl_VertexIndex & 1), float(gl_VertexIndex >> 1));
    vec2 px = corners[gl_VertexIndex];
    v_px = px;
    v_uv = pc.src.xy + pc.src.zw * frac;
    // output pixels, origin top-left, to clip space
    gl_Position = vec4(px / pc.out_size * 2.0 - 1.0, 0.0, 1.0);
}
