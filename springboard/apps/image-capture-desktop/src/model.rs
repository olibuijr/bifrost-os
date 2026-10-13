//! Image Capture's state and rules: the sources in the sidebar (cameras and SD cards, scanners), the pictures of the
//! selected camera and which of them are picked, the scanner shown with its settings and job (`scan_job`), and the
//! folders the work goes to. Pure: `worker.rs` touches the disk and the scanner command, `window.rs` lays out and draws.

use bifrost_ui::camera_device::{CameraContents, EntryKind, Media};
use bifrost_ui::device_browser::{Device, DeviceKind};
use bifrost_ui::scan_job::{ColorMode, DocumentFormat, FunctionalUnitType, ScanError, ScanSettings, ScannerDevice};
use bifrost_ui::scanner::{self, Capabilities, ScannerInfo};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// The id prefix of a scanner in the sidebar (the device name follows).
pub const SCANNER_PREFIX: &str = "scanner:";

/// One row of the sidebar: a camera or SD card (`id` is the volume's id) or a scanner (`id` carries its device name).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    pub id: String,
    pub name: String,
    pub kind: DeviceKind,
}

impl Source {
    /// The SANE device name of a scanner source.
    pub fn scanner_device(&self) -> Option<&str> {
        match self.kind {
            DeviceKind::Scanner => self.id.strip_prefix(SCANNER_PREFIX),
            DeviceKind::Camera => None,
        }
    }
}

/// A picture on a camera, with the path to copy it from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CameraFile {
    pub path: PathBuf,
    pub name: String,
    pub size: u64,
}

/// What the window asks the worker to do for a person's action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Look at the cameras and scanners again.
    Refresh,
    /// Copy the picked pictures (or all of them) into Pictures.
    Import { all: bool },
    /// Scan a quick preview of the selected scanner.
    Preview,
    /// Scan the selected scanner into the scan folder.
    Scan,
}

/// What the worker sends back.
#[derive(Clone, Debug, PartialEq)]
pub enum Reply {
    /// The cameras and SD cards with a DCIM folder, and the scanners (or why they could not be listed).
    Listed { volumes: Vec<Device>, scanners: Result<Vec<ScannerInfo>, String> },
    /// The pictures of the camera `volume`.
    Contents { volume: String, files: Vec<CameraFile> },
    /// What the scanner `device` can do.
    Capabilities { device: String, caps: Capabilities },
    /// This many pictures were copied.
    Imported(usize),
    /// A scan or preview is in this file.
    Scanned { preview: bool, path: PathBuf },
    /// The scan could not be made; the text is shown in the footer.
    ScanFailed(String),
    /// Another request failed; the text is shown in the footer.
    Failed(String),
}

/// The window's data.
#[derive(Default, Debug)]
pub struct Model {
    pub sources: Vec<Source>,
    /// index into `sources` of the one shown
    pub selected: Option<usize>,
    pub files: Vec<CameraFile>,
    /// indices into `files` that are picked for import
    pub picked: BTreeSet<usize>,
    /// the scanner shown: its settings and the job it runs (None until its capabilities are read)
    pub scanner: Option<ScannerDevice>,
    /// the colour modes the scanner offers
    pub modes: Vec<ColorMode>,
    /// the last preview file, shown above the scanner settings
    pub preview: Option<PathBuf>,
    /// the last result, shown in the footer
    pub status: String,
}

impl Model {
    /// Takes a reply. A listing keeps the selection when the source is still there; the pictures are kept only for
    /// the camera that is shown now.
    pub fn apply(&mut self, reply: Reply, volume_of_selection: Option<&str>) {
        match reply {
            Reply::Listed { volumes, scanners } => {
                let mut sources: Vec<Source> = volumes.into_iter().map(|d| Source { id: d.id, name: d.name, kind: d.kind }).collect();
                match scanners {
                    Ok(list) => sources.extend(scanner::device_entries(&list).into_iter().map(|d| Source { id: d.id, name: d.name, kind: d.kind })),
                    Err(e) => self.status = e,
                }
                let keep = self.selected.and_then(|i| self.sources.get(i)).map(|s| s.id.clone());
                self.selected = keep.and_then(|id| sources.iter().position(|s| s.id == id));
                self.sources = sources;
                if self.selected.is_none() {
                    self.clear_selection_state();
                }
            }
            Reply::Contents { volume, files } => {
                if volume_of_selection == Some(volume.as_str()) {
                    self.files = files;
                    self.picked.retain(|&i| i < self.files.len());
                    self.status = format!("{} pictures", self.files.len());
                }
            }
            Reply::Capabilities { device, caps } => {
                if self.selected_scanner() == Some(device.as_str()) {
                    let mut scanner = ScannerDevice::new(vec![caps.unit()], false, None);
                    let _ = scanner.select(FunctionalUnitType::Flatbed);
                    self.modes = caps.modes;
                    self.scanner = Some(scanner);
                }
            }
            Reply::Imported(n) => {
                self.status = match n {
                    0 => "No pictures were imported".into(),
                    1 => "Imported 1 picture into Pictures".into(),
                    n => format!("Imported {n} pictures into Pictures"),
                };
            }
            Reply::Scanned { preview, path } => {
                self.finish_job();
                self.status = if preview {
                    self.preview = Some(path);
                    "Preview ready".into()
                } else {
                    format!("Saved {}", path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())
                };
            }
            Reply::ScanFailed(text) => {
                self.finish_job();
                self.status = text;
            }
            Reply::Failed(text) => self.status = text,
        }
    }

