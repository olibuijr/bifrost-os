//! GLSL ES 3.00 generation for the Core Image colour filters (`CIColorMatrix`, `CIGammaAdjust`,
//! `CISourceOverCompositing`), so a filter chain can run as a fragment shader when a GL context
//! exists. Everything here is pure: shader source, uniform packing, a CPU reference evaluator of
//! the same per-pixel expression, and the backend choice (`CIContext` `useSoftwareRenderer`).
//! Pixels follow `bifrost_render::composite_filters`: red in the low byte, alpha in the top byte.
//! Colour stages read straight RGBA, so the generated shader unpremultiplies the sampled texel
//! first and premultiplies the result, as `gfx::Image` textures are premultiplied.

use bifrost_render::composite_filters::ColorMatrix;

pub const UNIFORM_TEX: &str = "u_tex";
pub const UNIFORM_FG: &str = "u_fg";
pub const UNIFORM_BG: &str = "u_bg";

/// One single-input colour stage of a `CIColorKernel` chain.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Stage {
    /// `CIColorMatrix`.
    Matrix(ColorMatrix),
    /// `CIGammaAdjust` (`inputPower`); powers at or below zero act as 1, as on the CPU.
    Gamma(f32),
}

/// A chain of colour stages run in one fragment shader (`CIColorKernel`).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ColorKernel {
    pub stages: Vec<Stage>,
}

/// Where a chain runs, per `CIContext(options: [.useSoftwareRenderer: ...])`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    Gpu,
    Cpu,
}

/// `CIContext` options relevant to the backend choice.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct ContextOptions {
    pub use_software_renderer: bool,
}

impl Backend {
    /// The GPU runs a chain only when a GL context exists and software rendering was not asked for.
    pub fn choose(has_gl_context: bool, options: ContextOptions) -> Backend {
        if has_gl_context && !options.use_software_renderer {
            Backend::Gpu
        } else {
            Backend::Cpu
        }
    }
}

/// The power the CPU path uses: non-finite or non-positive powers become 1.
pub fn effective_power(power: f32) -> f32 {
    if power.is_finite() && power > 0.0 {
        power
    } else {
        1.0
    }
}

/// Column-major `mat4` for `u_m<i>`: column j is `[r[j], g[j], b[j], a[j]]`, so `M * v` gives the
/// four row dot products.
pub fn pack_matrix(m: &ColorMatrix) -> [f32; 16] {
    let mut out = [0.0; 16];
    for j in 0..4 {
        out[j * 4] = m.r[j];
        out[j * 4 + 1] = m.g[j];
        out[j * 4 + 2] = m.b[j];
        out[j * 4 + 3] = m.a[j];
    }
    out
}

/// A uniform value for a stage, named as in the generated source.
#[derive(Clone, Debug, PartialEq)]
pub enum Uniform {
    Mat4(String, [f32; 16]),
    Vec4(String, [f32; 4]),
    Float(String, f32),
}

impl ColorKernel {
    pub fn new(stages: Vec<Stage>) -> Self {
        ColorKernel { stages }
    }

    /// Uniforms in stage order: `u_m<i>` and `u_b<i>` for a matrix, `u_g<i>` for gamma.
    pub fn uniforms(&self) -> Vec<Uniform> {
        let mut out = Vec::new();
        for (i, s) in self.stages.iter().enumerate() {
            match s {
                Stage::Matrix(m) => {
                    out.push(Uniform::Mat4(format!("u_m{i}"), pack_matrix(m)));
                    out.push(Uniform::Vec4(format!("u_b{i}"), m.bias));
                }
                Stage::Gamma(p) => out.push(Uniform::Float(format!("u_g{i}"), effective_power(*p))),
            }
        }
        out
    }

    /// The fragment shader: sample, unpremultiply, run each stage (clamping after each, as the CPU
    /// quantises between filters), premultiply.
    pub fn fragment_source(&self) -> String {
        let mut decl = String::new();
        let mut body = String::new();
        for (i, s) in self.stages.iter().enumerate() {
            match s {
                Stage::Matrix(_) => {
                    decl += &format!("uniform mat4 u_m{i};\nuniform vec4 u_b{i};\n");
                    body += &format!("    c = clamp(u_m{i} * c + u_b{i}, 0.0, 1.0);\n");
                }
                Stage::Gamma(_) => {
                    decl += &format!("uniform float u_g{i};\n");
                    body += &format!("    c = vec4(clamp(pow(c.rgb, vec3(u_g{i})), 0.0, 1.0), c.a);\n");
                }
            }
        }
        format!(
            "#version 300 es\n\
precision highp float;\n\
uniform sampler2D {UNIFORM_TEX};\n\
{decl}\
in vec2 v_uv;\n\
out vec4 o_color;\n\
void main() {{\n\
    vec4 t = texture({UNIFORM_TEX}, v_uv);\n\
    vec4 c = vec4(t.a > 0.0 ? t.rgb / t.a : vec3(0.0), t.a);\n\
{body}\
    o_color = vec4(c.rgb * c.a, c.a);\n\
}}\n"
        )
    }

