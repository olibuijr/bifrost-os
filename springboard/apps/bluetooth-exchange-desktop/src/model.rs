//! Bluetooth File Exchange's state and rules: the files in Downloads that can be sent, the queue of transfers with their
//! progress, the files received, and the device the next file goes to. Pure: `worker.rs` and `transport.rs` touch the
//! disk and the bus, `window.rs` lays out and draws.

use bifrost_core::bt_address::BdAddr;
use std::path::{Path, PathBuf};

/// A file in a folder: one that can be sent, or one that was received.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileEntry {
    pub name: String,
    pub path: PathBuf,
    pub size: u64,
    /// Seconds since the Unix epoch (0 when unknown).
    pub modified: i64,
}

/// Where a transfer is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Queued behind the transfer that is running, or behind nothing yet.
    Waiting,
    /// The transport is sending the file.
    Sending,
    Sent,
    /// The transfer failed; the text says why, in words a person can act on.
    Failed(String),
    Cancelled,
}

impl Phase {
    /// True when the transfer is over (sent, failed or cancelled) and no longer takes the radio.
    pub fn finished(&self) -> bool {
        matches!(self, Phase::Sent | Phase::Failed(_) | Phase::Cancelled)
    }
}

/// One file queued for one device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transfer {
    /// Identifies the transfer across replies from the worker; never reused.
    pub id: u64,
    pub file: FileEntry,
    pub device: BdAddr,
    /// The device's name (or address) when the file was queued.
    pub device_label: String,
    /// Bytes the transport reports as sent.
    pub sent: u64,
    pub phase: Phase,
}

impl Transfer {
    /// The share of the file sent, from 0.0 to 1.0. A finished transfer is complete; an empty file is zero until it is sent.
    pub fn fraction(&self) -> f32 {
        match (&self.phase, self.file.size) {
            (Phase::Sent, _) => 1.0,
            (_, 0) => 0.0,
            _ => (self.sent as f32 / self.file.size as f32).clamp(0.0, 1.0),
        }
    }

    /// The line under the file name in the transfers list.
    pub fn status_text(&self) -> String {
        match &self.phase {
            Phase::Waiting => "Waiting".into(),
            Phase::Sending => format!("{} of {}", size_text(self.sent), size_text(self.file.size)),
            Phase::Sent => format!("Sent to {}", self.device_label),
            Phase::Failed(text) => text.clone(),
            Phase::Cancelled => "Cancelled".into(),
        }
    }
}

/// What the transport reports for a transfer (BlueZ `Transfer1.Status`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Queued,
    Active,
    Complete,
    Error,
    Other,
}

/// Reads a BlueZ transfer status. A suspended transfer is still active.
pub fn parse_status(text: &str) -> Status {
    match text {
        "queued" => Status::Queued,
        "active" | "suspended" => Status::Active,
        "complete" => Status::Complete,
        "error" => Status::Error,
        _ => Status::Other,
    }
}

/// The folder the person's files to send are read from: Downloads under `home`.
pub fn outbox_dir(home: &Path) -> PathBuf {
    home.join("Downloads")
}

/// The folder received files are listed from: Bluetooth under Downloads.
pub fn inbox_dir(home: &Path) -> PathBuf {
    outbox_dir(home).join("Bluetooth")
}

/// The window's state. Transfers are kept in the order they were queued, oldest first.
#[derive(Debug, Default)]
pub struct Model {
    pub transfers: Vec<Transfer>,
    /// The files that can be sent, newest first.
    pub outbox: Vec<FileEntry>,
    /// The files received, newest first.
    pub inbox: Vec<FileEntry>,
    /// The last message for the person (the footer shows it until the next one).
    pub status: String,
    next_id: u64,
}