    fn finish_job(&mut self) {
        if let Some(scanner) = self.scanner.as_mut() {
            scanner.finish_job();
        }
    }

    /// Shows the source at `index`. Its pictures or capabilities are read again by the window.
    pub fn select(&mut self, index: usize) {
        if index >= self.sources.len() {
            return;
        }
        self.selected = Some(index);
        self.clear_selection_state();
    }

    /// Forgets what the selection held: the pictures, the scanner and its preview.
    fn clear_selection_state(&mut self) {
        self.files.clear();
        self.picked.clear();
        self.scanner = None;
        self.modes.clear();
        self.preview = None;
    }

    pub fn selected_source(&self) -> Option<&Source> {
        self.sources.get(self.selected?)
    }

    /// The id of the camera shown, if the selection is a camera.
    pub fn selected_volume(&self) -> Option<&str> {
        self.selected_source().filter(|s| s.kind == DeviceKind::Camera).map(|s| s.id.as_str())
    }

    /// The device name of the scanner shown, if the selection is a scanner.
    pub fn selected_scanner(&self) -> Option<&str> {
        self.selected_source()?.scanner_device()
    }

    /// Whether a scan or preview may start now: the scanner is read and no job is running.
    pub fn scanner_idle(&self) -> bool {
        self.scanner.as_ref().is_some_and(|s| s.job().is_none())
    }

    /// The settings of the scanner shown (the defaults until it is read).
    pub fn settings(&self) -> ScanSettings {
        self.scanner.as_ref().map(|s| s.settings().clone()).unwrap_or_default()
    }

    /// Picks or un-picks the picture at `i`.
    pub fn toggle_pick(&mut self, i: usize) {
        if i < self.files.len() && !self.picked.remove(&i) {
            self.picked.insert(i);
        }
    }

    /// The pictures an import copies: the picked ones, or every one when `all`.
    pub fn import_paths(&self, all: bool) -> Vec<PathBuf> {
        self.files
            .iter()
            .enumerate()
            .filter(|(i, _)| all || self.picked.contains(i))
            .map(|(_, f)| f.path.clone())
            .collect()
    }

    /// Replaces one setting of the scanner, leaving the rest. A change the scanner refuses (a job is running) is
    /// dropped.
    fn change(&mut self, edit: impl FnOnce(&mut ScanSettings)) {
        if let Some(scanner) = self.scanner.as_mut() {
            let mut next = scanner.settings().clone();
            edit(&mut next);
            let _ = scanner.configure(next);
        }
    }

    /// The next resolution the scanner offers; past the highest one it starts again at the lowest.
    pub fn cycle_dpi(&mut self) {
        let mut list: Vec<u32> = self.scanner.as_ref().and_then(|s| s.selected_unit()).map(|u| u.resolutions.clone()).unwrap_or_default();
        list.sort_unstable();
        let current = self.settings().resolution_dpi;
        let next = list.iter().copied().find(|&r| r > current).or(list.first().copied());
        if let Some(next) = next {
            self.change(|s| s.resolution_dpi = next);
        }
    }

    /// The next colour mode the scanner offers.
    pub fn cycle_mode(&mut self) {
        let mut mode = self.settings().color.next();
        for _ in 0..ColorMode::ALL.len() {
            if self.modes.contains(&mode) {
                break;
            }
            mode = mode.next();
        }
        if self.modes.contains(&mode) {
            self.change(|s| s.color = mode);
        }
    }

    /// The next file format. PDF is left out: the scan command writes images.
    pub fn cycle_format(&mut self) {
        let mut format = self.settings().format.next();
        if format == DocumentFormat::Pdf {
            format = format.next();
        }
        self.change(|s| s.format = format);
    }
}

/// The words for a refusal of a scan request.
pub fn refusal(e: ScanError) -> &'static str {
    match e {
        ScanError::UnitUnavailable => "This scanner has no flatbed",
        ScanError::NoUnitSelected => "Choose a scanner first",
        ScanError::Busy => "A scan is already running",
        ScanError::SessionRequired | ScanError::CredentialsRequired => "This scanner needs a session first",
        ScanError::UnsupportedResolution(_) => "This scanner does not offer that resolution",
        ScanError::OverviewUnsupported => "This scanner cannot make a preview",
        ScanError::InvalidSetting => "That setting is not usable",
    }
}

