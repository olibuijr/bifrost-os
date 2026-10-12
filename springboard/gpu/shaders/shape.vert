#version 450
// Instanced SDF shapes (gfx2d `shapes`). One instance is one quad (triangle strip, 4 vertices) in local pixels,
// expanded by the antialiasing margin and, for shadows, by three sigma. The instance layout is
// `gfx2d::shapes::Instance` (128 bytes, eight vec4 attributes at locations 0..7).
//
// Transform: device = [a c; b d] * local + (tx, ty), where xform = (a, b, c, d) and xform_t = (tx, ty, ., .).
// The fragment shader gets the local position, so every SDF is evaluated in local pixels.

layout(location = 0) in vec4 i_rect;      // x, y, w, h (local)
layout(location = 1) in vec4 i_params;    // radius, width | shadow sigma | gradient t0, kind, gradient t1
layout(location = 2) in vec4 i_color;     // straight RGBA 0..1 (gradient: colour at t0; mask: tint)
layout(location = 3) in vec4 i_color2;    // gradient colour at t1; image and mask uv rect (u0, v0, u1, v1)
layout(location = 4) in vec4 i_xform;     // a, b, c, d
layout(location = 5) in vec4 i_xform_t;   // tx, ty | gradient start x, y (local)
layout(location = 6) in vec4 i_clip;      // device-pixel clip x, y, w, h (w = 0: no clip)
layout(location = 7) in vec4 i_extra;     // gradient end x, y | clip corner radius | gradient flags

layout(push_constant) uniform Push {
    vec2 target_size;   // render target size in device pixels
} pc;

layout(location = 0) out vec2 v_local;
layout(location = 1) flat out vec4 v_rect;
layout(location = 2) flat out vec4 v_params;
layout(location = 3) flat out vec4 v_color;
layout(location = 4) flat out vec4 v_color2;
layout(location = 5) flat out vec4 v_xform_t;
layout(location = 6) flat out vec4 v_clip;
layout(location = 7) flat out vec4 v_extra;

void main() {
    int kind = int(i_params.z + 0.5);
    float sigma = max(i_params.y, 0.0);
    // One device pixel in local units: the smaller axis scale decides how wide the fringe must be.
    float scale = min(length(i_xform.xy), length(i_xform.zw));
    float margin = 1.0 / max(scale, 1e-3);
    if (kind == 2) {
        margin += 3.0 * sigma;
    }
    vec2 corner = vec2(float(gl_VertexIndex & 1), float(gl_VertexIndex >> 1));
    vec2 lp = i_rect.xy - vec2(margin) + corner * (i_rect.zw + vec2(2.0 * margin));
    vec2 dev = vec2(i_xform.x * lp.x + i_xform.z * lp.y, i_xform.y * lp.x + i_xform.w * lp.y) + i_xform_t.xy;
    gl_Position = vec4(dev / pc.target_size * 2.0 - 1.0, 0.0, 1.0);

    v_local = lp;
    v_rect = i_rect;
    v_params = i_params;
    v_color = i_color;
    v_color2 = i_color2;
    v_xform_t = i_xform_t;
    v_clip = i_clip;
    v_extra = i_extra;
}
