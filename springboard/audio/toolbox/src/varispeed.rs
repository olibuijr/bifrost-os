//! The adaptive resampler for drift compensation (task 11): a four-point cubic (Catmull-Rom) interpolator whose
//! ratio is slewed linearly across each call, so a changing DLL rate never clicks. It allocates only in `new`.
//!
//! Model: the virtual input is `[h0, h1, h2, in0, in1, ...]` where `h` is the three-frame history kept between
//! calls. The read position `pos` (in virtual frames, starting at 1.0) advances by `1 / ratio` per output frame; the
//! output at `pos = i + t` interpolates `v[i-1..=i+2]`. At ratio 1.0 the output is the input delayed by 2 frames.

const HIST: usize = 3;

/// Resamples one channel group at a slowly varying ratio.
pub struct Varispeed {
    pub channels: u32,
    hist: Vec<[f32; HIST]>,
    pos: f64,
    ratio_cur: f64,
    ratio_target: f64,
}

/// The ratio at output frame `j` of `n` when slewing from `r0` to `r1`.
#[inline]
fn slew(r0: f64, r1: f64, j: u32, n: u32) -> f64 {
    r0 + (r1 - r0) * f64::from(j + 1) / f64::from(n)
}

impl Varispeed {
    pub fn new(channels: u32) -> Varispeed {
        Varispeed {
            channels,
            hist: vec![[0.0; HIST]; channels as usize],
            pos: 1.0,
            ratio_cur: 1.0,
            ratio_target: 1.0,
        }
    }

    /// Sets the ratio (output rate / input rate), e.g. 1.0002 for a sub-device 200 ppm slow against the clock
    /// source. Applied smoothly over the next call. Clamped to 0.25..=4.0.
    pub fn set_ratio(&mut self, ratio: f64) {
        self.ratio_target = if ratio.is_finite() { ratio.clamp(0.25, 4.0) } else { 1.0 };
    }

    /// The target ratio last set.
    pub fn ratio(&self) -> f64 {
        self.ratio_target
    }

    /// Clears the history and phase (after a gap or a restart).
    pub fn reset(&mut self) {
        for h in &mut self.hist {
            *h = [0.0; HIST];
        }
        self.pos = 1.0;
        self.ratio_cur = self.ratio_target;
    }

    /// The read position after `n` output frames, and the integer part of the position of the last output.
    fn plan(&self, n: u32) -> (f64, usize) {
        let mut pos = self.pos;
        let mut last = pos.floor() as usize;
        for j in 0..n {
            last = pos.floor() as usize;
            pos += 1.0 / slew(self.ratio_cur, self.ratio_target, j, n);
        }
        (pos, last)
    }

    /// Input frames needed to produce `out_frames` frames at the current ratio and phase.
    pub fn input_needed(&self, out_frames: u32) -> u32 {
        if out_frames == 0 {
            return 0;
        }
        let (pos, last) = self.plan(out_frames);
        let used = (pos.floor() as usize).saturating_sub(1);
        last.max(used) as u32
    }

    /// Consumes input (per-channel slices) and fills `out_frames` frames of output; returns input frames used.
    /// If the input is short, the missing output is silence and fewer frames are used.
    pub fn process(&mut self, input: &[&[f32]], output: &mut [&mut [f32]], out_frames: u32) -> u32 {
        let ch = (self.channels as usize).min(input.len()).min(output.len());
        let avail = input.iter().take(ch).map(|s| s.len()).min().unwrap_or(0);
        let n = out_frames;
        let mut pos = self.pos;
        let mut done = 0u32;
        for j in 0..n {
            let i = pos.floor() as usize;
            if i > avail {
                break;
            }
            let t = (pos - i as f64) as f32;
            for c in 0..ch {
                let h = &self.hist[c];
                let inp = input[c];
                let v = |k: usize| if k < HIST { h[k] } else { inp[k - HIST] };
                let (p0, p1, p2, p3) = (v(i - 1), v(i), v(i + 1), v(i + 2));
                output[c][j as usize] =
                    p1 + 0.5 * t * (p2 - p0 + t * (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3 + t * (3.0 * (p1 - p2) + p3 - p0)));
            }
            pos += 1.0 / slew(self.ratio_cur, self.ratio_target, j, n);
            done += 1;
        }
        for c in 0..ch {
            for s in &mut output[c][done as usize..n as usize] {
                *s = 0.0;
            }
        }
        let used = (pos.floor() as usize).saturating_sub(1).min(avail);
        for c in 0..ch {
            let h = self.hist[c];
            let inp = input[c];
            for k in 0..HIST {
                let idx = used + k;
                self.hist[c][k] = if idx < HIST { h[idx] } else { inp[idx - HIST] };
            }
        }
        self.pos = pos - used as f64;
        self.ratio_cur = self.ratio_target;
        used as u32
    }

