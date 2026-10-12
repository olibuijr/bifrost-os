//! Export session model: the state of one export of a media asset to a new file, with a preset, an output file type,
//! a time range, a file length limit, progress and a status. Mirrors `AVAssetExportSession`: the statuses
//! (`unknown`, `waiting`, `exporting`, `completed`, `failed`, `cancelled`), the time range, `fileLengthLimit`, and the
//! rule that an export only starts for a file type the session supports.
//!
//! Pure state only. No media is read or written here; the caller feeds in how much of the export is done and how many
//! bytes the output has.
//!
//! * The time range is in microseconds. It must start at or after zero and end at or before the source duration.
//!   Without a range the whole source is exported.
//! * Progress is the fraction of the export duration processed, clamped to 0..=1. It never moves backwards.
//! * A session starts once, and only for a file type in the supported list. Finished sessions do not change.
//! * `fileLengthLimit` is a maximum: an output of exactly the limit is allowed. A limit of zero means no limit.
//! * The preset names and file type names are opaque strings; this module does not check them against a table.

/// The state of an export operation, as the official status values name them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportStatus {
    /// Not started yet, or the state is not known.
    Unknown,
    /// Waiting for more source data to become available.
    Waiting,
    /// The export is in progress.
    Exporting,
    /// The export finished and the output is complete.
    Completed,
    /// The export failed. The reason is in `ExportSession::error`.
    Failed,
    /// The export was cancelled.
    Cancelled,
}

impl ExportStatus {
    /// True for the three states an export does not leave: completed, failed and cancelled.
    pub fn is_finished(self) -> bool {
        matches!(self, ExportStatus::Completed | ExportStatus::Failed | ExportStatus::Cancelled)
    }
}

/// The reasons a session call is refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExportError {
    /// The preset name is empty.
    EmptyPreset,
    /// The source duration is zero or negative.
    NoSource,
    /// The time range starts before zero, has no length, or runs past the end of the source.
    RangeOutOfSource,
    /// The file type is not in the session's supported list.
    UnsupportedFileType,
    /// `start` was called on a session that already started or finished.
    AlreadyStarted,
    /// The call needs an export that is running.
    NotExporting,
    /// The session already finished, so it cannot be cancelled or completed again.
    Finished,
}

/// One export of a source asset. Create it with `new`, set the optional range and limit, then `start`.
#[derive(Clone, Debug, PartialEq)]
pub struct ExportSession {
    preset: String,
    source_us: i64,
    supported: Vec<String>,
    range: Option<(i64, i64)>,
    limit_bytes: u64,
    status: ExportStatus,
    processed_us: i64,
    error: Option<String>,
}

impl ExportSession {
    /// A session for a source of `source_us` microseconds with the named preset. The status starts as `Unknown`.
    pub fn new(preset: &str, source_us: i64) -> Result<Self, ExportError> {
        if preset.trim().is_empty() {
            return Err(ExportError::EmptyPreset);
        }
        if source_us <= 0 {
            return Err(ExportError::NoSource);
        }
        Ok(ExportSession {
            preset: preset.to_string(),
            source_us,
            supported: Vec::new(),
            range: None,
            limit_bytes: 0,
            status: ExportStatus::Unknown,
            processed_us: 0,
            error: None,
        })
    }

    /// The preset name this session exports with.
    pub fn preset(&self) -> &str {
        &self.preset
    }

    /// The file types the session can write, as the caller determined them for this preset and asset.
    pub fn set_supported_file_types(&mut self, types: &[&str]) {
        self.supported = types.iter().map(|t| t.to_string()).collect();
    }

    /// The file types the session can write.
    pub fn supported_file_types(&self) -> &[String] {
        &self.supported
    }

