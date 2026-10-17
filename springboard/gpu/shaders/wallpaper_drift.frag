#version 450
// Drift wallpaper: domain-warped fractal value noise coloured through a four-stop ramp, drawn with fullscreen.vert.
// The per-pixel version of render/src/wallpaper_shaders.rs (drift_value, drift_frame). Coordinates are divided by the
// longer side so the look is independent of resolution and aspect ratio. Not wired to a pipeline yet.
layout(push_constant) uniform Push {
    vec4 c0;
    vec4 c1;
    vec4 c2;
    vec4 c3;
    vec2 size;   // target size in pixels
    float time;  // seconds
    float scale; // noise cells across the longer side
    uint seed;
} pc;

layout(location = 0) in vec2 v_uv;
layout(location = 0) out vec4 out_color;

float hash(ivec2 p, uint seed) {
    uint h = uint(p.x) * 0x85EBCA6Bu ^ uint(p.y) * 0xC2B2AE35u ^ seed * 0x27D4EB2Fu;
    h ^= h >> 15;
    h *= 0x2C1B3C6Du;
    h ^= h >> 12;
    h *= 0x297A2D39u;
    h ^= h >> 15;
    return float(h >> 8) / 16777216.0;
}

float value_noise(vec2 p, uint seed) {
    vec2 i = floor(p);
    vec2 f = p - i;
    f = f * f * (3.0 - 2.0 * f);
    ivec2 ip = ivec2(i);
    float a = hash(ip, seed);
    float b = hash(ip + ivec2(1, 0), seed);
    float c = hash(ip + ivec2(0, 1), seed);
    float d = hash(ip + ivec2(1, 1), seed);
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

float fbm(vec2 p, int octaves, uint seed) {
    float sum = 0.0;
    float amp = 0.5;
    float norm = 0.0;
    for (int o = 0; o < octaves; ++o) {
        sum += amp * value_noise(p, seed + uint(o));
        norm += amp;
        amp *= 0.5;
        p *= 2.0;
    }
    return sum / norm;
}

vec3 ramp(float t) {
    t = clamp(t, 0.0, 1.0) * 3.0;
    if (t < 1.0) return mix(pc.c0.rgb, pc.c1.rgb, t);
    if (t < 2.0) return mix(pc.c1.rgb, pc.c2.rgb, t - 1.0);
    return mix(pc.c2.rgb, pc.c3.rgb, t - 2.0);
}

void main() {
    vec2 px = v_uv * pc.size;
    vec2 p = px / max(pc.size.x, pc.size.y) * pc.scale;
    float qx = fbm(p + vec2(pc.time * 0.05, 0.0), 3, pc.seed);
    float qy = fbm(p + vec2(5.2, 1.3 - pc.time * 0.04), 3, pc.seed + 7u);
    float f = fbm(p + 4.0 * vec2(qx, qy) + vec2(pc.time * 0.03, 0.0), 4, pc.seed + 13u);
    out_color = vec4(ramp(smoothstep(0.2, 0.8, f)), 1.0);
}
