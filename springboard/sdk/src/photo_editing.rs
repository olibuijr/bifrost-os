//! Photo editing for apps (the PhotoKit `PHContentEditingController` surface): the input an editor is handed, the output
//! it returns, and the session that keeps them. An edit is stored as an adjustment record (see
//! `bifrost_ui::photo_adjustment`), so the original file is never rewritten and the edit can be reverted.
//!
//! ```ignore
//! let mut session = PHContentEditingSession::begin("com.example.editor", &input)?;
//! // ... the editor changes the picture ...
//! session.finish(PHContentEditingOutput::new(record))?;
//! ```
//!
//! Gap: there is no extension host yet. The Photos app does not load third-party editors, so these types are the
//! contract an editor extension will use once the host exists.

use crate::controls::photo_adjustment::{AdjustmentData, EditError, EditSession};

/// `PHContentEditingInput`: the asset to edit and the adjustment record a previous edit left, if any.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PHContentEditingInput {
    pub asset_identifier: String,
    pub adjustment_data: Option<AdjustmentData>,
}

/// `PHContentEditingOutput`: the adjustment record an editor produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PHContentEditingOutput {
    pub adjustment_data: AdjustmentData,
}

impl PHContentEditingOutput {
    pub fn new(adjustment_data: AdjustmentData) -> PHContentEditingOutput {
        PHContentEditingOutput { adjustment_data }
    }
}

/// One editing session on one asset, for one editor format.
pub struct PHContentEditingSession {
    inner: EditSession,
}

impl PHContentEditingSession {
    /// Starts a session for `editor_format`. `None` when the format identifier is blank.
    pub fn begin(editor_format: &str, input: &PHContentEditingInput) -> Option<PHContentEditingSession> {
        Some(PHContentEditingSession { inner: EditSession::begin(editor_format, input.adjustment_data.clone())? })
    }

    /// The record the asset had when the session began.
    pub fn input_adjustment(&self) -> Option<&AdjustmentData> {
        self.inner.input()
    }

    /// Stores the editor's output. Refused when the record belongs to another editor format.
    pub fn finish(&mut self, output: PHContentEditingOutput) -> Result<(), EditError> {
        self.inner.commit(output.adjustment_data)
    }

    /// Drops the edit and returns the photo to its original.
    pub fn revert(&mut self) {
        self.inner.revert();
    }

    /// The output the session would save now, if the person edited the photo and did not revert it.
    pub fn output(&self) -> Option<PHContentEditingOutput> {
        self.inner.result().cloned().map(PHContentEditingOutput::new)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(format: &str, bytes: &[u8]) -> AdjustmentData {
        AdjustmentData::new(format, "1", bytes.to_vec()).unwrap()
    }

    fn input(adjustment: Option<AdjustmentData>) -> PHContentEditingInput {
        PHContentEditingInput { asset_identifier: "asset-1".into(), adjustment_data: adjustment }
    }

    #[test]
    fn a_session_saves_the_output_it_finishes_with() {
        let mut s = PHContentEditingSession::begin("com.example.crop", &input(None)).unwrap();
        assert!(s.input_adjustment().is_none());
        assert!(s.output().is_none());
        s.finish(PHContentEditingOutput::new(record("com.example.crop", b"x"))).unwrap();
        assert_eq!(s.output().unwrap().adjustment_data.data(), b"x");
    }

    #[test]
    fn a_session_starts_from_the_previous_edit() {
        let previous = record("com.example.crop", b"old");
        let s = PHContentEditingSession::begin("com.example.crop", &input(Some(previous.clone()))).unwrap();
        assert_eq!(s.input_adjustment(), Some(&previous));
    }

    #[test]
    fn another_editors_record_is_refused() {
        let mut s = PHContentEditingSession::begin("com.example.crop", &input(None)).unwrap();
        assert_eq!(s.finish(PHContentEditingOutput::new(record("com.other.filter", b"x"))), Err(EditError::FormatMismatch));
        assert!(s.output().is_none());
    }

    #[test]
    fn revert_drops_the_edit() {
        let mut s = PHContentEditingSession::begin("com.example.crop", &input(None)).unwrap();
        s.finish(PHContentEditingOutput::new(record("com.example.crop", b"x"))).unwrap();
        s.revert();
        assert!(s.output().is_none());
    }

    #[test]
    fn a_blank_editor_format_cannot_begin() {
        assert!(PHContentEditingSession::begin("  ", &input(None)).is_none());
    }
}
