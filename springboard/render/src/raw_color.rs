//! Colour matrix and tone curve for RAW processing, as a pure model (std only).
//!
//! After demosaic, a RAW frame is still in the camera's own primaries and linear light.
//! `apply_color_pipeline` takes interleaved 16-bit RGB, treats it as linear light, applies
//! a 3x3 colour matrix (camera RGB to linear sRGB), clamps the result to the display
//! range, and encodes it with the sRGB transfer curve (the tone curve) back to 16 bits.
//! The matrix comes from the camera profile; this module does not look one up.

/// A 3x3 colour matrix in row-major order. Row `i` gives output channel `i` as a weighted
/// sum of the input red, green and blue.
pub type Matrix3 = [[f64; 3]; 3];

/// Why a colour pipeline run was refused.
#[derive(Clone, Debug, PartialEq)]
pub enum ColorError {
    /// The sample count is not a multiple of three (interleaved RGB).
    NotRgb { actual: usize },
    /// A matrix entry is NaN or infinite. The field is the row index.
    NotFinite(usize),
}

/// The sRGB transfer function (IEC 61966-2-1), linear light to encoded value.
/// Inputs are clamped to 0..=1 first, so the result is always in 0..=1.
pub fn srgb_encode(linear: f64) -> f64 {
    let x = linear.clamp(0.0, 1.0);
    if x <= 0.003_130_8 {
        x * 12.92
    } else {
        1.055 * x.powf(1.0 / 2.4) - 0.055
    }
}

/// Applies a 3x3 matrix to one linear RGB triple.
pub fn apply_matrix(matrix: &Matrix3, rgb: [f64; 3]) -> [f64; 3] {
    let mut out = [0.0; 3];
    for (i, row) in matrix.iter().enumerate() {
        out[i] = row[0] * rgb[0] + row[1] * rgb[1] + row[2] * rgb[2];
    }
    out
}

/// Runs the colour pipeline on interleaved 16-bit RGB. Each output sample is the matrix
/// result, clamped to 0..=1, sRGB encoded, and rounded to 16 bits.
pub fn apply_color_pipeline(rgb: &[u16], matrix: &Matrix3) -> Result<Vec<u16>, ColorError> {
    if rgb.len() % 3 != 0 {
        return Err(ColorError::NotRgb { actual: rgb.len() });
    }
    for (row, values) in matrix.iter().enumerate() {
        if values.iter().any(|v| !v.is_finite()) {
            return Err(ColorError::NotFinite(row));
        }
    }
    let mut out = Vec::with_capacity(rgb.len());
    for pixel in rgb.chunks_exact(3) {
        let linear = [
            f64::from(pixel[0]) / 65535.0,
            f64::from(pixel[1]) / 65535.0,
            f64::from(pixel[2]) / 65535.0,
        ];
        let mapped = apply_matrix(matrix, linear);
        for value in mapped {
            let encoded = srgb_encode(value);
            out.push((encoded * 65535.0).round() as u16);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDENTITY: Matrix3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

    #[test]
    fn transfer_curve_fixes_black_and_white_and_is_linear_near_black() {
        assert_eq!(srgb_encode(0.0), 0.0);
        assert!((srgb_encode(1.0) - 1.0).abs() < 1e-12);
        assert!((srgb_encode(0.001) - 0.001 * 12.92).abs() < 1e-12);
    }

    #[test]
    fn transfer_curve_brightens_mid_grey_and_clamps_out_of_range_input() {
        let mid = srgb_encode(0.5);
        assert!(mid > 0.7 && mid < 0.75, "linear 0.5 encodes near 0.735, got {mid}");
        assert_eq!(srgb_encode(-3.0), 0.0);
        assert!((srgb_encode(7.0) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn identity_matrix_keeps_black_and_white_at_the_ends_of_the_range() {
        let out = apply_color_pipeline(&[0, 0, 0, 65535, 65535, 65535], &IDENTITY).unwrap();
        assert_eq!(out, vec![0, 0, 0, 65535, 65535, 65535]);
    }

    #[test]
    fn matrix_mixes_channels_before_the_curve() {
        // Output red is input red plus green (0.305 + 0.305); green and blue pass through.
        let mix = [[1.0, 1.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let out = apply_color_pipeline(&[20000, 20000, 0], &mix).unwrap();
        let linear_sum = 40000.0 / 65535.0;
        assert_eq!(out[0], (srgb_encode(linear_sum) * 65535.0).round() as u16);
        assert_eq!(out[1], (srgb_encode(20000.0 / 65535.0) * 65535.0).round() as u16);
        assert_eq!(out[2], 0);
    }

    #[test]
    fn over_range_matrix_output_is_clamped_to_white() {
        let gain = [[4.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let out = apply_color_pipeline(&[30000, 0, 0], &gain).unwrap();
        assert_eq!(out[0], 65535, "red 4x gain saturates at full scale");
        assert_eq!(out[1], 0);
    }

    #[test]
    fn partial_pixels_and_non_finite_matrices_are_refused() {
        assert_eq!(
            apply_color_pipeline(&[1, 2, 3, 4], &IDENTITY),
            Err(ColorError::NotRgb { actual: 4 })
        );
        let bad = [[1.0, 0.0, 0.0], [0.0, f64::NAN, 0.0], [0.0, 0.0, 1.0]];
        assert_eq!(apply_color_pipeline(&[1, 2, 3], &bad), Err(ColorError::NotFinite(1)));
    }

    #[test]
    fn empty_input_gives_empty_output() {
        assert_eq!(apply_color_pipeline(&[], &IDENTITY), Ok(Vec::new()));
    }
}
