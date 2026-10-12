//! Object push transfer panel: a pure model of one push of files to a nearby Bluetooth device, the state behind the
//! transfer sheet or panel. It holds the device, the title and icon, the file list, the transfer state and the bytes
//! sent so far. The caller drives it: `start` begins the push, `advance` reports the bytes the transport sent, and
//! `stop` cancels it. No radio is touched here, and no byte is sent; the transport and the drawn panel live elsewhere.
//!
//! Rules: a push needs at least one file and a device, and it starts once. Bytes past the total are clamped, so the
//! push finishes exactly when all bytes are sent. A push with only empty files finishes at `start`. `stop` only
//! cancels a push that is in progress.
//!
//! Remaining gaps: nothing speaks the object push profile, so no file reaches a device yet, and the panel is not drawn.

use std::fmt;

/// One file in a push, with its size in bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PushFile {
    pub name: String,
    pub bytes: u64,
}

impl PushFile {
    pub fn new(name: &str, bytes: u64) -> Self {
        PushFile { name: name.to_string(), bytes }
    }
}

/// Where the push is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PushState {
    /// Created and not started.
    Idle,
    InProgress,
    /// Every byte was sent.
    Finished,
    /// Cancelled with `stop` while in progress.
    Stopped,
}

/// Why `start` refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PushError {
    NoDevice,
    NoFiles,
    AlreadyStarted,
}

