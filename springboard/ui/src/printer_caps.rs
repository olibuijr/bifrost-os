//! Printer capabilities from an IPP Get-Printer-Attributes reply (Printer: `pageSize(forPaper:)`,
//! `statusForTable`, PMPrinterGetPaperList, PMPaperGetPPDPaperName). Pure model, no I/O.

use crate::cups::PrinterState;
use crate::ipp::Group;

/// A paper size named by its PWG self-describing name, e.g. `na_letter_8.5x11in`. Sizes are in hundredths of a millimetre.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaperSize {
    pub name: String,
    pub width: i32,
    pub height: i32,
}

impl PaperSize {
    pub fn letter() -> PaperSize {
        PaperSize { name: "na_letter_8.5x11in".into(), width: 21590, height: 27940 }
    }

    pub fn a4() -> PaperSize {
        PaperSize { name: "iso_a4_210x297mm".into(), width: 21000, height: 29700 }
    }

    /// Reads a PWG self-describing media name (`class_name_WxHunit`). `None` when the size cannot be read.
    pub fn from_pwg(name: &str) -> Option<PaperSize> {
        let dims = name.rsplit('_').next()?;
        let (body, per_unit) = if let Some(b) = dims.strip_suffix("mm") {
            (b, 100.0)
        } else if let Some(b) = dims.strip_suffix("in") {
            (b, 2540.0)
        } else {
            return None;
        };
        let (w, h) = body.split_once('x')?;
        let (w, h): (f64, f64) = (w.parse().ok()?, h.parse().ok()?);
        if w <= 0.0 || h <= 0.0 {
            return None;
        }
        Some(PaperSize { name: name.to_string(), width: (w * per_unit).round() as i32, height: (h * per_unit).round() as i32 })
    }

    /// PPD paper name (PMPaperGetPPDPaperName); unknown sizes keep the PWG name.
    pub fn ppd_name(&self) -> String {
        let known = [
            ("na_letter_", "Letter"),
            ("na_legal_", "Legal"),
            ("na_executive_", "Executive"),
            ("iso_a3_", "A3"),
            ("iso_a4_", "A4"),
            ("iso_a5_", "A5"),
            ("iso_a6_", "A6"),
            ("iso_b5_", "B5"),
            ("na_number-10_", "Env10"),
            ("iso_dl_", "EnvDL"),
        ];
        known
            .iter()
            .find(|(prefix, _)| self.name.starts_with(prefix))
            .map(|(_, ppd)| ppd.to_string())
            .unwrap_or_else(|| self.name.clone())
    }

