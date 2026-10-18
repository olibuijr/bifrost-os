//! Material-graph node library and a small shading-graph compiler. The node kernels are the procedural and
//! colour nodes of an open material-graph standard (gradient noise and fractal noise, cellular distance,
//! ramps, checkerboard, remap, contrast, HSV conversion, 2-D placement and a tangent-space normal-map
//! node), each as a plain function. [`ShaderGraph`] is a straight-line scalar program that evaluates on the
//! CPU and compiles to a GLSL function, with the noise helpers emitted as a prelude when the graph uses
//! them, so the CPU and the GPU run the same integer-hash gradient noise. The simpler graph evaluator in
//! `shading_graph` has no noise and no code generation; surface shading is `pbr`, scene materials are in
//! `bifrost-ui` `scene_material`. Written from the published node definitions and Perlin's improved-noise
//! formulation; the hash is our own.

use std::fmt::Write as _;

/// A well-mixed 32-bit integer hash (a public-domain style xorshift-multiply finaliser).
pub fn hash32(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^= x >> 16;
    x
}

fn lattice_hash(ix: i32, iy: i32, iz: i32) -> u32 {
    hash32((ix as u32).wrapping_add(hash32((iy as u32).wrapping_add(hash32(iz as u32)))))
}

fn fade(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

fn grad(h: u32, x: f32, y: f32, z: f32) -> f32 {
    let h = h & 15;
    let u = if h < 8 { x } else { y };
    let v = if h < 4 {
        y
    } else if h == 12 || h == 14 {
        x
    } else {
        z
    };
    (if h & 1 == 0 { u } else { -u }) + (if h & 2 == 0 { v } else { -v })
}

/// Improved gradient (Perlin) noise at a 3-D point, roughly in -1..=1, exactly 0 on integer lattice points.
pub fn noise3(x: f32, y: f32, z: f32) -> f32 {
    let (fx, fy, fz) = (x.floor(), y.floor(), z.floor());
    let (ix, iy, iz) = (fx as i32, fy as i32, fz as i32);
    let (dx, dy, dz) = (x - fx, y - fy, z - fz);
    let (u, v, w) = (fade(dx), fade(dy), fade(dz));
    let mut corner = [0.0f32; 8];
    for (n, c) in corner.iter_mut().enumerate() {
        let (ox, oy, oz) = ((n & 1) as i32, ((n >> 1) & 1) as i32, ((n >> 2) & 1) as i32);
        *c = grad(lattice_hash(ix + ox, iy + oy, iz + oz), dx - ox as f32, dy - oy as f32, dz - oz as f32);
    }
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let x00 = lerp(corner[0], corner[1], u);
    let x10 = lerp(corner[2], corner[3], u);
    let x01 = lerp(corner[4], corner[5], u);
    let x11 = lerp(corner[6], corner[7], u);
    lerp(lerp(x00, x10, v), lerp(x01, x11, v), w)
}

/// Fractal noise: `octaves` (1..=8) layers of [`noise3`], frequency doubling and amplitude halving.
pub fn fractal3(x: f32, y: f32, z: f32, octaves: u32) -> f32 {
    let mut sum = 0.0;
    let mut amp = 1.0;
    let mut freq = 1.0;
    for _ in 0..octaves.clamp(1, 8) {
        sum += amp * noise3(x * freq, y * freq, z * freq);
        amp *= 0.5;
        freq *= 2.0;
    }
    sum
}

/// Cellular (Worley) noise: the distance to the nearest of one jittered feature point per unit cell.
pub fn cellular3(x: f32, y: f32, z: f32) -> f32 {
    let (ix, iy, iz) = (x.floor() as i32, y.floor() as i32, z.floor() as i32);
    let mut best = f32::MAX;
    for dz in -1..=1 {
        for dy in -1..=1 {
            for dx in -1..=1 {
                let (cx, cy, cz) = (ix + dx, iy + dy, iz + dz);
                let h = lattice_hash(cx, cy, cz);
                let jitter = |shift: u32| ((hash32(h ^ shift) & 0xffff) as f32) / 65535.0;
                let px = cx as f32 + jitter(0x1111);
                let py = cy as f32 + jitter(0x2222);
                let pz = cz as f32 + jitter(0x3333);
                let d = ((px - x).powi(2) + (py - y).powi(2) + (pz - z).powi(2)).sqrt();
                best = best.min(d);
            }
        }
    }
    best
}

/// Left-to-right ramp: `left` at `s <= 0` blending to `right` at `s >= 1`.
pub fn ramp_lr(s: f32, left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
    let t = s.clamp(0.0, 1.0);
    [left[0] + (right[0] - left[0]) * t, left[1] + (right[1] - left[1]) * t, left[2] + (right[2] - left[2]) * t]
}

/// The two-colour split at `center` (0..=1): `left` below it, `right` from it on.
pub fn split_lr(s: f32, left: [f32; 3], right: [f32; 3], center: f32) -> [f32; 3] {
    if s < center {
        left
    } else {
        right
    }
}

/// Checkerboard of `freq` cells per unit in UV, alternating `a` and `b`.
pub fn checkerboard(uv: [f32; 2], freq: [f32; 2], a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    let i = (uv[0] * freq[0]).floor() as i64;
    let j = (uv[1] * freq[1]).floor() as i64;
    if (i + j).rem_euclid(2) == 0 {
        a
    } else {
        b
    }
}

/// Linear remap of `x` from `[in_lo, in_hi]` to `[out_lo, out_hi]`, unclamped. A zero-width input range
/// returns `out_lo`.
pub fn remap(x: f32, in_lo: f32, in_hi: f32, out_lo: f32, out_hi: f32) -> f32 {
    if (in_hi - in_lo).abs() < 1e-12 {
        return out_lo;
    }
    out_lo + (x - in_lo) / (in_hi - in_lo) * (out_hi - out_lo)
}

/// Contrast about a pivot: `(x - pivot) * amount + pivot`.
pub fn contrast(x: f32, amount: f32, pivot: f32) -> f32 {
    (x - pivot) * amount + pivot
}

/// Hue (0..1), saturation, value to RGB.
pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [f32; 3] {
    let h6 = (h - h.floor()) * 6.0;
    let i = h6.floor();
    let f = h6 - i;
    let p = v * (1.0 - s);
    let q = v * (1.0 - s * f);
    let t = v * (1.0 - s * (1.0 - f));
    match i as i32 {
        0 => [v, t, p],
        1 => [q, v, p],
        2 => [p, v, t],
        3 => [p, q, v],
        4 => [t, p, v],
        _ => [v, p, q],
    }
}

/// RGB to hue (0..1), saturation, value.
pub fn rgb_to_hsv(rgb: [f32; 3]) -> [f32; 3] {
    let mx = rgb[0].max(rgb[1]).max(rgb[2]);
    let mn = rgb[0].min(rgb[1]).min(rgb[2]);
    let d = mx - mn;
    let h = if d < 1e-9 {
        0.0
    } else if mx == rgb[0] {
        ((rgb[1] - rgb[2]) / d).rem_euclid(6.0) / 6.0
    } else if mx == rgb[1] {
        ((rgb[2] - rgb[0]) / d + 2.0) / 6.0
    } else {
        ((rgb[0] - rgb[1]) / d + 4.0) / 6.0
    };
    [h, if mx > 1e-9 { d / mx } else { 0.0 }, mx]
}

/// 2-D placement: scale about the pivot, rotate by `degrees` about it, then offset. UV in, UV out.
pub fn place2d(uv: [f32; 2], pivot: [f32; 2], scale: [f32; 2], degrees: f32, offset: [f32; 2]) -> [f32; 2] {
    let x = (uv[0] - pivot[0]) * scale[0];
    let y = (uv[1] - pivot[1]) * scale[1];
    let (s, c) = degrees.to_radians().sin_cos();
    [x * c - y * s + pivot[0] + offset[0], x * s + y * c + pivot[1] + offset[1]]
}

/// The normal-map node: a texture sample (RGB in 0..=1, tangent space) to a world-space unit normal using
/// the surface normal and tangent; `scale` strengthens the xy bend. `None` for degenerate frames.
pub fn normal_map(sample: [f32; 3], scale: f32, normal: [f32; 3], tangent: [f32; 3]) -> Option<[f32; 3]> {
    fn norm(v: [f32; 3]) -> Option<[f32; 3]> {
        let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        if l.is_finite() && l > 1e-9 {
            Some([v[0] / l, v[1] / l, v[2] / l])
        } else {
            None
        }
    }
    let n = norm(normal)?;
    let t = norm(tangent)?;
    let b = [n[1] * t[2] - n[2] * t[1], n[2] * t[0] - n[0] * t[2], n[0] * t[1] - n[1] * t[0]];
    let (x, y, z) = (sample[0] * 2.0 - 1.0, sample[1] * 2.0 - 1.0, sample[2] * 2.0 - 1.0);
    norm([
        t[0] * x * scale + b[0] * y * scale + n[0] * z,
        t[1] * x * scale + b[1] * y * scale + n[1] * z,
        t[2] * x * scale + b[2] * y * scale + n[2] * z,
    ])
}

/// One node of a [`ShaderGraph`]; operands are indices of earlier nodes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Op {
    Const(f32),
    /// An entry of the input slice (a missing input reads 0).
    Input(usize),
    Add(usize, usize),
    Sub(usize, usize),
    Mul(usize, usize),
    /// Division; a zero divisor gives 0.
    Div(usize, usize),
    Mix { a: usize, b: usize, t: usize },
    Clamp { v: usize, lo: f32, hi: f32 },
    Smoothstep { v: usize, lo: f32, hi: f32 },
    Abs(usize),
    Sin(usize),
    /// [`noise3`] of three operands.
    Noise3(usize, usize, usize),
    /// [`fractal3`] of three operands with an octave count of 1..=8.
    Fractal3 { x: usize, y: usize, z: usize, octaves: u32 },
}

