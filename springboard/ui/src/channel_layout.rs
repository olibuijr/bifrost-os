//! Channel layouts: the tag that names a speaker arrangement, the role of each channel in order, and the typed error
//! for a layout that cannot be built. This is the pure model behind a layout description. It carries no samples and
//! does not open a device; the audio service reads and writes the layout with the stream it plays.

/// The most channels a custom layout may describe.
pub const MAX_CHANNELS: usize = 64;

/// The name of a standard arrangement, `Unknown` when the arrangement is not given, or `Descriptions` when each
/// channel is given one by one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tag {
    Unknown,
    Mono,
    Stereo,
    Quadraphonic,
    Surround51,
    Surround71,
    Descriptions,
}

/// The role of one channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Label {
    Unknown,
    Left,
    Right,
    Center,
    Lfe,
    LeftSurround,
    RightSurround,
    LeftRear,
    RightRear,
}

/// Why a layout could not be built.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutError {
    /// A custom layout with no channels.
    Empty,
    /// More channels than `MAX_CHANNELS`.
    TooManyChannels { max: usize },
    /// The tag names no arrangement, so the caller must give the labels.
    NeedsLabels,
}

/// The labels of a standard tag, in channel order. `None` for a tag with no fixed arrangement.
fn standard(tag: Tag) -> Option<&'static [Label]> {
    match tag {
        Tag::Mono => Some(&[Label::Center]),
        Tag::Stereo => Some(&[Label::Left, Label::Right]),
        Tag::Quadraphonic => Some(&[Label::Left, Label::Right, Label::LeftSurround, Label::RightSurround]),
        Tag::Surround51 => Some(&[
            Label::Left,
            Label::Right,
            Label::Center,
            Label::Lfe,
            Label::LeftSurround,
            Label::RightSurround,
        ]),
        Tag::Surround71 => Some(&[
            Label::Left,
            Label::Right,
            Label::Center,
            Label::Lfe,
            Label::LeftSurround,
            Label::RightSurround,
            Label::LeftRear,
            Label::RightRear,
        ]),
        Tag::Unknown | Tag::Descriptions => None,
    }
}

/// A channel layout: its tag and one label per channel, in channel order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelLayout {
    tag: Tag,
    labels: Vec<Label>,
}

impl ChannelLayout {
    /// The layout a standard tag names. `Unknown` and `Descriptions` name no arrangement, so they are refused.
    pub fn from_tag(tag: Tag) -> Result<Self, LayoutError> {
        match standard(tag) {
            Some(labels) => Ok(ChannelLayout { tag, labels: labels.to_vec() }),
            None => Err(LayoutError::NeedsLabels),
        }
    }

    /// A layout from one label per channel. When the labels equal a standard arrangement, the tag is that
    /// arrangement, so `[Left, Right]` equals `from_tag(Tag::Stereo)`. Otherwise the tag is `Descriptions`.
    pub fn from_labels(labels: &[Label]) -> Result<Self, LayoutError> {
        if labels.is_empty() {
            return Err(LayoutError::Empty);
        }
        if labels.len() > MAX_CHANNELS {
            return Err(LayoutError::TooManyChannels { max: MAX_CHANNELS });
        }
        let tag = [Tag::Mono, Tag::Stereo, Tag::Quadraphonic, Tag::Surround51, Tag::Surround71]
            .iter()
            .copied()
            .find(|t| standard(*t) == Some(labels))
            .unwrap_or(Tag::Descriptions);
        Ok(ChannelLayout { tag, labels: labels.to_vec() })
    }

    pub fn tag(&self) -> Tag {
        self.tag
    }

    pub fn labels(&self) -> &[Label] {
        &self.labels
    }

    pub fn number_of_channels(&self) -> usize {
        self.labels.len()
    }

    /// The label of channel `index`, or `None` past the last channel.
    pub fn label_at(&self, index: usize) -> Option<Label> {
        self.labels.get(index).copied()
    }

    /// The first channel with `label`.
    pub fn index_of(&self, label: Label) -> Option<usize> {
        self.labels.iter().position(|l| *l == label)
    }

    /// Sets every label to `Unknown` and keeps the channel count. The tag becomes `Descriptions`, because the
    /// labels no longer describe a standard arrangement.
    pub fn set_all_to_unknown(&mut self) {
        for label in self.labels.iter_mut() {
            *label = Label::Unknown;
        }
        self.tag = Tag::Descriptions;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stereo_has_left_then_right() {
        let layout = ChannelLayout::from_tag(Tag::Stereo).unwrap();
        assert_eq!(layout.tag(), Tag::Stereo);
        assert_eq!(layout.number_of_channels(), 2);
        assert_eq!(layout.labels(), &[Label::Left, Label::Right]);
    }

    #[test]
    fn mono_is_one_center_channel() {
        let layout = ChannelLayout::from_tag(Tag::Mono).unwrap();
        assert_eq!(layout.labels(), &[Label::Center]);
        assert_eq!(layout.label_at(0), Some(Label::Center));
        assert_eq!(layout.label_at(1), None);
    }

    #[test]
    fn surround51_puts_lfe_fourth_and_right_surround_sixth() {
        let layout = ChannelLayout::from_tag(Tag::Surround51).unwrap();
        assert_eq!(layout.number_of_channels(), 6);
        assert_eq!(layout.index_of(Label::Lfe), Some(3));
        assert_eq!(layout.label_at(5), Some(Label::RightSurround));
        assert_eq!(layout.label_at(6), None);
        assert_eq!(layout.index_of(Label::LeftRear), None);
    }

    #[test]
    fn unknown_and_descriptions_tags_need_labels() {
        assert_eq!(ChannelLayout::from_tag(Tag::Unknown), Err(LayoutError::NeedsLabels));
        assert_eq!(ChannelLayout::from_tag(Tag::Descriptions), Err(LayoutError::NeedsLabels));
    }

    #[test]
    fn empty_and_oversized_label_lists_are_refused() {
        assert_eq!(ChannelLayout::from_labels(&[]), Err(LayoutError::Empty));
        assert_eq!(
            ChannelLayout::from_labels(&vec![Label::Center; 65]),
            Err(LayoutError::TooManyChannels { max: 64 })
        );
        let full = ChannelLayout::from_labels(&vec![Label::Center; 64]).unwrap();
        assert_eq!(full.number_of_channels(), 64);
        assert_eq!(full.tag(), Tag::Descriptions);
    }

    #[test]
    fn labels_that_match_a_standard_arrangement_take_its_tag() {
        let stereo = ChannelLayout::from_labels(&[Label::Left, Label::Right]).unwrap();
        assert_eq!(stereo, ChannelLayout::from_tag(Tag::Stereo).unwrap());
        let odd = ChannelLayout::from_labels(&[Label::Left, Label::Right, Label::Center]).unwrap();
        assert_eq!(odd.tag(), Tag::Descriptions);
    }

    #[test]
    fn set_all_to_unknown_keeps_the_count_and_drops_the_tag() {
        let mut layout = ChannelLayout::from_tag(Tag::Stereo).unwrap();
        layout.set_all_to_unknown();
        assert_eq!(layout.labels(), &[Label::Unknown, Label::Unknown]);
        assert_eq!(layout.number_of_channels(), 2);
        assert_eq!(layout.tag(), Tag::Descriptions);
    }
}
