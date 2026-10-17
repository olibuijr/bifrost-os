//! `VideoAnalyzer`: the measuring half of video analysis. `highlight_rank` ranks windows of per-frame scores and
//! `keyframe_pick` chooses the frame closest to all others by luma histogram, but neither looks at pixels. This takes
//! decoded frames one at a time, measures each (a luma histogram, and the motion since the previous frame as the mean
//! absolute luma difference), and answers the two requests from what it measured: the most active segments, and the
//! representative frame.
//!
//! The analysis is classical image statistics, so it runs on any frames the app decodes, with no model. It does not
//! recognise content (faces, scenes); that needs a model. Frames must all be the same size.

use crate::highlight_rank::{highlights, Segment};
use crate::keyframe_pick::{histogram, representative, Frame, KeyFrame};
use std::fmt;

/// Why a frame was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnalyzeError {
    /// The luma plane is empty or its length is not width times height.
    BadFrame,
    /// The frame is not the size of the first frame.
    SizeChanged,
}

impl fmt::Display for AnalyzeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            AnalyzeError::BadFrame => "the frame is empty or its size does not match its pixels",
            AnalyzeError::SizeChanged => "the frame is not the size of the earlier frames",
        })
    }
}

impl std::error::Error for AnalyzeError {}

/// The luma plane of packed pixels (red in the low byte, then green, blue, alpha), with the Rec. 601 weights.
pub fn luma_from_pixels(px: &[u32]) -> Vec<u8> {
    px.iter()
        .map(|&p| {
            let (r, g, b) = (p & 255, (p >> 8) & 255, (p >> 16) & 255);
            ((77 * r + 150 * g + 29 * b) >> 8) as u8
        })
        .collect()
}

/// Collects measurements of the frames of one video.
pub struct VideoAnalyzer {
    bins: usize,
    frames: Vec<Frame>,
    motion: Vec<f64>,
    size: Option<(usize, usize)>,
    previous: Vec<u8>,
}

impl VideoAnalyzer {
    /// An analyzer whose histograms have `bins` bins (clamped to 2..=256).
    pub fn new(bins: usize) -> VideoAnalyzer {
        VideoAnalyzer { bins: bins.clamp(2, 256), frames: Vec::new(), motion: Vec::new(), size: None, previous: Vec::new() }
    }

    /// Measures one decoded frame. The first frame has no motion; later ones are measured against the one before.
    pub fn push_luma(&mut self, time_ms: f64, luma: &[u8], width: usize, height: usize) -> Result<(), AnalyzeError> {
        if width == 0 || height == 0 || luma.len() != width * height {
            return Err(AnalyzeError::BadFrame);
        }
        if let Some(size) = self.size {
            if size != (width, height) {
                return Err(AnalyzeError::SizeChanged);
            }
        } else {
            self.size = Some((width, height));
        }
        let motion = if self.previous.is_empty() {
            0.0
        } else {
            let total: u64 = luma.iter().zip(&self.previous).map(|(&a, &b)| u64::from(a.abs_diff(b))).sum();
            total as f64 / (luma.len() as f64 * 255.0)
        };
        self.frames.push(Frame { time_ms, histogram: histogram(luma, self.bins) });
        self.motion.push(motion);
        self.previous = luma.to_vec();
        Ok(())
    }

    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }

    /// The motion of each frame, 0 (identical to the one before) to 1 (black to white).
    pub fn motion(&self) -> &[f64] {
        &self.motion
    }

    /// Up to `count` most active, non-overlapping windows of `window` frames, best first.
    pub fn highlights(&self, window: usize, count: usize) -> Vec<Segment> {
        highlights(&self.motion, window, count)
    }

    /// The frame closest to all the others by luma histogram.
    pub fn key_frame(&self) -> Option<KeyFrame> {
        representative(&self.frames)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(value: u8) -> Vec<u8> {
        vec![value; 16]
    }

    #[test]
    fn motion_is_the_mean_difference_to_the_previous_frame() {
        let mut a = VideoAnalyzer::new(8);
        a.push_luma(0.0, &flat(100), 4, 4).unwrap();
        a.push_luma(40.0, &flat(100), 4, 4).unwrap();
        a.push_luma(80.0, &flat(255), 4, 4).unwrap();
        assert_eq!(a.motion()[0], 0.0);
        assert_eq!(a.motion()[1], 0.0);
        assert!((a.motion()[2] - 155.0 / 255.0).abs() < 1e-9);
        assert_eq!(a.frame_count(), 3);
    }

    #[test]
    fn the_most_active_window_is_the_highlight() {
        let mut a = VideoAnalyzer::new(8);
        for (i, v) in [10u8, 10, 10, 240, 10, 10].into_iter().enumerate() {
            a.push_luma(i as f64 * 40.0, &flat(v), 4, 4).unwrap();
        }
        let best = a.highlights(1, 1);
        assert_eq!(best.len(), 1);
        // the jump to 240 is frame 3 and the jump back is frame 4; the larger step goes up to 240 minus 10 either way
        assert!(best[0].start == 3 || best[0].start == 4);
        let two = a.highlights(1, 2);
        let starts: Vec<usize> = two.iter().map(|s| s.start).collect();
        assert!(starts.contains(&3) && starts.contains(&4));
    }

    #[test]
    fn the_key_frame_is_the_one_most_like_the_rest() {
        let mut a = VideoAnalyzer::new(8);
        a.push_luma(0.0, &flat(20), 4, 4).unwrap();
        a.push_luma(100.0, &flat(25), 4, 4).unwrap();
        a.push_luma(200.0, &flat(30), 4, 4).unwrap();
        a.push_luma(300.0, &flat(250), 4, 4).unwrap();
        let key = a.key_frame().unwrap();
        // the three dark frames share a histogram bin, so the medoid is the earliest of them
        assert_eq!(key.index, 0);
        assert_eq!(key.time_ms, 0.0);
    }

    #[test]
    fn bad_frames_are_refused() {
        let mut a = VideoAnalyzer::new(8);
        assert_eq!(a.push_luma(0.0, &[], 0, 0), Err(AnalyzeError::BadFrame));
        assert_eq!(a.push_luma(0.0, &flat(1), 4, 3), Err(AnalyzeError::BadFrame));
        a.push_luma(0.0, &flat(1), 4, 4).unwrap();
        assert_eq!(a.push_luma(1.0, &vec![1; 6], 3, 2), Err(AnalyzeError::SizeChanged));
        assert_eq!(a.frame_count(), 1);
        assert!(VideoAnalyzer::new(8).key_frame().is_none());
        assert!(VideoAnalyzer::new(8).highlights(1, 1).is_empty());
    }

    #[test]
    fn luma_follows_the_green_heavy_weights() {
        let px = [u32::from_le_bytes([255, 0, 0, 255]), u32::from_le_bytes([0, 255, 0, 255]), u32::from_le_bytes([0, 0, 255, 255])];
        let y = luma_from_pixels(&px);
        assert!(y[1] > y[0] && y[0] > y[2]);
        assert_eq!(luma_from_pixels(&[u32::from_le_bytes([255, 255, 255, 255])]), vec![255]);
    }
}