/// The pictures of a camera's DCIM tree, as import rows. Movies, raw files and audio are left out: only the still
/// pictures the import copies are listed.
pub fn camera_files(root: &Path, contents: &CameraContents) -> Vec<CameraFile> {
    contents
        .files_of(Media::Image)
        .into_iter()
        .filter_map(|i| {
            let entry = contents.entry(i)?;
            let EntryKind::File { size, .. } = entry.kind else { return None };
            Some(CameraFile { path: root.join(contents.path(i)?), name: entry.name.clone(), size })
        })
        .collect()
}

/// Where imports go: Pictures, which the Photos app shows.
pub fn pictures_dir(home: &Path) -> PathBuf {
    home.join("Pictures")
}

/// Where scans are saved: the folder the scan settings name, under home.
pub fn scans_dir(home: &Path, folder: &str) -> PathBuf {
    home.join(folder)
}

/// The file a preview is scanned into (a cache file, replaced by each preview).
pub fn preview_path(home: &Path) -> PathBuf {
    home.join(".cache").join("image-capture").join("preview.png")
}

/// The time a scan starts, as `YYYY-MM-DD HH.MM.SS` in UTC, for the scan's file name.
pub fn stamp(unix_secs: i64) -> String {
    let (y, m, d) = civil(unix_secs.div_euclid(86_400));
    let s = unix_secs.rem_euclid(86_400);
    format!("{y:04}-{m:02}-{d:02} {:02}.{:02}.{:02}", s / 3600, s % 3600 / 60, s % 60)
}