    /// The processing delay in output frames (for latency reporting): the interpolator reads 2 input frames behind.
    pub fn latency(&self) -> f64 {
        2.0 / self.ratio_target
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(v: &mut Varispeed, input: &[f32], n: u32) -> (Vec<f32>, u32) {
        let mut out = vec![0.0f32; n as usize];
        let used = v.process(&[input], &mut [&mut out[..]], n);
        (out, used)
    }

    #[test]
    fn unity_ratio_is_a_two_frame_delay() {
        let mut v = Varispeed::new(1);
        let input: Vec<f32> = (1..=8).map(|x| x as f32).collect();
        assert_eq!(v.input_needed(8), 8);
        let (out, used) = run(&mut v, &input, 8);
        assert_eq!(used, 8);
        // v = [0,0,0,1,2,...]; output j = v[1 + j]: 0,0,1,2,3,4,5,6
        assert_eq!(out, vec![0.0, 0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
    }

    #[test]
    fn unity_across_calls_is_continuous() {
        let mut v = Varispeed::new(1);
        let a: Vec<f32> = (1..=4).map(|x| x as f32).collect();
        let b: Vec<f32> = (5..=8).map(|x| x as f32).collect();
        let (o1, u1) = run(&mut v, &a, 4);
        let (o2, u2) = run(&mut v, &b, 4);
        assert_eq!((u1, u2), (4, 4));
        assert_eq!(o1, vec![0.0, 0.0, 1.0, 2.0]);
        assert_eq!(o2, vec![3.0, 4.0, 5.0, 6.0]);
    }

    #[test]
    fn a_ramp_stays_a_ramp_at_ratio_two() {
        // Catmull-Rom reproduces a linear ramp exactly. Ratio 2 (upsampling): positions advance by 0.5.
        let mut v = Varispeed::new(1);
        v.set_ratio(2.0);
        v.ratio_cur = 2.0;
        let input: Vec<f32> = (1..=16).map(|x| x as f32).collect();
        let need = v.input_needed(8);
        // pos after 8 outputs = 1 + 4 = 5 -> used 4; last position 4.5 -> floor 4: need 4.
        assert_eq!(need, 4);
        let (out, used) = run(&mut v, &input[..need as usize], 8);
        assert_eq!(used, 4);
        // v = [0,0,0,1,2,3,4]; positions 1,1.5,2,2.5,3,3.5,4,4.5: from pos 3 (v[3]=1) it is the ramp.
        assert_eq!(out[4], 1.0);
        assert!((out[5] - 1.5).abs() < 1e-6);
        assert!((out[6] - 2.0).abs() < 1e-6);
        assert!((out[7] - 2.5).abs() < 1e-6);
    }

    #[test]
    fn a_slow_ratio_consumes_proportionally_fewer_frames() {
        // Ratio 1.0002 over 100000 output frames needs about 100000 / 1.0002 = 99980.004 input frames.
        let mut v = Varispeed::new(1);
        v.set_ratio(1.0002);
        v.ratio_cur = 1.0002;
        let need = v.input_needed(100_000);
        assert!((99_979..=99_982).contains(&need), "{need}");
    }

    #[test]
    fn a_sine_survives_drift_resampling() {
        // 1 kHz sine at 48 kHz, ratio 1.0002: the output is the same sine, time-scaled; compare to the analytic value.
        let mut v = Varispeed::new(1);
        let ratio = 1.0002f64;
        v.set_ratio(ratio);
        v.ratio_cur = ratio;
        let w = 2.0 * std::f64::consts::PI * 1000.0 / 48000.0;
        let mut in_idx = 0u64; // input frames consumed so far
        let mut out_idx = 0u64;
        let mut worst = 0.0f64;
        for _ in 0..200 {
            let n = 480u32;
            let need = v.input_needed(n);
            // The first input sample is x(1): v[3] = x(1), so v[p] = x(p - 2) and output m (at p = 1 + m / ratio)
            // is x(m / ratio - 1).
            let inp: Vec<f32> = (0..need).map(|k| (w * (in_idx + 1 + u64::from(k)) as f64).sin() as f32).collect();
            let (out, used) = run(&mut v, &inp, n);
            assert_eq!(used, need);
            for (j, o) in out.iter().enumerate() {
                if out_idx > 50 {
                    let exp = (w * ((out_idx + j as u64) as f64 / ratio - 1.0)).sin();
                    worst = worst.max((f64::from(*o) - exp).abs());
                }
            }
            in_idx += u64::from(used);
            out_idx += u64::from(n);
        }
        assert!(worst < 2e-3, "worst error {worst}");
    }

    #[test]
    fn ratio_change_is_smooth() {
        // A constant input stays constant through a ratio jump (no click).
        let mut v = Varispeed::new(1);
        let input = vec![0.5f32; 4096];
        let _ = run(&mut v, &input[..64], 64); // pass the zero history
        v.set_ratio(1.01);
        let need = v.input_needed(64) as usize;
        let (out, _) = run(&mut v, &input[..need], 64);
        assert!(out.iter().all(|s| (s - 0.5).abs() < 1e-6));
    }

    #[test]
    fn short_input_gives_silence_not_a_panic() {
        let mut v = Varispeed::new(2);
        let a = [1.0f32, 2.0];
        let mut o0 = [9.0f32; 8];
        let mut o1 = [9.0f32; 8];
        let used = v.process(&[&a, &a], &mut [&mut o0, &mut o1], 8);
        assert!(used <= 2);
        assert_eq!(&o0[5..], &[0.0, 0.0, 0.0]);
    }

    #[test]
    fn latency_is_two_frames_at_unity() {
        let mut v = Varispeed::new(1);
        assert_eq!(v.latency(), 2.0);
        v.set_ratio(2.0);
        assert_eq!(v.latency(), 1.0);
    }
}
