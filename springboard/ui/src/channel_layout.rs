//! Channel layouts: the speaker position of each channel in an interleaved stream, with a label per channel. A
//! layout can be built from presets (mono, stereo, quadraphonic, 5.1, 7.1) or from a list of labels, and it can
//! fold a frame down to stereo. The fold keeps the front pair, adds the centre and the rear or side channels at
//! -3 dB, and drops the low-frequency effects channel. Pure model: no samples are read from a device here.

use std::f32::consts::FRAC_1_SQRT_2;

/// The speaker position of one channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Label {
    Left,
    Right,
    Center,
    /// Low-frequency effects (the sub-woofer channel).
    Lfe,
    LeftSurround,
    RightSurround,
    BackLeft,
    BackRight,
}

/// The most channels a layout may hold.
pub const MAX_CHANNELS: usize = 16;

/// Failures of the layout model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutError {
    /// A layout needs at least one channel.
    Empty,
    /// The same position appears twice.
    Duplicate,
    /// More than `MAX_CHANNELS` channels.
    TooManyChannels,
    /// A frame or block does not match the channel count.
    LengthMismatch,
}

/// An ordered list of channel positions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelLayout {
    labels: Vec<Label>,
}

impl ChannelLayout {
    /// A layout from labels in stream order. It is refused when empty, longer than `MAX_CHANNELS`, or when a
    /// position repeats.
    pub fn from_labels(labels: Vec<Label>) -> Result<Self, LayoutError> {
        if labels.is_empty() {
            return Err(LayoutError::Empty);
        }
        if labels.len() > MAX_CHANNELS {
            return Err(LayoutError::TooManyChannels);
        }
        for (i, label) in labels.iter().enumerate() {
            if labels[..i].contains(label) {
                return Err(LayoutError::Duplicate);
            }
        }
        Ok(Self { labels })
    }

    /// One centre channel.
    pub fn mono() -> Self {
        Self { labels: vec![Label::Center] }
    }

    /// Left and right.
    pub fn stereo() -> Self {
        Self { labels: vec![Label::Left, Label::Right] }
    }

    /// Front pair and rear pair.
    pub fn quadraphonic() -> Self {
        Self { labels: vec![Label::Left, Label::Right, Label::LeftSurround, Label::RightSurround] }
    }

    /// Front pair, centre, low-frequency effects and side pair.
    pub fn surround_5_1() -> Self {
        Self {
            labels: vec![
                Label::Left,
                Label::Right,
                Label::Center,
                Label::Lfe,
                Label::LeftSurround,
                Label::RightSurround,
            ],
        }
    }

    /// The 5.1 layout with a rear pair added.
    pub fn surround_7_1() -> Self {
        Self {
            labels: vec![
                Label::Left,
                Label::Right,
                Label::Center,
                Label::Lfe,
                Label::LeftSurround,
                Label::RightSurround,
                Label::BackLeft,
                Label::BackRight,
            ],
        }
    }

    /// Number of channels.
    pub fn channel_count(&self) -> usize {
        self.labels.len()
    }

    /// The positions in stream order.
    pub fn labels(&self) -> &[Label] {
        &self.labels
    }

    /// The index of the channel at `label`, if the layout has one.
    pub fn index_of(&self, label: Label) -> Option<usize> {
        self.labels.iter().position(|&l| l == label)
    }

    /// Folds one frame (one sample per channel) to a stereo pair `[left, right]`.
    pub fn downmix_frame(&self, frame: &[f32]) -> Result<[f32; 2], LayoutError> {
        if frame.len() != self.labels.len() {
            return Err(LayoutError::LengthMismatch);
        }
        let mut out = [0.0f32; 2];
        for (label, &s) in self.labels.iter().zip(frame) {
            match label {
                Label::Left | Label::LeftSurround | Label::BackLeft => {
                    out[0] += if *label == Label::Left { s } else { s * FRAC_1_SQRT_2 };
                }
                Label::Right | Label::RightSurround | Label::BackRight => {
                    out[1] += if *label == Label::Right { s } else { s * FRAC_1_SQRT_2 };
                }
                Label::Center => {
                    out[0] += s * FRAC_1_SQRT_2;
                    out[1] += s * FRAC_1_SQRT_2;
                }
                Label::Lfe => {}
            }
        }
        Ok(out)
    }