impl Model {
    /// Queues `file` for the device at `device`. Returns the transfer's id.
    pub fn queue(&mut self, file: FileEntry, device: BdAddr, device_label: String) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        self.transfers.push(Transfer { id, file, device, device_label, sent: 0, phase: Phase::Waiting });
        id
    }

    /// True while a transfer is sending. Only one file goes out at a time.
    pub fn busy(&self) -> bool {
        self.transfers.iter().any(|t| t.phase == Phase::Sending)
    }

    /// The next transfer to start: the oldest waiting one, when nothing is sending.
    pub fn next_waiting(&self) -> Option<u64> {
        if self.busy() {
            return None;
        }
        self.transfers.iter().find(|t| t.phase == Phase::Waiting).map(|t| t.id)
    }

    /// The transfer with `id`, if it is still listed.
    pub fn transfer(&self, id: u64) -> Option<&Transfer> {
        self.transfers.iter().find(|t| t.id == id)
    }

    /// Marks a waiting transfer as sending.
    pub fn start(&mut self, id: u64) {
        if let Some(t) = self.find_mut(id).filter(|t| t.phase == Phase::Waiting) {
            t.phase = Phase::Sending;
        }
    }

    /// Records the bytes the transport has sent so far, while the transfer is sending.
    pub fn progress(&mut self, id: u64, sent: u64) {
        if let Some(t) = self.find_mut(id).filter(|t| t.phase == Phase::Sending) {
            t.sent = sent.min(t.file.size);
        }
    }

    /// The transport finished the transfer, or it failed with `Err`. A transfer that is already over keeps its state.
    pub fn finish(&mut self, id: u64, result: Result<(), String>) {
        if let Some(t) = self.find_mut(id).filter(|t| !t.phase.finished()) {
            match result {
                Ok(()) => {
                    t.sent = t.file.size;
                    t.phase = Phase::Sent;
                }
                Err(text) => t.phase = Phase::Failed(text),
            }
        }
    }

    /// The transport confirmed a cancel. Nothing happens to a transfer that is already over.
    pub fn cancelled(&mut self, id: u64) {
        if let Some(t) = self.find_mut(id).filter(|t| !t.phase.finished()) {
            t.phase = Phase::Cancelled;
        }
    }

    /// The person cancelled transfer `id`. A waiting transfer is cancelled here; a sending one must be stopped by the
    /// worker, so the return value is true and the caller asks for it.
    pub fn request_cancel(&mut self, id: u64) -> bool {
        match self.find_mut(id).map(|t| t.phase.clone()) {
            Some(Phase::Waiting) => {
                self.cancelled(id);
                false
            }
            Some(Phase::Sending) => true,
            _ => false,
        }
    }

    /// Drops the transfers that are over (sent, failed or cancelled).
    pub fn clear_finished(&mut self) {
        self.transfers.retain(|t| !t.phase.finished());
    }

    /// True when at least one transfer is over and could be cleared.
    pub fn any_finished(&self) -> bool {
        self.transfers.iter().any(|t| t.phase.finished())
    }

    /// Replaces the two file lists with a listing from the worker.
    pub fn set_listing(&mut self, outbox: Vec<FileEntry>, inbox: Vec<FileEntry>) {
        self.outbox = outbox;
        self.inbox = inbox;
    }

    fn find_mut(&mut self, id: u64) -> Option<&mut Transfer> {
        self.transfers.iter_mut().find(|t| t.id == id)
    }
}

/// A folder listing as the window shows it: regular files only, hidden names left out, newest first (ties by name).
pub fn listing(mut files: Vec<FileEntry>) -> Vec<FileEntry> {
    files.retain(|f| !f.name.starts_with('.'));
    files.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| a.name.cmp(&b.name)));
    files
}

