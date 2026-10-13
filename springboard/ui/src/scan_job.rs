//! Scanner device model: the functional units of a flatbed or document scanner, the scan settings, the
//! session gate for protected scanners, and the state of one overview or full scan. This is pure: the
//! transport that talks to the scanner reports when a job ends with `finish_job`, and the scan panel
//! config holds what the scan window shows. Opening the session checks that credentials were given; the
//! scanner itself decides whether they are right.

/// The part of a scanner that scans: glass, feeder or a transparency adapter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FunctionalUnitType {
    Flatbed,
    DocumentFeeder,
    PositiveTransparency,
    NegativeTransparency,
}

/// One functional unit and what it can do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FunctionalUnit {
    pub kind: FunctionalUnitType,
    /// Resolutions in dots per inch that the unit accepts.
    pub resolutions: Vec<u32>,
    /// Whether the unit can make a quick low-resolution preview scan.
    pub can_overview: bool,
}

/// How scan data reaches the app: as a file written to disk, or as data in memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferMode {
    FileBased,
    MemoryBased,
}

/// The format of the scanned document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DocumentFormat {
    Jpeg,
    Png,
    Tiff,
    Pdf,
}

impl DocumentFormat {
    pub fn mime_type(self) -> &'static str {
        match self {
            DocumentFormat::Jpeg => "image/jpeg",
            DocumentFormat::Png => "image/png",
            DocumentFormat::Tiff => "image/tiff",
            DocumentFormat::Pdf => "application/pdf",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            DocumentFormat::Jpeg => "jpg",
            DocumentFormat::Png => "png",
            DocumentFormat::Tiff => "tiff",
            DocumentFormat::Pdf => "pdf",
        }
    }

    /// The name shown for the format in a scan window.
    pub fn label(self) -> &'static str {
        match self {
            DocumentFormat::Jpeg => "JPEG",
            DocumentFormat::Png => "PNG",
            DocumentFormat::Tiff => "TIFF",
            DocumentFormat::Pdf => "PDF",
        }
    }

    /// The next format in the order JPEG, PNG, TIFF, PDF, then JPEG again.
    pub fn next(self) -> DocumentFormat {
        match self {
            DocumentFormat::Jpeg => DocumentFormat::Png,
            DocumentFormat::Png => DocumentFormat::Tiff,
            DocumentFormat::Tiff => DocumentFormat::Pdf,
            DocumentFormat::Pdf => DocumentFormat::Jpeg,
        }
    }
}

/// The colour modes a scan can take.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorMode {
    Color,
    Gray,
    Lineart,
}

impl ColorMode {
    pub const ALL: [ColorMode; 3] = [ColorMode::Color, ColorMode::Gray, ColorMode::Lineart];

    /// The name shown in a scan window.
    pub fn label(self) -> &'static str {
        match self {
            ColorMode::Color => "Colour",
            ColorMode::Gray => "Greyscale",
            ColorMode::Lineart => "Black & White",
        }
    }

    /// The next mode in the order Colour, Greyscale, Black & White, then Colour again.
    pub fn next(self) -> ColorMode {
        match self {
            ColorMode::Color => ColorMode::Gray,
            ColorMode::Gray => ColorMode::Lineart,
            ColorMode::Lineart => ColorMode::Color,
        }
    }
}

/// The two jobs a scanner runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobKind {
    Overview,
    Scan,
}

/// Why a scanner request was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScanError {
    /// The scanner has no functional unit of that kind.
    UnitUnavailable,
    /// A scan needs a functional unit to be selected first.
    NoUnitSelected,
    /// A job is already running.
    Busy,
    /// The scanner is protected and no session is open.
    SessionRequired,
    /// A protected scanner needs a username and a password to open a session.
    CredentialsRequired,
    /// The selected unit does not accept this resolution (dots per inch).
    UnsupportedResolution(u32),
    /// The selected unit cannot make an overview scan.
    OverviewUnsupported,
    /// A setting is empty or unusable.
    InvalidSetting,
}

/// Settings that apply to the next scan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScanSettings {
    pub resolution_dpi: u32,
    pub color: ColorMode,
    pub transfer_mode: TransferMode,
    pub format: DocumentFormat,
    pub document_name: String,
    pub downloads_directory: String,
    /// The largest band of memory a memory-based transfer may use, in bytes. Zero means no limit.
    pub max_memory_band_size: u64,
}

