//! Sample attachments: the flags a decoder or renderer reads for one sample (sync and dependency hints, display
//! hints, coding redundancy) and the values that describe a whole sample buffer (empty-media markers, the reason a
//! frame was dropped, the speed multiplier, decoder resets). `SampleList` keeps one optional entry per sample in a
//! buffer, and an entry is only created when a caller asks for it, as the reference model does. This is the pure data
//! model. The media pipeline fills it in and the player reads it. No I/O and no system calls.

/// Why a video frame was not shown. The reference model gives these three reasons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DroppedFrameReason {
    /// The frame arrived after its presentation time.
    FrameWasLate,
    /// The module that supplies frames had no free buffer.
    OutOfBuffers,
    /// An unknown number of frames were lost before this one.
    Discontinuity,
}

/// Attachments for one sample. The defaults match the reference model: a sample is a sync sample, is not a partial
/// sync, is not displayed immediately and is displayed. The `Option` fields are unset until a producer states them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SampleAttachments {
    not_sync: bool,
    partial_sync: bool,
    display_immediately: bool,
    do_not_display: bool,
    depends_on_others: Option<bool>,
    is_depended_on_by_others: Option<bool>,
    earlier_display_times_allowed: Option<bool>,
    has_redundant_coding: Option<bool>,
}

impl SampleAttachments {
    /// True when the sample can start decoding (it is not marked as a non-sync sample).
    pub fn is_sync(&self) -> bool {
        !self.not_sync
    }

    pub fn set_not_sync(&mut self, value: bool) {
        self.not_sync = value;
    }

    pub fn is_partial_sync(&self) -> bool {
        self.partial_sync
    }

    pub fn set_partial_sync(&mut self, value: bool) {
        self.partial_sync = value;
    }

    pub fn display_immediately(&self) -> bool {
        self.display_immediately
    }

    pub fn set_display_immediately(&mut self, value: bool) {
        self.display_immediately = value;
    }

    /// Decoded but not shown: a reference frame that only later samples use.
    pub fn do_not_display(&self) -> bool {
        self.do_not_display
    }

    pub fn set_do_not_display(&mut self, value: bool) {
        self.do_not_display = value;
    }

    pub fn depends_on_others(&self) -> Option<bool> {
        self.depends_on_others
    }

    pub fn set_depends_on_others(&mut self, value: Option<bool>) {
        self.depends_on_others = value;
    }

    pub fn is_depended_on_by_others(&self) -> Option<bool> {
        self.is_depended_on_by_others
    }

    pub fn set_is_depended_on_by_others(&mut self, value: Option<bool>) {
        self.is_depended_on_by_others = value;
    }

    pub fn earlier_display_times_allowed(&self) -> Option<bool> {
        self.earlier_display_times_allowed
    }

    pub fn set_earlier_display_times_allowed(&mut self, value: Option<bool>) {
        self.earlier_display_times_allowed = value;
    }

    pub fn has_redundant_coding(&self) -> Option<bool> {
        self.has_redundant_coding
    }

    pub fn set_has_redundant_coding(&mut self, value: Option<bool>) {
        self.has_redundant_coding = value;
    }
}

/// Attachments for a whole sample buffer. The speed multiplier defaults to 1.0, which is why this type has a manual
/// `Default` rather than a derived one.
#[derive(Clone, Debug, PartialEq)]
pub struct BufferAttachments {
    empty_media: bool,
    permanent_empty_media: bool,
    display_empty_media_immediately: bool,
    force_key_frame: bool,
    reverse: bool,
    reset_decoder_before_decoding: bool,
    dropped_frame: Option<DroppedFrameReason>,
    speed_multiplier: f64,
}

impl Default for BufferAttachments {
    fn default() -> Self {
        BufferAttachments {
            empty_media: false,
            permanent_empty_media: false,
            display_empty_media_immediately: false,
            force_key_frame: false,
            reverse: false,
            reset_decoder_before_decoding: false,
            dropped_frame: None,
            speed_multiplier: 1.0,
        }
    }
}

impl BufferAttachments {
    /// An intentionally empty interval in the sequence of samples.
    pub fn empty_media(&self) -> bool {
        self.empty_media
    }

    pub fn set_empty_media(&mut self, value: bool) {
        self.empty_media = value;
    }

    /// The end of the sequence: no sample follows this one.
    pub fn permanent_empty_media(&self) -> bool {
        self.permanent_empty_media
    }

    pub fn set_permanent_empty_media(&mut self, value: bool) {
        self.permanent_empty_media = value;
    }

    pub fn display_empty_media_immediately(&self) -> bool {
        self.display_empty_media_immediately
    }

    pub fn set_display_empty_media_immediately(&mut self, value: bool) {
        self.display_empty_media_immediately = value;
    }

    pub fn force_key_frame(&self) -> bool {
        self.force_key_frame
    }

    pub fn set_force_key_frame(&mut self, value: bool) {
        self.force_key_frame = value;
    }

    pub fn reverse(&self) -> bool {
        self.reverse
    }

    pub fn set_reverse(&mut self, value: bool) {
        self.reverse = value;
    }

    pub fn reset_decoder_before_decoding(&self) -> bool {
        self.reset_decoder_before_decoding
    }

