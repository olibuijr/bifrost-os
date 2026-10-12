//! Per-sample attachments and data-readiness notifications for a run of media samples. Each sample carries flags
//! that tell the pipeline how to decode and show it: whether it is a sync point, whether it needs earlier
//! samples, whether it is shown at all, and why a frame was dropped. A set also tracks whether each sample's
//! data is pending, ready or failed, and queues one notification per change. No media is decoded here.

/// Why a frame was dropped before display.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DroppedReason {
    /// The frame arrived after its presentation time.
    Late,
    /// The pipeline had no free buffer for it.
    OutOfBuffers,
    /// The source stream skipped ahead.
    Discontinuity,
}

/// Flags attached to one sample. The default is a sync sample that is shown and was not dropped.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SampleAttachments {
    /// The sample cannot be decoded without earlier samples.
    pub not_sync: bool,
    /// The sample depends on samples that come before it.
    pub depends_on_others: bool,
    /// The sample is decoded but not shown.
    pub dont_display: bool,
    /// The sample is shown as soon as it is ready, ignoring its timestamp.
    pub display_immediately: bool,
    /// The reason the frame was dropped, if it was.
    pub dropped: Option<DroppedReason>,
}

impl SampleAttachments {
    /// True when decoding can start at this sample.
    pub fn is_sync_point(&self) -> bool {
        !self.not_sync
    }

    /// True when the sample is shown: it is not hidden and was not dropped.
    pub fn is_displayable(&self) -> bool {
        !self.dont_display && self.dropped.is_none()
    }
}

/// Whether a sample's data is still loading, loaded, or failed to load.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DataState {
    Pending,
    Ready,
    Failed,
}

/// A change to a sample's data, queued until the owner takes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SampleEvent {
    /// The data of the sample at `index` became ready.
    DataBecameReady { index: usize },
    /// The data of the sample at `index` failed to load.
    DataFailed { index: usize },
}

/// Failures of the attachment model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttachError {
    /// No sample is at that index.
    NoSample,
    /// The sample's data is already ready or failed, so the change does not apply.
    InvalidState,
}

#[derive(Clone, Copy, Debug)]
struct Entry {
    attachments: SampleAttachments,
    state: DataState,
}

/// An ordered run of samples with their attachments, data states and a queue of notifications.
#[derive(Debug, Default)]
pub struct SampleSet {
    entries: Vec<Entry>,
    events: Vec<SampleEvent>,
}

impl SampleSet {
    /// An empty set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a sample whose data is pending, and returns its index.
    pub fn push(&mut self, attachments: SampleAttachments) -> usize {
        self.entries.push(Entry { attachments, state: DataState::Pending });
        self.entries.len() - 1
    }

    /// Number of samples.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when the set holds no samples.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The attachments of the sample at `index`.
    pub fn attachments(&self, index: usize) -> Option<&SampleAttachments> {
        self.entries.get(index).map(|e| &e.attachments)
    }

    /// Mutable attachments of the sample at `index`, for setting flags after the sample was pushed.
    pub fn attachments_mut(&mut self, index: usize) -> Option<&mut SampleAttachments> {
        self.entries.get_mut(index).map(|e| &mut e.attachments)
    }

    /// The data state of the sample at `index`.
    pub fn state(&self, index: usize) -> Option<DataState> {
        self.entries.get(index).map(|e| e.state)
    }

    /// Marks the data of a pending sample as ready and queues `DataBecameReady`. Marking a sample that is already
    /// ready again does nothing. A failed sample cannot become ready.
    pub fn mark_ready(&mut self, index: usize) -> Result<(), AttachError> {
        let entry = self.entries.get_mut(index).ok_or(AttachError::NoSample)?;
        match entry.state {
            DataState::Pending => {
                entry.state = DataState::Ready;
                self.events.push(SampleEvent::DataBecameReady { index });
                Ok(())
            }
            DataState::Ready => Ok(()),
            DataState::Failed => Err(AttachError::InvalidState),
        }
    }

    /// Marks the data of a pending sample as failed and queues `DataFailed`. A ready sample cannot fail.
    pub fn mark_failed(&mut self, index: usize) -> Result<(), AttachError> {
        let entry = self.entries.get_mut(index).ok_or(AttachError::NoSample)?;
        match entry.state {
            DataState::Pending => {
                entry.state = DataState::Failed;
                self.events.push(SampleEvent::DataFailed { index });
                Ok(())
            }
            DataState::Failed => Ok(()),
            DataState::Ready => Err(AttachError::InvalidState),
        }
    }

    /// Takes the queued notifications, oldest first, and leaves the queue empty.
    pub fn take_events(&mut self) -> Vec<SampleEvent> {
        std::mem::take(&mut self.events)
    }

    /// Indexes of every sample that is a sync point, in order.
    pub fn sync_points(&self) -> Vec<usize> {
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.attachments.is_sync_point())
            .map(|(i, _)| i)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_sample_is_a_displayable_sync_point() {
        let a = SampleAttachments::default();
        assert!(a.is_sync_point());
        assert!(a.is_displayable());
    }

    #[test]
    fn hidden_or_dropped_samples_are_not_displayable() {
        let hidden = SampleAttachments { dont_display: true, ..Default::default() };
        let late = SampleAttachments { dropped: Some(DroppedReason::Late), ..Default::default() };
        assert!(!hidden.is_displayable());
        assert!(!late.is_displayable());
        let predicted = SampleAttachments { not_sync: true, depends_on_others: true, ..Default::default() };
        assert!(!predicted.is_sync_point());
        assert!(predicted.is_displayable());
    }

    #[test]
    fn sync_points_skip_samples_flagged_not_sync() {
        let mut set = SampleSet::new();
        set.push(SampleAttachments::default());
        let second = set.push(SampleAttachments::default());
        set.push(SampleAttachments::default());
        set.attachments_mut(second).unwrap().not_sync = true;
        assert_eq!(set.sync_points(), vec![0, 2]);
        assert_eq!(set.len(), 3);
    }

    #[test]
    fn data_ready_queues_one_notification() {
        let mut set = SampleSet::new();
        let i = set.push(SampleAttachments::default());
        assert_eq!(set.state(i), Some(DataState::Pending));
        assert_eq!(set.mark_ready(i), Ok(()));
        assert_eq!(set.mark_ready(i), Ok(()));
        assert_eq!(set.state(i), Some(DataState::Ready));
        assert_eq!(set.take_events(), vec![SampleEvent::DataBecameReady { index: 0 }]);
        assert!(set.take_events().is_empty());
    }

    #[test]
    fn failed_and_ready_states_do_not_switch() {
        let mut set = SampleSet::new();
        let a = set.push(SampleAttachments::default());
        let b = set.push(SampleAttachments::default());
        assert_eq!(set.mark_failed(a), Ok(()));
        assert_eq!(set.mark_ready(a), Err(AttachError::InvalidState));
        assert_eq!(set.mark_ready(b), Ok(()));
        assert_eq!(set.mark_failed(b), Err(AttachError::InvalidState));
        assert_eq!(
            set.take_events(),
            vec![SampleEvent::DataFailed { index: 0 }, SampleEvent::DataBecameReady { index: 1 }]
        );
    }

    #[test]
    fn unknown_index_is_refused() {
        let mut set = SampleSet::new();
        assert_eq!(set.mark_ready(0), Err(AttachError::NoSample));
        assert_eq!(set.mark_failed(4), Err(AttachError::NoSample));
        assert!(set.attachments(0).is_none());
        assert!(set.is_empty());
    }
}
