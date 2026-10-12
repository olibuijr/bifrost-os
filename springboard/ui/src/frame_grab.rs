//! Frame grab model: which frame an image request for a time gets, and the size an image is scaled to. Mirrors
//! `AVAssetImageGenerator`: `requestedTimeToleranceBefore` and `requestedTimeToleranceAfter` bound how far from the
//! requested time a frame may be, `image(at:)` returns the frame together with its actual time, and `maximumSize`
//! bounds the generated image.
//!
//! Pure arithmetic over the frame times the caller already knows (for example the sample times of a track). No
//! decoding happens here.
//!
//! * A frame is a candidate when it lies within the tolerance window: at most `before_us` earlier than the requested
//!   time, or at most `after_us` later. A frame at exactly the requested time is always a candidate.
//! * Among the candidates the nearest frame wins. A tie goes to the earlier frame. This rule is this module's own
//!   choice; the official page does not state how ties are resolved.
//! * Frame times must be in non-decreasing order.
//! * `fit` scales an image to fit inside the maximum size, keeping its aspect ratio. It never enlarges an image,
//!   and the result is rounded down so it never exceeds the bound. Its sides are at least 1.

/// The reasons a frame or size request is refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FrameError {
    /// A tolerance is negative.
    NegativeTolerance,
    /// The frame times are not in non-decreasing order.
    Unsorted,
    /// A width, height or maximum bound is zero.
    ZeroSize,
}

/// How far from the requested time a frame may be, in microseconds. The default allows only an exact match.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tolerance {
    /// The largest allowed distance before the requested time.
    pub before_us: i64,
    /// The largest allowed distance after the requested time.
    pub after_us: i64,
}

impl Tolerance {
    /// A tolerance with the two bounds. Negative bounds are refused.
    pub fn new(before_us: i64, after_us: i64) -> Result<Self, FrameError> {
        if before_us < 0 || after_us < 0 {
            return Err(FrameError::NegativeTolerance);
        }
        Ok(Tolerance { before_us, after_us })
    }
}

/// The frame time to use for `requested_us`, chosen from `frames_us`. `Ok(None)` when no frame is inside the window.
pub fn pick(frames_us: &[i64], requested_us: i64, tolerance: Tolerance) -> Result<Option<i64>, FrameError> {
    if frames_us.windows(2).any(|w| w[0] > w[1]) {
        return Err(FrameError::Unsorted);
    }
    // The first frame at or after the request. The frame just before it is the closest one earlier.
    let index = frames_us.partition_point(|&t| t < requested_us);

    let before = index.checked_sub(1).map(|i| frames_us[i]).filter(|&t| {
        requested_us.saturating_sub(t) <= tolerance.before_us
    });
    let after = frames_us.get(index).copied().filter(|&t| {
        t.saturating_sub(requested_us) <= tolerance.after_us
    });

    Ok(match (before, after) {
        (Some(b), Some(a)) => {
            let db = requested_us.saturating_sub(b);
            let da = a.saturating_sub(requested_us);
            // A tie goes to the earlier frame.
            Some(if db <= da { b } else { a })
        }
        (Some(b), None) => Some(b),
        (None, Some(a)) => Some(a),
        (None, None) => None,
    })
}

/// The size an image of `width` by `height` is generated at, to fit inside `max_width` by `max_height`.
pub fn fit(width: u32, height: u32, max_width: u32, max_height: u32) -> Result<(u32, u32), FrameError> {
    if width == 0 || height == 0 || max_width == 0 || max_height == 0 {
        return Err(FrameError::ZeroSize);
    }
    if width <= max_width && height <= max_height {
        return Ok((width, height));
    }
    let (w, h) = (width as u64, height as u64);
    let (mw, mh) = (max_width as u64, max_height as u64);
    // Wider than the bound's aspect ratio: the width is the limit. Otherwise the height is.
    if w * mh > h * mw {
        let nh = (h * mw / w).max(1);
        Ok((max_width, nh as u32))
    } else {
        let nw = (w * mh / h).max(1);
        Ok((nw as u32, max_height))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: i64 = 1_000;

    #[test]
    fn exact_frame_is_picked_with_the_default_tolerance() {
        let frames = [0, 1000 * MS, 2000 * MS];
        assert_eq!(pick(&frames, 1000 * MS, Tolerance::default()), Ok(Some(1000 * MS)));
        assert_eq!(pick(&frames, 1500 * MS, Tolerance::default()), Ok(None));
    }

    #[test]
    fn a_tie_between_two_candidates_goes_to_the_earlier_frame() {
        let frames = [0, 2000 * MS];
        let tol = Tolerance::new(1000 * MS, 1000 * MS).unwrap();
        assert_eq!(pick(&frames, 1000 * MS, tol), Ok(Some(0)));
    }

    #[test]
    fn frames_outside_each_side_of_the_window_are_refused() {
        let frames = [0, 3000 * MS];
        let narrow = Tolerance::new(500 * MS, 500 * MS).unwrap();
        assert_eq!(pick(&frames, 1000 * MS, narrow), Ok(None));
        let wide_after = Tolerance::new(500 * MS, 2500 * MS).unwrap();
        assert_eq!(pick(&frames, 1000 * MS, wide_after), Ok(Some(3000 * MS)));
    }

    #[test]
    fn the_nearest_candidate_wins_and_a_request_before_all_frames_uses_the_first() {
        let frames = [0, 900 * MS, 2000 * MS];
        let tol = Tolerance::new(500 * MS, 1500 * MS).unwrap();
        // Before: 900 ms is 100 ms away. After: 2000 ms is 1000 ms away. The nearer one wins.
        assert_eq!(pick(&frames, 1000 * MS, tol), Ok(Some(900 * MS)));
        let later = [500 * MS];
        let after_only = Tolerance::new(0, 600 * MS).unwrap();
        assert_eq!(pick(&later, 0, after_only), Ok(Some(500 * MS)));
    }

    #[test]
    fn empty_and_unsorted_inputs_are_handled() {
        assert_eq!(pick(&[], 5, Tolerance::default()), Ok(None));
        assert_eq!(pick(&[2, 1], 0, Tolerance::default()), Err(FrameError::Unsorted));
        assert_eq!(Tolerance::new(-1, 0), Err(FrameError::NegativeTolerance));
    }

    #[test]
    fn fit_keeps_the_aspect_ratio_and_never_enlarges() {
        assert_eq!(fit(1920, 1080, 640, 640), Ok((640, 360)));
        assert_eq!(fit(100, 100, 640, 640), Ok((100, 100)));
        assert_eq!(fit(1000, 1000, 640, 320), Ok((320, 320)));
        assert_eq!(fit(0, 10, 640, 640), Err(FrameError::ZeroSize));
    }

    #[test]
    fn fit_rounds_down_so_the_bound_holds() {
        // 1000 x 333 into 100 x 100: width limits, height = 333 * 100 / 1000 = 33.3, rounded down to 33.
        assert_eq!(fit(1000, 333, 100, 100), Ok((100, 33)));
    }
}
