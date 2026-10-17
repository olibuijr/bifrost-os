#version 450
// Tone mapping of a scene-linear (1.0 = reference white) extended-range image into a display's headroom, drawn with
// fullscreen.vert. The per-pixel version of render/src/layer_composite.rs (tone_map, tone_map_rgb). Mode 0 clips, 1 is
// extended Reinhard, 2 the ACES-style fit (the soft-knee roll-off is layer_mask_edr.comp). Not wired to a pipeline yet.
layout(push_constant) uniform Push {
    float exposure;
    float white;    // extended Reinhard white point
    float pad;
    uint mode;
} pc;

layout(set = 0, binding = 0) uniform sampler2D src;

layout(location = 0) in vec2 v_uv;
layout(location = 0) out vec4 out_color;

float map1(float x) {
    x = max(x, 0.0);
    if (pc.mode == 1u) {
        float w2 = max(pc.white * pc.white, 1e-6);
        return x * (1.0 + x / w2) / (1.0 + x);
    }
    if (pc.mode == 2u) {
        float a = x * (2.51 * x + 0.03);
        float b = x * (2.43 * x + 0.59) + 0.14;
        return clamp(a / b, 0.0, 1.0);
    }
    return min(x, 1.0);
}

void main() {
    vec4 c = texture(src, v_uv);
    // The source is premultiplied: map the straight colour, then premultiply again.
    vec3 rgb = c.a > 0.0 ? c.rgb / c.a : vec3(0.0);
    rgb *= pc.exposure;
    out_color = vec4(vec3(map1(rgb.r), map1(rgb.g), map1(rgb.b)) * c.a, c.a);
}
