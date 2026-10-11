//! GLSL ES 3.00 sources. One program draws every primitive: a per-vertex
//! `kind` picks between a rounded fill, a blurred shadow and a textured
//! rounded quad, so fills and textures batch into the same draw call.

/// Vertex attribute locations (must match `batch::Vertex`).
pub const LOC_POS: u32 = 0;
pub const LOC_UV: u32 = 1;
pub const LOC_LOCAL: u32 = 2;
pub const LOC_HALF: u32 = 3;
pub const LOC_COLOR: u32 = 4;
pub const LOC_PARAMS: u32 = 5;

pub const KIND_FILL: f32 = 0.0;
pub const KIND_SHADOW: f32 = 1.0;
pub const KIND_TEXTURE: f32 = 2.0;

pub const UNIFORM_VIEW: &str = "u_view";
pub const UNIFORM_TEX: &str = "u_tex";
pub const UNIFORM_ALPHA: &str = "u_alpha";

pub fn vertex_source() -> String {
    format!(
        "#version 300 es\n\
precision highp float;\n\
layout(location = {LOC_POS}) in vec2 a_pos;\n\
layout(location = {LOC_UV}) in vec2 a_uv;\n\
layout(location = {LOC_LOCAL}) in vec2 a_local;\n\
layout(location = {LOC_HALF}) in vec2 a_half;\n\
layout(location = {LOC_COLOR}) in vec4 a_color;\n\
layout(location = {LOC_PARAMS}) in vec4 a_params;\n\
uniform vec2 {UNIFORM_VIEW};\n\
out vec2 v_uv;\n\
out vec2 v_local;\n\
out vec2 v_half;\n\
out vec4 v_color;\n\
out vec4 v_params;\n\
void main() {{\n\
    // pixel coordinates, row 0 on top: the dma-buf's first row is GL's y = -1\n\
    gl_Position = vec4(a_pos / {UNIFORM_VIEW} * 2.0 - 1.0, 0.0, 1.0);\n\
    v_uv = a_uv;\n\
    v_local = a_local;\n\
    v_half = a_half;\n\
    v_color = a_color;\n\
    v_params = a_params;\n\
}}\n"
    )
}

pub fn fragment_source() -> String {
    format!(
        "#version 300 es\n\
precision highp float;\n\
uniform sampler2D {UNIFORM_TEX};\n\
in vec2 v_uv;\n\
in vec2 v_local;\n\
in vec2 v_half;\n\
in vec4 v_color;\n\
in vec4 v_params;\n\
out vec4 o_color;\n\
// signed distance to a rounded box centred on the origin\n\
float sd_rrect(vec2 p, vec2 half_size, float r) {{\n\
    vec2 q = abs(p) - half_size + r;\n\
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;\n\
}}\n\
void main() {{\n\
    float d = sd_rrect(v_local, v_half, v_params.x);\n\
    float kind = v_params.y;\n\
    if (kind < {k1:.1}) {{\n\
        float cov = clamp(0.5 - d, 0.0, 1.0);\n\
        o_color = vec4(v_color.rgb * v_color.a, v_color.a) * cov;\n\
    }} else if (kind < {k2:.1}) {{\n\
        float t = clamp(0.5 - d / max(v_params.z, 0.001), 0.0, 1.0);\n\
        float cov = t * t * (3.0 - 2.0 * t);\n\
        o_color = vec4(v_color.rgb * v_color.a, v_color.a) * cov;\n\
    }} else {{\n\
        float cov = clamp(0.5 - d, 0.0, 1.0);\n\
        o_color = texture({UNIFORM_TEX}, v_uv) * (v_params.z * cov);\n\
    }}\n\
}}\n",
        k1 = KIND_SHADOW - 0.5,
        k2 = KIND_TEXTURE - 0.5,
    )
}

/// Layer program (video picture, uploaded frame or overlay): position and texture coordinate.
pub const LAYER_LOC_POS: u32 = 0;
pub const LAYER_LOC_UV: u32 = 1;

pub fn layer_vertex_source() -> String {
    format!(
        "#version 300 es\n\
precision highp float;\n\
layout(location = {LAYER_LOC_POS}) in vec2 a_pos;\n\
layout(location = {LAYER_LOC_UV}) in vec2 a_uv;\n\
uniform vec2 {UNIFORM_VIEW};\n\
out vec2 v_uv;\n\
void main() {{\n\
    gl_Position = vec4(a_pos / {UNIFORM_VIEW} * 2.0 - 1.0, 0.0, 1.0);\n\
    v_uv = a_uv;\n\
}}\n"
    )
}