impl Default for ScanSettings {
    fn default() -> Self {
        ScanSettings {
            resolution_dpi: 300,
            color: ColorMode::Color,
            transfer_mode: TransferMode::FileBased,
            format: DocumentFormat::Jpeg,
            document_name: "Scan".to_string(),
            downloads_directory: "Downloads".to_string(),
            max_memory_band_size: 0,
        }
    }
}

/// The resolution in `available` closest to `wanted`. Ties go to the lower one.
fn nearest_resolution(available: &[u32], wanted: u32) -> Option<u32> {
    let mut best: Option<u32> = None;
    for &r in available {
        let better = match best {
            None => true,
            Some(b) => {
                let (dr, db) = (r.abs_diff(wanted), b.abs_diff(wanted));
                dr < db || (dr == db && r < b)
            }
        };
        if better {
            best = Some(r);
        }
    }
    best
}

/// A scanner: its units, whether it is protected, and the job that is running.
#[derive(Debug)]
pub struct ScannerDevice {
    units: Vec<FunctionalUnit>,
    selected: Option<usize>,
    protected: bool,
    default_username: Option<String>,
    session_open: bool,
    job: Option<JobKind>,
    settings: ScanSettings,
}

impl ScannerDevice {
    /// A scanner with these units and nothing selected. `default_username` is only offered for a protected scanner.
    pub fn new(units: Vec<FunctionalUnit>, protected: bool, default_username: Option<String>) -> Self {
        ScannerDevice {
            units,
            selected: None,
            protected,
            default_username,
            session_open: false,
            job: None,
            settings: ScanSettings::default(),
        }
    }

    pub fn units(&self) -> &[FunctionalUnit] {
        &self.units
    }

    pub fn selected_unit(&self) -> Option<&FunctionalUnit> {
        self.selected.and_then(|i| self.units.get(i))
    }

    /// Selects the unit of `kind`. The resolution moves to the nearest value the unit accepts when the
    /// current one is not on its list.
    pub fn select(&mut self, kind: FunctionalUnitType) -> Result<(), ScanError> {
        if self.job.is_some() {
            return Err(ScanError::Busy);
        }
        let index = self.units.iter().position(|u| u.kind == kind).ok_or(ScanError::UnitUnavailable)?;
        self.selected = Some(index);
        let unit_resolutions = self.units[index].resolutions.clone();
        if !unit_resolutions.contains(&self.settings.resolution_dpi) {
            if let Some(r) = nearest_resolution(&unit_resolutions, 300) {
                self.settings.resolution_dpi = r;
            }
        }
        Ok(())
    }

    pub fn is_protected(&self) -> bool {
        self.protected
    }

    /// The username to offer on the login sheet. Only a protected scanner has one.
    pub fn default_username(&self) -> Option<&str> {
        if self.protected {
            self.default_username.as_deref()
        } else {
            None
        }
    }

    pub fn session_open(&self) -> bool {
        self.session_open
    }

    /// Opens a session on a protected scanner with a username and password. An unprotected scanner
    /// needs no session, so the call succeeds and does nothing. The password is checked only for being
    /// present here, and is not kept.
    pub fn open_session(&mut self, credentials: Option<(&str, &str)>) -> Result<(), ScanError> {
        if !self.protected {
            return Ok(());
        }
        match credentials {
            Some((user, pass)) if !user.trim().is_empty() && !pass.is_empty() => {
                self.session_open = true;
                Ok(())
            }
            _ => Err(ScanError::CredentialsRequired),
        }
    }

    /// Closes the session. Refused while a job runs.
    pub fn close_session(&mut self) -> Result<(), ScanError> {
        if self.job.is_some() {
            return Err(ScanError::Busy);
        }
        self.session_open = false;
        Ok(())
    }

    pub fn settings(&self) -> &ScanSettings {
        &self.settings
    }

    /// Replaces the settings for the next scan. Refused while a job runs, for an empty document name or
    /// download folder, or for a resolution the selected unit does not accept. A refused change leaves the
    /// settings as they were.
    pub fn configure(&mut self, settings: ScanSettings) -> Result<(), ScanError> {
        if self.job.is_some() {
            return Err(ScanError::Busy);
        }
        let document_name = settings.document_name.trim().to_string();
        if document_name.is_empty() || settings.downloads_directory.trim().is_empty() {
            return Err(ScanError::InvalidSetting);
        }
        if let Some(unit) = self.selected_unit() {
            if !unit.resolutions.contains(&settings.resolution_dpi) {
                return Err(ScanError::UnsupportedResolution(settings.resolution_dpi));
            }
        }
        self.settings = ScanSettings { document_name, ..settings };
        Ok(())
    }

    /// The running job, if any.
    pub fn job(&self) -> Option<JobKind> {
        self.job
    }

