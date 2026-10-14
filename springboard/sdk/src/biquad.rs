//! Biquad filters: second-order IIR sections for low-pass, high-pass and peaking equalisation.
//!
//! The coefficients follow the standard audio cookbook (Robert Bristow-Johnson's formulas), the same biquadratic
//! sections that the reference audio DSP library exposes. A section is five normalised coefficients and two delay
//! values of history. Frequencies are in hertz, gains in decibels and the quality factor Q is dimensionless.
//!
//! Samples are `f64` so that long filter chains keep their precision. Filters process one channel each: a stereo
//! stream uses two `Biquad` values with the same coefficients.

use std::f64::consts::PI;

/// The response a section is designed for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Design {
    /// Passes frequencies below the cutoff and attenuates those above it.
    LowPass,
    /// Passes frequencies above the cutoff and attenuates those below it.
    HighPass,
    /// Boosts or cuts a band around the centre frequency by `gain_db`, leaving the rest of the spectrum alone.
    Peak { gain_db: f64 },
}

/// Why a filter could not be designed.
#[derive(Clone, Debug, PartialEq)]
pub enum FilterError {
    /// The sample rate is not a positive, finite number.
    BadSampleRate,
    /// The frequency is not above zero and below half the sample rate.
    FrequencyOutOfRange,
    /// The quality factor is not a positive, finite number.
    BadQ,
}

impl std::fmt::Display for FilterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FilterError::BadSampleRate => write!(f, "a filter needs a sample rate above zero"),
            FilterError::FrequencyOutOfRange => write!(f, "a filter frequency must be above zero and below half the sample rate"),
            FilterError::BadQ => write!(f, "a filter quality factor must be above zero"),
        }
    }
}

impl std::error::Error for FilterError {}

/// The normalised coefficients of one section: the transfer function is
/// `(b0 + b1 z^-1 + b2 z^-2) / (1 + a1 z^-1 + a2 z^-2)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Coefficients {
    pub b0: f64,
    pub b1: f64,
    pub b2: f64,
    pub a1: f64,
    pub a2: f64,
}

impl Coefficients {
    /// Designs a section for `design` at `frequency` Hz, with quality factor `q`, for audio at `sample_rate` Hz.
    pub fn design(design: Design, sample_rate: f64, frequency: f64, q: f64) -> Result<Coefficients, FilterError> {
        if !(sample_rate.is_finite() && sample_rate > 0.0) {
            return Err(FilterError::BadSampleRate);
        }
        if !(frequency > 0.0 && frequency < sample_rate / 2.0) {
            return Err(FilterError::FrequencyOutOfRange);
        }
        if !(q.is_finite() && q > 0.0) {
            return Err(FilterError::BadQ);
        }
        let w0 = 2.0 * PI * frequency / sample_rate;
        let (sin, cos) = w0.sin_cos();
        let alpha = sin / (2.0 * q);
        let (b0, b1, b2, a0, a1, a2) = match design {
            Design::LowPass => {
                let b1 = 1.0 - cos;
                (b1 / 2.0, b1, b1 / 2.0, 1.0 + alpha, -2.0 * cos, 1.0 - alpha)
            }
            Design::HighPass => {
                let b1 = -(1.0 + cos);
                (-b1 / 2.0, b1, -b1 / 2.0, 1.0 + alpha, -2.0 * cos, 1.0 - alpha)
            }
            Design::Peak { gain_db } => {
                let a = 10f64.powf(gain_db / 40.0);
                (1.0 + alpha * a, -2.0 * cos, 1.0 - alpha * a, 1.0 + alpha / a, -2.0 * cos, 1.0 - alpha / a)
            }
        };
        Ok(Coefficients { b0: b0 / a0, b1: b1 / a0, b2: b2 / a0, a1: a1 / a0, a2: a2 / a0 })
    }

    /// The gain of the section at `frequency` Hz, in decibels.
    pub fn magnitude_db(&self, frequency: f64, sample_rate: f64) -> f64 {
        let w = 2.0 * PI * frequency / sample_rate;
        let (re1, im1) = (w.cos(), -w.sin());
        let (re2, im2) = ((2.0 * w).cos(), -(2.0 * w).sin());
        let num_re = self.b0 + self.b1 * re1 + self.b2 * re2;
        let num_im = self.b1 * im1 + self.b2 * im2;
        let den_re = 1.0 + self.a1 * re1 + self.a2 * re2;
        let den_im = self.a1 * im1 + self.a2 * im2;
        let num = num_re.hypot(num_im);
        let den = den_re.hypot(den_im);
        20.0 * (num / den).log10()
    }
}

/// A running section for one channel: its coefficients and its two delay values.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Biquad {
    coefficients: Coefficients,
    x1: f64,
    x2: f64,
    y1: f64,
    y2: f64,
}

impl Biquad {
    /// A section with the given coefficients and no history.
    pub fn new(coefficients: Coefficients) -> Biquad {
        Biquad { coefficients, x1: 0.0, x2: 0.0, y1: 0.0, y2: 0.0 }
    }

    /// Filters one sample.
    pub fn process(&mut self, x: f64) -> f64 {
        let c = self.coefficients;
        let y = c.b0 * x + c.b1 * self.x1 + c.b2 * self.x2 - c.a1 * self.y1 - c.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }

    /// Filters a run of samples in place, in order.
    pub fn process_block(&mut self, samples: &mut [f64]) {
        for sample in samples.iter_mut() {
            *sample = self.process(*sample);
        }
    }

    /// Forgets the history, as at the start of a new stream.
    pub fn reset(&mut self) {
        self.x1 = 0.0;
        self.x2 = 0.0;
        self.y1 = 0.0;
        self.y2 = 0.0;
    }
}