    /// Size in points (1/72 inch).
    pub fn points(&self) -> (f64, f64) {
        (self.width as f64 * 72.0 / 2540.0, self.height as f64 * 72.0 / 2540.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DuplexMode {
    None,
    LongEdge,
    ShortEdge,
}

impl DuplexMode {
    /// IPP `sides` keyword.
    pub fn keyword(self) -> &'static str {
        match self {
            DuplexMode::None => "one-sided",
            DuplexMode::LongEdge => "two-sided-long-edge",
            DuplexMode::ShortEdge => "two-sided-short-edge",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorMode {
    Auto,
    Color,
    Monochrome,
}

impl ColorMode {
    /// IPP `print-color-mode` keyword.
    pub fn keyword(self) -> &'static str {
        match self {
            ColorMode::Auto => "auto",
            ColorMode::Color => "color",
            ColorMode::Monochrome => "monochrome",
        }
    }
}

/// Printer `statusForTable`: state, state reasons (without `none`) and the printer's message.
#[derive(Debug, Clone, PartialEq)]
pub struct PrinterStatus {
    pub state: PrinterState,
    pub reasons: Vec<String>,
    pub message: String,
}

impl PrinterStatus {
    /// True when the printer can take a job: idle or processing with no error or stop reason.
    pub fn is_ready(&self) -> bool {
        matches!(self.state, PrinterState::Idle | PrinterState::Processing)
            && !self.reasons.iter().any(|r| r.ends_with("-error") || r.starts_with("offline") || r.starts_with("paused"))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PrinterCapabilities {
    pub papers: Vec<PaperSize>,
    pub default_paper: PaperSize,
    pub duplex: Vec<DuplexMode>,
    pub color_modes: Vec<ColorMode>,
    pub status: PrinterStatus,
    /// Unprintable border the hardware leaves on every sheet, in hundredths of a millimetre: left, bottom, right, top.
    /// The largest `media-*-margin-supported` value of each edge, so the area inside is printable on every paper.
    /// All zero when the printer reports none.
    pub hardware_margins: [i32; 4],
}

fn hundredths_to_points(v: i32) -> f64 {
    v as f64 * 72.0 / 2540.0
}

fn values<'a>(g: &'a Group, name: &str) -> impl Iterator<Item = &'a str> {
    g.attribute(name).into_iter().flat_map(|a| a.values.iter().filter_map(|v| v.as_str()))
}

impl PrinterCapabilities {
    /// Reads a printer-attributes group. Missing attributes fall back to Letter and A4, one-sided, auto colour.
    pub fn from_group(g: &Group) -> PrinterCapabilities {
        let mut papers: Vec<PaperSize> = Vec::new();
        for p in values(g, "media-supported").filter_map(PaperSize::from_pwg) {
            if !papers.contains(&p) {
                papers.push(p);
            }
        }
        if papers.is_empty() {
            papers = vec![PaperSize::letter(), PaperSize::a4()];
        }
        let default_paper = g
            .text("media-default")
            .and_then(PaperSize::from_pwg)
            .filter(|d| papers.iter().any(|p| p.name == d.name))
            .unwrap_or_else(|| papers[0].clone());

        let mut duplex: Vec<DuplexMode> = values(g, "sides-supported")
            .filter_map(|s| match s {
                "one-sided" => Some(DuplexMode::None),
                "two-sided-long-edge" => Some(DuplexMode::LongEdge),
                "two-sided-short-edge" => Some(DuplexMode::ShortEdge),
                _ => None,
            })
            .collect();
        if duplex.is_empty() {
            duplex.push(DuplexMode::None);
        }
        let mut color_modes: Vec<ColorMode> = values(g, "print-color-mode-supported")
            .filter_map(|s| match s {
                "auto" => Some(ColorMode::Auto),
                "color" => Some(ColorMode::Color),
                "monochrome" => Some(ColorMode::Monochrome),
                _ => None,
            })
            .collect();
        if color_modes.is_empty() {
            color_modes.push(ColorMode::Auto);
        }
        let status = PrinterStatus {
            state: PrinterState::from_code(g.int("printer-state").unwrap_or(0)),
            reasons: values(g, "printer-state-reasons").filter(|r| *r != "none").map(str::to_string).collect(),
            message: g.text("printer-state-message").unwrap_or("").to_string(),
        };
        let edge = |name: &str| g.ints(name).into_iter().max().unwrap_or(0).max(0);
        let hardware_margins = [
            edge("media-left-margin-supported"),
            edge("media-bottom-margin-supported"),
            edge("media-right-margin-supported"),
            edge("media-top-margin-supported"),
        ];
        PrinterCapabilities { papers, default_paper, duplex, color_modes, status, hardware_margins }
    }

    /// Printer `pageSize(forPaper:)`: finds a paper by PWG name or PPD name.
    pub fn page_size_for_paper(&self, name: &str) -> Option<&PaperSize> {
        self.papers.iter().find(|p| p.name == name || p.ppd_name() == name)
    }

    /// Printer `paperSize(forPaperName:)`: the paper's width and height in points, by PWG or PPD name.
    pub fn paper_size_in_points(&self, name: &str) -> Option<(f64, f64)> {
        self.page_size_for_paper(name).map(|p| (hundredths_to_points(p.width), hundredths_to_points(p.height)))
    }

    /// Printer `imageableRect(forPaper:)`: the printable rect (x, y, width, height) in points on the named paper, origin
    /// at the lower left corner of the sheet. `None` for a paper the printer does not list.
    pub fn imageable_rect(&self, name: &str) -> Option<(f64, f64, f64, f64)> {
        let (w, h) = self.paper_size_in_points(name)?;
        let [left, bottom, right, top] = self.hardware_margins.map(hundredths_to_points);
        Some((left, bottom, (w - left - right).max(0.0), (h - bottom - top).max(0.0)))
    }

    pub fn supports_duplex(&self) -> bool {
        self.duplex.iter().any(|d| *d != DuplexMode::None)
    }

    pub fn supports_color(&self) -> bool {
        self.color_modes.contains(&ColorMode::Color)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipp::{group, Attribute, Value};

    fn multi(name: &str, vals: &[&str]) -> Attribute {
        Attribute { name: name.into(), values: vals.iter().map(|v| Value::keyword(v)).collect() }
    }

    fn canned() -> Group {
        Group {
            tag: group::PRINTER,
            attributes: vec![
                multi("media-supported", &["na_letter_8.5x11in", "iso_a4_210x297mm", "na_legal_8.5x14in", "bogus"]),
                multi("media-default", &["iso_a4_210x297mm"]),
                multi("sides-supported", &["one-sided", "two-sided-long-edge", "two-sided-short-edge"]),
                multi("print-color-mode-supported", &["monochrome", "color", "auto"]),
                Attribute { name: "printer-state".into(), values: vec![Value::Enum(3)] },
                multi("printer-state-reasons", &["media-low-warning"]),
            ],
        }
    }

    #[test]
    fn pwg_names_give_sizes() {
        let a4 = PaperSize::from_pwg("iso_a4_210x297mm").unwrap();
        assert_eq!((a4.width, a4.height), (21000, 29700));
        let letter = PaperSize::from_pwg("na_letter_8.5x11in").unwrap();
        assert_eq!((letter.width, letter.height), (21590, 27940));
        assert!(PaperSize::from_pwg("bogus").is_none());
        assert!(PaperSize::from_pwg("x_y_0x0mm").is_none());
    }

    #[test]
    fn ppd_names_map_known_sizes() {
        assert_eq!(PaperSize::letter().ppd_name(), "Letter");
        assert_eq!(PaperSize::a4().ppd_name(), "A4");
        assert_eq!(PaperSize::from_pwg("custom_foo_10x20mm").unwrap().ppd_name(), "custom_foo_10x20mm");
    }

    #[test]
    fn capabilities_read_media_sides_colour_and_default() {
        let c = PrinterCapabilities::from_group(&canned());
        assert_eq!(c.papers.len(), 3);
        assert_eq!(c.default_paper, PaperSize::a4());
        assert_eq!(c.duplex, vec![DuplexMode::None, DuplexMode::LongEdge, DuplexMode::ShortEdge]);
        assert!(c.supports_duplex() && c.supports_color());
        assert_eq!(c.page_size_for_paper("Legal").unwrap().height, 35560);
        assert!(c.page_size_for_paper("Tabloid").is_none());
    }

    #[test]
    fn status_maps_state_and_reasons() {
        let c = PrinterCapabilities::from_group(&canned());
        assert_eq!(c.status.state, PrinterState::Idle);
        assert_eq!(c.status.reasons, vec!["media-low-warning".to_string()]);
        assert!(c.status.is_ready());
        let mut g = canned();
        g.attributes.retain(|a| a.name != "printer-state-reasons");
        g.attributes.push(multi("printer-state-reasons", &["media-empty-error"]));
        assert!(!PrinterCapabilities::from_group(&g).status.is_ready());
    }

    #[test]
    fn missing_attributes_fall_back() {
        let c = PrinterCapabilities::from_group(&Group { tag: group::PRINTER, attributes: vec![] });
        assert_eq!(c.papers, vec![PaperSize::letter(), PaperSize::a4()]);
        assert_eq!(c.default_paper, PaperSize::letter());
        assert_eq!(c.duplex, vec![DuplexMode::None]);
        assert_eq!(c.color_modes, vec![ColorMode::Auto]);
        assert!(!c.supports_duplex());
    }

    #[test]
    fn imageable_rect_subtracts_the_hardware_margins() {
        let mut g = canned();
        g.attributes.push(Attribute { name: "media-left-margin-supported".into(), values: vec![Value::Integer(0), Value::Integer(635)] });
        g.attributes.push(Attribute { name: "media-right-margin-supported".into(), values: vec![Value::Integer(635)] });
        g.attributes.push(Attribute { name: "media-top-margin-supported".into(), values: vec![Value::Integer(1270)] });
        let c = PrinterCapabilities::from_group(&g);
        assert_eq!(c.hardware_margins, [635, 0, 635, 1270]);
        let (w, h) = c.paper_size_in_points("A4").unwrap();
        assert!((w - 595.28).abs() < 0.01 && (h - 841.89).abs() < 0.01);
        let (x, y, iw, ih) = c.imageable_rect("iso_a4_210x297mm").unwrap();
        assert!((x - 18.0).abs() < 0.01 && y == 0.0);
        assert!((iw - (w - 36.0)).abs() < 0.01 && (ih - (h - 36.0)).abs() < 0.01);
        assert!(c.imageable_rect("Tabloid").is_none());
    }

    #[test]
    fn a_printer_without_margins_prints_edge_to_edge() {
        let c = PrinterCapabilities::from_group(&canned());
        assert_eq!(c.hardware_margins, [0; 4]);
        let (x, y, w, h) = c.imageable_rect("Letter").unwrap();
        assert_eq!((x, y), (0.0, 0.0));
        assert!((w - 612.0).abs() < 0.01 && (h - 792.0).abs() < 0.01);
    }
}