/// A validated straight-line scalar program; the value of the last node is the result.
#[derive(Clone, Debug, PartialEq)]
pub struct ShaderGraph {
    ops: Vec<Op>,
}

impl ShaderGraph {
    /// The graph, or `None` when it is empty, an operand does not refer to an earlier node, a constant is
    /// not finite, a range is reversed or an octave count is outside 1..=8.
    pub fn new(ops: Vec<Op>) -> Option<Self> {
        if ops.is_empty() {
            return None;
        }
        for (i, op) in ops.iter().enumerate() {
            let ok = match *op {
                Op::Const(c) => c.is_finite(),
                Op::Input(_) => true,
                Op::Add(a, b) | Op::Sub(a, b) | Op::Mul(a, b) | Op::Div(a, b) => a < i && b < i,
                Op::Mix { a, b, t } => a < i && b < i && t < i,
                Op::Clamp { v, lo, hi } | Op::Smoothstep { v, lo, hi } => v < i && lo.is_finite() && hi.is_finite() && lo < hi,
                Op::Abs(a) | Op::Sin(a) => a < i,
                Op::Noise3(x, y, z) => x < i && y < i && z < i,
                Op::Fractal3 { x, y, z, octaves } => x < i && y < i && z < i && (1..=8).contains(&octaves),
            };
            if !ok {
                return None;
            }
        }
        Some(ShaderGraph { ops })
    }

