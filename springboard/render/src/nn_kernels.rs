//! Neural-network layer kernels on flat `f32` slices (std only), the CPU references for the GPU compute shaders
//! `conv2d.comp` (softmax is `array_softmax.comp`): activations (ReLU family, ELU, sigmoid, tanh, GELU, SiLU, Mish,
//! hard-swish, softplus), batch normalisation (row softmax and layer norm are in `kernel_array_ops`), direct and im2col+GEMM convolution,
//! depthwise and transposed convolution, nearest and bilinear upsampling, global average pooling, and scaled
//! dot-product attention with an optional causal mask.
//!
//! Images use channel-first layout (`C x H x W`, row-major). Convolution weights are `[c_out][c_in][kh][kw]`,
//! depthwise weights `[c][kh][kw]`, transposed-convolution weights `[c_in][c_out][kh][kw]`. Trained weights are not part
//! of this module: it supplies the arithmetic. The eager layers with a model container (`Network`, LSTM, GRU) are in
//! `bifrost-ui` (`cnn_layers`); array and matrix operations are in `tensor_ops`.

use crate::kernel_array_ops::softmax_rows;
use crate::tensor_ops::gemm;

/// An element-wise activation function.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Activation {
    Relu,
    LeakyRelu(f32),
    Elu(f32),
    Sigmoid,
    Tanh,
    /// The tanh approximation of GELU.
    Gelu,
    /// `x * sigmoid(x)`.
    Silu,
    /// `x * tanh(softplus(x))`.
    Mish,
    /// `x * clamp(x + 3, 0, 6) / 6`.
    HardSwish,
    Softplus,
}

pub fn sigmoid(x: f32) -> f32 {
    if x >= 0.0 {
        1.0 / (1.0 + (-x).exp())
    } else {
        let e = x.exp();
        e / (1.0 + e)
    }
}

pub fn softplus(x: f32) -> f32 {
    if x > 20.0 {
        x
    } else {
        x.exp().ln_1p()
    }
}

/// One activation value.
pub fn activate(a: Activation, x: f32) -> f32 {
    match a {
        Activation::Relu => x.max(0.0),
        Activation::LeakyRelu(s) => {
            if x >= 0.0 {
                x
            } else {
                s * x
            }
        }
        Activation::Elu(s) => {
            if x >= 0.0 {
                x
            } else {
                s * (x.exp() - 1.0)
            }
        }
        Activation::Sigmoid => sigmoid(x),
        Activation::Tanh => x.tanh(),
        Activation::Gelu => 0.5 * x * (1.0 + (0.797_884_6 * (x + 0.044_715 * x * x * x)).tanh()),
        Activation::Silu => x * sigmoid(x),
        Activation::Mish => x * softplus(x).tanh(),
        Activation::HardSwish => x * (x + 3.0).clamp(0.0, 6.0) / 6.0,
        Activation::Softplus => softplus(x),
    }
}

/// Applies an activation to every element in place.
pub fn apply_activation(a: Activation, data: &mut [f32]) {
    for v in data.iter_mut() {
        *v = activate(a, *v);
    }
}

/// Batch normalisation for inference over `n x c x s` data (`s` the spatial size) with stored per-channel statistics:
/// `gamma * (x - mean) / sqrt(var + eps) + beta`. `None` for wrong lengths.
#[allow(clippy::too_many_arguments)]
pub fn batch_norm(x: &[f32], n: usize, c: usize, s: usize, mean: &[f32], var: &[f32], gamma: &[f32], beta: &[f32], eps: f32) -> Option<Vec<f32>> {
    let total = n.checked_mul(c)?.checked_mul(s)?;
    if x.len() != total || [mean.len(), var.len(), gamma.len(), beta.len()].iter().any(|&l| l != c) {
        return None;
    }
    let mut out = vec![0.0f32; total];
    for i in 0..n {
        for ch in 0..c {
            let inv = 1.0 / (var[ch] + eps).sqrt();
            for j in 0..s {
                let p = (i * c + ch) * s + j;
                out[p] = gamma[ch] * (x[p] - mean[ch]) * inv + beta[ch];
            }
        }
    }
    Some(out)
}

