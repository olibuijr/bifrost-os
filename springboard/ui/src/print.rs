//! Printing (`NativePrintInfo` / `NativePrintOperation` / `NativePrintPanel`): describe a job, write its pages as a PDF
//! and submit it through the CUPS client.
//!
//! [`PrintInfo`] carries paper, orientation, margins, copies and page range. A [`PrintOperation`] has one of two sources:
//! a file ([`PrintOperation::new`]: a PDF, or an image the scheduler can convert) or pages the app built
//! ([`PrintOperation::pages`]: text laid out to [`PrintInfo::imageable`], or raster slices). Pages are written with
//! [`bifrost_render::cg::pdf::writer`]. [`PrintOperation::run`] submits the job, or, for [`Destination::Pdf`], saves it
//! (Save as PDF).
//!
//! Submission goes through [`crate::cups::Client::print_document_with`]: copies, media, orientation and page range are
//! job-template attributes. Margins are not attributes, because the document is already laid out inside them. Plug-in
//! panel options (`print_panel`) are added as job-template attributes too. The printers come from [`crate::cups`]; this
//! module does not list them. Without a scheduler a print fails with [`PrintError::NoCups`], and [`available`] is false.
//!
//! Gap (see docs/bifrost-ui.md "Printing"): the print sheet is not shell-drawn yet, so there is no `run_modal(window)`;
//! apps pick the printer and options with their own kit controls and call `run`.

use crate::cups::{self, Client, CupsError};
use crate::ipp::{tag, Value};
use bifrost_render::cg::pdf::writer;
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub use bifrost_render::cg::pdf::writer::{Page as PdfPage, PageImage};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaperSize {
    A4,
    A3,
    A5,
    Letter,
    Legal,
}

impl PaperSize {
    /// The paper's name as the print dialog shows it.
    pub fn media(self) -> &'static str {
        match self {
            PaperSize::A4 => "A4",
            PaperSize::A3 => "A3",
            PaperSize::A5 => "A5",
            PaperSize::Letter => "Letter",
            PaperSize::Legal => "Legal",
        }
    }
    /// The PWG media keyword the scheduler takes as the `media` job-template attribute.
    pub fn pwg(self) -> &'static str {
        match self {
            PaperSize::A4 => "iso_a4_210x297mm",
            PaperSize::A3 => "iso_a3_297x420mm",
            PaperSize::A5 => "iso_a5_148x210mm",
            PaperSize::Letter => "na_letter_8.5x11in",
            PaperSize::Legal => "na_legal_8.5x14in",
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
    /// The job-template attributes for this info: copies, media, orientation and the page range (an IPP `rangeOfInteger`,
    /// lower then upper bound).
    pub fn attributes(&self) -> Vec<(String, Value)> {
        let orientation = if self.orientation == Orientation::Landscape { 4 } else { 3 };
        let mut a = vec![
            ("copies".to_string(), Value::Integer(self.copies.max(1) as i32)),
            ("media".to_string(), Value::keyword(self.paper.pwg())),
            ("orientation-requested".to_string(), Value::Enum(orientation)),
        ];
        if let Some((first, last)) = self.pages {
            let mut bytes = (first as i32).to_be_bytes().to_vec();
            bytes.extend((last as i32).to_be_bytes());
            a.push(("page-ranges".to_string(), Value::Other { tag: tag::RANGE_OF_INTEGER, bytes }));
        }
        a
    }
}

