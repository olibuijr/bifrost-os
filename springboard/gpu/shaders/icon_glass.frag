#version 450
// Frosted-glass icon layer, drawn with fullscreen.vert. The per-pixel version of render/src/layer_effects.rs
// (render_glass_layer): the blurred backdrop is mixed with the tint, the shape's softened height field gives a bevel
// whose slope against the light lights one rim (screen) and shades the other (multiply), and the inner shadow plane
// darkens the edge. Output is premultiplied. The blurs come from the Kawase passes; not wired to a pipeline yet.
layout(push_constant) uniform Push {
    vec4 tint;          // rgb tint, a = how much of the tint replaces the backdrop
    vec2 light;         // unit vector toward the light, texture space (y down)
    vec2 texel;         // 1 / size of the height texture
    float bevel_depth;
    float specular;
    float inner_strength;
    float pad;
} pc;

layout(set = 0, binding = 0) uniform sampler2D mask;      // shape coverage (r)
layout(set = 0, binding = 1) uniform sampler2D height;    // softened shape coverage (r)
layout(set = 0, binding = 2) uniform sampler2D backdrop;  // blurred backdrop (rgb)
layout(set = 0, binding = 3) uniform sampler2D inner;     // inner shadow coverage (r), already clipped to the shape

layout(location = 0) in vec2 v_uv;
layout(location = 0) out vec4 out_color;

void main() {
    float cover = clamp(texture(mask, v_uv).r, 0.0, 1.0);
    float gx = (texture(height, v_uv + vec2(pc.texel.x, 0.0)).r - texture(height, v_uv - vec2(pc.texel.x, 0.0)).r) * 0.5;
    float gy = (texture(height, v_uv + vec2(0.0, pc.texel.y)).r - texture(height, v_uv - vec2(0.0, pc.texel.y)).r) * 0.5;
    float s = -(gx * pc.light.x + gy * pc.light.y) * pc.bevel_depth;
    float hi = clamp(s, 0.0, 1.0) * cover * pc.specular;
    float lo = clamp(-s, 0.0, 1.0) * cover * pc.specular * 0.5;
    float shade = clamp(max(lo, texture(inner, v_uv).r * pc.inner_strength), 0.0, 1.0);

    vec3 base = mix(texture(backdrop, v_uv).rgb, pc.tint.rgb, clamp(pc.tint.a, 0.0, 1.0));
    vec3 lit = base + (1.0 - base) * clamp(hi, 0.0, 1.0);
    vec3 rgb = clamp(lit * (1.0 - shade), 0.0, 1.0);
    out_color = vec4(rgb * cover, cover);
}
