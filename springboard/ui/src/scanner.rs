//! Scanners over the SANE command line (`scanimage`): the list `scanimage -L` prints, the options a device's `--help`
//! lists, the arguments that run a scan, and the commands themselves. The model of a scan (its settings, the job and
//! its gate) is in `scan_job`; this module is the transport that carries it out. Settings Print & Scan and the Image
//! Capture app both read the list through here. The parsing and the arguments are pure and tested.

use crate::device_browser::{Device, DeviceKind, Transport};
use crate::scan_job::{ColorMode, DocumentFormat, FunctionalUnit, FunctionalUnitType, ScanSettings};
use std::fmt;
use std::path::Path;
use std::process::Command;

/// The program that talks to scanners.
pub const PROGRAM: &str = "scanimage";
/// Resolutions a device is assumed to offer when its help does not list them (dots per inch).
pub const DEFAULT_RESOLUTIONS: [u32; 7] = [75, 100, 150, 200, 300, 600, 1200];
/// The resolution a preview is scanned at: fast, and enough to see the page.
pub const PREVIEW_DPI: u32 = 75;

/// One scanner from `scanimage -L`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScannerInfo {
    /// The device name SANE addresses it by, such as `airscan:w0:Brother MFC` or `genesys:libusb:001,004`.
    pub device: String,
    /// The description after "is a", such as `Brother MFC-L2750DW series`.
    pub description: String,
}

impl ScannerInfo {
    /// The name shown to the person: the description, or the device name when there is none.
    pub fn name(&self) -> &str {
        if self.description.is_empty() { &self.device } else { &self.description }
    }

    /// How the scanner is connected, read from its device name.
    pub fn transport(&self) -> Transport {
        if self.device.starts_with("airscan") || self.device.starts_with("escl") || self.device.starts_with("net") {
            Transport::Network
        } else if self.device.contains("usb") {
            Transport::Usb
        } else {
            Transport::Other
        }
    }
}

/// Reads the output of `scanimage -L`. Each device line has the form `device 'NAME' is a DESCRIPTION`; a line
/// that does not (the program's notes, a blank line) is skipped.
pub fn parse_list(text: &str) -> Vec<ScannerInfo> {
    text.lines()
        .filter_map(|line| {
            let rest = line.trim().strip_prefix("device '")?;
            let (device, after) = rest.split_once("' is a")?;
            let description = after.trim().trim_end_matches("virtual device").trim();
            Some(ScannerInfo { device: device.to_string(), description: description.to_string() })
        })
        .collect()
}

/// The scanners as device-browser entries, so the device model and its add and remove events apply to them too.
pub fn device_entries(list: &[ScannerInfo]) -> Vec<Device> {
    list.iter()
        .map(|s| Device { id: format!("scanner:{}", s.device), name: s.name().to_string(), kind: DeviceKind::Scanner, transport: s.transport(), serial: None })
        .collect()
}

/// What one device can do, read from its `--help`: the resolutions it takes and the colour modes it offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Capabilities {
    pub resolutions: Vec<u32>,
    pub modes: Vec<ColorMode>,
}

impl Default for Capabilities {
    fn default() -> Self {
        Capabilities { resolutions: DEFAULT_RESOLUTIONS.to_vec(), modes: ColorMode::ALL.to_vec() }
    }
}

impl Capabilities {
    /// The device as a functional unit of `scan_job`: its resolutions, and a preview it can always make.
    pub fn unit(&self) -> FunctionalUnit {
        FunctionalUnit { kind: FunctionalUnitType::Flatbed, resolutions: self.resolutions.clone(), can_overview: true }
    }
}