/// True when a CUPS scheduler answers on the local port, so a job can be submitted.
pub fn available() -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], cups::DEFAULT_PORT));
    TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_ok()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Destination {
    /// A CUPS queue; `None` is the scheduler's default printer.
    Printer(Option<String>),
    /// Save as PDF: write the document to this path.
    Pdf(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrintError {
    /// The scheduler is not running or cannot be reached.
    NoCups,
    /// No printer was named and the scheduler has no default.
    NoPrinter,
    Failed(String),
}

/// What a job prints: a file the app already has, or pages the app built (written as a PDF when the job runs).
#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    File(PathBuf),
    Pages { pages: Vec<PdfPage>, fonts: Vec<&'static str> },
}

/// `NativePrintOperation`.
#[derive(Debug, Clone)]
pub struct PrintOperation {
    pub source: Source,
    pub info: PrintInfo,
    pub destination: Destination,
    pub job_name: String,
    /// Extra job-template pairs from print dialog plug-ins; see the SDK's `print_panel::PlugIns::job_options`.
    pub options: Vec<(String, String)>,
}

impl PrintOperation {
    /// A job that prints a file. The job name is the file name.
    pub fn new(document: impl Into<PathBuf>) -> Self {
        let document: PathBuf = document.into();
        let job_name = document.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Untitled".into());
        PrintOperation {
            source: Source::File(document),
            info: PrintInfo::default(),
            destination: Destination::Printer(None),
            job_name,
            options: Vec::new(),
        }
    }
    /// A job that prints pages the app built. `fonts` are standard font names, `/F0`.. in the order the pages' content uses
    /// them (see [`writer::write`]).
    pub fn pages(job_name: impl Into<String>, pages: Vec<PdfPage>, fonts: Vec<&'static str>) -> Self {
        PrintOperation {
            source: Source::Pages { pages, fonts },
            info: PrintInfo::default(),
            destination: Destination::Printer(None),
            job_name: job_name.into(),
            options: Vec::new(),
        }
    }
    pub fn with_info(mut self, info: PrintInfo) -> Self {
        self.info = info;
        self
    }
    /// Sets the plug-in job-template pairs sent with the job.
    pub fn with_options(mut self, options: Vec<(String, String)>) -> Self {
        self.options = options;
        self
    }
    pub fn to(mut self, destination: Destination) -> Self {
        self.destination = destination;
        self
    }
    /// The document bytes this job sends, and their IPP format.
    pub fn document(&self) -> Result<(&'static str, Vec<u8>), PrintError> {
        match &self.source {
            Source::File(path) => {
                let data = std::fs::read(path).map_err(|e| PrintError::Failed(e.to_string()))?;
                Ok((document_format(path), data))
            }
            Source::Pages { pages, fonts } => Ok(("application/pdf", writer::write(pages, fonts))),
        }
    }
    /// Runs the job. Returns the request id (`printer-job`) for a printer, or the saved path for Save as PDF.
    pub fn run(&self) -> Result<String, PrintError> {
        let (format, data) = self.document()?;
        match &self.destination {
            Destination::Pdf(to) => {
                std::fs::write(to, &data).map_err(|e| PrintError::Failed(e.to_string()))?;
                Ok(to.to_string_lossy().into_owned())
            }
            Destination::Printer(name) => submit(&self.job_name, name.as_deref(), &self.info, &self.options, format, data),
        }
    }
}

/// The IPP document format for a file, from its extension. Unknown types go as `application/octet-stream`, which the
/// scheduler detects itself.
pub fn document_format(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref() {
        Some("pdf") => "application/pdf",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("txt") => "text/plain",
        _ => "application/octet-stream",
    }
}

fn from_cups(e: CupsError) -> PrintError {
    match e {
        CupsError::Io(_) | CupsError::Http(_) => PrintError::NoCups,
        other => PrintError::Failed(other.to_string()),
    }
}

/// The value of a plug-in option: a number or a boolean when it reads as one, otherwise a keyword.
fn option_value(v: &str) -> Value {
    if let Ok(n) = v.parse::<i32>() {
        Value::Integer(n)
    } else if v == "true" || v == "false" {
        Value::Boolean(v == "true")
    } else {
        Value::keyword(v)
    }
}

/// Sends a document to a queue (`None` is the default printer) through the CUPS client. Returns `printer-job`, the job's
/// name in CUPS' request form (`<printer>-<id>`).
fn submit(
    job_name: &str,
    printer: Option<&str>,
    info: &PrintInfo,
    options: &[(String, String)],
    format: &str,
    data: Vec<u8>,
) -> Result<String, PrintError> {
    let mut client = Client::local();
    let printer = match printer {
        Some(name) => name.to_string(),
        None => client.default_printer().map_err(from_cups)?.ok_or(PrintError::NoPrinter)?,
    };
    let mut template = info.attributes();
    template.extend(options.iter().map(|(k, v)| (k.clone(), option_value(v))));
    let user = std::env::var("USER").unwrap_or_else(|_| "bifrost".to_string());
    let id = client.print_document_with(&printer, job_name, &user, format, data, template).map_err(from_cups)?;
    Ok(format!("{printer}-{id}"))
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
    fn attributes_map_the_print_info_and_margins_stay_in_the_document() {
        let i = PrintInfo { copies: 2, pages: Some((3, 5)), orientation: Orientation::Landscape, ..Default::default() };
        let a = i.attributes();
        assert_eq!(a[0], ("copies".to_string(), Value::Integer(2)));
        assert_eq!(a[1], ("media".to_string(), Value::keyword("iso_a4_210x297mm")));
        assert_eq!(a[2], ("orientation-requested".to_string(), Value::Enum(4)));
        let mut range = 3i32.to_be_bytes().to_vec();
        range.extend(5i32.to_be_bytes());
        assert_eq!(a[3], ("page-ranges".to_string(), Value::Other { tag: tag::RANGE_OF_INTEGER, bytes: range }));
        assert!(a.iter().all(|(name, _)| !name.starts_with("page-top") && !name.starts_with("page-left")));
        assert_eq!(PrintInfo { copies: 0, ..Default::default() }.attributes()[0].1, Value::Integer(1));
    }

    #[test]
    fn every_paper_has_a_pwg_keyword_and_a_name() {
        assert_eq!(PaperSize::Letter.pwg(), "na_letter_8.5x11in");
        assert_eq!(PaperSize::A3.media(), "A3");
    }

    #[test]
    fn document_formats_follow_the_extension() {
        assert_eq!(document_format(Path::new("/x/a.PDF")), "application/pdf");
        assert_eq!(document_format(Path::new("b.jpeg")), "image/jpeg");
        assert_eq!(document_format(Path::new("c.bin")), "application/octet-stream");
        assert_eq!(document_format(Path::new("noext")), "application/octet-stream");
    }

    #[test]
    fn plug_in_values_are_numbers_booleans_or_keywords() {
        assert_eq!(option_value("3"), Value::Integer(3));
        assert_eq!(option_value("true"), Value::Boolean(true));
        assert_eq!(option_value("two-sided-long-edge"), Value::keyword("two-sided-long-edge"));
    }

    #[test]
    fn a_file_job_reads_the_file_and_saves_a_copy_for_save_as_pdf() {
        let dir = std::env::temp_dir().join(format!("bifrost-print-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("in.pdf"), b"%PDF").unwrap();
        let out = dir.join("out.pdf");
        let r = PrintOperation::new(dir.join("in.pdf")).to(Destination::Pdf(out.clone())).run();
        assert_eq!(r, Ok(out.to_string_lossy().into_owned()));
        assert_eq!(std::fs::read(&out).unwrap(), b"%PDF");
        let missing = PrintOperation::new(dir.join("missing")).to(Destination::Pdf(dir.join("x"))).run();
        assert!(matches!(missing, Err(PrintError::Failed(_))));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_page_job_writes_a_pdf_of_its_pages_and_saves_it() {
        let page = PdfPage { width: 595.0, height: 842.0, content: b"BT ET".to_vec(), images: Vec::new() };
        let dir = std::env::temp_dir().join(format!("bifrost-print-pages-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("pages.pdf");
        let op = PrintOperation::pages("Notes", vec![page.clone(), page], vec!["Helvetica"]).to(Destination::Pdf(out.clone()));
        assert_eq!(op.run(), Ok(out.to_string_lossy().into_owned()));
        let bytes = std::fs::read(&out).unwrap();
        assert!(bytes.starts_with(b"%PDF-1.4"));
        assert!(String::from_utf8_lossy(&bytes).contains("/Count 2"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
