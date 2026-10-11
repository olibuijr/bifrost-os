//! Uniform partitioned convolution (overlap-save with radix-2 FFTs). Used by the Reverb and Space devices.
//! Adds one block (512 frames) of latency.

pub const BLOCK: usize = 512;
const N: usize = BLOCK * 2;

/// Radix-2 complex FFT of size `N`, precomputed twiddles.
struct Fft {
    cos: Vec<f64>,
    sin: Vec<f64>,
    rev: Vec<usize>,
}

impl Fft {
    fn new() -> Self {
        let bits = N.trailing_zeros();
        let rev = (0..N).map(|i| i.reverse_bits() >> (usize::BITS - bits)).collect();
        let (cos, sin) = (0..N / 2)
            .map(|k| {
                let a = -2.0 * std::f64::consts::PI * k as f64 / N as f64;
                (a.cos(), a.sin())
            })
            .unzip();
        Fft { cos, sin, rev }
    }

    /// In-place transform. `inverse` flips the twiddle sign; the caller scales by 1/N.
    fn run(&self, re: &mut [f64], im: &mut [f64], inverse: bool) {
        for i in 0..N {
            let j = self.rev[i];
            if j > i {
                re.swap(i, j);
                im.swap(i, j);
            }
        }
        let sgn = if inverse { -1.0 } else { 1.0 };
        let mut len = 2;
        while len <= N {
            let half = len / 2;
            let step = N / len;
            for start in (0..N).step_by(len) {
                for k in 0..half {
                    let (wr, wi) = (self.cos[k * step], sgn * self.sin[k * step]);
                    let (a, b) = (start + k, start + k + half);
                    let tr = re[b] * wr - im[b] * wi;
                    let ti = re[b] * wi + im[b] * wr;
                    re[b] = re[a] - tr;
                    im[b] = im[a] - ti;
                    re[a] += tr;
                    im[a] += ti;
                }
            }
            len *= 2;
        }
    }
}

type Spectrum = (Vec<f64>, Vec<f64>);

/// One convolver for one channel.
pub struct Convolver {
    fft: Fft,
    parts: Vec<Spectrum>,
    hist: Vec<Spectrum>,
    hpos: usize,
    prev: Vec<f32>,
    inbuf: Vec<f32>,
    outbuf: Vec<f32>,
    pos: usize,
}

impl Convolver {
    /// `ir` is the impulse response (already scaled as wanted).
    pub fn new(ir: &[f32]) -> Self {
        let fft = Fft::new();
        let count = ir.len().div_ceil(BLOCK).max(1);
        let mut parts = Vec::with_capacity(count);
        for k in 0..count {
            let mut re = vec![0.0f64; N];
            let mut im = vec![0.0f64; N];
            for i in 0..BLOCK {
                re[i] = ir.get(k * BLOCK + i).copied().unwrap_or(0.0) as f64;
            }
            fft.run(&mut re, &mut im, false);
            parts.push((re, im));
        }
        let hist = (0..count).map(|_| (vec![0.0; N], vec![0.0; N])).collect();
        Convolver {
            fft,
            parts,
            hist,
            hpos: 0,
            prev: vec![0.0; BLOCK],
            inbuf: vec![0.0; BLOCK],
            outbuf: vec![0.0; BLOCK],
            pos: 0,
        }
    }

    /// Convolves one sample.
    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        let y = self.outbuf[self.pos];
        self.inbuf[self.pos] = x;
        self.pos += 1;
        if self.pos == BLOCK {
            self.block();
            self.pos = 0;
        }
        y
    }

    fn block(&mut self) {
        let count = self.parts.len();
        // X_t: the previous block followed by the current one
        let mut re = vec![0.0f64; N];
        let mut im = vec![0.0f64; N];
        for i in 0..BLOCK {
            re[i] = self.prev[i] as f64;
            re[BLOCK + i] = self.inbuf[i] as f64;
        }
        self.fft.run(&mut re, &mut im, false);
        self.hist[self.hpos] = (re, im);
        // Y = sum_k H_k * X_{t-k}
        let mut yr = vec![0.0f64; N];
        let mut yi = vec![0.0f64; N];
        for k in 0..count {
            let idx = (self.hpos + self.hist.len() - k) % self.hist.len();
            let (xr, xi) = &self.hist[idx];
            let (hr, hi) = &self.parts[k];
            for i in 0..N {
                yr[i] += hr[i] * xr[i] - hi[i] * xi[i];
                yi[i] += hr[i] * xi[i] + hi[i] * xr[i];
            }
        }
        self.fft.run(&mut yr, &mut yi, true);
        let scale = 1.0 / N as f64;
        for i in 0..BLOCK {
            self.outbuf[i] = (yr[BLOCK + i] * scale) as f32;
        }
        self.hpos = (self.hpos + 1) % self.hist.len();
        self.prev.copy_from_slice(&self.inbuf);
    }
}

/// Scales an impulse response so its energy is 1 across all channels (the web ConvolverNode normalises the
/// same way, approximately). Returns the gain that was applied.
pub fn normalize(irs: &mut [Vec<f32>]) -> f32 {
    let energy: f64 = irs.iter().flat_map(|c| c.iter()).map(|&x| (x as f64) * (x as f64)).sum();
    if energy <= 0.0 {
        return 1.0;
    }
    let g = (1.0 / energy.sqrt()) as f32;
    for c in irs.iter_mut() {
        for x in c.iter_mut() {
            *x *= g;
        }
    }
    g
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_impulse_is_identity_after_latency() {
        let mut c = Convolver::new(&[1.0]);
        let mut out = vec![];
        let input: Vec<f32> = (0..3000).map(|i| ((i * 37) % 101) as f32 / 101.0 - 0.5).collect();
        for &x in &input {
            out.push(c.process(x));
        }
        for i in BLOCK..input.len() {
            assert!((out[i] - input[i - BLOCK]).abs() < 1e-4, "sample {i}");
        }
    }
}
