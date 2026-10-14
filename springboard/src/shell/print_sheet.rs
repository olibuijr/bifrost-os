//! File > Print…: the shell's print sheet (`docs/parity/wire5-printing.md`).
//!
//! The sheet prints the desktop as the document. It captures the frame (the same drawing the screenshot uses), splits it
//! into pages that fit the paper's printable area ([`layout`], the model behind `NativePrintInfo.imageableBounds` and the
//! page scale), and shows the first page in a preview with Previous and Next. The printer list comes from the printing
//! service (`bifrost_ui::cups`); a stopped printer is left out, and "Save as PDF" is always last. The Pages row prints
//! every page or only the one in the preview. Print writes the pages into one PDF
//! ([`pdf_pages`]: one RGB image per page, written by the shared `cg::pdf` writer) and sends it through `PrintOperation`
//! to the chosen queue, or saves it to Documents for Save as PDF. Print submits in the background so the desktop does not wait for CUPS.
//!
//! The sheet takes every touch while it is open (`modal_event` in `mod.rs`). Keyboard focus is not wired yet (gap).

use super::ui::{Ui, Weight, CARD_BG, FILL_GRAY, LABEL, LINK, MAC_BODY_PT, MAC_TITLE3_PT, SEPARATOR, WHITE};
use super::Shell;
use crate::gfx::{Image, Surface};
use bifrost_ui::cups::{Client, Printer, PrinterState};
use bifrost_ui::print::{Destination, Orientation, PageImage, PaperSize, PdfPage, PrintError, PrintInfo, PrintOperation};
use std::path::Path;

/// The last printer choice; it writes a PDF rather than sending the job to a queue.
pub const SAVE_AS_PDF: &str = "Save as PDF";
/// Margins on every side of a page, in points (half an inch).
const MARGIN_PT: f32 = 36.0;
/// Scales the page can be printed at, in percent. The sheet cycles through them.
const SCALES: [u32; 3] = [50, 75, 100];
/// The paper sizes the sheet offers, in the order it cycles through them.
const PAPERS: [PaperSize; 5] = [PaperSize::A4, PaperSize::Letter, PaperSize::A3, PaperSize::A5, PaperSize::Legal];
/// The page preview box, in points: the page fits inside it.
const PREVIEW_W_PT: f32 = 384.0;
const PREVIEW_H_PT: f32 = 154.0;
/// The rows, in order: printer, paper, orientation, scale, copies, pages.
const ROWS: usize = 6;
const ROW_NAMES: [&str; ROWS] = ["Printer", "Paper", "Orientation", "Scale", "Copies", "Pages"];

// indices into `PrintSheet::labels`
const L_TITLE: usize = 0;
const L_NAME: usize = 1;
const L_VALUE: usize = L_NAME + ROWS;
const L_PAGE: usize = L_VALUE + ROWS;
const L_PREV: usize = L_PAGE + 1;
const L_NEXT: usize = L_PREV + 1;
const L_CANCEL: usize = L_NEXT + 1;
const L_PRINT: usize = L_CANCEL + 1;

/// Where the job goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// A CUPS queue by name.
    Printer(String),
    /// Save as PDF, into Documents.
    Pdf,
}

/// A printer the sheet offers: its queue is running (a stopped queue is paused by the person, or disabled).
fn usable(p: &Printer) -> bool {
    p.state != PrinterState::Stopped
}

/// The targets for the usable printers, then Save as PDF.
pub fn targets(printers: &[Printer]) -> Vec<Target> {
    printers.iter().filter(|p| usable(p)).map(|p| Target::Printer(p.name.clone())).chain(std::iter::once(Target::Pdf)).collect()
}

/// The index of the default printer among [`targets`], or the first target when there is no default.
pub fn default_index(printers: &[Printer]) -> usize {
    printers.iter().filter(|p| usable(p)).position(|p| p.is_default).unwrap_or(0)
}

/// What the page is printed on: the paper, the orientation and the scale it is drawn at.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Setup {
    pub target: usize,
    pub paper: usize,
    pub landscape: bool,
    pub scale: usize,
    pub copies: u32,
    /// Print only the page in the preview instead of every page.
    pub this_page: bool,
}