    /// Evaluates the graph on the CPU.
    pub fn eval(&self, inputs: &[f32]) -> f32 {
        let mut v: Vec<f32> = Vec::with_capacity(self.ops.len());
        for op in &self.ops {
            let x = match *op {
                Op::Const(c) => c,
                Op::Input(i) => inputs.get(i).copied().unwrap_or(0.0),
                Op::Add(a, b) => v[a] + v[b],
                Op::Sub(a, b) => v[a] - v[b],
                Op::Mul(a, b) => v[a] * v[b],
                Op::Div(a, b) => {
                    if v[b] == 0.0 {
                        0.0
                    } else {
                        v[a] / v[b]
                    }
                }
                Op::Mix { a, b, t } => v[a] + (v[b] - v[a]) * v[t],
                Op::Clamp { v: s, lo, hi } => v[s].clamp(lo, hi),
                Op::Smoothstep { v: s, lo, hi } => {
                    let t = ((v[s] - lo) / (hi - lo)).clamp(0.0, 1.0);
                    t * t * (3.0 - 2.0 * t)
                }
                Op::Abs(a) => v[a].abs(),
                Op::Sin(a) => v[a].sin(),
                Op::Noise3(x, y, z) => noise3(v[x], v[y], v[z]),
                Op::Fractal3 { x, y, z, octaves } => fractal3(v[x], v[y], v[z], octaves),
            };
            v.push(x);
        }
        v[v.len() - 1]
    }

