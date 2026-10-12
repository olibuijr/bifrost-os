#version 450
// Composes one quad into the target (premultiplied output, blended source-over by the pipeline). Flags (see
// wm/src/render_server/compose.rs): 1 samples the texture, 2 draws a shadow of shadow_rect, 4 is glass: the sampled
// backdrop with saturation, the tone (brightness and luminance mapping, carried in the shadow fields: shadow_sigma is
// the brightness, shadow_radius the luminance amount, shadow_rect the four luminance values) and the tint (color,
// premultiplied) over it. Every path is multiplied by the opacity and
// by the rounded clip's coverage when clip.z > 0.
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

layout(set = 0, binding = 0) uniform texture2D tex;
layout(set = 0, binding = 1) uniform sampler smp;

layout(location = 0) in vec2 v_px;
layout(location = 1) in vec2 v_uv;
layout(location = 0) out vec4 out_color;

// Signed distance to a rounded rectangle r = (x, y, w, h) with corner radius rad, in pixels.
float rrect_sdf(vec2 p, vec4 r, float rad) {
    vec2 h = r.zw * 0.5;
    vec2 c = r.xy + h;
    float rr = min(rad, min(h.x, h.y));
    vec2 q = abs(p - c) - h + rr;
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - rr;
}

// The luminance curve: the cubic Bezier through four control values (wm/src/material_filter.rs, curve).
float lum_curve(vec4 v, float x) {
    float u = 1.0 - x;
    return u * u * u * v.x + 3.0 * u * u * x * v.y + 3.0 * u * x * x * v.z + x * x * x * v.w;
}

void main() {
    float cov = 1.0;
    if (pc.clip.z > 0.0) {
        cov = clamp(0.5 - rrect_sdf(v_px, pc.clip, pc.clip_radius), 0.0, 1.0);
    }

    vec4 c;
    if ((pc.flags & 2u) != 0u) {
        // Gaussian edge: the logistic approximation of the normal CDF of the signed distance.
        float d = rrect_sdf(v_px, pc.shadow_rect, pc.shadow_radius);
        float a = 1.0 - 1.0 / (1.0 + exp(-1.702 * d / max(pc.shadow_sigma, 0.001)));
        c = pc.color * a;
    } else if ((pc.flags & 1u) != 0u) {
        c = texture(sampler2D(tex, smp), v_uv);
    } else {
        c = pc.color;
    }

    if ((pc.flags & 4u) != 0u) {
        float lum = dot(c.rgb, vec3(0.2126, 0.7152, 0.0722));
        c.rgb = clamp(mix(vec3(lum), c.rgb, pc.saturation), 0.0, 1.0);
        if (pc.shadow_radius != 0.0) {
            // luminance mapping: every channel moves by how far the luma moves towards the curve
            float l = dot(c.rgb, vec3(0.2126, 0.7152, 0.0722));
            c.rgb = clamp(c.rgb + pc.shadow_radius * (lum_curve(pc.shadow_rect, clamp(l, 0.0, 1.0)) - l), 0.0, 1.0);
        }
        c.rgb = clamp(c.rgb + pc.shadow_sigma, 0.0, 1.0);
        // premultiplied tint over the saturated backdrop
        c = pc.color + c * (1.0 - pc.color.a);
    }

    out_color = c * (pc.opacity * cov);
}