    /// Exports `start_us` to `start_us + duration_us` of the source. Refused unless the start is not negative, the
    /// duration is positive, and the range ends within the source.
    pub fn set_time_range(&mut self, start_us: i64, duration_us: i64) -> Result<(), ExportError> {
        if self.status != ExportStatus::Unknown {
            return Err(ExportError::AlreadyStarted);
        }
        if start_us < 0 || duration_us <= 0 {
            return Err(ExportError::RangeOutOfSource);
        }
        let end = start_us.checked_add(duration_us).ok_or(ExportError::RangeOutOfSource)?;
        if end > self.source_us {
            return Err(ExportError::RangeOutOfSource);
        }
        self.range = Some((start_us, duration_us));
        Ok(())
    }

    /// The range that will be exported: `(start_us, duration_us)`. Without a range, the whole source.
    pub fn time_range(&self) -> (i64, i64) {
        self.range.unwrap_or((0, self.source_us))
    }

    /// Sets the maximum output size in bytes. Zero removes the limit.
    pub fn set_file_length_limit(&mut self, bytes: u64) {
        self.limit_bytes = bytes;
    }

    /// True when an output of `bytes` is within the limit. With no limit, every size is.
    pub fn fits_limit(&self, bytes: u64) -> bool {
        self.limit_bytes == 0 || bytes <= self.limit_bytes
    }

    /// Starts the export to `file_type`. The file type must be in the supported list.
    pub fn start(&mut self, file_type: &str) -> Result<(), ExportError> {
        if self.status != ExportStatus::Unknown {
            return Err(ExportError::AlreadyStarted);
        }
        if !self.supported.iter().any(|t| t.as_str() == file_type) {
            return Err(ExportError::UnsupportedFileType);
        }
        self.status = ExportStatus::Exporting;
        Ok(())
    }

    /// Records that `processed_us` microseconds of the export duration are done. Values above the duration are
    /// clamped, and a lower value does not move progress back.
    pub fn report_progress(&mut self, processed_us: i64) -> Result<(), ExportError> {
        match self.status {
            ExportStatus::Exporting | ExportStatus::Waiting => {}
            ExportStatus::Unknown => return Err(ExportError::NotExporting),
            _ => return Err(ExportError::Finished),
        }
        let duration = self.time_range().1;
        let clamped = processed_us.clamp(0, duration);
        if clamped > self.processed_us {
            self.processed_us = clamped;
        }
        Ok(())
    }

    /// The progress fraction, 0 to 1. A completed export reports 1.
    pub fn progress(&self) -> f64 {
        if self.status == ExportStatus::Completed {
            return 1.0;
        }
        let duration = self.time_range().1 as f64;
        (self.processed_us as f64 / duration).clamp(0.0, 1.0)
    }

    /// Marks the export as waiting for more source data. Only a running export can wait.
    pub fn wait_for_data(&mut self) -> Result<(), ExportError> {
        match self.status {
            ExportStatus::Exporting => {
                self.status = ExportStatus::Waiting;
                Ok(())
            }
            ExportStatus::Unknown => Err(ExportError::NotExporting),
            _ => Err(ExportError::Finished),
        }
    }

    /// Marks the export as running again after a wait.
    pub fn resume(&mut self) -> Result<(), ExportError> {
        match self.status {
            ExportStatus::Waiting => {
                self.status = ExportStatus::Exporting;
                Ok(())
            }
            ExportStatus::Unknown => Err(ExportError::NotExporting),
            _ => Err(ExportError::Finished),
        }
    }

    /// Completes the export. Progress becomes 1.
    pub fn complete(&mut self) -> Result<(), ExportError> {
        match self.status {
            ExportStatus::Exporting | ExportStatus::Waiting => {
                self.status = ExportStatus::Completed;
                self.processed_us = self.time_range().1;
                Ok(())
            }
            ExportStatus::Unknown => Err(ExportError::NotExporting),
            _ => Err(ExportError::Finished),
        }
    }

    /// Fails the export with a reason. The status becomes `Failed`.
    pub fn fail(&mut self, reason: &str) -> Result<(), ExportError> {
        match self.status {
            ExportStatus::Exporting | ExportStatus::Waiting => {
                self.status = ExportStatus::Failed;
                self.error = Some(reason.to_string());
                Ok(())
            }
            ExportStatus::Unknown => Err(ExportError::NotExporting),
            _ => Err(ExportError::Finished),
        }
    }

