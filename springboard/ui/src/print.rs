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
use crate::print_pages::{self, PrintableView, Rect};
use bifrost_render::cg::pdf::writer::{self, text_literal};
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
    /// Every paper size, in the order the print sheet lists them.
    pub const ALL: [PaperSize; 5] = [PaperSize::A4, PaperSize::Letter, PaperSize::A3, PaperSize::A5, PaperSize::Legal];
    /// The paper whose [`PaperSize::media`] name this is (case-insensitive), for reading a saved preset.
    pub fn from_media(name: &str) -> Option<PaperSize> {
        PaperSize::ALL.into_iter().find(|p| p.media().eq_ignore_ascii_case(name.trim()))
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

/// What happens to a finished job (`jobDisposition`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum JobDisposition {
    /// Send the job to the printer.
    #[default]
    Spool,
    /// Write the job as a PDF to [`PrintInfo::saving_path`].
    Save,
    /// Write the job as a PDF in the temporary directory and return its path for a viewer to open.
    Preview,
}

/// Two-sided printing (`sides`). The default is one-sided, which is not sent with the job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Sides {
    #[default]
    OneSided,
    TwoSidedLongEdge,
    TwoSidedShortEdge,
}

impl Sides {
    /// The IPP keyword for `sides`.
    pub fn keyword(self) -> &'static str {
        match self {
            Sides::OneSided => "one-sided",
            Sides::TwoSidedLongEdge => "two-sided-long-edge",
            Sides::TwoSidedShortEdge => "two-sided-short-edge",
        }
    }
}

/// Colour or black and white (`print-color-mode`). The default is colour, which is not sent with the job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorMode {
    #[default]
    Color,
    Monochrome,
}

impl ColorMode {
    /// The IPP keyword for `print-color-mode`.
    pub fn keyword(self) -> &'static str {
        match self {
            ColorMode::Color => "color",
            ColorMode::Monochrome => "monochrome",
        }
    }
}

/// The smallest and largest scaling factors (10% and 1000%).
pub const SCALING_RANGE: (f32, f32) = (0.1, 10.0);

/// `NativePrintInfo`. Margins are in points: top, left, bottom, right (default is 72 on each side).
#[derive(Debug, Clone, PartialEq)]
pub struct PrintInfo {
    pub paper: PaperSize,
    pub orientation: Orientation,
    pub margins: [f32; 4],
    pub copies: u32,
    /// 1-based inclusive page range; `None` prints everything.
    pub pages: Option<(u32, u32)>,
    /// Several 1-based inclusive ranges from the print sheet's page list, sent as one `page-ranges` attribute with a
    /// value per range. When it is not empty it replaces [`PrintInfo::pages`]; see [`PrintInfo::page_ranges`].
    pub page_list: Vec<(u32, u32)>,
    /// How large the content prints: 1.0 is actual size, 0.5 half size. Read through [`PrintInfo::scaling`], which keeps
    /// it inside [`SCALING_RANGE`].
    pub scaling_factor: f32,
    pub job_disposition: JobDisposition,
    /// Where [`JobDisposition::Save`] writes the PDF.
    pub saving_path: Option<PathBuf>,
    /// Two-sided printing; sent as the `sides` job-template attribute when it is not one-sided.
    pub sides: Sides,
    /// Colour or monochrome; sent as `print-color-mode` when it is monochrome.
    pub color: ColorMode,
}

impl Default for PrintInfo {
    fn default() -> Self {
        PrintInfo {
            paper: PaperSize::A4,
            orientation: Orientation::Portrait,
            margins: [72.0; 4],
            copies: 1,
            pages: None,
            page_list: Vec::new(),
            scaling_factor: 1.0,
            job_disposition: JobDisposition::Spool,
            saving_path: None,
            sides: Sides::OneSided,
            color: ColorMode::Color,
        }
    }
}

impl PrintInfo {
    /// Paper size in points after orientation (`NativePrintInfo.paperSize`).
    pub fn paper_size(&self) -> (f32, f32) {
        let (w, h) = self.paper.points();
        if self.orientation == Orientation::Landscape { (h, w) } else { (w, h) }
    }
    /// The scaling factor limited to [`SCALING_RANGE`]; a value that is not a number counts as 1.0.
    pub fn scaling(&self) -> f32 {
        if self.scaling_factor.is_nan() { 1.0 } else { self.scaling_factor.clamp(SCALING_RANGE.0, SCALING_RANGE.1) }
    }
    /// The printable rect (x, y, w, h) inside the margins (`imageablePageBounds`).
    pub fn imageable(&self) -> (f32, f32, f32, f32) {
        let (w, h) = self.paper_size();
        let [t, l, b, r] = self.margins;
        (l, t, (w - l - r).max(0.0), (h - t - b).max(0.0))
    }
    /// The ranges this job prints: [`PrintInfo::page_list`] when it is set, otherwise [`PrintInfo::pages`]. Empty means
    /// every page.
    pub fn page_ranges(&self) -> Vec<(u32, u32)> {
        if !self.page_list.is_empty() {
            return self.page_list.clone();
        }
        self.pages.into_iter().collect()
    }