/// Where each page of the captured document starts, and how many points one document pixel takes on the paper.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub pt_per_px: f32,
    /// (first row, rows) of each page, in document pixels.
    pub slices: Vec<(usize, usize)>,
}

/// Splits a `img_w` x `img_h` document into pages for `info`, drawn at `scale_pct` percent. A document wider than the
/// printable width is reduced to fit it. Each page takes as many rows as the printable height holds at that size.
pub fn layout(img_w: usize, img_h: usize, info: &PrintInfo, scale_pct: u32) -> Layout {
    let (_, _, iw, ih) = info.imageable();
    let s = scale_pct as f32 / 100.0;
    if img_w == 0 || img_h == 0 || iw <= 0.0 || ih <= 0.0 || s <= 0.0 {
        return Layout { pt_per_px: s, slices: Vec::new() };
    }
    let fit = (iw / (img_w as f32 * s)).min(1.0);
    let pt_per_px = s * fit;
    let page_px = ((ih / pt_per_px).floor() as usize).max(1);
    let mut slices = Vec::new();
    let mut y = 0;
    while y < img_h {
        let h = page_px.min(img_h - y);
        slices.push((y, h));
        y += h;
    }
    Layout { pt_per_px, slices }
}

/// The PDF pages for `pages`: each page is one raster image placed at `origin` (left and top, in points) at `pt_per_px`
/// points a pixel, on a page of `paper` points (width, height). The image is RGB; the PDF writer compresses it.
pub fn pdf_pages(pages: &[Image], paper: (f32, f32), origin: (f32, f32), pt_per_px: f32) -> Vec<PdfPage> {
    pages
        .iter()
        .map(|img| {
            let w = img.w as f32 * pt_per_px;
            let h = img.h as f32 * pt_per_px;
            let y = paper.1 - origin.1 - h;
            let content = format!("q {:.3} 0 0 {:.3} {:.3} {:.3} cm /Im0 Do Q", w, h, origin.0, y);
            let rgb: Vec<u8> = img.px.iter().take(img.w * img.h).flat_map(|&p| [p as u8, (p >> 8) as u8, (p >> 16) as u8]).collect();
            PdfPage {
                width: paper.0 as f64,
                height: paper.1 as f64,
                content: content.into_bytes(),
                images: vec![PageImage { width: img.w, height: img.h, rgb }],
            }
        })
        .collect()
}

/// One print job, ready to run in the background.
pub struct Job {
    pub target: Target,
    pub info: PrintInfo,
    pub pages: Vec<PdfPage>,
}

fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

impl Job {
    /// Sends the job through the print operation. Save as PDF writes `Print <unix>.pdf` into `documents` and returns its
    /// path; a printer gets the job from CUPS and returns the request id.
    pub fn run(self, documents: &Path) -> Result<String, String> {
        let stamp = unix_now();
        let job_name = format!("Print {stamp}");
        let destination = match &self.target {
            Target::Pdf => {
                std::fs::create_dir_all(documents).map_err(|e| e.to_string())?;
                Destination::Pdf(documents.join(format!("Print {stamp}.pdf")))
            }
            Target::Printer(name) => Destination::Printer(Some(name.clone())),
        };
        PrintOperation::pages(job_name, self.pages, Vec::new())
            .with_info(self.info)
            .to(destination)
            .run()
            .map_err(|e: PrintError| format!("{e:?}"))
    }
}

type Rect4 = (f32, f32, f32, f32);

/// The sheet's rectangles in pixels, centred on a `w` x `h` screen. Drawing and hit tests both read them.
struct Geo {
    card: Rect4,
    rows: [Rect4; ROWS],
    preview: Rect4,
    prev: Rect4,
    next: Rect4,
    cancel: Rect4,
    print: Rect4,
}

