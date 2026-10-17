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
use crate::runtime::Ctx;
use crate::service;
use bifrost_core::json::Json;

/// The shell's `editing` call: `get`, `commit` or `revert` the adjustment record of one asset.
fn editing_params(op: &str, asset: &str, extra: Vec<(&str, Json)>) -> Json {
    let mut fields = vec![("op".to_string(), Json::Str(op.into())), ("id".to_string(), Json::Str(asset.into()))];
    fields.extend(extra.into_iter().map(|(k, v)| (k.to_string(), v)));
    Json::Obj(fields)
}

/// Asks the shell for the adjustment record of `asset`. The reply decodes with [`decode_adjustment`].
pub fn load_adjustment(ctx: &Ctx, asset: &str) -> u32 {
    service::call(ctx, crate::photo_library::SERVICE, "editing", &editing_params("get", asset, vec![]))
}

/// Asks the shell to drop the edit of `asset`; the photo is the original again.
pub fn revert_adjustment(ctx: &Ctx, asset: &str) -> u32 {
    service::call(ctx, crate::photo_library::SERVICE, "editing", &editing_params("revert", asset, vec![]))
}

/// The record from a `get` reply: outer None for a malformed reply, inner None when the asset has no edit.
pub fn decode_adjustment(reply: &Json) -> Option<Option<AdjustmentData>> {
    let a = match reply.get("adjustment")? {
        Json::Null => return Some(None),
        other => other,
    };
    let bytes = from_hex(&a.str_field("data")?)?;
    AdjustmentData::new(&a.str_field("format_identifier")?, &a.str_field("format_version")?, bytes).map(Some)
}

fn from_hex(text: &str) -> Option<Vec<u8>> {
    if text.len() % 2 != 0 {
        return None;
    }
    (0..text.len()).step_by(2).map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok()).collect()
}

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

    /// Asks the shell to store this record as the edit of `asset` (the `editing` call, `op` `commit`). The shell keeps
    /// the original picture; the reply is `{committed: true}` or an error.
    pub fn save(&self, ctx: &Ctx, asset: &str) -> u32 {
        let hex: String = self.adjustment_data.data().iter().map(|b| format!("{b:02x}")).collect();
        let extra = vec![
            ("format_identifier", Json::Str(self.adjustment_data.format_identifier().into())),
            ("format_version", Json::Str(self.adjustment_data.format_version().into())),
            ("data", Json::Str(hex)),
        ];
        service::call(ctx, crate::photo_library::SERVICE, "editing", &editing_params("commit", asset, extra))
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

    #[test]
    fn a_get_reply_decodes_to_the_stored_record() {
        let reply = bifrost_core::json::parse(r#"{"adjustment":{"format_identifier":"com.example.crop","format_version":"1","data":"0a0b"}}"#).unwrap();
        let a = decode_adjustment(&reply).unwrap().unwrap();
        assert_eq!((a.format_identifier(), a.data()), ("com.example.crop", &[0x0a, 0x0b][..]));
        assert_eq!(decode_adjustment(&bifrost_core::json::parse(r#"{"adjustment":null}"#).unwrap()), Some(None));
        assert!(decode_adjustment(&bifrost_core::json::parse(r#"{"adjustment":{"format_identifier":"x","format_version":"1","data":"zz"}}"#).unwrap()).is_none());
    }
}