/// Output size of a convolution along one axis.
fn conv_out(size: usize, k: usize, stride: usize, pad: usize) -> Option<usize> {
    if stride == 0 || k == 0 {
        return None;
    }
    ((size + 2 * pad).checked_sub(k)?).checked_div(stride).map(|q| q + 1)
}

/// Direct convolution (cross-correlation, as in every deep-learning framework). Returns the output and its height and
/// width. `None` for wrong lengths, a zero stride or kernel, or a kernel larger than the padded input.
#[allow(clippy::too_many_arguments)]
pub fn conv2d(input: &[f32], c_in: usize, h: usize, w: usize, weights: &[f32], c_out: usize, kh: usize, kw: usize, bias: Option<&[f32]>, stride: usize, pad: usize) -> Option<(Vec<f32>, usize, usize)> {
    if input.len() != c_in * h * w || weights.len() != c_out * c_in * kh * kw || bias.is_some_and(|b| b.len() != c_out) {
        return None;
    }
    let (oh, ow) = (conv_out(h, kh, stride, pad)?, conv_out(w, kw, stride, pad)?);
    let mut out = vec![0.0f32; c_out * oh * ow];
    for co in 0..c_out {
        for y in 0..oh {
            for x in 0..ow {
                let mut sum = bias.map_or(0.0, |b| b[co]);
                for ci in 0..c_in {
                    for ky in 0..kh {
                        let iy = (y * stride + ky) as i64 - pad as i64;
                        if iy < 0 || iy >= h as i64 {
                            continue;
                        }
                        for kx in 0..kw {
                            let ix = (x * stride + kx) as i64 - pad as i64;
                            if ix < 0 || ix >= w as i64 {
                                continue;
                            }
                            sum += input[(ci * h + iy as usize) * w + ix as usize] * weights[((co * c_in + ci) * kh + ky) * kw + kx];
                        }
                    }
                }
                out[(co * oh + y) * ow + x] = sum;
            }
        }
    }
    Some((out, oh, ow))
}

#[allow(clippy::too_many_arguments)]
/// Unrolls the input patches into a `(c_in * kh * kw) x (oh * ow)` matrix (zeros for padding), so convolution becomes
/// one matrix product. Returns the matrix and `oh`, `ow`.
pub fn im2col(input: &[f32], c_in: usize, h: usize, w: usize, kh: usize, kw: usize, stride: usize, pad: usize) -> Option<(Vec<f32>, usize, usize)> {
    if input.len() != c_in * h * w {
        return None;
    }
    let (oh, ow) = (conv_out(h, kh, stride, pad)?, conv_out(w, kw, stride, pad)?);
    let cols = oh * ow;
    let mut out = vec![0.0f32; c_in * kh * kw * cols];
    for ci in 0..c_in {
        for ky in 0..kh {
            for kx in 0..kw {
                let row = (ci * kh + ky) * kw + kx;
                for y in 0..oh {
                    for x in 0..ow {
                        let iy = (y * stride + ky) as i64 - pad as i64;
                        let ix = (x * stride + kx) as i64 - pad as i64;
                        if iy >= 0 && iy < h as i64 && ix >= 0 && ix < w as i64 {
                            out[row * cols + y * ow + x] = input[(ci * h + iy as usize) * w + ix as usize];
                        }
                    }
                }
            }
        }
    }
    Some((out, oh, ow))
}