    /// CPU reference of the generated expression on one straight packed pixel, in f32 with the same
    /// clamps; quantised once at the end.
    pub fn eval_pixel(&self, px: u32) -> u32 {
        let ch = |s: u32| ((px >> s) & 0xff) as f32 / 255.0;
        let mut c = [ch(0), ch(8), ch(16), ch(24)];
        for s in &self.stages {
            match s {
                Stage::Matrix(m) => {
                    let row = |r: &[f32; 4], b: f32| r[0] * c[0] + r[1] * c[1] + r[2] * c[2] + r[3] * c[3] + b;
                    c = [
                        row(&m.r, m.bias[0]).clamp(0.0, 1.0),
                        row(&m.g, m.bias[1]).clamp(0.0, 1.0),
                        row(&m.b, m.bias[2]).clamp(0.0, 1.0),
                        row(&m.a, m.bias[3]).clamp(0.0, 1.0),
                    ];
                }
                Stage::Gamma(p) => {
                    let p = effective_power(*p);
                    for v in c.iter_mut().take(3) {
                        *v = v.powf(p).clamp(0.0, 1.0);
                    }
                }
            }
        }
        let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u32;
        q(c[0]) | (q(c[1]) << 8) | (q(c[2]) << 16) | (q(c[3]) << 24)
    }
}

/// `CISourceOverCompositing` on premultiplied inputs: `fg + bg * (1 - fg.a)`.
pub fn source_over_fragment_source() -> String {
    format!(
        "#version 300 es\n\
precision highp float;\n\
uniform sampler2D {UNIFORM_FG};\n\
uniform sampler2D {UNIFORM_BG};\n\
in vec2 v_uv;\n\
out vec4 o_color;\n\
void main() {{\n\
    vec4 f = texture({UNIFORM_FG}, v_uv);\n\
    vec4 b = texture({UNIFORM_BG}, v_uv);\n\
    o_color = f + b * (1.0 - f.a);\n\
}}\n"
    )
}

/// CPU reference of the source-over shader on premultiplied packed pixels.
pub fn source_over_eval(fg: u32, bg: u32) -> u32 {
    let ch = |p: u32, s: u32| ((p >> s) & 0xff) as f32 / 255.0;
    let fa = ch(fg, 24);
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u32;
    let mix = |s: u32| q(ch(fg, s) + ch(bg, s) * (1.0 - fa)) << s;
    mix(0) | mix(8) | mix(16) | mix(24)
}