fn geo(ui: &Ui, w: i32, h: i32) -> Geo {
    let pt = |v: f32| ui.pt(v);
    let (cw, ch) = (pt(440.0), pt(540.0));
    let x0 = ((w as f32 - cw) / 2.0).max(0.0);
    let y0 = ((h as f32 - ch) / 2.0).max(0.0);
    let pad = pt(20.0);
    let inner_w = cw - 2.0 * pad;
    let row_h = pt(30.0);
    let rows_y = y0 + pt(20.0 + 22.0 + 10.0);
    let mut rows = [(0.0, 0.0, 0.0, 0.0); ROWS];
    for (i, r) in rows.iter_mut().enumerate() {
        *r = (x0 + pad, rows_y + i as f32 * row_h, inner_w, row_h);
    }
    let preview_y = rows_y + ROWS as f32 * row_h + pt(12.0);
    let preview_h = pt(170.0);
    let preview = (x0 + pad, preview_y, inner_w, preview_h);
    let nav_y = preview_y + preview_h + pt(10.0);
    let btn = pt(28.0);
    let prev = (x0 + pad, nav_y, btn, btn);
    let next = (x0 + pad + inner_w - btn, nav_y, btn, btn);
    let bottom = y0 + ch - pt(20.0) - pt(30.0);
    let cancel = (x0 + pad, bottom, pt(90.0), pt(30.0));
    let print = (x0 + pad + inner_w - pt(90.0), bottom, pt(90.0), pt(30.0));
    Geo { card: (x0, y0, cw, ch), rows, preview, prev, next, cancel, print }
}

/// The scale that fits a page of `pw` x `ph` points into the preview box, in pixels a point.
fn page_scale(ui: &Ui, pw: f32, ph: f32) -> f32 {
    (ui.pt(PREVIEW_W_PT) / pw).min(ui.pt(PREVIEW_H_PT) / ph)
}

fn inside(r: Rect4, x: f32, y: f32) -> bool {
    x >= r.0 && x < r.0 + r.2 && y >= r.1 && y < r.1 + r.3
}

/// The top-left corner that centres `img` in `r`.
fn centred(img: &Image, r: Rect4) -> (i32, i32) {
    ((r.0 + (r.2 - img.w as f32) / 2.0) as i32, (r.1 + (r.3 - img.h as f32) / 2.0) as i32)
}

/// What a touch on the sheet does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Row(usize),
    Prev,
    Next,
    Cancel,
    Print,
}

/// The result of a touch.
pub enum Outcome {
    /// Nothing under the touch.
    Idle,
    /// A setting changed or the page turned: redraw.
    Redraw,
    /// Cancel: the sheet closes and nothing prints.
    Cancel,
    /// Print: the sheet closes and the job runs.
    Print(Job),
}

/// The print sheet: the captured document, its pages and the print settings.
pub struct PrintSheet {
    image: Image,
    targets: Vec<Target>,
    setup: Setup,
    layout: Layout,
    page: usize,
    /// labels, rendered white and drawn in their colour (see `L_*`)
    labels: Vec<Image>,
    /// the current page, scaled to the preview
    thumb: Image,
}

impl PrintSheet {
    /// The sheet for `image` (the captured desktop) with the enabled `printers` and Save as PDF.
    pub fn new(ui: &Ui, image: Image, printers: &[Printer]) -> PrintSheet {
        let targets = targets(printers);
        let setup = Setup { target: default_index(printers), paper: 0, landscape: false, scale: 2, copies: 1, this_page: false };
        let mut sheet = PrintSheet {
            image,
            targets,
            setup,
            layout: Layout { pt_per_px: 1.0, slices: Vec::new() },
            page: 0,
            labels: Vec::new(),
            thumb: Image::new(1, 1),
        };
        sheet.relayout(ui);
        sheet
    }

    /// The page settings as `PrintInfo`.
    pub fn info(&self) -> PrintInfo {
        PrintInfo {
            paper: PAPERS[self.setup.paper],
            orientation: if self.setup.landscape { Orientation::Landscape } else { Orientation::Portrait },
            margins: [MARGIN_PT; 4],
            copies: self.setup.copies,
            pages: None,
        }
    }

    fn target_label(&self) -> String {
        match &self.targets[self.setup.target] {
            Target::Printer(name) => name.clone(),
            Target::Pdf => SAVE_AS_PDF.to_string(),
        }
    }

    /// Recomputes the pages after a paper, orientation or scale change, then the labels and preview.
    fn relayout(&mut self, ui: &Ui) {
        let info = self.info();
        self.layout = layout(self.image.w, self.image.h, &info, SCALES[self.setup.scale]);
        if self.page >= self.layout.slices.len() {
            self.page = self.layout.slices.len().saturating_sub(1);
        }
        self.refresh(ui);
    }