    /// The checks every job needs: nothing running, a unit selected, and a session on a protected scanner.
    fn gate(&self) -> Result<usize, ScanError> {
        if self.job.is_some() {
            return Err(ScanError::Busy);
        }
        let index = self.selected.ok_or(ScanError::NoUnitSelected)?;
        if self.protected && !self.session_open {
            return Err(ScanError::SessionRequired);
        }
        Ok(index)
    }

    /// Starts an overview (preview) scan on the selected unit.
    pub fn request_overview(&mut self) -> Result<(), ScanError> {
        let index = self.gate()?;
        if !self.units[index].can_overview {
            return Err(ScanError::OverviewUnsupported);
        }
        self.job = Some(JobKind::Overview);
        Ok(())
    }

    /// Starts a full scan at the configured resolution on the selected unit.
    pub fn request_scan(&mut self) -> Result<(), ScanError> {
        let index = self.gate()?;
        if !self.units[index].resolutions.contains(&self.settings.resolution_dpi) {
            return Err(ScanError::UnsupportedResolution(self.settings.resolution_dpi));
        }
        self.job = Some(JobKind::Scan);
        Ok(())
    }

    /// Cancels the running job. Returns false when nothing was running.
    pub fn cancel_scan(&mut self) -> bool {
        self.job.take().is_some()
    }

    /// Called by the transport when the running job ends. Returns the job that ended, or None when
    /// nothing was running.
    pub fn finish_job(&mut self) -> Option<JobKind> {
        self.job.take()
    }
}

/// How the scan window presents its controls.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayMode {
    Simple,
    Advanced,
}

/// What the scan window shows: the display mode, the button labels, the downloads folder control and the
/// post-processing app. Every label has a default, so an empty label never leaves a button blank.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScanPanel {
    pub mode: DisplayMode,
    pub overview_label: String,
    pub scan_label: String,
    pub shows_downloads_control: bool,
    pub shows_post_process_control: bool,
    pub post_process_app: Option<String>,
}

impl Default for ScanPanel {
    fn default() -> Self {
        ScanPanel {
            mode: DisplayMode::Simple,
            overview_label: "Overview".to_string(),
            scan_label: "Scan".to_string(),
            shows_downloads_control: true,
            shows_post_process_control: false,
            post_process_app: None,
        }
    }
}

impl ScanPanel {
    /// Sets the button labels. A blank label keeps the default.
    pub fn set_labels(&mut self, overview: &str, scan: &str) {
        let overview = overview.trim();
        if !overview.is_empty() {
            self.overview_label = overview.to_string();
        }
        let scan = scan.trim();
        if !scan.is_empty() {
            self.scan_label = scan.to_string();
        }
    }

    /// Sets the post-processing app. A blank name clears it and hides the control.
    pub fn set_post_process_app(&mut self, app: &str) {
        let app = app.trim();
        if app.is_empty() {
            self.post_process_app = None;
            self.shows_post_process_control = false;
        } else {
            self.post_process_app = Some(app.to_string());
            self.shows_post_process_control = true;
        }
    }