/// The same convolution as `conv2d`, computed as `weights x im2col(input)`: the form a GPU runs with its matrix
/// multiply kernel.
#[allow(clippy::too_many_arguments)]
pub fn conv2d_gemm(input: &[f32], c_in: usize, h: usize, w: usize, weights: &[f32], c_out: usize, kh: usize, kw: usize, bias: Option<&[f32]>, stride: usize, pad: usize) -> Option<(Vec<f32>, usize, usize)> {
    if weights.len() != c_out * c_in * kh * kw || bias.is_some_and(|b| b.len() != c_out) {
        return None;
    }
    let (cols, oh, ow) = im2col(input, c_in, h, w, kh, kw, stride, pad)?;
    let mut out = gemm(weights, &cols, c_out, c_in * kh * kw, oh * ow, false, false, 1.0, 0.0, None)?;
    if let Some(b) = bias {
        for co in 0..c_out {
            for v in out[co * oh * ow..(co + 1) * oh * ow].iter_mut() {
                *v += b[co];
            }
        }
    }
    Some((out, oh, ow))
}

/// Depthwise convolution: every channel is filtered by its own `kh x kw` kernel (weights `[c][kh][kw]`).
#[allow(clippy::too_many_arguments)]
pub fn depthwise_conv2d(input: &[f32], c: usize, h: usize, w: usize, weights: &[f32], kh: usize, kw: usize, stride: usize, pad: usize) -> Option<(Vec<f32>, usize, usize)> {
    if input.len() != c * h * w || weights.len() != c * kh * kw {
        return None;
    }
    let (oh, ow) = (conv_out(h, kh, stride, pad)?, conv_out(w, kw, stride, pad)?);
    let mut out = vec![0.0f32; c * oh * ow];
    for ch in 0..c {
        for y in 0..oh {
            for x in 0..ow {
                let mut sum = 0.0f32;
                for ky in 0..kh {
                    let iy = (y * stride + ky) as i64 - pad as i64;
                    if iy < 0 || iy >= h as i64 {
                        continue;
                    }
                    for kx in 0..kw {
                        let ix = (x * stride + kx) as i64 - pad as i64;
                        if ix < 0 || ix >= w as i64 {
                            continue;
                        }
                        sum += input[(ch * h + iy as usize) * w + ix as usize] * weights[(ch * kh + ky) * kw + kx];
                    }
                }
                out[(ch * oh + y) * ow + x] = sum;
            }
        }
    }
    Some((out, oh, ow))
}

/// Transposed convolution (the gradient of `conv2d`, used for learned upsampling): every input value paints a scaled
/// copy of the kernel into the output at `stride` spacing. Output size is `(h - 1) * stride + kh - 2 * pad`.
#[allow(clippy::too_many_arguments)]
pub fn conv_transpose2d(input: &[f32], c_in: usize, h: usize, w: usize, weights: &[f32], c_out: usize, kh: usize, kw: usize, stride: usize, pad: usize) -> Option<(Vec<f32>, usize, usize)> {
    if input.len() != c_in * h * w || weights.len() != c_in * c_out * kh * kw || stride == 0 || h == 0 || w == 0 {
        return None;
    }
    let oh = ((h - 1) * stride + kh).checked_sub(2 * pad).filter(|v| *v > 0)?;
    let ow = ((w - 1) * stride + kw).checked_sub(2 * pad).filter(|v| *v > 0)?;
    let mut out = vec![0.0f32; c_out * oh * ow];
    for ci in 0..c_in {
        for y in 0..h {
            for x in 0..w {
                let v = input[(ci * h + y) * w + x];
                for co in 0..c_out {
                    for ky in 0..kh {
                        let oy = (y * stride + ky) as i64 - pad as i64;
                        if oy < 0 || oy >= oh as i64 {
                            continue;
                        }
                        for kx in 0..kw {
                            let ox = (x * stride + kx) as i64 - pad as i64;
                            if ox < 0 || ox >= ow as i64 {
                                continue;
                            }
                            out[(co * oh + oy as usize) * ow + ox as usize] += v * weights[((ci * c_out + co) * kh + ky) * kw + kx];
                        }
                    }
                }
            }
        }
    }
    Some((out, oh, ow))
}

/// Nearest-neighbour enlargement by an integer `factor`.
pub fn upsample_nearest(x: &[f32], c: usize, h: usize, w: usize, factor: usize) -> Option<Vec<f32>> {
    if x.len() != c * h * w || factor == 0 {
        return None;
    }
    let (oh, ow) = (h * factor, w * factor);
    let mut out = vec![0.0f32; c * oh * ow];
    for ch in 0..c {
        for y in 0..oh {
            for xx in 0..ow {
                out[(ch * oh + y) * ow + xx] = x[(ch * h + y / factor) * w + xx / factor];
            }
        }
    }
    Some(out)
}