/// A size in bytes as people read it: KB, MB or GB.
pub fn size_text(bytes: u64) -> String {
    let kb = bytes as f64 / 1024.0;
    if kb < 1024.0 {
        format!("{:.0} KB", kb.max(1.0))
    } else if kb < 1024.0 * 1024.0 {
        format!("{:.1} MB", kb / 1024.0)
    } else {
        format!("{:.2} GB", kb / (1024.0 * 1024.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, size: u64, modified: i64) -> FileEntry {
        FileEntry { name: name.into(), path: PathBuf::from(format!("/home/u/Downloads/{name}")), size, modified }
    }

    fn device() -> BdAddr {
        "AA:BB:CC:DD:EE:FF".parse().unwrap()
    }

    #[test]
    fn statuses_read_as_bluez_writes_them() {
        assert_eq!(parse_status("queued"), Status::Queued);
        assert_eq!(parse_status("active"), Status::Active);
        assert_eq!(parse_status("suspended"), Status::Active);
        assert_eq!(parse_status("complete"), Status::Complete);
        assert_eq!(parse_status("error"), Status::Error);
        assert_eq!(parse_status("unknown"), Status::Other);
    }

    #[test]
    fn the_folders_sit_under_downloads() {
        let home = Path::new("/home/u");
        assert_eq!(outbox_dir(home), PathBuf::from("/home/u/Downloads"));
        assert_eq!(inbox_dir(home), PathBuf::from("/home/u/Downloads/Bluetooth"));
    }

    #[test]
    fn transfers_go_one_at_a_time_in_queue_order() {
        let mut m = Model::default();
        let a = m.queue(file("a.pdf", 10, 0), device(), "Speaker".into());
        let b = m.queue(file("b.pdf", 10, 0), device(), "Speaker".into());
        assert_eq!(m.next_waiting(), Some(a));
        m.start(a);
        assert!(m.busy());
        assert_eq!(m.next_waiting(), None, "nothing else starts while one is sending");
        m.finish(a, Ok(()));
        assert!(!m.busy());
        assert_eq!(m.next_waiting(), Some(b));
    }

    #[test]
    fn a_finished_transfer_is_complete_and_keeps_its_state() {
        let mut m = Model::default();
        let id = m.queue(file("a.jpg", 4096, 0), device(), "Speaker".into());
        m.start(id);
        m.progress(id, 1024);
        assert!((m.transfer(id).unwrap().fraction() - 0.25).abs() < 1e-6);
        m.finish(id, Ok(()));
        let t = m.transfer(id).unwrap();
        assert_eq!(t.phase, Phase::Sent);
        assert_eq!(t.sent, 4096);
        assert_eq!(t.fraction(), 1.0);
        m.cancelled(id);
        m.finish(id, Err("late".into()));
        assert_eq!(m.transfer(id).unwrap().phase, Phase::Sent, "a sent transfer is not changed later");
    }

    #[test]
    fn progress_is_capped_at_the_file_size_and_ignored_when_not_sending() {
        let mut m = Model::default();
        let id = m.queue(file("a.bin", 100, 0), device(), "Speaker".into());
        m.progress(id, 50);
        assert_eq!(m.transfer(id).unwrap().sent, 0, "a waiting transfer takes no progress");
        m.start(id);
        m.progress(id, 500);
        assert_eq!(m.transfer(id).unwrap().sent, 100);
    }

    #[test]
    fn an_empty_file_has_no_fraction_until_it_is_sent() {
        let mut m = Model::default();
        let id = m.queue(file("empty.txt", 0, 0), device(), "Speaker".into());
        m.start(id);
        assert_eq!(m.transfer(id).unwrap().fraction(), 0.0);
        m.finish(id, Ok(()));
        assert_eq!(m.transfer(id).unwrap().fraction(), 1.0);
    }

    #[test]
    fn cancelling_a_waiting_transfer_is_local_and_a_sending_one_is_passed_on() {
        let mut m = Model::default();
        let a = m.queue(file("a", 1, 0), device(), "Speaker".into());
        let b = m.queue(file("b", 1, 0), device(), "Speaker".into());
        m.start(a);
        assert!(!m.request_cancel(b), "a waiting transfer is cancelled without the worker");
        assert_eq!(m.transfer(b).unwrap().phase, Phase::Cancelled);
        assert!(m.request_cancel(a), "the sending transfer must be stopped by the worker");
        m.cancelled(a);
        assert_eq!(m.transfer(a).unwrap().phase, Phase::Cancelled);
    }

    #[test]
    fn a_failure_keeps_its_reason_for_the_list() {
        let mut m = Model::default();
        let id = m.queue(file("a", 1, 0), device(), "Speaker".into());
        m.start(id);
        m.finish(id, Err("No session bus".into()));
        assert_eq!(m.transfer(id).unwrap().status_text(), "No session bus");
    }

    #[test]
    fn clear_finished_keeps_what_is_still_going() {
        let mut m = Model::default();
        let a = m.queue(file("a", 1, 0), device(), "Speaker".into());
        let b = m.queue(file("b", 1, 0), device(), "Speaker".into());
        m.start(a);
        m.finish(a, Ok(()));
        assert!(m.any_finished());
        m.clear_finished();
        assert!(m.transfer(a).is_none());
        assert_eq!(m.transfer(b).unwrap().phase, Phase::Waiting);
        assert!(!m.any_finished());
    }

    #[test]
    fn a_listing_drops_hidden_names_and_sorts_newest_first() {
        let files = vec![file(".cache", 1, 900), file("old.txt", 1, 100), file("b.txt", 1, 500), file("a.txt", 1, 500)];
        let names: Vec<String> = listing(files).into_iter().map(|f| f.name).collect();
        assert_eq!(names, vec!["a.txt", "b.txt", "old.txt"]);
    }

    #[test]
    fn sizes_read_in_kilobytes_megabytes_and_gigabytes() {
        assert_eq!(size_text(0), "1 KB");
        assert_eq!(size_text(2048), "2 KB");
        assert_eq!(size_text(3 * 1024 * 1024 / 2), "1.5 MB");
        assert_eq!(size_text(5 * 1024 * 1024 * 1024), "5.00 GB");
    }
}