    /// Rebuilds the preview thumbnail of the current page and the label images.
    fn refresh(&mut self, ui: &Ui) {
        let info = self.info();
        let (pw, ph) = info.paper_size();
        let k = page_scale(ui, pw, ph);
        let ppp = self.layout.pt_per_px;
        self.thumb = match self.layout.slices.get(self.page) {
            Some(&(y, h)) => {
                let slice = self.image.crop(0, y, self.image.w, h);
                let tw = ((self.image.w as f32 * ppp * k).round() as usize).max(1);
                let th = ((h as f32 * ppp * k).round() as usize).max(1);
                slice.resized(tw, th)
            }
            None => Image::new(1, 1),
        };
        let pages = self.layout.slices.len();
        let values = [
            self.target_label(),
            PAPERS[self.setup.paper].media().to_string(),
            if self.setup.landscape { "Landscape".to_string() } else { "Portrait".to_string() },
            format!("{}%", SCALES[self.setup.scale]),
            self.setup.copies.to_string(),
            if self.setup.this_page { format!("Page {}", self.page + 1) } else { "All".to_string() },
        ];
        let mut labels = vec![ui.text("Print", MAC_TITLE3_PT, Weight::Semibold, WHITE)];
        for name in ROW_NAMES {
            labels.push(ui.text(name, MAC_BODY_PT, Weight::Regular, WHITE));
        }
        for value in &values {
            labels.push(ui.text(value, MAC_BODY_PT, Weight::Regular, WHITE));
        }
        labels.push(ui.text(&format!("Page {} of {}", self.page + 1, pages.max(1)), MAC_BODY_PT, Weight::Regular, WHITE));
        labels.push(ui.text("<", MAC_BODY_PT, Weight::Semibold, WHITE));
        labels.push(ui.text(">", MAC_BODY_PT, Weight::Semibold, WHITE));
        labels.push(ui.text("Cancel", MAC_BODY_PT, Weight::Regular, WHITE));
        labels.push(ui.text("Print", MAC_BODY_PT, Weight::Semibold, WHITE));
        self.labels = labels;
    }

    /// The rectangle under (x, y), if any. The sheet takes touches inside its card and ignores the rest.
    pub fn hit(&self, ui: &Ui, w: i32, h: i32, x: f32, y: f32) -> Option<Hit> {
        let g = geo(ui, w, h);
        if inside(g.cancel, x, y) {
            return Some(Hit::Cancel);
        }
        if inside(g.print, x, y) {
            return Some(Hit::Print);
        }
        if inside(g.prev, x, y) {
            return Some(Hit::Prev);
        }
        if inside(g.next, x, y) {
            return Some(Hit::Next);
        }
        g.rows.iter().position(|r| inside(*r, x, y)).map(Hit::Row)
    }

    /// A tap on the sheet: changes the setting or page, cancels, or prints.
    pub fn tap(&mut self, ui: &Ui, w: i32, h: i32, x: f32, y: f32) -> Outcome {
        let Some(hit) = self.hit(ui, w, h, x, y) else { return Outcome::Idle };
        match hit {
            Hit::Cancel => Outcome::Cancel,
            Hit::Print => {
                if self.layout.slices.is_empty() {
                    Outcome::Cancel
                } else {
                    Outcome::Print(self.job())
                }
            }
            Hit::Prev => {
                if self.page > 0 {
                    self.page -= 1;
                    self.refresh(ui);
                }
                Outcome::Redraw
            }
            Hit::Next => {
                if self.page + 1 < self.layout.slices.len() {
                    self.page += 1;
                    self.refresh(ui);
                }
                Outcome::Redraw
            }
            Hit::Row(row) => {
                self.cycle(row);
                self.relayout(ui);
                Outcome::Redraw
            }
        }
    }

    /// Advances one row's setting: printer or Save as PDF, paper, orientation, scale, copies (1 to 9), all pages or the
    /// page in the preview.
    fn cycle(&mut self, row: usize) {
        match row {
            0 => self.setup.target = (self.setup.target + 1) % self.targets.len(),
            1 => self.setup.paper = (self.setup.paper + 1) % PAPERS.len(),
            2 => self.setup.landscape = !self.setup.landscape,
            3 => self.setup.scale = (self.setup.scale + 1) % SCALES.len(),
            4 => self.setup.copies = self.setup.copies % 9 + 1,
            _ => self.setup.this_page = !self.setup.this_page,
        }
    }