/// Samples a plain RGBA texture (`external` false) or an external image, which is how an NV12
/// dma-buf is sampled: the driver converts YCbCr to RGB with the import's colour hints.
pub fn layer_fragment_source(external: bool) -> String {
    let (ext, sampler) = if external { ("#extension GL_OES_EGL_image_external_essl3 : require\n", "samplerExternalOES") } else { ("", "sampler2D") };
    format!(
        "#version 300 es\n\
{ext}precision mediump float;\n\
uniform {sampler} {UNIFORM_TEX};\n\
in vec2 v_uv;\n\
out vec4 o_color;\n\
void main() {{\n\
    o_color = texture({UNIFORM_TEX}, v_uv);\n\
}}\n"
    )
}

/// The layer sampler scaled by `u_alpha` (the rotation turn's cross-fade: the old picture at
/// 1 - k replaces, the new one at k is added).
pub fn fade_fragment_source() -> String {
    format!(
        "#version 300 es\n\
precision mediump float;\n\
uniform sampler2D {UNIFORM_TEX};\n\
uniform float {UNIFORM_ALPHA};\n\
in vec2 v_uv;\n\
out vec4 o_color;\n\
void main() {{\n\
    o_color = texture({UNIFORM_TEX}, v_uv) * {UNIFORM_ALPHA};\n\
}}\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layer_sources_are_es3_and_the_extension_comes_right_after_the_version() {
        let v = layer_vertex_source();
        assert!(v.starts_with("#version 300 es\n"));
        assert!(v.contains(&format!("layout(location = {LAYER_LOC_POS}) in vec2 a_pos;")));
        assert!(v.contains(&format!("layout(location = {LAYER_LOC_UV}) in vec2 a_uv;")));
        let e = layer_fragment_source(true);
        assert!(e.starts_with("#version 300 es\n#extension GL_OES_EGL_image_external_essl3 : require\n"));
        assert!(e.contains(&format!("uniform samplerExternalOES {UNIFORM_TEX};")));
        let p = layer_fragment_source(false);
        assert!(!p.contains("#extension"));
        assert!(p.contains(&format!("uniform sampler2D {UNIFORM_TEX};")));
        for s in [v, e, p] {
            assert!(!s.contains("{{") && !s.contains("}}"));
            assert_eq!(s.matches('{').count(), s.matches('}').count());
        }
    }

    #[test]
    fn fade_source_scales_by_alpha() {
        let f = fade_fragment_source();
        assert!(f.starts_with("#version 300 es\n"));
        assert!(f.contains("uniform float u_alpha;") && f.contains("* u_alpha;"));
        assert_eq!(f.matches('{').count(), f.matches('}').count());
    }

    #[test]
    fn both_stages_are_es3() {
        assert!(vertex_source().starts_with("#version 300 es\n"));
        assert!(fragment_source().starts_with("#version 300 es\n"));
    }

    #[test]
    fn attribute_locations_are_declared() {
        let v = vertex_source();
        for (loc, name) in [
            (LOC_POS, "a_pos"),
            (LOC_UV, "a_uv"),
            (LOC_LOCAL, "a_local"),
            (LOC_HALF, "a_half"),
            (LOC_COLOR, "a_color"),
            (LOC_PARAMS, "a_params"),
        ] {
            assert!(v.contains(&format!("layout(location = {loc}) in")), "{name}");
            assert!(v.contains(name));
        }
    }

    #[test]
    fn locations_are_distinct() {
        let mut l = [LOC_POS, LOC_UV, LOC_LOCAL, LOC_HALF, LOC_COLOR, LOC_PARAMS];
        l.sort();
        assert_eq!(l, [0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn varyings_match_between_stages() {
        let (v, f) = (vertex_source(), fragment_source());
        for name in ["v_uv", "v_local", "v_half", "v_color", "v_params"] {
            assert!(v.contains("out ") && v.contains(name));
            assert!(f.contains(name), "{name}");
        }
    }

    #[test]
    fn uniforms_present() {
        assert!(vertex_source().contains(&format!("uniform vec2 {UNIFORM_VIEW};")));
        assert!(fragment_source().contains(&format!("uniform sampler2D {UNIFORM_TEX};")));
    }

    #[test]
    fn kind_thresholds_sit_between_the_kinds() {
        let f = fragment_source();
        assert!(f.contains("kind < 0.5"));
        assert!(f.contains("kind < 1.5"));
        assert!(KIND_FILL < 0.5 && KIND_SHADOW > 0.5 && KIND_SHADOW < 1.5 && KIND_TEXTURE > 1.5);
    }

    #[test]
    fn no_unformatted_braces_left() {
        for s in [vertex_source(), fragment_source()] {
            assert!(!s.contains("{{") && !s.contains("}}"));
            assert_eq!(s.matches('{').count(), s.matches('}').count());
        }
    }
}
