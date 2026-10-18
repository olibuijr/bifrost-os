//! Adjustment records for edited photos, and the editing session that reads one and writes the next. An adjustment
//! record keeps the edit as data (an editor's format identifier, the format's version and the editor's payload), so the
//! original file is never rewritten and the edit can be read back or dropped. A session starts from the record a
//! previous edit left, accepts a committed record only from the same editor format, and can revert the photo to its
//! original. The model is pure: the shell stores the record and renders the picture.

/// One edit of a photo as data: the editor's format identifier, the format version and the editor's payload bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdjustmentData {
    format_identifier: String,
    format_version: String,
    data: Vec<u8>,
}

impl AdjustmentData {
    /// A record with a trimmed, non-empty format identifier and version. None when either is blank.
    pub fn new(format_identifier: &str, format_version: &str, data: Vec<u8>) -> Option<Self> {
        let format_identifier = format_identifier.trim();
        let format_version = format_version.trim();
        if format_identifier.is_empty() || format_version.is_empty() {
            return None;
        }
        Some(AdjustmentData {
            format_identifier: format_identifier.to_string(),
            format_version: format_version.to_string(),
            data,
        })
    }

    pub fn format_identifier(&self) -> &str {
        &self.format_identifier
    }

    pub fn format_version(&self) -> &str {
        &self.format_version
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }
}

/// Why a committed record was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditError {
    /// The record was written by another editor format than the one this session belongs to.
    FormatMismatch,
}

/// What the session does to the photo. `Untouched` keeps whatever record the photo had when the session began.
#[derive(Clone, Debug, PartialEq, Eq)]
enum State {
    Untouched,
    Committed(AdjustmentData),
    Reverted,
}

/// One editing session on one photo, for one editor format.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditSession {
    format_identifier: String,
    input: Option<AdjustmentData>,
    state: State,
}

impl EditSession {
    /// A session for an editor with the given format identifier, starting from the record the photo already has. None
    /// when the identifier is blank. A record written by another editor format is kept as the input but cannot be
    /// read by this session, so it is not offered as the input.
    pub fn begin(format_identifier: &str, input: Option<AdjustmentData>) -> Option<Self> {
        let format_identifier = format_identifier.trim();
        if format_identifier.is_empty() {
            return None;
        }
        let input = input.filter(|record| record.format_identifier() == format_identifier);
        Some(EditSession { format_identifier: format_identifier.to_string(), input, state: State::Untouched })
    }

    /// The record the photo had when the session began, if this editor can read it.
    pub fn input(&self) -> Option<&AdjustmentData> {
        self.input.as_ref()
    }

    /// Stores `record` as the photo's edit. Returns an error and changes nothing when the record names another format.
    pub fn commit(&mut self, record: AdjustmentData) -> Result<(), EditError> {
        if record.format_identifier() != self.format_identifier {
            return Err(EditError::FormatMismatch);
        }
        self.state = State::Committed(record);
        Ok(())
    }

    /// Drops the edit and returns the photo to its original. Reverting works even when nothing was committed.
    pub fn revert(&mut self) {
        self.state = State::Reverted;
    }

    /// The record the photo keeps once the session ends: the committed record, none after a revert, and the input record
    /// when the session did not change the photo.
    pub fn result(&self) -> Option<&AdjustmentData> {
        match &self.state {
            State::Untouched => self.input.as_ref(),
            State::Committed(record) => Some(record),
            State::Reverted => None,
        }
    }

    /// True when the photo keeps an edit once the session ends.
    pub fn has_adjustments(&self) -> bool {
        self.result().is_some()
    }
}

/// The editors a host has installed, by format identifier. An edit is read back only by the editor whose format wrote
/// it, so the host finds that editor from the record alone, and a session starts only for an installed editor.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EditorRegistry {
    /// (format identifier, display name), in install order
    editors: Vec<(String, String)>,
}

impl EditorRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Installs an editor for a format under a display name. False, and nothing changes, when either is blank or the
    /// format already has an editor.
    pub fn install(&mut self, format_identifier: &str, name: &str) -> bool {
        let format_identifier = format_identifier.trim();
        let name = name.trim();
        if format_identifier.is_empty() || name.is_empty() || self.editors.iter().any(|(f, _)| f == format_identifier) {
            return false;
        }
        self.editors.push((format_identifier.to_string(), name.to_string()));
        true
    }

    /// Removes the editor of a format. False when none was installed.
    pub fn uninstall(&mut self, format_identifier: &str) -> bool {
        let before = self.editors.len();
        let format_identifier = format_identifier.trim();
        self.editors.retain(|(f, _)| f != format_identifier);
        self.editors.len() != before
    }

    /// The display name of the installed editor that can read `record`, if there is one.
    pub fn editor_for(&self, record: &AdjustmentData) -> Option<&str> {
        self.editors.iter().find(|(f, _)| f == record.format_identifier()).map(|(_, name)| name.as_str())
    }

    /// Starts an editing session for the installed editor of `format_identifier`, from the photo's record. None when no
    /// editor of that format is installed.
    pub fn session_for(&self, format_identifier: &str, input: Option<AdjustmentData>) -> Option<EditSession> {
        let format_identifier = format_identifier.trim();
        if !self.editors.iter().any(|(f, _)| f == format_identifier) {
            return None;
        }
        EditSession::begin(format_identifier, input)
    }

    /// The installed editors in install order, as (format identifier, display name).
    pub fn editors(&self) -> impl Iterator<Item = (&str, &str)> {
        self.editors.iter().map(|(f, n)| (f.as_str(), n.as_str()))
    }

    pub fn len(&self) -> usize {
        self.editors.len()
    }

    pub fn is_empty(&self) -> bool {
        self.editors.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(format: &str) -> AdjustmentData {
        AdjustmentData::new(format, "1", vec![1, 2, 3]).unwrap()
    }

    #[test]
    fn an_editor_is_installed_once_per_format() {
        let mut r = EditorRegistry::new();
        assert!(r.install("com.example.grain", "Grain"));
        assert!(!r.install(" com.example.grain ", "Other"), "the format already has an editor");
        assert!(!r.install("", "Blank"), "a blank format");
        assert!(!r.install("com.example.blank", "  "), "a blank name");
        assert_eq!(r.len(), 1);
        assert_eq!(r.editors().collect::<Vec<_>>(), vec![("com.example.grain", "Grain")]);
    }

    #[test]
    fn the_editor_of_a_record_is_found_by_its_format() {
        let mut r = EditorRegistry::new();
        r.install("com.example.grain", "Grain");
        assert_eq!(r.editor_for(&record("com.example.grain")), Some("Grain"));
        assert_eq!(r.editor_for(&record("com.example.other")), None);
    }

    #[test]
    fn a_session_starts_only_for_an_installed_editor() {
        let mut r = EditorRegistry::new();
        assert!(r.session_for("com.example.grain", None).is_none(), "nothing installed");
        r.install("com.example.grain", "Grain");
        let session = r.session_for("com.example.grain", Some(record("com.example.grain")));
        assert!(session.is_some_and(|s| s.input().is_some()), "the record is this editor's input");
    }

    #[test]
    fn uninstalling_removes_the_editor() {
        let mut r = EditorRegistry::new();
        r.install("com.example.grain", "Grain");
        assert!(r.uninstall("com.example.grain"));
        assert!(!r.uninstall("com.example.grain"), "already gone");
        assert!(r.is_empty());
        assert_eq!(r.editor_for(&record("com.example.grain")), None);
    }
}
