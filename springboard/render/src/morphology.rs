//! Kernel convolution and grey-level morphology on single-channel 8-bit planes.
//!
//! A plane is `w * h` samples in row-major order. Convolution repeats the nearest edge sample outwards, so a flat
//! plane stays flat. Dilation takes the largest sample under a rectangular structuring element and erosion the
//! smallest; both look only at samples inside the plane. Opening (erosion then dilation) removes bright specks
//! smaller than the element, and closing (dilation then erosion) fills dark holes. The work is a direct sum or a
//! direct scan, with no separable shortcut and no GPU pass.

/// Why a request was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MorphologyError {
    /// The plane has a zero dimension.
    ZeroSize,
    /// The sample count does not match the plane size.
    LengthMismatch,
    /// The kernel has a zero or even side, so it has no centre sample, or its weight count is wrong.
    BadKernel,
}

impl core::fmt::Display for MorphologyError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            MorphologyError::ZeroSize => write!(f, "morphology: a plane dimension is zero"),
            MorphologyError::LengthMismatch => write!(f, "morphology: sample count does not match the plane size"),
            MorphologyError::BadKernel => write!(f, "morphology: kernel sides must be odd and match the weight count"),
        }
    }
}

/// A rectangular kernel of weights in row-major order, with an odd width and an odd height so it has a centre.
#[derive(Clone, Debug, PartialEq)]
pub struct Kernel {
    width: usize,
    height: usize,
    weights: Vec<f32>,
}

impl Kernel {
    /// Builds a kernel from `width * height` weights. Both sides must be odd and nonzero.
    pub fn new(width: usize, height: usize, weights: Vec<f32>) -> Result<Self, MorphologyError> {
        let sides_ok = width % 2 == 1 && height % 2 == 1;
        if !sides_ok || width.checked_mul(height) != Some(weights.len()) {
            return Err(MorphologyError::BadKernel);
        }
        Ok(Self { width, height, weights })
    }

    /// Width in samples.
    pub fn width(&self) -> usize {
        self.width
    }

    /// Height in samples.
    pub fn height(&self) -> usize {
        self.height
    }
}

/// Checks the plane size against the sample count.
fn check(plane: &[u8], w: usize, h: usize) -> Result<(), MorphologyError> {
    if w == 0 || h == 0 {
        return Err(MorphologyError::ZeroSize);
    }
    match w.checked_mul(h) {
        Some(n) if n == plane.len() => Ok(()),
        _ => Err(MorphologyError::LengthMismatch),
    }
}

/// The sample at `(x, y)`, with coordinates outside the plane clamped to the nearest edge.
fn edge_sample(plane: &[u8], w: usize, h: usize, x: isize, y: isize) -> u8 {
    let cx = x.clamp(0, w as isize - 1) as usize;
    let cy = y.clamp(0, h as isize - 1) as usize;
    plane[cy * w + cx]
}

/// Convolves a plane with `kernel`. Each output sample is the weighted sum of the samples under the kernel, rounded
/// and clamped to 0..=255. The kernel is applied without flipping (a correlation); for the symmetric kernels used
/// for blur and sharpen the two definitions agree.
pub fn convolve(plane: &[u8], w: usize, h: usize, kernel: &Kernel) -> Result<Vec<u8>, MorphologyError> {
    check(plane, w, h)?;
    let rx = (kernel.width / 2) as isize;
    let ry = (kernel.height / 2) as isize;
    let mut out = Vec::with_capacity(plane.len());
    for y in 0..h as isize {
        for x in 0..w as isize {
            let mut acc = 0.0f32;
            for ky in 0..kernel.height {
                for kx in 0..kernel.width {
                    let s = edge_sample(plane, w, h, x + kx as isize - rx, y + ky as isize - ry);
                    acc += kernel.weights[ky * kernel.width + kx] * s as f32;
                }
            }
            out.push(acc.round().clamp(0.0, 255.0) as u8);
        }
    }
    Ok(out)
}

/// Scans the rectangle of `rx` and `ry` samples around each position and keeps the sample `pick` selects.
fn scan(plane: &[u8], w: usize, h: usize, rx: usize, ry: usize, pick: fn(u8, u8) -> u8) -> Result<Vec<u8>, MorphologyError> {
    check(plane, w, h)?;
    let mut out = Vec::with_capacity(plane.len());
    for y in 0..h {
        for x in 0..w {
            let (y0, y1) = (y.saturating_sub(ry), (y + ry).min(h - 1));
            let (x0, x1) = (x.saturating_sub(rx), (x + rx).min(w - 1));
            let mut v = plane[y * w + x];
            for yy in y0..=y1 {
                for xx in x0..=x1 {
                    v = pick(v, plane[yy * w + xx]);
                }
            }
            out.push(v);
        }
    }
    Ok(out)
}

/// Grey dilation: each sample becomes the largest sample within `rx` columns and `ry` rows of it.
pub fn dilate(plane: &[u8], w: usize, h: usize, rx: usize, ry: usize) -> Result<Vec<u8>, MorphologyError> {
    scan(plane, w, h, rx, ry, u8::max)
}

/// Grey erosion: each sample becomes the smallest sample within `rx` columns and `ry` rows of it.
pub fn erode(plane: &[u8], w: usize, h: usize, rx: usize, ry: usize) -> Result<Vec<u8>, MorphologyError> {
    scan(plane, w, h, rx, ry, u8::min)
}

/// Opening: erosion followed by dilation with the same element. Bright features smaller than the element vanish.
pub fn open(plane: &[u8], w: usize, h: usize, rx: usize, ry: usize) -> Result<Vec<u8>, MorphologyError> {
    let eroded = erode(plane, w, h, rx, ry)?;
    dilate(&eroded, w, h, rx, ry)
}

/// Closing: dilation followed by erosion with the same element. Dark features smaller than the element are filled.
pub fn close(plane: &[u8], w: usize, h: usize, rx: usize, ry: usize) -> Result<Vec<u8>, MorphologyError> {
    let dilated = dilate(plane, w, h, rx, ry)?;
    erode(&dilated, w, h, rx, ry)
}