    /// The advanced controls appear only in advanced mode.
    pub fn shows_advanced_controls(&self) -> bool {
        self.mode == DisplayMode::Advanced
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flatbed() -> FunctionalUnit {
        FunctionalUnit { kind: FunctionalUnitType::Flatbed, resolutions: vec![75, 150, 300, 600], can_overview: true }
    }

    fn feeder() -> FunctionalUnit {
        FunctionalUnit { kind: FunctionalUnitType::DocumentFeeder, resolutions: vec![200, 400], can_overview: false }
    }

    fn scanner(protected: bool) -> ScannerDevice {
        ScannerDevice::new(vec![flatbed(), feeder()], protected, Some("owner".to_string()))
    }

    #[test]
    fn a_scan_needs_a_selected_unit_and_the_selection_must_exist() {
        let mut s = scanner(false);
        assert_eq!(s.request_scan(), Err(ScanError::NoUnitSelected));
        assert_eq!(s.select(FunctionalUnitType::PositiveTransparency), Err(ScanError::UnitUnavailable));
        assert!(s.select(FunctionalUnitType::Flatbed).is_ok());
        assert_eq!(s.selected_unit().unwrap().kind, FunctionalUnitType::Flatbed);
    }

    #[test]
    fn a_protected_scanner_refuses_jobs_until_a_session_is_open() {
        let mut s = scanner(true);
        s.select(FunctionalUnitType::Flatbed).unwrap();
        assert_eq!(s.request_scan(), Err(ScanError::SessionRequired));
        assert_eq!(s.open_session(None), Err(ScanError::CredentialsRequired));
        assert_eq!(s.open_session(Some(("  ", "secret"))), Err(ScanError::CredentialsRequired));
        assert_eq!(s.open_session(Some(("owner", ""))), Err(ScanError::CredentialsRequired));
        assert!(s.open_session(Some(("owner", "secret"))).is_ok());
        assert!(s.session_open());
        assert!(s.request_scan().is_ok());
        assert_eq!(s.default_username(), Some("owner"));
    }

    #[test]
    fn an_unprotected_scanner_needs_no_session_and_offers_no_username() {
        let mut s = scanner(false);
        s.select(FunctionalUnitType::Flatbed).unwrap();
        assert!(s.request_overview().is_ok());
        assert!(s.default_username().is_none(), "the username is only for protected scanners");
        assert!(s.open_session(None).is_ok(), "no credentials are needed when nothing is protected");
    }

    #[test]
    fn only_one_job_runs_at_a_time_and_overview_needs_unit_support() {
        let mut s = scanner(false);
        s.select(FunctionalUnitType::DocumentFeeder).unwrap();
        assert_eq!(s.request_overview(), Err(ScanError::OverviewUnsupported));
        s.select(FunctionalUnitType::Flatbed).unwrap();
        assert!(s.request_overview().is_ok());
        assert_eq!(s.job(), Some(JobKind::Overview));
        assert_eq!(s.request_scan(), Err(ScanError::Busy));
        assert_eq!(s.select(FunctionalUnitType::DocumentFeeder), Err(ScanError::Busy));
        assert!(s.cancel_scan());
        assert!(!s.cancel_scan(), "nothing is left to cancel");
        assert!(s.request_scan().is_ok());
    }

    #[test]
    fn configure_checks_the_resolution_and_leaves_settings_alone_when_refused() {
        let mut s = scanner(false);
        s.select(FunctionalUnitType::Flatbed).unwrap();
        let before = s.settings().clone();
        let mut bad = before.clone();
        bad.resolution_dpi = 1234;
        assert_eq!(s.configure(bad), Err(ScanError::UnsupportedResolution(1234)));
        assert_eq!(s.settings(), &before);
        let mut good = before.clone();
        good.resolution_dpi = 600;
        good.document_name = "  Invoice  ".to_string();
        good.format = DocumentFormat::Pdf;
        assert!(s.configure(good).is_ok());
        assert_eq!(s.settings().document_name, "Invoice", "the name is trimmed");
        assert_eq!(s.settings().format.extension(), "pdf");
        assert_eq!(s.settings().format.mime_type(), "application/pdf");
    }

    #[test]
    fn a_blank_name_is_refused_and_settings_cannot_change_mid_job() {
        let mut s = scanner(false);
        s.select(FunctionalUnitType::Flatbed).unwrap();
        let mut blank = ScanSettings::default();
        blank.document_name = "   ".to_string();
        assert_eq!(s.configure(blank), Err(ScanError::InvalidSetting));
        s.request_scan().unwrap();
        assert_eq!(s.configure(ScanSettings::default()), Err(ScanError::Busy));
        assert_eq!(s.finish_job(), Some(JobKind::Scan));
        assert_eq!(s.finish_job(), None, "a finished job is gone");
        assert!(s.configure(ScanSettings::default()).is_ok());
    }

    #[test]
    fn the_resolution_follows_the_selected_unit_to_the_nearest_value() {
        let mut s = scanner(false);
        s.select(FunctionalUnitType::Flatbed).unwrap();
        assert_eq!(s.settings().resolution_dpi, 300);
        // The feeder accepts only 200 and 400: 300 is a tie, and the lower value wins.
        s.select(FunctionalUnitType::DocumentFeeder).unwrap();
        assert_eq!(s.settings().resolution_dpi, 200);
    }

    #[test]
    fn a_scan_panel_keeps_a_default_label_when_given_a_blank_one() {
        let mut panel = ScanPanel::default();
        panel.set_labels("  Preview ", "   ");
        assert_eq!(panel.overview_label, "Preview");
        assert_eq!(panel.scan_label, "Scan");
        assert!(!panel.shows_advanced_controls());
        panel.mode = DisplayMode::Advanced;
        assert!(panel.shows_advanced_controls());
        panel.set_post_process_app("Editor");
        assert!(panel.shows_post_process_control);
        panel.set_post_process_app("  ");
        assert!(panel.post_process_app.is_none() && !panel.shows_post_process_control);
    }
}