    fn uses_noise(&self) -> bool {
        self.ops.iter().any(|o| matches!(o, Op::Noise3(..) | Op::Fractal3 { .. }))
    }

    /// Compiles the graph to GLSL source: an optional noise prelude and `float <name>(float in0, ...)` with
    /// `input_count` scalar parameters, one statement per node. A missing input reads 0.0.
    pub fn to_glsl(&self, name: &str, input_count: usize) -> String {
        let mut s = String::new();
        if self.uses_noise() {
            s.push_str(GLSL_NOISE_PRELUDE);
        }
        let params: Vec<String> = (0..input_count).map(|i| format!("float in{i}")).collect();
        let _ = writeln!(s, "float {name}({}) {{", params.join(", "));
        for (i, op) in self.ops.iter().enumerate() {
            let expr = match *op {
                Op::Const(c) => glsl_float(c),
                Op::Input(k) => {
                    if k < input_count {
                        format!("in{k}")
                    } else {
                        "0.0".to_string()
                    }
                }
                Op::Add(a, b) => format!("n{a} + n{b}"),
                Op::Sub(a, b) => format!("n{a} - n{b}"),
                Op::Mul(a, b) => format!("n{a} * n{b}"),
                Op::Div(a, b) => format!("(n{b} == 0.0 ? 0.0 : n{a} / n{b})"),
                Op::Mix { a, b, t } => format!("n{a} + (n{b} - n{a}) * n{t}"),
                Op::Clamp { v, lo, hi } => format!("clamp(n{v}, {}, {})", glsl_float(lo), glsl_float(hi)),
                Op::Smoothstep { v, lo, hi } => format!("smoothstep({}, {}, n{v})", glsl_float(lo), glsl_float(hi)),
                Op::Abs(a) => format!("abs(n{a})"),
                Op::Sin(a) => format!("sin(n{a})"),
                Op::Noise3(x, y, z) => format!("mx_noise3(vec3(n{x}, n{y}, n{z}))"),
                Op::Fractal3 { x, y, z, octaves } => format!("mx_fractal3(vec3(n{x}, n{y}, n{z}), {octaves})"),
            };
            let _ = writeln!(s, "    float n{i} = {expr};");
        }
        let _ = writeln!(s, "    return n{};", self.ops.len() - 1);
        s.push_str("}\n");
        s
    }
}

fn glsl_float(c: f32) -> String {
    let t = format!("{c:?}");
    if t.contains('.') || t.contains('e') || t.contains('E') {
        t
    } else {
        format!("{t}.0")
    }
}