    /// The job for the current settings: the pages (every page, or only the one in the preview), as one PDF.
    pub fn job(&self) -> Job {
        let info = self.info();
        let slices: Vec<(usize, usize)> = if self.setup.this_page {
            self.layout.slices.get(self.page).copied().into_iter().collect()
        } else {
            self.layout.slices.clone()
        };
        let pages: Vec<Image> = slices.iter().map(|&(y, h)| self.image.crop(0, y, self.image.w, h)).collect();
        let (l, t, _, _) = info.imageable();
        let pages = pdf_pages(&pages, info.paper_size(), (l, t), self.layout.pt_per_px);
        Job { target: self.targets[self.setup.target].clone(), info, pages }
    }

    pub fn draw(&self, s: &mut Surface, ui: &Ui, w: i32, h: i32) {
        let g = geo(ui, w, h);
        let (cx, cy, cw, ch) = g.card;
        s.fill_rrect(cx, cy, cw, ch, ui.pt(13.0), CARD_BG.now());
        s.stroke_rrect(cx, cy, cw, ch, ui.pt(13.0), 1.0, SEPARATOR);
        let title = &self.labels[L_TITLE];
        s.blit_colored(title, (cx + ui.pt(20.0)) as i32, (cy + ui.pt(20.0)) as i32, LABEL.now(), 255);
        for (i, r) in g.rows.iter().enumerate() {
            let name = &self.labels[L_NAME + i];
            let ny = r.1 + (r.3 - name.h as f32) / 2.0;
            s.blit_colored(name, r.0 as i32, ny as i32, LABEL.now(), 255);
            let value = &self.labels[L_VALUE + i];
            let (vx, vy) = centred(value, (r.0 + r.2 - value.w as f32, r.1, value.w as f32, r.3));
            s.blit_colored(value, vx, vy, LINK, 255);
        }
        let p = g.preview;
        s.fill_rrect(p.0, p.1, p.2, p.3, ui.pt(9.0), FILL_GRAY);
        let info = self.info();
        let (pw, ph) = info.paper_size();
        let k = page_scale(ui, pw, ph);
        let (px, py, page_w, page_h) = (p.0 + (p.2 - pw * k) / 2.0, p.1 + (p.3 - ph * k) / 2.0, pw * k, ph * k);
        s.fill_rrect(px, py, page_w, page_h, 0.0, WHITE);
        s.stroke_rrect(px, py, page_w, page_h, 0.0, 1.0, SEPARATOR);
        let (l, t, _, _) = info.imageable();
        s.blit(&self.thumb, (px + l * k) as i32, (py + t * k) as i32, WHITE, 255);
        let page_label = &self.labels[L_PAGE];
        // the page label is centred between the two arrows
        let (lx, ly) = centred(page_label, (g.prev.0 + g.prev.2, g.prev.1, p.2 - 2.0 * g.prev.2, g.prev.3));
        s.blit_colored(page_label, lx, ly, LABEL.now(), 255);
        let pages = self.layout.slices.len();
        let (prev_x, prev_y) = centred(&self.labels[L_PREV], g.prev);
        s.blit_colored(&self.labels[L_PREV], prev_x, prev_y, if self.page > 0 { LINK } else { super::ui::SECONDARY }, 255);
        let (next_x, next_y) = centred(&self.labels[L_NEXT], g.next);
        s.blit_colored(&self.labels[L_NEXT], next_x, next_y, if self.page + 1 < pages { LINK } else { super::ui::SECONDARY }, 255);
        let (cx2, cy2) = centred(&self.labels[L_CANCEL], g.cancel);
        s.blit_colored(&self.labels[L_CANCEL], cx2, cy2, LINK, 255);
        s.fill_rrect(g.print.0, g.print.1, g.print.2, g.print.3, ui.pt(6.0), LINK);
        let (px2, py2) = centred(&self.labels[L_PRINT], g.print);
        s.blit_colored(&self.labels[L_PRINT], px2, py2, WHITE, 255);
    }
}

impl Shell {
    /// File > Print…: captures the desktop as the document and opens the sheet over it.
    /// Print of a picture (Photos' File > Print): the print sheet over the picture itself.
    pub(super) fn print_image_open(&mut self, img: Image) {
        if self.print_sheet.is_some() {
            return;
        }
        let printers = Client::local().printers().unwrap_or_default();
        self.print_sheet = Some(PrintSheet::new(&self.ui, img, &printers));
        self.dirty = true;
    }