    pub fn set_reset_decoder_before_decoding(&mut self, value: bool) {
        self.reset_decoder_before_decoding = value;
    }

    pub fn dropped_frame(&self) -> Option<DroppedFrameReason> {
        self.dropped_frame
    }

    pub fn set_dropped_frame(&mut self, reason: Option<DroppedFrameReason>) {
        self.dropped_frame = reason;
    }

    pub fn speed_multiplier(&self) -> f64 {
        self.speed_multiplier
    }

    /// Sets the presentation speed factor. A factor must be finite and above zero; anything else is refused and the
    /// previous value stays. Returns whether the value was accepted.
    pub fn set_speed_multiplier(&mut self, factor: f64) -> bool {
        if factor.is_finite() && factor > 0.0 {
            self.speed_multiplier = factor;
            true
        } else {
            false
        }
    }
}

/// One optional attachment entry per sample of a buffer, in sample order. Entries start out unset; a read never
/// creates one, and `sample_mut` with `create` set does.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SampleList {
    items: Vec<Option<SampleAttachments>>,
}

impl SampleList {
    /// A list for a buffer of `count` samples, with no attachments set yet.
    pub fn new(count: usize) -> Self {
        SampleList { items: vec![None; count] }
    }

    /// The number of samples the list covers.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The attachments of sample `index`, if they were set. Out of range and unset both give `None`.
    pub fn sample(&self, index: usize) -> Option<&SampleAttachments> {
        self.items.get(index)?.as_ref()
    }

    /// The attachments of sample `index` for writing. When `create` is set and the entry is unset, it is created with
    /// the defaults. Out of range always gives `None`.
    pub fn sample_mut(&mut self, index: usize, create: bool) -> Option<&mut SampleAttachments> {
        let slot = self.items.get_mut(index)?;
        if slot.is_none() && create {
            *slot = Some(SampleAttachments::default());
        }
        slot.as_mut()
    }

    /// How many samples are marked as sync samples (set entries with `is_sync`). Unset samples count as sync, because
    /// the default is a sync sample.
    pub fn sync_count(&self) -> usize {
        self.items
            .iter()
            .filter(|slot| slot.as_ref().map_or(true, |a| a.is_sync()))
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_defaults_are_sync_and_displayed() {
        let a = SampleAttachments::default();
        assert!(a.is_sync());
        assert!(!a.is_partial_sync());
        assert!(!a.display_immediately());
        assert!(!a.do_not_display());
        assert_eq!(a.depends_on_others(), None);
        assert_eq!(a.has_redundant_coding(), None);
    }

    #[test]
    fn not_sync_flag_turns_off_sync() {
        let mut a = SampleAttachments::default();
        a.set_not_sync(true);
        assert!(!a.is_sync());
        a.set_not_sync(false);
        assert!(a.is_sync());
    }

    #[test]
    fn list_reads_do_not_create_entries() {
        let mut list = SampleList::new(3);
        assert_eq!(list.len(), 3);
        assert!(list.sample(1).is_none());
        assert!(list.sample_mut(1, false).is_none());
        assert!(list.sample(1).is_none());
        assert!(list.sample_mut(5, true).is_none());
    }

    #[test]
    fn create_flag_makes_a_default_entry_that_keeps_writes() {
        let mut list = SampleList::new(2);
        list.sample_mut(0, true).unwrap().set_do_not_display(true);
        let kept = list.sample(0).unwrap();
        assert!(kept.do_not_display());
        assert!(list.sample(1).is_none());
        // Sample 0 is a non-sync sample only after it is written; unset sample 1 still counts as sync.
        list.sample_mut(0, false).unwrap().set_not_sync(true);
        assert_eq!(list.sync_count(), 1);
    }

    #[test]
    fn buffer_defaults_and_dropped_reason() {
        let mut b = BufferAttachments::default();
        assert_eq!(b.speed_multiplier(), 1.0);
        assert_eq!(b.dropped_frame(), None);
        assert!(!b.empty_media());
        b.set_dropped_frame(Some(DroppedFrameReason::FrameWasLate));
        assert_eq!(b.dropped_frame(), Some(DroppedFrameReason::FrameWasLate));
        b.set_dropped_frame(None);
        assert_eq!(b.dropped_frame(), None);
    }

    #[test]
    fn speed_multiplier_refuses_zero_negative_and_non_finite() {
        let mut b = BufferAttachments::default();
        assert!(b.set_speed_multiplier(2.0));
        assert_eq!(b.speed_multiplier(), 2.0);
        assert!(!b.set_speed_multiplier(0.0));
        assert!(!b.set_speed_multiplier(-1.5));
        assert!(!b.set_speed_multiplier(f64::NAN));
        assert!(!b.set_speed_multiplier(f64::INFINITY));
        assert_eq!(b.speed_multiplier(), 2.0);
    }

    #[test]
    fn buffer_flags_round_trip() {
        let mut b = BufferAttachments::default();
        b.set_force_key_frame(true);
        b.set_reverse(true);
        b.set_permanent_empty_media(true);
        assert!(b.force_key_frame());
        assert!(b.reverse());
        assert!(b.permanent_empty_media());
        assert!(!b.reset_decoder_before_decoding());
    }
}