/// The civil date of a day count since 1970-01-01 (Howard Hinnant's algorithm).
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn camera(id: &str, name: &str) -> Device {
        Device { id: id.into(), name: name.into(), kind: DeviceKind::Camera, transport: bifrost_ui::device_browser::Transport::Other, serial: None }
    }

    fn scanner_info(device: &str) -> ScannerInfo {
        ScannerInfo { device: device.into(), description: "Canon LiDE 220".into() }
    }

    fn listed(model: &mut Model) {
        model.apply(
            Reply::Listed { volumes: vec![camera("volume:/run/media/u/EOS", "EOS")], scanners: Ok(vec![scanner_info("genesys:libusb:001,004")]) },
            None,
        );
    }

    fn file(name: &str) -> CameraFile {
        CameraFile { path: PathBuf::from(format!("/run/media/u/EOS/DCIM/{name}")), name: name.into(), size: 10 }
    }

    /// The scanner shown, as the window has it after its capabilities arrive.
    fn read_scanner(model: &mut Model, resolutions: Vec<u32>, modes: Vec<ColorMode>) {
        model.apply(Reply::Capabilities { device: "genesys:libusb:001,004".into(), caps: Capabilities { resolutions, modes } }, None);
    }

    #[test]
    fn cameras_and_scanners_share_the_sidebar_and_a_scanner_knows_its_device() {
        let mut m = Model::default();
        listed(&mut m);
        assert_eq!(m.sources.len(), 2);
        assert_eq!(m.sources[0].kind, DeviceKind::Camera);
        assert_eq!(m.sources[1].scanner_device(), Some("genesys:libusb:001,004"));
        assert_eq!(m.sources[0].scanner_device(), None);
    }

    #[test]
    fn a_failed_scanner_listing_is_shown_and_the_cameras_stay() {
        let mut m = Model::default();
        m.apply(Reply::Listed { volumes: vec![camera("volume:/m/a", "A")], scanners: Err("Scanning is not available: x".into()) }, None);
        assert_eq!(m.sources.len(), 1);
        assert!(m.status.contains("not available"));
    }

    #[test]
    fn the_selection_follows_its_source_across_a_listing() {
        let mut m = Model::default();
        listed(&mut m);
        m.select(1);
        listed(&mut m);
        assert_eq!(m.selected_scanner(), Some("genesys:libusb:001,004"));
        m.apply(Reply::Listed { volumes: vec![], scanners: Ok(vec![]) }, None);
        assert_eq!(m.selected, None);
    }

    #[test]
    fn picks_toggle_and_an_import_copies_the_picked_or_all() {
        let mut m = Model::default();
        m.apply(Reply::Contents { volume: "v".into(), files: vec![file("a.jpg"), file("b.jpg")] }, Some("v"));
        m.toggle_pick(1);
        assert_eq!(m.import_paths(false), vec![PathBuf::from("/run/media/u/EOS/DCIM/b.jpg")]);
        assert_eq!(m.import_paths(true).len(), 2);
        m.toggle_pick(1);
        assert!(m.import_paths(false).is_empty());
    }

    #[test]
    fn contents_for_another_camera_are_ignored() {
        let mut m = Model::default();
        m.apply(Reply::Contents { volume: "other".into(), files: vec![file("a.jpg")] }, Some("v"));
        assert!(m.files.is_empty());
    }

    #[test]
    fn a_scanner_is_read_with_its_resolutions_and_its_colour_modes() {
        let mut m = Model::default();
        listed(&mut m);
        m.select(1);
        read_scanner(&mut m, vec![150, 600], vec![ColorMode::Gray]);
        assert!(m.scanner_idle());
        assert_eq!(m.settings().resolution_dpi, 150, "300 moves to the nearest offered value");
        assert_eq!(m.modes, vec![ColorMode::Gray]);
    }

    #[test]
    fn a_reply_for_another_scanner_is_ignored() {
        let mut m = Model::default();
        listed(&mut m);
        m.select(1);
        m.apply(Reply::Capabilities { device: "other:dev".into(), caps: Capabilities::default() }, None);
        assert!(m.scanner.is_none());
    }

    #[test]
    fn cycling_the_settings_stays_within_what_the_scanner_offers() {
        let mut m = Model::default();
        listed(&mut m);
        m.select(1);
        read_scanner(&mut m, vec![150, 300], vec![ColorMode::Color, ColorMode::Lineart]);
        m.cycle_mode();
        assert_eq!(m.settings().color, ColorMode::Lineart);
        m.cycle_mode();
        assert_eq!(m.settings().color, ColorMode::Color);
        m.cycle_dpi();
        assert_eq!(m.settings().resolution_dpi, 150, "300 is the highest, so it starts again at the lowest");
        m.cycle_dpi();
        assert_eq!(m.settings().resolution_dpi, 300);
        m.cycle_format();
        assert_eq!(m.settings().format, DocumentFormat::Png);
        m.cycle_format();
        m.cycle_format();
        assert_eq!(m.settings().format, DocumentFormat::Jpeg, "PDF is skipped, so the cycle comes back to JPEG");
    }

    #[test]
    fn a_finished_scan_ends_its_job_and_a_preview_is_kept() {
        let mut m = Model::default();
        listed(&mut m);
        m.select(1);
        read_scanner(&mut m, vec![300], vec![ColorMode::Color]);
        m.scanner.as_mut().unwrap().request_overview().unwrap();
        assert!(!m.scanner_idle());
        m.apply(Reply::Scanned { preview: true, path: PathBuf::from("/h/.cache/image-capture/preview.png") }, None);
        assert!(m.scanner_idle());
        assert!(m.preview.is_some());
        assert_eq!(m.status, "Preview ready");
        m.apply(Reply::Scanned { preview: false, path: PathBuf::from("/h/Downloads/Scan x.jpg") }, None);
        assert_eq!(m.status, "Saved Scan x.jpg");
    }

    #[test]
    fn a_failed_scan_ends_its_job_and_says_why() {
        let mut m = Model::default();
        listed(&mut m);
        m.select(1);
        read_scanner(&mut m, vec![300], vec![ColorMode::Color]);
        m.scanner.as_mut().unwrap().request_scan().unwrap();
        m.apply(Reply::ScanFailed("The scan failed: device busy".into()), None);
        assert!(m.scanner_idle());
        assert_eq!(m.status, "The scan failed: device busy");
    }

    #[test]
    fn import_counts_read_in_plain_words() {
        let mut m = Model::default();
        m.apply(Reply::Imported(0), None);
        assert_eq!(m.status, "No pictures were imported");
        m.apply(Reply::Imported(3), None);
        assert_eq!(m.status, "Imported 3 pictures into Pictures");
    }

    #[test]
    fn a_refused_request_has_plain_words() {
        assert_eq!(refusal(ScanError::Busy), "A scan is already running");
        assert_eq!(refusal(ScanError::NoUnitSelected), "Choose a scanner first");
    }

    #[test]
    fn the_folders_sit_under_home() {
        let home = Path::new("/home/u");
        assert_eq!(pictures_dir(home), PathBuf::from("/home/u/Pictures"));
        assert_eq!(scans_dir(home, "Downloads"), PathBuf::from("/home/u/Downloads"));
        assert_eq!(preview_path(home), PathBuf::from("/home/u/.cache/image-capture/preview.png"));
    }

    #[test]
    fn the_stamp_is_utc_to_the_second() {
        assert_eq!(stamp(0), "1970-01-01 00.00.00");
        assert_eq!(stamp(951_782_400), "2000-02-29 00.00.00");
        assert_eq!(stamp(1_781_000_000 + 3661), "2026-06-09 11.14.21");
    }
}
