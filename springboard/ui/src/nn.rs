//! Output sizes of windowed operators (convolution and pooling) and the scale factor of a Fourier transform.
//! These are the shape rules a graph needs before such an operator is recorded. Nothing is convolved here.

/// How a window is laid over the input. `Explicit` names the pixels added before and after; `Same` and `Valid`
/// derive them from the input size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Padding {
    Explicit { before: usize, after: usize },
    /// The output is `ceil(input / stride)`; the padding that makes it so is split with the odd pixel after.
    Same,
    /// No padding: the window must fit inside the input.
    Valid,
}

/// Why a window cannot be applied to an input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowError {
    ZeroKernel,
    ZeroStride,
    ZeroDilation,
    /// The input is smaller than the dilated kernel with no padding to cover the difference.
    TooSmall,
}

/// One spatial dimension of a windowed operator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Window {
    pub kernel: usize,
    pub stride: usize,
    /// Spacing between kernel taps: 1 is dense, 2 skips every other input pixel.
    pub dilation: usize,
    pub padding: Padding,
}

impl Window {
    /// Extent of the kernel once its taps are spread by the dilation.
    pub fn effective_kernel(&self) -> usize {
        self.dilation * self.kernel.saturating_sub(1) + 1
    }

    fn check(&self) -> Result<(), WindowError> {
        if self.kernel == 0 {
            return Err(WindowError::ZeroKernel);
        }
        if self.stride == 0 {
            return Err(WindowError::ZeroStride);
        }
        if self.dilation == 0 {
            return Err(WindowError::ZeroDilation);
        }
        Ok(())
    }

    /// Pixels added before and after the input along this dimension.
    pub fn pads(&self, input: usize) -> Result<(usize, usize), WindowError> {
        let out = self.output_len(input)?;
        Ok(match self.padding {
            Padding::Explicit { before, after } => (before, after),
            Padding::Valid => (0, 0),
            Padding::Same => {
                let total = (out.saturating_sub(1) * self.stride + self.effective_kernel()).saturating_sub(input);
                let before = total / 2;
                (before, total - before)
            }
        })
    }

    /// Number of window positions along this dimension.
    pub fn output_len(&self, input: usize) -> Result<usize, WindowError> {
        self.check()?;
        match self.padding {
            Padding::Same => Ok(input.div_ceil(self.stride)),
            Padding::Explicit { before, after } => self.positions(input + before + after),
            Padding::Valid => self.positions(input),
        }
    }

    fn positions(&self, padded: usize) -> Result<usize, WindowError> {
        let span = self.effective_kernel();
        if padded < span {
            return Err(WindowError::TooSmall);
        }
        Ok((padded - span) / self.stride + 1)
    }
}

/// Output height and width of a two-dimensional windowed operator, given the input's `[height, width]`.
pub fn output_size(input: [usize; 2], windows: [Window; 2]) -> Result<[usize; 2], WindowError> {
    Ok([windows[0].output_len(input[0])?, windows[1].output_len(input[1])?])
}

/// Scale applied to the outputs of a Fourier transform. `n` is the number of points over all transformed dimensions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FftScaling {
    None,
    /// Divide by `n`.
    Size,
    /// Divide by `sqrt(n)`, which keeps the signal energy the same before and after the transform.
    Unitary,
}

impl FftScaling {
    pub fn factor(self, n: usize) -> f64 {
        match self {
            FftScaling::None => 1.0,
            FftScaling::Size => 1.0 / n as f64,
            FftScaling::Unitary => 1.0 / (n as f64).sqrt(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win(kernel: usize, stride: usize, dilation: usize, padding: Padding) -> Window {
        Window { kernel, stride, dilation, padding }
    }

    #[test]
    fn valid_windows_count_positions_that_fit() {
        let w = win(3, 1, 1, Padding::Valid);
        assert_eq!(w.output_len(5), Ok(3));
        assert_eq!(win(3, 2, 1, Padding::Valid).output_len(5), Ok(2), "the last stride would run past the input");
        assert_eq!(w.pads(5), Ok((0, 0)));
    }

    #[test]
    fn same_padding_gives_ceil_of_input_over_stride_and_splits_the_pad() {
        let w = win(3, 2, 1, Padding::Same);
        assert_eq!(w.output_len(5), Ok(3));
        assert_eq!(w.pads(5), Ok((1, 1)), "total pad is (3-1)*2+3-5 = 2");
        assert_eq!(win(2, 1, 1, Padding::Same).pads(4), Ok((0, 1)), "an odd total puts the extra pixel after");
    }

    #[test]
    fn explicit_padding_adds_to_the_input_before_counting() {
        let w = win(3, 1, 1, Padding::Explicit { before: 1, after: 1 });
        assert_eq!(w.output_len(4), Ok(4), "a 3-tap window over 4+2 pixels has 4 positions");
        assert_eq!(w.pads(4), Ok((1, 1)));
    }

    #[test]
    fn dilation_widens_the_kernel_it_is_checked_against() {
        let w = win(3, 1, 2, Padding::Valid);
        assert_eq!(w.effective_kernel(), 5);
        assert_eq!(w.output_len(5), Ok(1));
        assert_eq!(w.output_len(4), Err(WindowError::TooSmall), "a 5-tap span does not fit in 4 pixels");
    }

    #[test]
    fn zero_kernel_stride_or_dilation_is_refused() {
        assert_eq!(win(0, 1, 1, Padding::Valid).output_len(5), Err(WindowError::ZeroKernel));
        assert_eq!(win(3, 0, 1, Padding::Same).output_len(5), Err(WindowError::ZeroStride));
        assert_eq!(win(3, 1, 0, Padding::Valid).output_len(5), Err(WindowError::ZeroDilation));
    }

    #[test]
    fn two_dimensional_output_applies_each_axis_on_its_own() {
        let rows = win(2, 2, 1, Padding::Valid);
        let cols = win(3, 1, 1, Padding::Same);
        assert_eq!(output_size([8, 5], [rows, cols]), Ok([4, 5]));
    }

    #[test]
    fn fft_scaling_factors_follow_the_total_point_count() {
        assert_eq!(FftScaling::None.factor(16), 1.0);
        assert_eq!(FftScaling::Size.factor(16), 0.0625);
        assert_eq!(FftScaling::Unitary.factor(16), 0.25);
        assert_eq!(FftScaling::Unitary.factor(4), 0.5);
    }
}