    pub(super) fn print_sheet_open(&mut self, t: f64) {
        if self.print_sheet.is_some() {
            return;
        }
        let mut img = Image::new(self.w as usize, self.h as usize);
        self.drawn = super::Drawn::Nothing;
        self.draw(&mut Surface { w: img.w, h: img.h, stride: img.w, px: &mut img.px, y0: 0 }, t);
        self.drawn = super::Drawn::Nothing;
        // no printing service means no queues: the sheet then offers Save as PDF only
        let printers = Client::local().printers().unwrap_or_default();
        self.print_sheet = Some(PrintSheet::new(&self.ui, img, &printers));
        self.dirty = true;
    }

    /// A touch while the sheet is open: the sheet takes every touch until Cancel or Print.
    pub(super) fn print_sheet_touch(&mut self, x: f32, y: f32) {
        let outcome = match self.print_sheet.as_mut() {
            Some(sheet) => sheet.tap(&self.ui, self.w, self.h, x, y),
            None => return,
        };
        match outcome {
            Outcome::Idle | Outcome::Redraw => {}
            Outcome::Cancel => self.print_sheet = None,
            Outcome::Print(job) => {
                self.print_sheet = None;
                let documents = self.data_dir.join("Documents");
                crate::bare::sys::spawn_named("print-job", move || match job.run(&documents) {
                    Ok(to) => eprintln!("BifrOSt print: {to}"),
                    Err(e) => eprintln!("BifrOSt print: {e}"),
                });
            }
        }
        self.drawn = super::Drawn::Nothing;
        self.dirty = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn printer(name: &str, enabled: bool, is_default: bool) -> Printer {
        Printer {
            name: name.to_string(),
            uri: bifrost_ui::cups::printer_uri(name),
            state: if enabled { PrinterState::Idle } else { PrinterState::Stopped },
            state_message: String::new(),
            accepting_jobs: enabled,
            is_default,
            make_and_model: String::new(),
            location: String::new(),
            info: String::new(),
            device_uri: String::new(),
            queued_jobs: 0,
            state_reasons: Vec::new(),
            supplies: Vec::new(),
            shared: false,
            members: Vec::new(),
            is_class: false,
        }
    }

    fn margins_zero() -> PrintInfo {
        PrintInfo { margins: [0.0; 4], ..Default::default() }
    }

    #[test]
    fn a_page_holds_the_rows_the_printable_height_allows() {
        // A4 is 595 x 842 pt with no margins: a 595 x 1684 document is two full pages at 100 %
        let two = layout(595, 1684, &margins_zero(), 100);
        assert_eq!(two.slices, vec![(0, 842), (842, 842)]);
        assert_eq!(two.pt_per_px, 1.0);
        // at 50 % each row is half a point, so one page holds 1684 rows
        let half = layout(595, 1684, &margins_zero(), 50);
        assert_eq!(half.slices, vec![(0, 1684)]);
        assert_eq!(half.pt_per_px, 0.5);
    }

    #[test]
    fn a_document_wider_than_the_paper_shrinks_to_fit() {
        // 1190 px at 100 % is twice the 595 pt width: it prints at half a point a pixel, so 100 rows fit one page
        let fit = layout(1190, 100, &margins_zero(), 100);
        assert_eq!(fit.pt_per_px, 0.5);
        assert_eq!(fit.slices, vec![(0, 100)]);
        assert!(layout(0, 10, &margins_zero(), 100).slices.is_empty(), "an empty document has no pages");
    }

    #[test]
    fn targets_list_enabled_printers_then_save_as_pdf() {
        let printers = [printer("Office", true, false), printer("Spare", false, true), printer("Lab", true, true)];
        assert_eq!(
            targets(&printers),
            vec![Target::Printer("Office".into()), Target::Printer("Lab".into()), Target::Pdf]
        );
        // the default is Lab (index 1 among the enabled ones); the disabled Spare does not count
        assert_eq!(default_index(&printers), 1);
        assert_eq!(targets(&[]), vec![Target::Pdf]);
        assert_eq!(default_index(&[]), 0);
    }

    #[test]
    fn each_slice_is_one_page_with_its_image_placed_at_the_origin() {
        let pages = [Image::new(2, 2), Image::new(2, 1)];
        let out = pdf_pages(&pages, (595.0, 842.0), (36.0, 36.0), 1.0);
        assert_eq!(out.len(), 2);
        assert_eq!((out[0].width, out[0].height), (595.0, 842.0));
        assert_eq!((out[1].images[0].width, out[1].images[0].height), (2, 1));
        assert_eq!(out[1].images[0].rgb.len(), 2 * 1 * 3);
        // the image's top edge sits 36 pt below the paper's top: y = 842 - 36 - 1
        assert!(String::from_utf8_lossy(&out[1].content).contains("2.000 0 0 1.000 36.000 805.000 cm /Im0 Do"));
    }

    #[test]
    fn the_sheet_hits_its_rows_and_buttons_and_prints_a_pdf_by_default_to_the_printer() {
        let ui = crate::shell::tests::ui();
        let printers = [printer("Office", true, false), printer("Lab", true, true)];
        let mut sheet = PrintSheet::new(&ui, Image::new(1200, 780), &printers);
        let g = geo(&ui, 1200, 780);
        assert_eq!(sheet.setup.target, 1, "Lab is the default");
        assert_eq!(sheet.layout.slices, vec![(0, 780)], "a 1200 px frame fits one page at 100 %");
        assert_eq!(sheet.hit(&ui, 1200, 780, g.print.0 + 1.0, g.print.1 + 1.0), Some(Hit::Print));
        assert_eq!(sheet.hit(&ui, 1200, 780, g.cancel.0 + 1.0, g.cancel.1 + 1.0), Some(Hit::Cancel));
        assert_eq!(sheet.hit(&ui, 1200, 780, g.rows[3].0 + 1.0, g.rows[3].1 + 1.0), Some(Hit::Row(3)));
        assert_eq!(sheet.hit(&ui, 1200, 780, 1.0, 1.0), None, "outside the card hits nothing");
        // tapping the printer row moves to Save as PDF, the last target
        assert!(matches!(sheet.tap(&ui, 1200, 780, g.rows[0].0 + 1.0, g.rows[0].1 + 1.0), Outcome::Redraw));
        assert_eq!(sheet.setup.target, 2);
        match sheet.tap(&ui, 1200, 780, g.print.0 + 1.0, g.print.1 + 1.0) {
            Outcome::Print(job) => {
                assert_eq!(job.target, Target::Pdf);
                assert_eq!(job.pages.len(), 1, "the preview page only");
            }
            _ => panic!("Print prints"),
        }
    }

    #[test]
    fn copies_wrap_from_nine_back_to_one_and_landscape_swaps_the_paper() {
        let ui = crate::shell::tests::ui();
        let mut sheet = PrintSheet::new(&ui, Image::new(100, 100), &[]);
        for _ in 0..9 {
            sheet.cycle(4);
        }
        assert_eq!(sheet.setup.copies, 1, "9 wraps to 1");
        sheet.cycle(2);
        assert!(sheet.info().paper_size().0 > sheet.info().paper_size().1, "landscape is wider than tall");
    }

    #[test]
    fn the_pages_row_prints_the_previewed_page_or_all_of_them() {
        let ui = crate::shell::tests::ui();
        // 400 px wide fits the 523 pt printable width at 100 %; 1684 rows then take three pages of 770 pt
        let mut sheet = PrintSheet::new(&ui, Image::new(400, 1684), &[]);
        assert_eq!(sheet.layout.slices.len(), 3);
        assert_eq!(sheet.job().pages.len(), 3, "all pages by default");
        sheet.page = 1;
        sheet.cycle(5);
        assert!(sheet.setup.this_page);
        assert_eq!(sheet.job().pages.len(), 1, "only the previewed page");
        sheet.cycle(5);
        assert!(!sheet.setup.this_page, "the row toggles back to all pages");
    }

    #[test]
    fn stopped_printers_are_not_offered() {
        let printers = [printer("Off", false, true), printer("On", true, false)];
        assert_eq!(targets(&printers), vec![Target::Printer("On".into()), Target::Pdf]);
        assert_eq!(default_index(&printers), 0, "a stopped default is skipped, so the first usable target is picked");
    }
}