/// Bilinear enlargement by an integer `factor`, sampling at pixel centres (the half-pixel convention) with edges clamped.
pub fn upsample_bilinear(x: &[f32], c: usize, h: usize, w: usize, factor: usize) -> Option<Vec<f32>> {
    if x.len() != c * h * w || factor == 0 || h == 0 || w == 0 {
        return None;
    }
    let (oh, ow) = (h * factor, w * factor);
    let mut out = vec![0.0f32; c * oh * ow];
    let src = |d: usize, size: usize| -> (usize, usize, f32) {
        let s = ((d as f32 + 0.5) / factor as f32 - 0.5).clamp(0.0, (size - 1) as f32);
        let lo = s.floor() as usize;
        (lo, (lo + 1).min(size - 1), s - lo as f32)
    };
    for ch in 0..c {
        for y in 0..oh {
            let (y0, y1, fy) = src(y, h);
            for xx in 0..ow {
                let (x0, x1, fx) = src(xx, w);
                let at = |yy: usize, x2: usize| x[(ch * h + yy) * w + x2];
                let top = at(y0, x0) * (1.0 - fx) + at(y0, x1) * fx;
                let bottom = at(y1, x0) * (1.0 - fx) + at(y1, x1) * fx;
                out[(ch * oh + y) * ow + xx] = top * (1.0 - fy) + bottom * fy;
            }
        }
    }
    Some(out)
}

/// Mean of each channel over its `hw` values.
pub fn global_avg_pool(x: &[f32], c: usize, hw: usize) -> Option<Vec<f32>> {
    if hw == 0 || x.len() != c.checked_mul(hw)? {
        return None;
    }
    Some((0..c).map(|ch| x[ch * hw..(ch + 1) * hw].iter().sum::<f32>() / hw as f32).collect())
}