impl Capabilities {
    /// Reads a device's `--help` text. `--resolution` lists its values as `75|100|300dpi` (a range leaves the default
    /// list in place) and `--mode` lists `Color|Gray|Lineart`. A missing option keeps the default for that part.
    pub fn parse_help(help: &str) -> Capabilities {
        let mut caps = Capabilities::default();
        for line in help.lines() {
            let line = line.trim();
            if let Some(rest) = line.strip_prefix("--resolution") {
                let values: Vec<u32> = option_values(rest)
                    .filter(|t| t.contains('|') || t.ends_with("dpi"))
                    .map(|t| t.split('|').filter_map(|v| v.trim_end_matches("dpi").parse::<u32>().ok()).collect())
                    .unwrap_or_default();
                if !values.is_empty() {
                    caps.resolutions = values;
                }
            } else if let Some(rest) = line.strip_prefix("--mode") {
                let modes: Vec<ColorMode> = option_values(rest)
                    .filter(|t| t.contains('|'))
                    .map(|t| {
                        t.split('|')
                            .filter_map(|name| match name.trim() {
                                "Color" => Some(ColorMode::Color),
                                "Gray" => Some(ColorMode::Gray),
                                "Lineart" => Some(ColorMode::Lineart),
                                _ => None,
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                if !modes.is_empty() {
                    caps.modes = modes;
                }
            }
        }
        caps
    }
}

/// The first token after an option name that holds its value list (`75|100|300dpi` or `Color|Gray`), split at spaces
/// and brackets.
fn option_values(rest: &str) -> Option<&str> {
    rest.split(|c: char| c.is_whitespace() || c == '[' || c == ']').find(|t| !t.is_empty())
}

/// The option words `scanimage` takes for a colour mode.
pub fn mode_option(mode: ColorMode) -> &'static str {
    match mode {
        ColorMode::Color => "Color",
        ColorMode::Gray => "Gray",
        ColorMode::Lineart => "Lineart",
    }
}

/// The word `scanimage --format` takes, or None for a format the command cannot write (PDF is made from a page scan
/// by another tool, not here).
pub fn format_option(format: DocumentFormat) -> Option<&'static str> {
    match format {
        DocumentFormat::Png => Some("png"),
        DocumentFormat::Jpeg => Some("jpeg"),
        DocumentFormat::Tiff => Some("tiff"),
        DocumentFormat::Pdf => None,
    }
}

/// The arguments that scan `device` with `s`. A preview scans at `PREVIEW_DPI` in PNG, in the chosen colour mode, so
/// it is quick to show. A device option the command rejects is the command's to report.
pub fn scan_args(device: &str, s: &ScanSettings, preview: bool) -> Result<Vec<String>, CommandError> {
    let (dpi, format) = if preview { (PREVIEW_DPI, DocumentFormat::Png) } else { (s.resolution_dpi, s.format) };
    let format = format_option(format).ok_or_else(|| CommandError::Failed("PDF scans are not supported yet".into()))?;
    Ok(vec![
        "--device-name".into(),
        device.into(),
        format!("--resolution={dpi}"),
        format!("--mode={}", mode_option(s.color)),
        format!("--format={format}"),
    ])
}

/// The arguments that ask a device for its options (`--help` with the device named).
pub fn help_args(device: &str) -> Vec<String> {
    vec!["--device-name".into(), device.into(), "--help".into()]
}

/// The file name a finished scan is saved under: `<document name> <stamp>.<ext>`, where `stamp` is the time the scan
/// started (the caller formats it, such as `2026-10-13 14.05.09`).
pub fn scan_file_name(document: &str, stamp: &str, format: DocumentFormat) -> String {
    format!("{document} {stamp}.{}", format.extension())
}

/// Why a scanner command failed. The window shows the text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommandError {
    /// The program is not installed or could not start.
    NotInstalled(String),
    /// The program ran and reported an error (its stderr, trimmed), or the request cannot be made.
    Failed(String),
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CommandError::NotInstalled(e) => write!(f, "Scanning is not available: {e}"),
            CommandError::Failed(e) if e.is_empty() => write!(f, "The scan failed"),
            CommandError::Failed(e) => write!(f, "The scan failed: {e}"),
        }
    }
}

/// Runs `scanimage` with `args` and returns its standard output.
fn run(args: &[String]) -> Result<Vec<u8>, CommandError> {
    let out = Command::new(PROGRAM).args(args).output().map_err(|e| CommandError::NotInstalled(e.to_string()))?;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        Err(CommandError::Failed(String::from_utf8_lossy(&out.stderr).trim().to_string()))
    }
}

/// The scanners the system reports now (`scanimage -L`). A scanner that is off or not reachable is not listed.
pub fn list() -> Result<Vec<ScannerInfo>, CommandError> {
    run(&["-L".to_string()]).map(|out| parse_list(&String::from_utf8_lossy(&out)))
}

/// What `device` can do, read from its `--help`. A device whose help cannot be read gets the default capabilities.
pub fn capabilities_of(device: &str) -> Capabilities {
    match run(&help_args(device)) {
        Ok(out) => Capabilities::parse_help(&String::from_utf8_lossy(&out)),
        Err(_) => Capabilities::default(),
    }
}