/// The vertex shader for a full-target quad; `v_uv` comes straight from the attribute.
pub fn vertex_source() -> String {
    "#version 300 es\n\
precision highp float;\n\
layout(location = 0) in vec2 a_pos;\n\
layout(location = 1) in vec2 a_uv;\n\
out vec2 v_uv;\n\
void main() {\n\
    gl_Position = vec4(a_pos, 0.0, 1.0);\n\
    v_uv = a_uv;\n\
}\n"
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_render::composite_filters::{composite, gamma, Blend};

    /// A structural parse check: version first, balanced braces and parens, `main`, and every
    /// statement line ends with `;`, `{` or `}`.
    fn parses(src: &str) -> bool {
        let (mut brace, mut paren) = (0i32, 0i32);
        for c in src.chars() {
            match c {
                '{' => brace += 1,
                '}' => brace -= 1,
                '(' => paren += 1,
                ')' => paren -= 1,
                _ => {}
            }
            if brace < 0 || paren < 0 {
                return false;
            }
        }
        src.starts_with("#version 300 es\n")
            && brace == 0
            && paren == 0
            && src.contains("void main()")
            && src.lines().skip(1).all(|l| {
                let t = l.trim_end();
                t.is_empty() || t.starts_with("precision") || t.ends_with(';') || t.ends_with('{') || t.ends_with('}')
            })
    }

    fn samples() -> Vec<u32> {
        vec![0xff00_0000, 0xffff_ffff, 0xff80_8080, 0xff11_2233, 0x80ff_8040, 0x0000_0000, 0xc010_f0a0]
    }

    fn close(a: u32, b: u32, tol: u32) -> bool {
        (0..4).all(|s| ((a >> (s * 8)) & 0xff).abs_diff((b >> (s * 8)) & 0xff) <= tol)
    }

    fn swap_rb() -> ColorMatrix {
        let mut m = ColorMatrix::identity();
        m.r = [0.0, 0.0, 1.0, 0.0];
        m.b = [1.0, 0.0, 0.0, 0.0];
        m.bias = [0.1, -0.05, 0.0, 0.0];
        m
    }

    #[test]
    fn every_generated_shader_parses_and_declares_its_uniforms() {
        let k = ColorKernel::new(vec![Stage::Matrix(swap_rb()), Stage::Gamma(2.2), Stage::Matrix(ColorMatrix::identity())]);
        let src = k.fragment_source();
        assert!(parses(&src), "{src}");
        for u in k.uniforms() {
            let name = match &u {
                Uniform::Mat4(n, _) | Uniform::Vec4(n, _) | Uniform::Float(n, _) => n,
            };
            assert!(src.contains(&format!(" {name};")), "{name} undeclared");
        }
        assert!(parses(&ColorKernel::default().fragment_source()));
        assert!(parses(&source_over_fragment_source()));
        assert!(parses(&vertex_source()));
    }

    #[test]
    fn matrix_uniform_is_column_major_and_matches_cpu_rows() {
        let mut m = ColorMatrix::identity();
        m.r = [1.0, 2.0, 3.0, 4.0];
        m.g = [5.0, 6.0, 7.0, 8.0];
        m.b = [9.0, 10.0, 11.0, 12.0];
        m.a = [13.0, 14.0, 15.0, 16.0];
        let p = pack_matrix(&m);
        assert_eq!(&p[0..4], &[1.0, 5.0, 9.0, 13.0]);
        assert_eq!(&p[12..16], &[4.0, 8.0, 12.0, 16.0]);
        let v = [0.1, 0.2, 0.3, 0.4];
        let rows = [m.r, m.g, m.b, m.a];
        for (i, row) in rows.iter().enumerate() {
            let gl: f32 = (0..4).map(|j| p[j * 4 + i] * v[j]).sum();
            let cpu: f32 = (0..4).map(|j| row[j] * v[j]).sum();
            assert!((gl - cpu).abs() < 1e-4);
        }
    }

    #[test]
    fn gamma_uniform_matches_cpu_power_rules() {
        let u = ColorKernel::new(vec![Stage::Gamma(0.0), Stage::Gamma(2.0), Stage::Gamma(f32::NAN)]).uniforms();
        assert_eq!(
            u,
            vec![Uniform::Float("u_g0".into(), 1.0), Uniform::Float("u_g1".into(), 2.0), Uniform::Float("u_g2".into(), 1.0)]
        );
    }

    #[test]
    fn matrix_reference_matches_cpu_within_one_step() {
        let m = swap_rb();
        let k = ColorKernel::new(vec![Stage::Matrix(m)]);
        for px in samples() {
            assert!(close(k.eval_pixel(px), m.apply_pixel(px), 1), "{px:08x}");
        }
    }

    #[test]
    fn gamma_reference_matches_cpu_within_one_step() {
        for power in [0.5f32, 1.0, 2.0, 2.2, 0.0] {
            let k = ColorKernel::new(vec![Stage::Gamma(power)]);
            for px in samples() {
                assert!(close(k.eval_pixel(px), gamma(&[px], power)[0], 1), "{px:08x} p{power}");
            }
        }
    }

    #[test]
    fn chain_reference_matches_stage_by_stage_cpu() {
        let m = swap_rb();
        let k = ColorKernel::new(vec![Stage::Matrix(m), Stage::Gamma(2.0)]);
        for px in samples() {
            let cpu = gamma(&[m.apply_pixel(px)], 2.0)[0];
            assert!(close(k.eval_pixel(px), cpu, 2), "{px:08x}");
        }
    }

    #[test]
    fn source_over_reference_matches_cpu_composite() {
        // Premultiplied inputs: scale colour by alpha so the pairs are valid.
        let pre = |p: u32| {
            let a = (p >> 24) & 0xff;
            let m = |s: u32| ((((p >> s) & 0xff) * a + 127) / 255) << s;
            m(0) | m(8) | m(16) | (a << 24)
        };
        let px: Vec<u32> = samples().into_iter().map(pre).collect();
        for &f in &px {
            for &b in &px {
                let cpu = composite(&[f], &[b], 1, 1, Blend::SourceOver).unwrap()[0];
                assert!(close(source_over_eval(f, b), cpu, 1), "{f:08x} over {b:08x}");
            }
        }
    }

    #[test]
    fn backend_falls_back_to_cpu_without_a_context_or_when_software_is_asked() {
        let gpu = ContextOptions { use_software_renderer: false };
        let soft = ContextOptions { use_software_renderer: true };
        assert_eq!(Backend::choose(true, gpu), Backend::Gpu);
        assert_eq!(Backend::choose(false, gpu), Backend::Cpu);
        assert_eq!(Backend::choose(true, soft), Backend::Cpu);
    }
}
