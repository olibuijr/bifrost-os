#version 450
// Window drop shadow: the signed distance to the rounded rect (offset as the reference platform's large soft
// active-window shadow and smaller inactive one), turned into a Gaussian falloff with the erf of the distance over
// sigma * sqrt(2). Drawn with `fullscreen.vert` scissored to the shadow's bounds; premultiplied output.
// The push block is `ShadowPush` in render_server/material.rs (48 bytes).

layout(location = 0) out vec4 out_color;

layout(push_constant) uniform Push {
    vec4 rect;    // window rectangle in target pixels: x, y, w, h
    vec2 offset;  // shadow offset, target pixels
    float radius; // corner radius, target pixels
    float sigma;  // Gaussian sigma, target pixels
    vec4 color;   // premultiplied shadow colour
} pc;

// Abramowitz and Stegun 7.1.26 (absolute error below 1.5e-7).
float erf_approx(float x) {
    float s = sign(x);
    float a = abs(x);
    float t = 1.0 / (1.0 + 0.3275911 * a);
    float y = 1.0 - (((((1.061405429 * t - 1.453152027) * t) + 1.421413741) * t - 0.284496736) * t + 0.254829592) * t * exp(-a * a);
    return s * y;
}

float rounded_box_distance(vec2 p, vec2 centre, vec2 half_size, float r) {
    vec2 q = abs(p - centre) - half_size + vec2(r);
    return length(max(q, vec2(0.0))) + min(max(q.x, q.y), 0.0) - r;
}

void main() {
    vec2 centre = pc.rect.xy + 0.5 * pc.rect.zw + pc.offset;
    vec2 half_size = 0.5 * pc.rect.zw;
    float d = rounded_box_distance(gl_FragCoord.xy, centre, half_size, pc.radius);
    float coverage = 0.5 * (1.0 - erf_approx(d / (max(pc.sigma, 0.01) * 1.41421356)));
    out_color = pc.color * coverage;
}
