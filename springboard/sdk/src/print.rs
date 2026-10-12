//! Printing (`NativePrintInfo` / `NativePrintOperation` / `NativePrintPanel`): describe a job, list the printers CUPS knows, and submit
//! the job with `lp`.
//!
//! [`PrintInfo`] carries paper, orientation, margins, copies and page range. [`PrintOperation::new`] takes the document as
//! a file (PDF or image; anything `lp` accepts). [`printers`] reads `lpstat -p -d`; [`PrintOperation::run`] submits with the
//! print info mapped to `lp` options, or, for [`Destination::Pdf`], copies the document to the chosen path (Save as PDF).
//! Everything shells out, so a machine without CUPS gets [`PrintError::NoCups`] and [`available`] is false.
//!
//! Gap (see docs/bifrost-ui.md "Printing"): the print sheet is not shell-drawn yet, so there is no `run_modal(window)`;
//! apps pick the printer and options with their own kit controls and call `run`. No shell proto tags are used.
//! Print dialog plug-in panels (`print_panel`) feed extra `lp` options through [`PrintOperation::with_options`].

use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaperSize {
    A4,
    A3,
    A5,
    Letter,
    Legal,
}

impl PaperSize {
    /// The CUPS `media=` name.
    pub fn media(self) -> &'static str {
        match self {
            PaperSize::A4 => "A4",
            PaperSize::A3 => "A3",
            PaperSize::A5 => "A5",
            PaperSize::Letter => "Letter",
            PaperSize::Legal => "Legal",
        }
    }
    /// Width and height in points (portrait).
    pub fn points(self) -> (f32, f32) {
        match self {
            PaperSize::A4 => (595.0, 842.0),
            PaperSize::A3 => (842.0, 1191.0),
            PaperSize::A5 => (420.0, 595.0),
            PaperSize::Letter => (612.0, 792.0),
            PaperSize::Legal => (612.0, 1008.0),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orientation {
    Portrait,
    Landscape,
}

/// `NativePrintInfo`. Margins are in points: top, left, bottom, right (default is 72 on each side).
#[derive(Debug, Clone, PartialEq)]
pub struct PrintInfo {
    pub paper: PaperSize,
    pub orientation: Orientation,
    pub margins: [f32; 4],
    pub copies: u32,
    /// 1-based inclusive page range; `None` prints everything.
    pub pages: Option<(u32, u32)>,
}

impl Default for PrintInfo {
    fn default() -> Self {
        PrintInfo { paper: PaperSize::A4, orientation: Orientation::Portrait, margins: [72.0; 4], copies: 1, pages: None }
    }
}

impl PrintInfo {
    /// Paper size in points after orientation (`NativePrintInfo.paperSize`).
    pub fn paper_size(&self) -> (f32, f32) {
        let (w, h) = self.paper.points();
        if self.orientation == Orientation::Landscape { (h, w) } else { (w, h) }
    }
    /// The printable rect (x, y, w, h) inside the margins (`imageablePageBounds`).
    pub fn imageable(&self) -> (f32, f32, f32, f32) {
        let (w, h) = self.paper_size();
        let [t, l, b, r] = self.margins;
        (l, t, (w - l - r).max(0.0), (h - t - b).max(0.0))
    }
    /// The `lp` option list for this info (everything after `lp`, before the file).
    pub fn lp_options(&self) -> Vec<String> {
        let mut o = vec!["-n".into(), self.copies.max(1).to_string(), "-o".into(), format!("media={}", self.paper.media())];
        if self.orientation == Orientation::Landscape {
            o.extend(["-o".into(), "landscape".into()]);
        }
        let [t, l, b, r] = self.margins;
        for (name, v) in [("page-top", t), ("page-left", l), ("page-bottom", b), ("page-right", r)] {
            o.extend(["-o".into(), format!("{name}={}", v.round() as i32)]);
        }
        if let Some((a, z)) = self.pages {
            o.extend(["-P".into(), if a == z { a.to_string() } else { format!("{a}-{z}") }]);
        }
        o
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Printer {
    pub name: String,
    pub enabled: bool,
    pub is_default: bool,
}

/// Parses `lpstat -p -d` output.
pub fn parse_lpstat(text: &str) -> Vec<Printer> {
    let mut default = None;
    let mut out = Vec::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("system default destination:") {
            default = Some(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("printer ") {
            let mut it = rest.splitn(2, ' ');
            let name = it.next().unwrap_or("").to_string();
            let tail = it.next().unwrap_or("");
            out.push(Printer { name, enabled: !tail.contains("disabled"), is_default: false });
        }
    }
    for p in &mut out {
        p.is_default = default.as_deref() == Some(p.name.as_str());
    }
    out
}

/// The printers CUPS lists; empty when `lpstat` is absent or the scheduler is not running.
pub fn printers() -> Vec<Printer> {
    Command::new("lpstat").args(["-p", "-d"]).output().map(|o| parse_lpstat(&String::from_utf8_lossy(&o.stdout))).unwrap_or_default()
}

/// True when `lp` can be run (CUPS client tools are installed).
pub fn available() -> bool {
    Command::new("lp").arg("-?").output().is_ok()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Destination {
    /// A CUPS queue; `None` is the system default.
    Printer(Option<String>),
    /// Save as PDF: copy the document here.
    Pdf(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrintError {
    NoCups,
    Failed(String),
}

/// `NativePrintOperation`.
#[derive(Debug, Clone)]
pub struct PrintOperation {
    pub document: PathBuf,
    pub info: PrintInfo,
    pub destination: Destination,
    pub job_name: String,
    /// Extra `lp` option pairs (`-o key=value`) from print dialog plug-ins; see [`crate::print_panel::PlugIns::lp_options`].
    pub options: Vec<String>,
}

impl PrintOperation {
    pub fn new(document: impl Into<PathBuf>) -> Self {
        let document: PathBuf = document.into();
        let job_name = document.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Untitled".into());
        PrintOperation { document, info: PrintInfo::default(), destination: Destination::Printer(None), job_name, options: Vec::new() }
    }
    pub fn with_info(mut self, info: PrintInfo) -> Self {
        self.info = info;
        self
    }
    /// Sets the plug-in option pairs appended to the `lp` call.
    pub fn with_options(mut self, options: Vec<String>) -> Self {
        self.options = options;
        self
    }
    pub fn to(mut self, destination: Destination) -> Self {
        self.destination = destination;
        self
    }
    /// The full `lp` argument list for a printer destination.
    pub fn lp_args(&self) -> Vec<String> {
        let mut a = Vec::new();
        if let Destination::Printer(Some(name)) = &self.destination {
            a.extend(["-d".into(), name.clone()]);
        }
        a.extend(["-t".into(), self.job_name.clone()]);
        a.extend(self.info.lp_options());
        a.extend(self.options.iter().cloned());
        a.push("--".into());
        a.push(self.document.to_string_lossy().into_owned());
        a
    }
    /// Submits the job. Returns the CUPS request line (`request id is ...`) or the saved path.
    pub fn run(&self) -> Result<String, PrintError> {
        match &self.destination {
            Destination::Pdf(to) => save_copy(&self.document, to),
            Destination::Printer(_) => {
                let out = Command::new("lp").args(self.lp_args()).output().map_err(|_| PrintError::NoCups)?;
                if out.status.success() {
                    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
                } else {
                    Err(PrintError::Failed(String::from_utf8_lossy(&out.stderr).trim().to_string()))
                }
            }
        }
    }
}

fn save_copy(from: &Path, to: &Path) -> Result<String, PrintError> {
    std::fs::copy(from, to).map_err(|e| PrintError::Failed(e.to_string()))?;
    Ok(to.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn landscape_swaps_paper_and_margins_shrink_the_imageable_rect() {
        let mut i = PrintInfo::default();
        assert_eq!(i.paper_size(), (595.0, 842.0));
        i.orientation = Orientation::Landscape;
        assert_eq!(i.paper_size(), (842.0, 595.0));
        assert_eq!(i.imageable(), (72.0, 72.0, 698.0, 451.0));
    }

    #[test]
    fn lp_options_map_the_print_info() {
        let i = PrintInfo { copies: 2, pages: Some((3, 5)), orientation: Orientation::Landscape, ..Default::default() };
        let o = i.lp_options().join(" ");
        assert!(o.starts_with("-n 2 -o media=A4 -o landscape"));
        assert!(o.contains("-o page-left=72"));
        assert!(o.ends_with("-P 3-5"));
        assert!(PrintInfo { copies: 0, ..Default::default() }.lp_options().contains(&"1".to_string()));
    }

    #[test]
    fn lpstat_output_lists_printers_and_the_default() {
        let t = "printer Office is idle.  enabled since Mon\nprinter Spare disabled since Mon -\nsystem default destination: Office\n";
        let p = parse_lpstat(t);
        assert_eq!(p.len(), 2);
        assert!(p[0].is_default && p[0].enabled);
        assert!(!p[1].is_default && !p[1].enabled);
        assert!(parse_lpstat("lpstat: No destinations added.").is_empty());
    }

    #[test]
    fn operation_builds_lp_args_and_saves_a_pdf_copy() {
        let op = PrintOperation::new("/tmp/a b.pdf").to(Destination::Printer(Some("Office".into())));
        let a = op.lp_args();
        assert_eq!(&a[..2], ["-d", "Office"]);
        assert_eq!(a.last().unwrap(), "/tmp/a b.pdf");
        assert_eq!(a[3], "a b.pdf");
        let dir = std::env::temp_dir().join(format!("bifrost-print-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("in.pdf"), b"%PDF").unwrap();
        let out = dir.join("out.pdf");
        let r = PrintOperation::new(dir.join("in.pdf")).to(Destination::Pdf(out.clone())).run();
        assert_eq!(r, Ok(out.to_string_lossy().into_owned()));
        assert_eq!(std::fs::read(&out).unwrap(), b"%PDF");
        assert!(matches!(PrintOperation::new(dir.join("missing")).to(Destination::Pdf(dir.join("x"))).run(), Err(PrintError::Failed(_))));
        let _ = std::fs::remove_dir_all(dir);
    }
}