/// Scans `device` with `s` into `path`. A preview is scanned the same way at the preview resolution.
pub fn scan_to(device: &str, s: &ScanSettings, preview: bool, path: &Path) -> Result<(), CommandError> {
    let mut args = scan_args(device, s, preview)?;
    args.push(format!("--output-file={}", path.display()));
    run(&args).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST: &str = "\
device 'airscan:w0:Brother MFC' is a Brother MFC-L2750DW series virtual device
device 'genesys:libusb:001,004' is a Canon LiDE 220 flatbed scanner
some note the program prints
";

    const HELP: &str = "\
Options specific to device `genesys:libusb:001,004':
  --resolution 75|100|150|200|300|600|1200dpi [300]
    Sets the resolution of the scanned image.
  --mode Color|Gray|Lineart [Color]
    Selects the scan mode.
";

    #[test]
    fn the_device_list_names_each_scanner_and_skips_other_lines() {
        let list = parse_list(LIST);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].device, "airscan:w0:Brother MFC");
        assert_eq!(list[0].name(), "Brother MFC-L2750DW series");
        assert_eq!(list[1].description, "Canon LiDE 220 flatbed scanner");
    }

    #[test]
    fn a_scanner_with_no_description_is_named_by_its_device() {
        let s = ScannerInfo { device: "pixma:04A9".into(), description: String::new() };
        assert_eq!(s.name(), "pixma:04A9");
    }

    #[test]
    fn transport_is_read_from_the_device_name() {
        let net = ScannerInfo { device: "airscan:w0:x".into(), description: String::new() };
        let usb = ScannerInfo { device: "genesys:libusb:001,004".into(), description: String::new() };
        let other = ScannerInfo { device: "test:0".into(), description: String::new() };
        assert_eq!(net.transport(), Transport::Network);
        assert_eq!(usb.transport(), Transport::Usb);
        assert_eq!(other.transport(), Transport::Other);
    }

    #[test]
    fn scanners_become_scanner_entries_in_the_device_model() {
        let entries = device_entries(&parse_list(LIST));
        assert_eq!(entries[0].id, "scanner:airscan:w0:Brother MFC");
        assert_eq!(entries[0].kind, DeviceKind::Scanner);
        assert_eq!(entries[1].name, "Canon LiDE 220 flatbed scanner");
    }

    #[test]
    fn the_help_text_gives_the_resolutions_and_modes() {
        let c = Capabilities::parse_help(HELP);
        assert_eq!(c.resolutions, vec![75, 100, 150, 200, 300, 600, 1200]);
        assert_eq!(c.modes, vec![ColorMode::Color, ColorMode::Gray, ColorMode::Lineart]);
    }

    #[test]
    fn a_resolution_range_or_no_option_keeps_the_default_list() {
        let range = Capabilities::parse_help("  --resolution 1..1200dpi [300]\n");
        assert_eq!(range.resolutions, DEFAULT_RESOLUTIONS.to_vec());
        assert_eq!(Capabilities::parse_help("nothing useful here"), Capabilities::default());
    }

    #[test]
    fn a_device_without_greyscale_only_offers_what_it_has() {
        let c = Capabilities::parse_help("  --mode Color|Lineart [Color]\n");
        assert_eq!(c.modes, vec![ColorMode::Color, ColorMode::Lineart]);
    }

    #[test]
    fn capabilities_become_a_flatbed_unit_that_can_preview() {
        let unit = Capabilities { resolutions: vec![150, 600], modes: vec![] }.unit();
        assert_eq!(unit.kind, FunctionalUnitType::Flatbed);
        assert_eq!(unit.resolutions, vec![150, 600]);
        assert!(unit.can_overview);
    }

    #[test]
    fn the_scan_arguments_name_the_device_and_the_settings() {
        let s = ScanSettings { resolution_dpi: 600, color: ColorMode::Gray, format: DocumentFormat::Jpeg, ..ScanSettings::default() };
        assert_eq!(
            scan_args("genesys:libusb:001,004", &s, false).unwrap(),
            vec!["--device-name", "genesys:libusb:001,004", "--resolution=600", "--mode=Gray", "--format=jpeg"]
        );
    }

    #[test]
    fn a_preview_scans_at_the_preview_resolution_in_png() {
        let s = ScanSettings { resolution_dpi: 600, color: ColorMode::Lineart, format: DocumentFormat::Tiff, ..ScanSettings::default() };
        let args = scan_args("d", &s, true).unwrap();
        assert!(args.contains(&format!("--resolution={PREVIEW_DPI}")));
        assert!(args.contains(&"--mode=Lineart".to_string()));
        assert!(args.contains(&"--format=png".to_string()));
    }

    #[test]
    fn a_pdf_scan_is_refused_by_the_command_layer() {
        let s = ScanSettings { format: DocumentFormat::Pdf, ..ScanSettings::default() };
        assert!(scan_args("d", &s, false).is_err());
        assert!(scan_args("d", &s, true).is_ok(), "a preview is always PNG");
    }

    #[test]
    fn a_scan_is_named_by_its_document_name_start_time_and_format() {
        assert_eq!(scan_file_name("Scan", "2026-10-13 14.05.09", DocumentFormat::Jpeg), "Scan 2026-10-13 14.05.09.jpg");
        assert_eq!(scan_file_name("Receipt", "t", DocumentFormat::Tiff), "Receipt t.tiff");
    }

    #[test]
    fn the_errors_say_what_failed() {
        assert_eq!(CommandError::Failed(String::new()).to_string(), "The scan failed");
        assert_eq!(CommandError::Failed("device busy".into()).to_string(), "The scan failed: device busy");
    }
}