/// The GLSL twin of [`hash32`], [`noise3`] and [`fractal3`], emitted ahead of a compiled graph that uses noise.
pub const GLSL_NOISE_PRELUDE: &str = "uint mx_hash(uint x) {
    x ^= x >> 16u;
    x *= 0x7feb352du;
    x ^= x >> 15u;
    x *= 0x846ca68bu;
    x ^= x >> 16u;
    return x;
}
float mx_fade(float t) { return t * t * t * (t * (t * 6.0 - 15.0) + 10.0); }
float mx_grad(uint h, vec3 d) {
    h &= 15u;
    float u = h < 8u ? d.x : d.y;
    float v = h < 4u ? d.y : ((h == 12u || h == 14u) ? d.x : d.z);
    return (((h & 1u) == 0u) ? u : -u) + (((h & 2u) == 0u) ? v : -v);
}
float mx_noise3(vec3 p) {
    vec3 f = floor(p);
    ivec3 i = ivec3(f);
    vec3 d = p - f;
    vec3 w = vec3(mx_fade(d.x), mx_fade(d.y), mx_fade(d.z));
    float c[8];
    for (int n = 0; n < 8; n++) {
        ivec3 o = ivec3(n & 1, (n >> 1) & 1, (n >> 2) & 1);
        ivec3 q = i + o;
        uint h = mx_hash(uint(q.x) + mx_hash(uint(q.y) + mx_hash(uint(q.z))));
        c[n] = mx_grad(h, d - vec3(o));
    }
    float x00 = mix(c[0], c[1], w.x);
    float x10 = mix(c[2], c[3], w.x);
    float x01 = mix(c[4], c[5], w.x);
    float x11 = mix(c[6], c[7], w.x);
    return mix(mix(x00, x10, w.y), mix(x01, x11, w.y), w.z);
}
float mx_fractal3(vec3 p, int octaves) {
    float sum = 0.0;
    float amp = 1.0;
    float freq = 1.0;
    for (int k = 0; k < octaves; k++) {
        sum += amp * mx_noise3(p * freq);
        amp *= 0.5;
        freq *= 2.0;
    }
    return sum;
}
";

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn noise_is_zero_on_the_lattice_bounded_and_deterministic() {
        for (x, y, z) in [(0.0, 0.0, 0.0), (3.0, -2.0, 7.0), (-5.0, 1.0, 4.0)] {
            assert_eq!(noise3(x, y, z), 0.0);
        }
        let mut distinct = std::collections::BTreeSet::new();
        for i in 0..200 {
            let t = i as f32 * 0.37;
            let n = noise3(t, t * 0.61 + 0.3, t * 1.7 - 2.0);
            assert!(n.abs() <= 1.5, "{n}");
            distinct.insert((n * 1000.0) as i32);
            assert_eq!(n, noise3(t, t * 0.61 + 0.3, t * 1.7 - 2.0));
        }
        assert!(distinct.len() > 50, "noise varies");
    }

    #[test]
    fn noise_is_continuous() {
        let a = noise3(1.234, 5.678, 9.1);
        let b = noise3(1.2341, 5.678, 9.1);
        assert!((a - b).abs() < 0.01);
    }

    #[test]
    fn fractal_with_one_octave_is_plain_noise_and_more_octaves_add_detail() {
        assert_eq!(fractal3(0.3, 0.7, 1.9, 1), noise3(0.3, 0.7, 1.9));
        assert_ne!(fractal3(0.3, 0.7, 1.9, 4), noise3(0.3, 0.7, 1.9));
        assert_eq!(fractal3(0.3, 0.7, 1.9, 99), fractal3(0.3, 0.7, 1.9, 8));
    }

    #[test]
    fn cellular_distance_is_small_and_varies() {
        let mut lo = f32::MAX;
        let mut hi = 0.0f32;
        for i in 0..100 {
            let d = cellular3(i as f32 * 0.31, i as f32 * 0.17, 0.5);
            assert!(d >= 0.0 && d < 1.8);
            lo = lo.min(d);
            hi = hi.max(d);
        }
        assert!(hi - lo > 0.2);
    }

    #[test]
    fn ramps_checker_remap_contrast() {
        assert_eq!(ramp_lr(0.5, [0.0; 3], [1.0, 0.5, 0.0]), [0.5, 0.25, 0.0]);
        assert_eq!(ramp_lr(-3.0, [0.2; 3], [1.0; 3]), [0.2; 3]);
        assert_eq!(split_lr(0.4, [1.0; 3], [0.0; 3], 0.5), [1.0; 3]);
        assert_eq!(split_lr(0.5, [1.0; 3], [0.0; 3], 0.5), [0.0; 3]);
        let (a, b) = ([1.0; 3], [0.0; 3]);
        assert_eq!(checkerboard([0.1, 0.1], [2.0, 2.0], a, b), a);
        assert_eq!(checkerboard([0.6, 0.1], [2.0, 2.0], a, b), b);
        assert_eq!(checkerboard([-0.1, 0.1], [2.0, 2.0], a, b), b);
        assert!(close(remap(5.0, 0.0, 10.0, 100.0, 200.0), 150.0));
        assert_eq!(remap(5.0, 1.0, 1.0, 7.0, 9.0), 7.0);
        assert!(close(contrast(0.75, 2.0, 0.5), 1.0));
    }

    #[test]
    fn hsv_round_trips() {
        for rgb in [[1.0, 0.0, 0.0], [0.2, 0.6, 0.4], [0.9, 0.9, 0.1], [0.3, 0.3, 0.3]] {
            let hsv = rgb_to_hsv(rgb);
            let back = hsv_to_rgb(hsv[0], hsv[1], hsv[2]);
            for i in 0..3 {
                assert!(close(back[i], rgb[i]), "{rgb:?} -> {hsv:?} -> {back:?}");
            }
        }
        assert!(close(rgb_to_hsv([0.0, 1.0, 0.0])[0], 1.0 / 3.0));
    }

    #[test]
    fn place2d_scales_rotates_and_offsets() {
        let out = place2d([1.0, 0.0], [0.0, 0.0], [2.0, 2.0], 90.0, [0.5, 0.0]);
        assert!(close(out[0], 0.5) && close(out[1], 2.0));
        assert_eq!(place2d([0.3, 0.4], [0.0; 2], [1.0; 2], 0.0, [0.0; 2]), [0.3, 0.4]);
    }

    #[test]
    fn flat_normal_map_returns_the_surface_normal() {
        let n = normal_map([0.5, 0.5, 1.0], 1.0, [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]).unwrap();
        assert!(close(n[0], 0.0) && close(n[1], 0.0) && close(n[2], 1.0));
        let bent = normal_map([1.0, 0.5, 0.6], 1.0, [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]).unwrap();
        assert!(bent[0] > 0.5);
        assert!(normal_map([0.5; 3], 1.0, [0.0; 3], [1.0, 0.0, 0.0]).is_none());
    }

    #[test]
    fn graph_validation_and_evaluation() {
        assert!(ShaderGraph::new(vec![]).is_none());
        assert!(ShaderGraph::new(vec![Op::Add(0, 0)]).is_none());
        assert!(ShaderGraph::new(vec![Op::Const(1.0), Op::Clamp { v: 0, lo: 1.0, hi: 0.0 }]).is_none());
        assert!(ShaderGraph::new(vec![Op::Const(f32::NAN)]).is_none());
        assert!(ShaderGraph::new(vec![Op::Const(0.0), Op::Fractal3 { x: 0, y: 0, z: 0, octaves: 9 }]).is_none());
        // (in0 + in1) / 2 smoothed.
        let g = ShaderGraph::new(vec![
            Op::Input(0),
            Op::Input(1),
            Op::Add(0, 1),
            Op::Const(2.0),
            Op::Div(2, 3),
            Op::Smoothstep { v: 4, lo: 0.0, hi: 1.0 },
        ])
        .unwrap();
        assert!(close(g.eval(&[0.5, 0.5]), 0.5));
        assert!(close(g.eval(&[2.0, 2.0]), 1.0));
        let z = ShaderGraph::new(vec![Op::Const(1.0), Op::Const(0.0), Op::Div(0, 1)]).unwrap();
        assert_eq!(z.eval(&[]), 0.0);
    }

    #[test]
    fn noise_graph_matches_the_direct_call() {
        let g = ShaderGraph::new(vec![Op::Input(0), Op::Input(1), Op::Const(0.5), Op::Noise3(0, 1, 2)]).unwrap();
        assert_eq!(g.eval(&[1.3, 2.7]), noise3(1.3, 2.7, 0.5));
    }

    #[test]
    fn glsl_output_has_one_statement_per_node_and_the_prelude_only_for_noise() {
        let plain = ShaderGraph::new(vec![Op::Input(0), Op::Const(2.0), Op::Mul(0, 1), Op::Clamp { v: 2, lo: 0.0, hi: 1.0 }]).unwrap();
        let src = plain.to_glsl("mx_graph", 1);
        assert!(src.starts_with("float mx_graph(float in0) {"));
        assert!(src.contains("float n1 = 2.0;"));
        assert!(src.contains("float n2 = n0 * n1;"));
        assert!(src.contains("clamp(n2, 0.0, 1.0)"));
        assert!(src.contains("return n3;"));
        assert!(!src.contains("mx_noise3"));
        let noisy = ShaderGraph::new(vec![Op::Input(0), Op::Input(1), Op::Input(2), Op::Fractal3 { x: 0, y: 1, z: 2, octaves: 3 }]).unwrap();
        let src = noisy.to_glsl("tex", 3);
        assert!(src.contains("float mx_noise3(vec3 p)"));
        assert!(src.contains("mx_fractal3(vec3(n0, n1, n2), 3)"));
        assert!(src.contains("float tex(float in0, float in1, float in2) {"));
        let open = src.matches('{').count();
        let close_count = src.matches('}').count();
        assert_eq!(open, close_count, "balanced braces");
    }

    #[test]
    fn glsl_floats_always_have_a_decimal_point() {
        assert_eq!(glsl_float(3.0), "3.0");
        assert_eq!(glsl_float(0.25), "0.25");
        assert!(glsl_float(1e-7).contains('e'));
    }
}