    /// Folds interleaved frames to interleaved stereo. The block must hold a whole number of frames.
    pub fn downmix_stereo(&self, samples: &[f32]) -> Result<Vec<f32>, LayoutError> {
        if samples.len() % self.labels.len() != 0 {
            return Err(LayoutError::LengthMismatch);
        }
        let mut out = Vec::with_capacity(samples.len() / self.labels.len() * 2);
        for frame in samples.chunks_exact(self.labels.len()) {
            let [l, r] = self.downmix_frame(frame)?;
            out.push(l);
            out.push(r);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn presets_have_the_expected_channel_counts() {
        assert_eq!(ChannelLayout::mono().channel_count(), 1);
        assert_eq!(ChannelLayout::stereo().channel_count(), 2);
        assert_eq!(ChannelLayout::quadraphonic().channel_count(), 4);
        assert_eq!(ChannelLayout::surround_5_1().channel_count(), 6);
        assert_eq!(ChannelLayout::surround_7_1().channel_count(), 8);
    }

    #[test]
    fn index_of_follows_stream_order() {
        let l = ChannelLayout::surround_5_1();
        assert_eq!(l.index_of(Label::Lfe), Some(3));
        assert_eq!(l.index_of(Label::BackLeft), None);
    }

    #[test]
    fn from_labels_refuses_empty_duplicate_and_oversized_layouts() {
        assert_eq!(ChannelLayout::from_labels(vec![]), Err(LayoutError::Empty));
        assert_eq!(
            ChannelLayout::from_labels(vec![Label::Left, Label::Left]),
            Err(LayoutError::Duplicate)
        );
        assert_eq!(
            ChannelLayout::from_labels(vec![Label::Center; 17]),
            Err(LayoutError::TooManyChannels)
        );
        let ok = ChannelLayout::from_labels(vec![Label::Right, Label::Left]).unwrap();
        assert_eq!(ok.labels(), &[Label::Right, Label::Left]);
    }

    #[test]
    fn stereo_frame_passes_through_unchanged() {
        let l = ChannelLayout::stereo();
        assert_eq!(l.downmix_frame(&[0.3, 0.4]), Ok([0.3, 0.4]));
    }

    #[test]
    fn mono_centre_is_split_at_minus_three_decibels() {
        let l = ChannelLayout::mono();
        let [a, b] = l.downmix_frame(&[1.0]).unwrap();
        assert!(close(a, FRAC_1_SQRT_2));
        assert!(close(b, FRAC_1_SQRT_2));
    }

    #[test]
    fn surround_fold_drops_lfe_and_folds_side_channels() {
        let l = ChannelLayout::surround_5_1();
        // L, R, C, LFE, Ls, Rs
        let [a, b] = l.downmix_frame(&[0.0, 0.0, 0.0, 1.0, 1.0, 0.0]).unwrap();
        assert!(close(a, FRAC_1_SQRT_2));
        assert!(close(b, 0.0));
        let [c, d] = l.downmix_frame(&[1.0, 0.5, 0.0, 1.0, 0.0, 0.0]).unwrap();
        assert!(close(c, 1.0));
        assert!(close(d, 0.5));
    }

    #[test]
    fn block_downmix_checks_length_and_folds_each_frame() {
        let l = ChannelLayout::surround_5_1();
        assert_eq!(l.downmix_stereo(&[0.0; 5]), Err(LayoutError::LengthMismatch));
        let frames = [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0];
        let out = l.downmix_stereo(&frames).unwrap();
        assert_eq!(out.len(), 4);
        assert!(close(out[0], 1.0));
        assert!(close(out[1], 0.0));
        assert!(close(out[2], 0.0));
        assert!(close(out[3], 1.0));
        assert_eq!(l.downmix_frame(&[0.0; 3]), Err(LayoutError::LengthMismatch));
    }
}
