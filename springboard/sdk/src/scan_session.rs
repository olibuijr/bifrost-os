//! Scanning for apps (the TWAIN source session): open a scanner, configure it, scan pages into files.
//!
//! [`ScanSession`] wraps one SANE device (`scanimage`) in the scan job model of `bifrost_ui::scan_job`. The same gate
//! decides whether a scan may start (one job at a time, a functional unit selected, a session on a protected scanner),
//! and `bifrost_ui::scanner` carries the job out. Each page is written to a file in the session's directory, named
//! `Page NNN.<ext>` in scan order. A scan blocks until the device finishes, so call [`ScanSession::scan_page`] from a
//! worker thread, not the UI thread. Limits: one flatbed unit per device, no memory transfer, and no PDF output.

use bifrost_ui::scan_job::{DocumentFormat, JobKind, ScanError, ScanSettings, ScannerDevice};
use bifrost_ui::scanner::{self, CommandError};
use std::fmt;
use std::path::{Path, PathBuf};

/// Why a session call failed: the job gate refused it, or the scanner command ran and failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionError {
    Scan(ScanError),
    Command(CommandError),
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SessionError::Scan(e) => write!(f, "The scanner refused the request: {e:?}"),
            SessionError::Command(e) => write!(f, "{e}"),
        }
    }
}

/// The file a page is written to: its position in the session (from 1) and the document format's extension.
pub fn page_file_name(index: usize, format: DocumentFormat) -> String {
    format!("Page {index:03}.{}", format.extension())
}

/// A scanner session: the device, its job state, and the page files scanned so far.
#[derive(Debug)]
pub struct ScanSession {
    device: String,
    scanner: ScannerDevice,
    dir: PathBuf,
    pages: Vec<PathBuf>,
}

impl ScanSession {
    /// Opens a session on the SANE device `device`, reading what it can do from its `--help`. Pages go to `dir`.
    pub fn open(device: &str, dir: &Path) -> Self {
        let unit = scanner::capabilities_of(device).unit();
        Self::with_device(device, ScannerDevice::new(vec![unit], false, None), dir)
    }

    /// A session over a scanner the caller has already described (its functional units and protection).
    pub fn with_device(device: &str, scanner: ScannerDevice, dir: &Path) -> Self {
        ScanSession { device: device.to_string(), scanner, dir: dir.to_path_buf(), pages: Vec::new() }
    }

    /// The SANE device name this session talks to.
    pub fn device(&self) -> &str {
        &self.device
    }

    /// The settings the next scan uses.
    pub fn settings(&self) -> &ScanSettings {
        self.scanner.settings()
    }

    /// Replaces the settings the next scan uses. Refused while a job runs.
    pub fn configure(&mut self, settings: ScanSettings) -> Result<(), SessionError> {
        self.scanner.configure(settings).map_err(SessionError::Scan)
    }

    /// The job that is running, if any. A session scans one job at a time, so this is `Some` only inside a call.
    pub fn job(&self) -> Option<JobKind> {
        self.scanner.job()
    }

    /// The page files scanned by this session, in scan order.
    pub fn pages(&self) -> &[PathBuf] {
        &self.pages
    }

    /// Scans one page at the configured settings and returns the file it was written to.
    pub fn scan_page(&mut self) -> Result<PathBuf, SessionError> {
        self.scanner.request_scan().map_err(SessionError::Scan)?;
        let path = self.dir.join(page_file_name(self.pages.len() + 1, self.scanner.settings().format));
        let result = scanner::scan_to(&self.device, self.scanner.settings(), false, &path);
        self.scanner.finish_job();
        result.map_err(SessionError::Command)?;
        self.pages.push(path.clone());
        Ok(path)
    }

    /// Makes a quick low-resolution preview of the selected unit and returns its file. It is not a page of the session.
    pub fn preview(&mut self) -> Result<PathBuf, SessionError> {
        self.scanner.request_overview().map_err(SessionError::Scan)?;
        let path = self.dir.join("Preview.png");
        let result = scanner::scan_to(&self.device, self.scanner.settings(), true, &path);
        self.scanner.finish_job();
        result.map(|_| path).map_err(SessionError::Command)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_ui::scan_job::{FunctionalUnit, FunctionalUnitType};

    fn flatbed(can_overview: bool, protected: bool) -> ScanSession {
        let unit = FunctionalUnit { kind: FunctionalUnitType::Flatbed, resolutions: vec![150, 300, 600], can_overview };
        let scanner = ScannerDevice::new(vec![unit], protected, None);
        ScanSession::with_device("test:device", scanner, Path::new("/nonexistent/scan-session-test"))
    }

    #[test]
    fn page_files_are_numbered_in_scan_order_with_the_format_extension() {
        assert_eq!(page_file_name(1, DocumentFormat::Png), "Page 001.png");
        assert_eq!(page_file_name(12, DocumentFormat::Jpeg), "Page 012.jpg");
    }

    #[test]
    fn a_scan_needs_a_unit_selected_before_any_command_runs() {
        let mut s = flatbed(true, false);
        assert_eq!(s.scan_page(), Err(SessionError::Scan(ScanError::NoUnitSelected)));
        assert!(s.pages().is_empty());
        assert_eq!(s.job(), None);
    }

    #[test]
    fn a_preview_is_refused_when_the_unit_cannot_make_one() {
        let mut s = flatbed(false, false);
        s.scanner.select(FunctionalUnitType::Flatbed).unwrap();
        assert_eq!(s.preview(), Err(SessionError::Scan(ScanError::OverviewUnsupported)));
    }

    #[test]
    fn a_protected_scanner_needs_a_session_first() {
        let mut s = flatbed(true, true);
        s.scanner.select(FunctionalUnitType::Flatbed).unwrap();
        assert_eq!(s.scan_page(), Err(SessionError::Scan(ScanError::SessionRequired)));
    }

    #[test]
    fn a_resolution_the_unit_does_not_take_is_refused_by_configure() {
        let mut s = flatbed(true, false);
        s.scanner.select(FunctionalUnitType::Flatbed).unwrap();
        let mut settings = s.settings().clone();
        settings.resolution_dpi = 1200;
        assert_eq!(s.configure(settings), Err(SessionError::Scan(ScanError::UnsupportedResolution(1200))));
        assert!(s.pages().is_empty());
    }
}