/// Scaled dot-product attention for one head: `softmax(Q K^T / sqrt(d)) V`. `q` is `n_q x d`, `k` is `n_k x d`, `v` is
/// `n_k x dv`; the result is `n_q x dv`. With `causal`, query `i` ignores keys after position `i`. `None` for wrong
/// lengths or `d == 0`.
#[allow(clippy::too_many_arguments)]
pub fn attention(q: &[f32], k: &[f32], v: &[f32], n_q: usize, n_k: usize, d: usize, dv: usize, causal: bool) -> Option<Vec<f32>> {
    if d == 0 || q.len() != n_q * d || k.len() != n_k * d || v.len() != n_k * dv {
        return None;
    }
    let scale = 1.0 / (d as f32).sqrt();
    let mut scores = gemm(q, k, n_q, d, n_k, false, true, scale, 0.0, None)?;
    if causal {
        for i in 0..n_q {
            for j in (i + 1)..n_k {
                scores[i * n_k + j] = f32::NEG_INFINITY;
            }
        }
    }
    let p = softmax_rows(&scores, n_q, n_k)?;
    gemm(&p, v, n_q, n_k, dv, false, false, 1.0, 0.0, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: &[f32], b: &[f32], tol: f32) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() <= tol)
    }

    #[test]
    fn activation_values() {
        assert_eq!(activate(Activation::Relu, -2.0), 0.0);
        assert!((activate(Activation::LeakyRelu(0.1), -2.0) + 0.2).abs() < 1e-6);
        assert!((activate(Activation::Elu(1.0), -1.0) + 0.632_120_6).abs() < 1e-5);
        assert_eq!(activate(Activation::Sigmoid, 0.0), 0.5);
        assert!(activate(Activation::Sigmoid, -100.0) >= 0.0 && activate(Activation::Sigmoid, 100.0) <= 1.0);
        assert!((activate(Activation::Gelu, 1.0) - 0.841_192).abs() < 1e-3);
        assert_eq!(activate(Activation::Gelu, 0.0), 0.0);
        assert!((activate(Activation::Gelu, 10.0) - 10.0).abs() < 1e-3);
        assert!((activate(Activation::Silu, 1.0) - 0.731_058_6).abs() < 1e-5);
        assert!((activate(Activation::HardSwish, 1.0) - 2.0 / 3.0).abs() < 1e-6);
        assert_eq!(activate(Activation::HardSwish, -3.0), 0.0);
        assert!((activate(Activation::Mish, 1.0) - 0.865_098).abs() < 1e-4);
        assert!((activate(Activation::Softplus, 0.0) - std::f32::consts::LN_2).abs() < 1e-6);
        assert_eq!(activate(Activation::Softplus, 50.0), 50.0);
        let mut v = [-1.0, 2.0];
        apply_activation(Activation::Relu, &mut v);
        assert_eq!(v, [0.0, 2.0]);
    }

    #[test]
    fn batch_norm_uses_stored_statistics() {
        let out = batch_norm(&[1.0, 3.0, 10.0, 20.0], 1, 2, 2, &[2.0, 15.0], &[1.0, 25.0], &[1.0, 2.0], &[0.0, 1.0], 0.0).unwrap();
        assert_eq!(out, vec![-1.0, 1.0, -1.0, 3.0]);
        assert!(batch_norm(&[1.0; 4], 1, 2, 2, &[0.0], &[1.0; 2], &[1.0; 2], &[0.0; 2], 0.0).is_none());
    }

    fn pattern(n: usize) -> Vec<f32> {
        (0..n).map(|i| ((i * 37 % 11) as f32) / 5.0 - 1.0).collect()
    }

    #[test]
    fn convolution_by_gemm_matches_the_direct_form() {
        let (c_in, h, w, c_out, kh, kw) = (2, 5, 6, 3, 3, 3);
        let input = pattern(c_in * h * w);
        let weights = pattern(c_out * c_in * kh * kw);
        let bias = [0.5, -0.25, 1.0];
        for (stride, pad) in [(1, 0), (1, 1), (2, 1), (2, 0)] {
            let (a, oh, ow) = conv2d(&input, c_in, h, w, &weights, c_out, kh, kw, Some(&bias), stride, pad).unwrap();
            let (b, oh2, ow2) = conv2d_gemm(&input, c_in, h, w, &weights, c_out, kh, kw, Some(&bias), stride, pad).unwrap();
            assert_eq!((oh, ow), (oh2, ow2));
            assert_eq!(a.len(), c_out * oh * ow);
            assert!(close(&a, &b, 1e-4), "stride {stride} pad {pad}");
        }
    }

    #[test]
    fn convolution_known_values_and_errors() {
        // A 2x2 all-ones kernel sums 2x2 windows.
        let input = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0];
        let (out, oh, ow) = conv2d(&input, 1, 3, 3, &[1.0; 4], 1, 2, 2, None, 1, 0).unwrap();
        assert_eq!((oh, ow), (2, 2));
        assert_eq!(out, vec![12.0, 16.0, 24.0, 28.0]);
        assert!(conv2d(&input, 1, 3, 3, &[1.0; 4], 1, 2, 2, None, 0, 0).is_none());
        assert!(conv2d(&input, 1, 3, 3, &[1.0; 25], 1, 5, 5, None, 1, 0).is_none());
        assert!(conv2d(&input, 1, 3, 3, &[1.0; 3], 1, 2, 2, None, 1, 0).is_none());
    }

    #[test]
    fn depthwise_filters_each_channel_separately() {
        let mut input = vec![1.0f32; 9];
        input.extend(vec![2.0f32; 9]);
        let (out, oh, ow) = depthwise_conv2d(&input, 2, 3, 3, &[1.0; 8], 2, 2, 1, 0).unwrap();
        assert_eq!((oh, ow), (2, 2));
        assert_eq!(out, vec![4.0, 4.0, 4.0, 4.0, 8.0, 8.0, 8.0, 8.0]);
        // Per-channel kernels differ.
        let (out, _, _) = depthwise_conv2d(&input, 2, 3, 3, &[1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0], 2, 2, 1, 0).unwrap();
        assert_eq!(out, vec![1.0, 1.0, 1.0, 1.0, 2.0, 2.0, 2.0, 2.0]);
    }

    #[test]
    fn transposed_convolution_paints_kernels() {
        let input = [1.0, 2.0, 3.0, 4.0];
        let (out, oh, ow) = conv_transpose2d(&input, 1, 2, 2, &[1.0; 4], 1, 2, 2, 2, 0).unwrap();
        assert_eq!((oh, ow), (4, 4));
        assert_eq!(out, vec![1.0, 1.0, 2.0, 2.0, 1.0, 1.0, 2.0, 2.0, 3.0, 3.0, 4.0, 4.0, 3.0, 3.0, 4.0, 4.0]);
        let (out, oh, ow) = conv_transpose2d(&input, 1, 2, 2, &[1.0; 4], 1, 2, 2, 1, 0).unwrap();
        assert_eq!((oh, ow), (3, 3));
        assert_eq!(out, vec![1.0, 3.0, 2.0, 4.0, 10.0, 6.0, 3.0, 7.0, 4.0]);
        assert!(conv_transpose2d(&input, 1, 2, 2, &[1.0; 4], 1, 2, 2, 0, 0).is_none());
        assert!(conv_transpose2d(&input, 1, 2, 2, &[1.0; 4], 1, 2, 2, 1, 5).is_none());
    }

    #[test]
    fn upsampling() {
        let x = [1.0, 2.0, 3.0, 4.0];
        let n = upsample_nearest(&x, 1, 2, 2, 2).unwrap();
        assert_eq!(n, vec![1.0, 1.0, 2.0, 2.0, 1.0, 1.0, 2.0, 2.0, 3.0, 3.0, 4.0, 4.0, 3.0, 3.0, 4.0, 4.0]);
        assert_eq!(upsample_bilinear(&x, 1, 2, 2, 1).unwrap(), x.to_vec());
        let b = upsample_bilinear(&x, 1, 2, 2, 2).unwrap();
        assert_eq!(b.len(), 16);
        assert!(b.iter().all(|v| (1.0 - 1e-6..=4.0 + 1e-6).contains(v)));
        assert_eq!(b[0], 1.0);
        assert_eq!(b[15], 4.0);
        // Smooth growth along the first row.
        assert!(b[0] < b[1] && b[1] < b[2] && b[2] < b[3]);
        let flat = upsample_bilinear(&[5.0; 9], 1, 3, 3, 3).unwrap();
        assert!(flat.iter().all(|v| (v - 5.0).abs() < 1e-5));
        assert!(upsample_nearest(&x, 1, 2, 2, 0).is_none());
    }

    #[test]
    fn global_pool_means_each_channel() {
        assert_eq!(global_avg_pool(&[1.0, 3.0, 5.0, 7.0], 2, 2).unwrap(), vec![2.0, 6.0]);
        assert!(global_avg_pool(&[1.0], 2, 2).is_none());
    }

    #[test]
    fn attention_mixes_values_by_similarity() {
        let q = [1.0, 0.0, 0.0, 1.0];
        let v = [10.0, 0.0, 0.0, 20.0];
        let out = attention(&q, &q, &v, 2, 2, 2, 2, false).unwrap();
        assert!(close(&out[0..2], &[6.6976, 6.6049], 5e-3), "{out:?}");
        // Causal: the first query only sees the first key.
        let c = attention(&q, &q, &v, 2, 2, 2, 2, true).unwrap();
        assert!(close(&c[0..2], &[10.0, 0.0], 1e-4));
        assert!(close(&c[2..4], &[3.302, 13.395], 5e-3), "{c:?}");
        assert!(attention(&q, &q, &v, 2, 2, 0, 2, false).is_none());
        assert!(attention(&q, &q, &v[..3], 2, 2, 2, 2, false).is_none());
    }
}