    /// The job-template attributes for this info: copies, media, orientation and the page ranges (an IPP `rangeOfInteger`
    /// per range, lower then upper bound).
    pub fn attributes(&self) -> Vec<(String, Value)> {
        let orientation = if self.orientation == Orientation::Landscape { 4 } else { 3 };
        let mut a = vec![
            ("copies".to_string(), Value::Integer(self.copies.max(1) as i32)),
            ("media".to_string(), Value::keyword(self.paper.pwg())),
            ("orientation-requested".to_string(), Value::Enum(orientation)),
        ];
        for (first, last) in self.page_ranges() {
            let mut bytes = (first as i32).to_be_bytes().to_vec();
            bytes.extend((last as i32).to_be_bytes());
            a.push(("page-ranges".to_string(), Value::Other { tag: tag::RANGE_OF_INTEGER, bytes }));
        }
        if self.sides != Sides::OneSided {
            a.push(("sides".to_string(), Value::keyword(self.sides.keyword())));
        }
        if self.color == ColorMode::Monochrome {
            a.push(("print-color-mode".to_string(), Value::keyword(self.color.keyword())));
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

/// What a job prints: a file the app already has, pages the app built (written as a PDF when the job runs), or plain text
/// laid out to the job's page (see [`text_pages`]).
#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    File(PathBuf),
    Pages { pages: Vec<PdfPage>, fonts: Vec<&'static str> },
    Text(String),
}

/// The size plain text prints at, in points.
pub const TEXT_SIZE: f32 = 11.0;

/// Helvetica advance widths for the printable ASCII characters 32 to 126, in thousandths of an em (the standard face's
/// metrics).
const HELVETICA: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, //
    556, 556, 556, 556, 556, 556, 556, 556, 556, 556, 278, 278, 584, 584, 584, 556, 1015, //
    667, 667, 722, 722, 667, 611, 778, 722, 278, 500, 667, 556, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667,
    667, 611, 278, 278, 278, 469, 556, 333, //
    556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500, 222, 833, 556, 556, 556, 556, 333, 500, 278, 556, 500, 722, 500,
    500, 500, 334, 260, 334, 584,
];

/// The advance of a character in points at `size`. Characters outside printable ASCII count as a digit's width.
fn advance(ch: char, size: f32) -> f32 {
    let units = match ch {
        ' '..='~' => HELVETICA[ch as usize - 32] as f32,
        _ => 556.0,
    };
    units * size / 1000.0
}

/// One paragraph word-wrapped to `width` points. A word wider than a line is not broken: it runs past the edge.
fn wrap(paragraph: &str, size: f32, width: f32) -> Vec<String> {
    let space = advance(' ', size);
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut line_w = 0.0;
    for word in paragraph.split(' ') {
        let word_w: f32 = word.chars().map(|c| advance(c, size)).sum();
        if !line.is_empty() && line_w + space + word_w > width {
            lines.push(std::mem::take(&mut line));
            line_w = 0.0;
        }
        if !line.is_empty() {
            line.push(' ');
            line_w += space;
        }
        line.push_str(word);
        line_w += word_w;
    }
    lines.push(line);
    lines
}

/// Plain text laid out for `info`: Helvetica at `size` points, one line per line of `text` wrapped to the printable width,
/// and pages of as many lines as the printable height holds. Tabs print as four spaces; line feeds start a new line.
pub fn text_pages(text: &str, size: f32, info: &PrintInfo) -> Vec<PdfPage> {
    let (left, top, width, height) = info.imageable();
    let (w, h) = info.paper_size();
    let leading = size * 1.25;
    let per_page = ((height / leading).floor() as usize).max(1);
    let normalized = text.replace('\t', "    ").replace('\r', "");
    let lines: Vec<String> = normalized.split('\n').flat_map(|p| wrap(p, size, width)).collect();
    lines
        .chunks(per_page)
        .map(|page| {
            let mut content = Vec::new();
            for (i, line) in page.iter().enumerate() {
                let y = h - top - (i as f32 + 1.0) * leading;
                content.extend(format!("BT /F0 {size} Tf {left:.2} {y:.2} Td ").into_bytes());
                text_literal(line, &mut content);
                content.extend(b" Tj ET\n");
            }
            PdfPage { width: w as f64, height: h as f64, content, images: Vec::new() }
        })
        .collect()
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
    /// Whether [`PrintOperation::run_modal`] asks the print panel first. On for a print operation, off for a PDF one.
    pub shows_print_panel: bool,
    /// Whether the app shows a progress panel while the job runs. The operation does not draw it; the flag is the app's
    /// switch.
    pub shows_progress_panel: bool,
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
            shows_print_panel: true,
            shows_progress_panel: true,
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
            shows_print_panel: true,
            shows_progress_panel: true,
        }
    }
    /// A job that prints `text` in Helvetica, laid out for the job's page info when it runs (see [`text_pages`]).
    pub fn text(job_name: impl Into<String>, text: impl Into<String>) -> Self {
        PrintOperation {
            source: Source::Text(text.into()),
            info: PrintInfo::default(),
            destination: Destination::Printer(None),
            job_name: job_name.into(),
            options: Vec::new(),
            shows_print_panel: true,
            shows_progress_panel: true,
        }
    }
    /// `printOperation(with:printInfo:)`: a job that prints `view`, paginated and drawn for `info` now.
    ///
    /// The page range in `info` cuts the view before it is drawn, so the document already holds only those pages. The
    /// operation's info therefore drops the range: a `page-ranges` job attribute would cut the pages a second time.
    pub fn print_operation(job_name: impl Into<String>, view: &dyn PrintableView, info: PrintInfo) -> Self {
        let pages = print_pages::render_pages(view, &info);
        let mut op = PrintOperation::pages(job_name, pages, vec!["Helvetica"]);
        op.info = PrintInfo { pages: None, page_list: Vec::new(), ..info };
        op
    }
    /// `pdfOperation(with:inside:to:)`: a job that writes the part of `view` inside `rect` as a one-page PDF at `to`.
    /// It shows neither panel.
    pub fn pdf_operation(view: &dyn PrintableView, rect: Rect, to: impl Into<PathBuf>) -> Self {
        let page = print_pages::render_region(view, rect);
        let mut op = PrintOperation::pages("Untitled", vec![page], vec!["Helvetica"]).to(Destination::Pdf(to.into()));
        op.shows_print_panel = false;
        op.shows_progress_panel = false;
        op
    }
    /// `runModal(for:delegate:didRun:contextInfo:)`: when [`PrintOperation::shows_print_panel`] is set, `panel` edits the
    /// print info and answers whether the user chose Print. Then the job runs. `did_run` gets whether it ran and
    /// succeeded. Returns `Ok(None)` when the panel was cancelled.
    ///
    /// Paper and margin changes made in the panel reach a text job and the job-template attributes. The pages of a view
    /// job were drawn when the operation was made, so change those before calling [`PrintOperation::print_operation`].
    pub fn run_modal(
        mut self,
        panel: impl FnOnce(&mut PrintInfo) -> bool,
        did_run: impl FnOnce(bool),
    ) -> Result<Option<String>, PrintError> {
        if self.shows_print_panel && !panel(&mut self.info) {
            did_run(false);
            return Ok(None);
        }
        let result = self.run();
        did_run(result.is_ok());
        result.map(Some)
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
            Source::Text(text) => {
                let pages = text_pages(text, TEXT_SIZE, &self.info);
                Ok(("application/pdf", writer::write(&pages, &["Helvetica"])))
            }
        }
    }
    /// Runs the job. Returns the request id (`printer-job`) for a printer, or the saved path for Save as PDF.
    pub fn run(&self) -> Result<String, PrintError> {
        let (format, data) = self.document()?;
        let write = |to: &Path| -> Result<String, PrintError> {
            std::fs::write(to, &data).map_err(|e| PrintError::Failed(e.to_string()))?;
            Ok(to.to_string_lossy().into_owned())
        };
        match self.info.job_disposition {
            JobDisposition::Spool => {}
            JobDisposition::Save => {
                let to = self.info.saving_path.as_deref().ok_or_else(|| PrintError::Failed("no saving path".into()))?;
                return write(to);
            }
            JobDisposition::Preview => {
                let name: String = self.job_name.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
                return write(&std::env::temp_dir().join(format!("bifrost-preview-{name}.pdf")));
            }
        }
        match &self.destination {
            Destination::Pdf(to) => write(to),
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
    fn several_page_ranges_become_one_attribute_with_a_value_per_range() {
        let info = PrintInfo { page_list: vec![(1, 2), (5, 5)], ..Default::default() };
        assert_eq!(info.page_ranges(), vec![(1, 2), (5, 5)]);
        let merged = crate::cups::merge_attributes(info.attributes());
        let ranges = merged.iter().find(|(name, _)| name == "page-ranges").unwrap();
        assert_eq!(ranges.1.len(), 2, "one attribute, one value per range");
        let all = PrintInfo { pages: Some((3, 4)), page_list: Vec::new(), ..Default::default() };
        assert_eq!(all.page_ranges(), vec![(3, 4)]);
        assert!(PrintInfo::default().page_ranges().is_empty());
    }

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
    fn sides_and_colour_are_sent_only_when_they_differ_from_the_default() {
        let plain = PrintInfo::default().attributes();
        assert_eq!(plain.len(), 3, "copies, media and orientation only");
        let duplex = PrintInfo { sides: Sides::TwoSidedShortEdge, color: ColorMode::Monochrome, pages: Some((1, 2)), ..Default::default() }.attributes();
        let names: Vec<&str> = duplex.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["copies", "media", "orientation-requested", "page-ranges", "sides", "print-color-mode"]);
        assert_eq!(duplex[4].1, Value::keyword("two-sided-short-edge"));
        assert_eq!(duplex[5].1, Value::keyword("monochrome"));
        assert_eq!(Sides::TwoSidedLongEdge.keyword(), "two-sided-long-edge");
    }

    #[test]
    fn paper_names_read_back_from_media() {
        assert_eq!(PaperSize::from_media(" letter "), Some(PaperSize::Letter));
        assert_eq!(PaperSize::from_media("A3"), Some(PaperSize::A3));
        assert_eq!(PaperSize::from_media("Tabloid"), None);
        for p in PaperSize::ALL {
            assert_eq!(PaperSize::from_media(p.media()), Some(p));
        }
    }

    #[test]
    fn a_view_job_is_cut_by_the_page_range_once() {
        struct Three;
        impl PrintableView for Three {
            fn bounds(&self) -> Rect {
                (0.0, 0.0, 100.0, 2000.0)
            }
            fn draw(&self, _rect: Rect, out: &mut dyn print_pages::DrawOps) {
                out.fill_rect((0.0, 0.0, 10.0, 10.0));
            }
        }
        let info = PrintInfo { margins: [0.0; 4], pages: Some((2, 3)), ..Default::default() };
        let op = PrintOperation::print_operation("Three", &Three, info);
        assert_eq!(op.info.pages, None, "the document is already cut");
        let (_, bytes) = op.document().unwrap();
        assert!(String::from_utf8_lossy(&bytes).contains("/Count 2"));
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

    #[test]
    fn helvetica_widths_cover_printable_ascii() {
        assert_eq!(HELVETICA.len(), 95);
        assert_eq!(advance('a', 1000.0), 556.0, "1000 pt makes one unit a point");
        assert_eq!(advance('W', 1000.0), 944.0);
        assert_eq!(advance('\u{e9}', 1000.0), 556.0, "outside ASCII counts as a digit");
    }

    #[test]
    fn wrap_breaks_at_spaces_and_keeps_empty_paragraphs() {
        // at 1000 pt a 'W' is 944 pt and a space 278 pt: "WW WW" does not fit on one 1000 pt line
        assert_eq!(wrap("WW WW", 1000.0, 1000.0), vec!["WW".to_string(), "WW".to_string()]);
        assert_eq!(wrap("a b", 11.0, 500.0), vec!["a b".to_string()]);
        assert_eq!(wrap("", 11.0, 500.0), vec![String::new()]);
    }

    #[test]
    fn text_pages_fill_pages_by_the_printable_height() {
        let info = PrintInfo::default();
        let (_, _, _, h) = info.imageable();
        let per_page = (h / (TEXT_SIZE * 1.25)).floor() as usize;
        let text = (0..per_page + 2).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");
        let pages = text_pages(&text, TEXT_SIZE, &info);
        assert_eq!(pages.len(), 2);
        assert_eq!((pages[0].width, pages[0].height), (595.0, 842.0));
        let first = String::from_utf8_lossy(&pages[0].content).into_owned();
        assert!(first.starts_with("BT /F0 11 Tf 72.00 "), "the first baseline sits inside the top margin");
        assert!(first.contains("(line 0) Tj"));
        assert!(String::from_utf8_lossy(&pages[1].content).contains("(line 51) Tj"), "the overflow lines start page 2");
    }

    #[test]
    fn a_text_job_lays_out_for_its_page_info_when_it_runs() {
        let op = PrintOperation::text("Notes", "hello").with_info(PrintInfo { orientation: Orientation::Landscape, ..Default::default() });
        let (format, bytes) = op.document().unwrap();
        assert_eq!(format, "application/pdf");
        let text = String::from_utf8_lossy(&bytes).into_owned();
        assert!(text.contains("/MediaBox [0 0 842 595]"));
        assert!(text.contains("(hello) Tj"));
    }

    struct Square;

    impl PrintableView for Square {
        fn bounds(&self) -> Rect {
            (0.0, 0.0, 100.0, 100.0)
        }
        fn draw(&self, rect: Rect, out: &mut dyn print_pages::DrawOps) {
            out.fill_rect(rect);
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bifrost-print-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_scaling_factor_is_kept_inside_its_range() {
        let at = |f: f32| PrintInfo { scaling_factor: f, ..Default::default() }.scaling();
        assert_eq!((at(1.0), at(0.0), at(50.0), at(f32::NAN)), (1.0, 0.1, 10.0, 1.0));
    }

    #[test]
    fn a_view_operation_draws_its_pages_for_the_info() {
        let op = PrintOperation::print_operation("Square", &Square, PrintInfo::default());
        assert!(op.shows_print_panel && op.shows_progress_panel);
        let (format, bytes) = op.document().unwrap();
        assert_eq!(format, "application/pdf");
        assert!(String::from_utf8_lossy(&bytes).contains("/Count 1"));
    }

    #[test]
    fn a_pdf_operation_writes_the_region_without_panels() {
        let dir = scratch("region");
        let op = PrintOperation::pdf_operation(&Square, (10.0, 10.0, 50.0, 40.0), dir.join("r.pdf"));
        assert!(!op.shows_print_panel && !op.shows_progress_panel);
        op.run().unwrap();
        assert!(String::from_utf8_lossy(&std::fs::read(dir.join("r.pdf")).unwrap()).contains("/MediaBox [0 0 50 40]"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn save_disposition_writes_to_the_saving_path_and_needs_one() {
        let dir = scratch("save");
        let mut info = PrintInfo { job_disposition: JobDisposition::Save, ..Default::default() };
        let op = PrintOperation::print_operation("Square", &Square, info.clone());
        assert_eq!(op.run(), Err(PrintError::Failed("no saving path".into())));
        info.saving_path = Some(dir.join("s.pdf"));
        let op = PrintOperation::print_operation("Square", &Square, info);
        assert_eq!(op.run(), Ok(dir.join("s.pdf").to_string_lossy().into_owned()));
        assert!(std::fs::read(dir.join("s.pdf")).unwrap().starts_with(b"%PDF"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn preview_disposition_returns_a_temporary_pdf() {
        let info = PrintInfo { job_disposition: JobDisposition::Preview, ..Default::default() };
        let path = PrintOperation::print_operation("Preview test", &Square, info).run().unwrap();
        assert!(path.ends_with("bifrost-preview-Previewtest.pdf"));
        assert!(std::fs::read(&path).unwrap().starts_with(b"%PDF"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn run_modal_applies_the_panel_and_reports_through_did_run() {
        let dir = scratch("modal");
        let mut ran = None;
        let op = PrintOperation::text("Notes", "hello").to(Destination::Pdf(dir.join("m.pdf")));
        let r = op.run_modal(
            |info| {
                info.orientation = Orientation::Landscape;
                true
            },
            |ok| ran = Some(ok),
        );
        assert!(matches!(r, Ok(Some(_))));
        assert_eq!(ran, Some(true));
        assert!(String::from_utf8_lossy(&std::fs::read(dir.join("m.pdf")).unwrap()).contains("/MediaBox [0 0 842 595]"));

        let mut ran = None;
        let cancelled = PrintOperation::text("Notes", "hello").to(Destination::Pdf(dir.join("n.pdf")));
        assert_eq!(cancelled.run_modal(|_| false, |ok| ran = Some(ok)), Ok(None));
        assert_eq!(ran, Some(false));
        assert!(!dir.join("n.pdf").exists());

        let mut asked = false;
        let mut quiet = PrintOperation::text("Notes", "hello").to(Destination::Pdf(dir.join("q.pdf")));
        quiet.shows_print_panel = false;
        quiet
            .run_modal(
                |_| {
                    asked = true;
                    true
                },
                |_| {},
            )
            .unwrap();
        assert!(!asked);
        let _ = std::fs::remove_dir_all(dir);
    }
}