impl fmt::Display for PushError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PushError::NoDevice => write!(f, "no device chosen for the push"),
            PushError::NoFiles => write!(f, "no files to push"),
            PushError::AlreadyStarted => write!(f, "the push was already started"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectPush {
    device: Option<String>,
    title: String,
    icon: Option<String>,
    files: Vec<PushFile>,
    state: PushState,
    sent: u64,
}

impl ObjectPush {
    /// A push of `files` to `device`.
    pub fn to_device(device: &str, files: Vec<PushFile>) -> Self {
        let mut push = Self::with_files(files);
        push.device = Some(device.to_string());
        push
    }

    /// A push of `files` with no device yet; the panel asks for one before `start`.
    pub fn with_files(files: Vec<PushFile>) -> Self {
        ObjectPush {
            device: None,
            title: "Send File".to_string(),
            icon: None,
            files,
            state: PushState::Idle,
            sent: 0,
        }
    }

    pub fn device(&self) -> Option<&str> {
        self.device.as_deref()
    }

    pub fn set_device(&mut self, device: &str) {
        self.device = Some(device.to_string());
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn set_title(&mut self, title: &str) {
        self.title = title.to_string();
    }

    pub fn icon(&self) -> Option<&str> {
        self.icon.as_deref()
    }

    /// Sets the icon by name; `None` clears it.
    pub fn set_icon(&mut self, icon: Option<&str>) {
        self.icon = icon.map(|s| s.to_string());
    }

    pub fn files(&self) -> &[PushFile] {
        &self.files
    }

    pub fn state(&self) -> PushState {
        self.state
    }

    pub fn is_transfer_in_progress(&self) -> bool {
        self.state == PushState::InProgress
    }

    /// Sum of all file sizes, saturating at `u64::MAX`.
    pub fn total_bytes(&self) -> u64 {
        self.files.iter().fold(0u64, |acc, f| acc.saturating_add(f.bytes))
    }

    pub fn sent_bytes(&self) -> u64 {
        self.sent
    }

    /// Fraction sent, 0.0 to 1.0. A push with no bytes reads 0.0 until it finishes.
    pub fn progress(&self) -> f32 {
        let total = self.total_bytes();
        if total == 0 {
            return 0.0;
        }
        self.sent as f32 / total as f32
    }

    /// Begins the push. Refuses with no device, no files, or a push that was already started.
    /// A push whose files are all empty finishes at once.
    pub fn start(&mut self) -> Result<(), PushError> {
        if self.state != PushState::Idle {
            return Err(PushError::AlreadyStarted);
        }
        if self.device.is_none() {
            return Err(PushError::NoDevice);
        }
        if self.files.is_empty() {
            return Err(PushError::NoFiles);
        }
        self.state = PushState::InProgress;
        if self.total_bytes() == 0 {
            self.state = PushState::Finished;
        }
        Ok(())
    }

    /// Records `bytes` sent by the transport and returns the new state. Ignored unless the push is in progress.
    /// The count is clamped to the total, and the push finishes when it reaches it.
    pub fn advance(&mut self, bytes: u64) -> PushState {
        if self.state != PushState::InProgress {
            return self.state;
        }
        let total = self.total_bytes();
        self.sent = self.sent.saturating_add(bytes).min(total);
        if self.sent == total {
            self.state = PushState::Finished;
        }
        self.state
    }

    /// Cancels a push in progress. Does nothing in any other state.
    pub fn stop(&mut self) {
        if self.state == PushState::InProgress {
            self.state = PushState::Stopped;
        }
    }

    /// The file being sent: the first file whose bytes are not all sent yet. Empty files are skipped.
    pub fn current_file(&self) -> Option<&PushFile> {
        let mut start = 0u64;
        for file in &self.files {
            let end = start.saturating_add(file.bytes);
            if self.sent < end {
                return Some(file);
            }
            start = end;
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn two_files() -> ObjectPush {
        // 100 + 50 = 150 bytes in total.
        ObjectPush::to_device("Phone", vec![PushFile::new("a.txt", 100), PushFile::new("b.png", 50)])
    }

    #[test]
    fn progress_follows_bytes_and_the_current_file() {
        let mut push = two_files();
        assert_eq!(push.total_bytes(), 150);
        assert_eq!(push.start(), Ok(()));
        assert!(push.is_transfer_in_progress());
        assert_eq!(push.advance(30), PushState::InProgress);
        assert!((push.progress() - 0.2).abs() < 1e-6);
        assert_eq!(push.current_file().map(|f| f.name.as_str()), Some("a.txt"));
        push.advance(80); // 110 sent: 100 of a.txt and 10 of b.png
        assert_eq!(push.current_file().map(|f| f.name.as_str()), Some("b.png"));
        assert!((push.progress() - 110.0 / 150.0).abs() < 1e-6);
    }

    #[test]
    fn overshoot_is_clamped_and_finishes() {
        let mut push = two_files();
        push.start().unwrap();
        assert_eq!(push.advance(1000), PushState::Finished);
        assert_eq!(push.sent_bytes(), 150);
        assert!(!push.is_transfer_in_progress());
        assert!((push.progress() - 1.0).abs() < 1e-6);
        assert_eq!(push.current_file(), None);
    }

    #[test]
    fn start_refuses_without_device_or_files_and_only_once() {
        let mut no_device = ObjectPush::with_files(vec![PushFile::new("a", 1)]);
        assert_eq!(no_device.start(), Err(PushError::NoDevice));
        no_device.set_device("Keyboard");
        assert_eq!(no_device.start(), Ok(()));
        assert_eq!(no_device.start(), Err(PushError::AlreadyStarted));

        let mut empty = ObjectPush::to_device("Phone", vec![]);
        assert_eq!(empty.start(), Err(PushError::NoFiles));
        assert_eq!(empty.state(), PushState::Idle);
    }

    #[test]
    fn stop_cancels_only_a_push_in_progress() {
        let mut push = two_files();
        push.stop();
        assert_eq!(push.state(), PushState::Idle);
        push.start().unwrap();
        push.advance(20);
        push.stop();
        assert_eq!(push.state(), PushState::Stopped);
        assert_eq!(push.advance(500), PushState::Stopped);
        assert_eq!(push.sent_bytes(), 20);
    }

    #[test]
    fn all_empty_files_finish_at_start_with_zero_progress() {
        let mut push = ObjectPush::to_device("Phone", vec![PushFile::new("empty", 0)]);
        assert_eq!(push.start(), Ok(()));
        assert_eq!(push.state(), PushState::Finished);
        assert_eq!(push.progress(), 0.0);
    }

    #[test]
    fn title_and_icon_are_stored_and_cleared() {
        let mut push = two_files();
        assert_eq!(push.title(), "Send File");
        push.set_title("Send to Phone");
        push.set_icon(Some("phone"));
        assert_eq!(push.title(), "Send to Phone");
        assert_eq!(push.icon(), Some("phone"));
        push.set_icon(None);
        assert_eq!(push.icon(), None);
    }
}