    /// Cancels a session that has not finished. A session that never started can be cancelled too.
    pub fn cancel(&mut self) -> Result<(), ExportError> {
        if self.status.is_finished() {
            return Err(ExportError::Finished);
        }
        self.status = ExportStatus::Cancelled;
        Ok(())
    }

    /// The current status.
    pub fn status(&self) -> ExportStatus {
        self.status
    }

    /// The failure reason, once the export has failed.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECOND: i64 = 1_000_000;

    fn session() -> ExportSession {
        let mut s = ExportSession::new("high", 10 * SECOND).unwrap();
        s.set_supported_file_types(&["mp4", "mov"]);
        s
    }

    #[test]
    fn new_rejects_empty_preset_and_empty_source() {
        assert_eq!(ExportSession::new("  ", SECOND), Err(ExportError::EmptyPreset));
        assert_eq!(ExportSession::new("high", 0), Err(ExportError::NoSource));
        let s = ExportSession::new("high", SECOND).unwrap();
        assert_eq!(s.status(), ExportStatus::Unknown);
        assert_eq!(s.time_range(), (0, SECOND));
    }

    #[test]
    fn time_range_must_lie_inside_the_source() {
        let mut s = session();
        assert_eq!(s.set_time_range(-1, SECOND), Err(ExportError::RangeOutOfSource));
        assert_eq!(s.set_time_range(0, 0), Err(ExportError::RangeOutOfSource));
        assert_eq!(s.set_time_range(9 * SECOND, 2 * SECOND), Err(ExportError::RangeOutOfSource));
        assert_eq!(s.set_time_range(2 * SECOND, 3 * SECOND), Ok(()));
        assert_eq!(s.time_range(), (2 * SECOND, 3 * SECOND));
    }

    #[test]
    fn start_needs_a_supported_file_type_and_runs_once() {
        let mut s = session();
        assert_eq!(s.start("avi"), Err(ExportError::UnsupportedFileType));
        assert_eq!(s.start("mov"), Ok(()));
        assert_eq!(s.status(), ExportStatus::Exporting);
        assert_eq!(s.start("mp4"), Err(ExportError::AlreadyStarted));
    }

    #[test]
    fn progress_is_a_clamped_fraction_of_the_range_and_never_moves_back() {
        let mut s = session();
        s.set_time_range(0, 4 * SECOND).unwrap();
        assert_eq!(s.report_progress(SECOND), Err(ExportError::NotExporting));
        s.start("mp4").unwrap();
        s.report_progress(SECOND).unwrap();
        assert!((s.progress() - 0.25).abs() < 1e-12);
        s.report_progress(SECOND / 2).unwrap();
        assert!((s.progress() - 0.25).abs() < 1e-12);
        s.report_progress(99 * SECOND).unwrap();
        assert!((s.progress() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn completed_export_reports_full_progress_and_cannot_be_cancelled() {
        let mut s = session();
        s.start("mp4").unwrap();
        s.complete().unwrap();
        assert_eq!(s.status(), ExportStatus::Completed);
        assert_eq!(s.progress(), 1.0);
        assert_eq!(s.cancel(), Err(ExportError::Finished));
        assert_eq!(s.complete(), Err(ExportError::Finished));
    }

    #[test]
    fn failure_keeps_the_reason_and_cancel_works_before_start() {
        let mut s = session();
        s.cancel().unwrap();
        assert_eq!(s.status(), ExportStatus::Cancelled);
        assert!(s.status().is_finished());

        let mut f = session();
        f.start("mp4").unwrap();
        f.wait_for_data().unwrap();
        f.resume().unwrap();
        f.fail("encoder stopped").unwrap();
        assert_eq!(f.status(), ExportStatus::Failed);
        assert_eq!(f.error(), Some("encoder stopped"));
    }

    #[test]
    fn file_length_limit_is_inclusive_and_zero_means_none() {
        let mut s = session();
        assert!(s.fits_limit(u64::MAX));
        s.set_file_length_limit(1000);
        assert!(s.fits_limit(1000));
        assert!(!s.fits_limit(1001));
    }
}
