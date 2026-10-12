#version 450
// Shape fragment shader (gfx2d `shapes`). Output is premultiplied RGBA for PremultipliedOver blending.
//
// Kinds: 0 fill (rounded rect), 1 stroke (|d| band of width w inside the outline), 2 shadow (Gaussian of the
// distance: 0.5 * erfc(d / (sigma * sqrt 2))), 3 linear gradient segment (colour mixed between t0 and t1, pads
// at the ends when the flags say so), 4 image (texture RGBA, premultiplied, times the colour alpha), 5 mask (texture
// red channel as coverage, filled with the colour).
// Shape coverage is 1 px analytic antialiasing: clamp(0.5 - d / fwidth(d)), which is exactly 1 px in device pixels
// whatever the transform. The rounded clip is evaluated in device pixels from gl_FragCoord.

layout(location = 0) in vec2 v_local;
layout(location = 1) flat in vec4 v_rect;
layout(location = 2) flat in vec4 v_params;
layout(location = 3) flat in vec4 v_color;
layout(location = 4) flat in vec4 v_color2;
layout(location = 5) flat in vec4 v_xform_t;
layout(location = 6) flat in vec4 v_clip;
layout(location = 7) flat in vec4 v_extra;

layout(location = 0) out vec4 out_color;

layout(set = 0, binding = 0) uniform texture2D u_tex;
layout(set = 0, binding = 1) uniform sampler u_samp;

// Signed distance to a rounded rectangle (negative inside). `r` must not exceed the half size.
float sd_rrect(vec2 p, vec2 centre, vec2 half_size, float r) {
    vec2 q = abs(p - centre) - (half_size - vec2(r));
    return length(max(q, vec2(0.0))) + min(max(q.x, q.y), 0.0) - r;
}

// 1 px antialiased inside test for a signed distance d.
float coverage(float d) {
    return clamp(0.5 - d / max(fwidth(d), 1e-6), 0.0, 1.0);
}

// erf by the Abramowitz and Stegun 7.1.26 approximation (absolute error below 1.5e-7).
float erf_approx(float x) {
    float s = sign(x);
    float ax = abs(x);
    float t = 1.0 / (1.0 + 0.3275911 * ax);
    float poly = (((((1.061405429 * t - 1.453152027) * t) + 1.421413741) * t - 0.284496736) * t + 0.254829592) * t;
    return s * (1.0 - poly * exp(-ax * ax));
}

void main() {
    int kind = int(v_params.z + 0.5);
    int flags = int(v_extra.w + 0.5);

    vec2 half_size = v_rect.zw * 0.5;
    vec2 centre = v_rect.xy + half_size;
    float radius = min(v_params.x, min(half_size.x, half_size.y));
    float d = sd_rrect(v_local, centre, half_size, radius);

    // Coverage terms that need derivatives are computed before any branch.
    float shape_cov = coverage(d);
    float stroke_w = v_params.y;
    float stroke_cov = coverage(abs(d + 0.5 * stroke_w) - 0.5 * stroke_w);

    // Rounded clip in device pixels (w = 0 means no clip).
    float clip_cov = 1.0;
    if (v_clip.z > 0.0) {
        vec2 clip_half = v_clip.zw * 0.5;
        vec2 clip_centre = v_clip.xy + clip_half;
        float clip_r = min(v_extra.z, min(clip_half.x, clip_half.y));
        clip_cov = coverage(sd_rrect(gl_FragCoord.xy, clip_centre, clip_half, clip_r));
    }

    vec4 colour = v_color;
    float cov = shape_cov;

    if (kind == 4 || kind == 5) {
        vec2 f = (v_local - v_rect.xy) / max(v_rect.zw, vec2(1e-6));
        vec2 uv = mix(v_color2.xy, v_color2.zw, f);
        vec4 texel = texture(sampler2D(u_tex, u_samp), uv);
        float a = v_color.a * shape_cov * clip_cov;
        if (kind == 4) {
            out_color = texel * a;
        } else {
            float m = texel.r * a;
            out_color = vec4(v_color.rgb * m, m);
        }
        return;
    }

    if (kind == 1) {
        cov = stroke_cov;
    } else if (kind == 2) {
        float sigma = max(v_params.y, 0.01);
        cov = 0.5 * (1.0 - erf_approx(d / (sigma * 1.41421356237)));
    } else if (kind == 3) {
        float t0 = v_params.y;
        float t1 = v_params.w;
        vec2 seg = v_extra.xy - v_xform_t.zw;
        float len2 = dot(seg, seg);
        float t = len2 > 0.0 ? dot(v_local - v_xform_t.zw, seg) / len2 : 0.0;
        bool pad_before = (flags & 1) != 0;
        bool pad_after = (flags & 2) != 0;
        if ((t < t0 && !pad_before) || (t > t1 && !pad_after)) {
            cov = 0.0;
        }
        float u = clamp((clamp(t, t0, t1) - t0) / max(t1 - t0, 1e-9), 0.0, 1.0);
        colour = mix(v_color, v_color2, u);
    }

    float a = colour.a * cov * clip_cov;
    out_color = vec4(colour.rgb * a, a);
}
