//! The Export As panel: the format, the JPEG quality and the export size as rows of segmented buttons, and an Export
//! button, on the right of the window. This module holds the choices and the geometry as plain data. The window draws
//! the buttons it returns and runs the action a click on one of them returns.

use crate::canvas::Action;

/// The formats Export As writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Png,
    Jpeg,
    Pdf,
}

impl Format {
    /// The name on the format's segment.
    pub fn label(self) -> &'static str {
        match self {
            Format::Png => "PNG",
            Format::Jpeg => "JPEG",
            Format::Pdf => "PDF",
        }
    }

    /// The label of the Export button for this format.
    pub fn export_label(self) -> &'static str {
        match self {
            Format::Png => "Export PNG",
            Format::Jpeg => "Export JPEG",
            Format::Pdf => "Export PDF",
        }
    }

    /// Whether the JPEG quality applies to this format.
    pub fn uses_quality(self) -> bool {
        self == Format::Jpeg
    }

    /// Whether the export size applies: a PDF keeps its own page size.
    pub fn uses_scale(self) -> bool {
        self != Format::Pdf
    }
}

/// What the panel shows: the chosen format, quality and size, and whether a document is open to export.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Choices {
    pub format: Format,
    pub quality: u8,
    pub scale: f32,
    pub open: bool,
}

/// The sizes of the panel: its left edge, its top (under the toolbar), its width, the inner padding, the height of a
/// button row, the height of a heading and the gap between groups. All in window pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Metrics {
    pub left: i32,
    pub top: i32,
    pub width: i32,
    pub pad: i32,
    pub row: i32,
    pub heading: i32,
    pub gap: i32,
}

/// One button of the panel, at a window position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Button {
    pub action: Action,
    pub label: &'static str,
    pub active: bool,
    pub enabled: bool,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// The whole panel: the headings with their window y, and the buttons.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Panel {
    pub headings: Vec<(i32, &'static str)>,
    pub buttons: Vec<Button>,
}

/// Whether the choice `action` is the one the panel shows as chosen.
fn chosen(action: Action, c: &Choices) -> bool {
    match action {
        Action::ChooseFormat(f) => f == c.format,
        Action::QualityHigh | Action::QualityMedium | Action::QualityLow => action.jpeg_quality() == Some(c.quality),
        _ => action.export_scale() == Some(c.scale),
    }
}

/// The panel's headings and buttons for `c`, laid out from `m`. Quality is only enabled for JPEG and the size only
/// for the formats that scale; the choices are enabled once a document is open, and the Export button is too.
pub fn panel(m: &Metrics, c: &Choices) -> Panel {
    let inner = (m.width - 2 * m.pad).max(1);
    let quality = c.open && c.format.uses_quality();
    let size = c.open && c.format.uses_scale();
    let rows: [(&'static str, Vec<(Action, &'static str, bool)>); 3] = [
        (
            "Format",
            vec![(Action::ChooseFormat(Format::Png), "PNG", c.open), (Action::ChooseFormat(Format::Jpeg), "JPEG", c.open), (Action::ChooseFormat(Format::Pdf), "PDF", c.open)],
        ),
        ("JPEG Quality", vec![(Action::QualityHigh, "High", quality), (Action::QualityMedium, "Medium", quality), (Action::QualityLow, "Low", quality)]),
        (
            "Size",
            vec![(Action::ExportAt25, "25%", size), (Action::ExportAt50, "50%", size), (Action::ExportAt100, "100%", size), (Action::ExportAt200, "200%", size)],
        ),
    ];
    let mut headings = Vec::new();
    let mut buttons = Vec::new();
    let mut y = m.top + m.pad;
    for (title, row) in &rows {
        headings.push((y, *title));
        y += m.heading;
        push_row(&mut buttons, m, row, c, y, inner);
        y += m.row + m.gap;
    }
    buttons.push(Button {
        action: Action::ExportNow,
        label: c.format.export_label(),
        active: false,
        enabled: c.open,
        x: m.left + m.pad,
        y,
        w: inner,
        h: m.row,
    });
    Panel { headings, buttons }
}

/// Adds one row of segmented buttons at window `y`, the segments sharing the inner width equally.
fn push_row(out: &mut Vec<Button>, m: &Metrics, row: &[(Action, &'static str, bool)], c: &Choices, y: i32, inner: i32) {
    let n = row.len().max(1) as i32;
    let seg = inner / n;
    for (i, &(action, label, enabled)) in row.iter().enumerate() {
        out.push(Button { action, label, active: chosen(action, c), enabled, x: m.left + m.pad + i as i32 * seg, y, w: seg, h: m.row });
    }
}

/// The enabled button under a window point, if any.
pub fn hit(panel: &Panel, x: f32, y: f32) -> Option<Action> {
    panel
        .buttons
        .iter()
        .find(|b| b.enabled && x >= b.x as f32 && x < (b.x + b.w) as f32 && y >= b.y as f32 && y < (b.y + b.h) as f32)
        .map(|b| b.action)
}

#[cfg(test)]
mod tests {
    use super::*;

    const METRICS: Metrics = Metrics { left: 600, top: 48, width: 200, pad: 8, row: 24, heading: 16, gap: 8 };

    fn choices(format: Format, open: bool) -> Choices {
        Choices { format, quality: 90, scale: 1.0, open }
    }

    fn find(panel: &Panel, label: &str) -> Button {
        *panel.buttons.iter().find(|b| b.label == label).unwrap_or_else(|| panic!("no button {label}"))
    }

    #[test]
    fn quality_and_size_apply_to_the_formats_that_use_them() {
        let png = panel(&METRICS, &choices(Format::Png, true));
        assert!(!find(&png, "High").enabled, "PNG has no quality");
        assert!(find(&png, "50%").enabled);
        let jpeg = panel(&METRICS, &choices(Format::Jpeg, true));
        assert!(find(&jpeg, "High").enabled);
        let pdf = panel(&METRICS, &choices(Format::Pdf, true));
        assert!(!find(&pdf, "High").enabled && !find(&pdf, "100%").enabled, "a PDF keeps its size and has no quality");
        assert!(find(&pdf, "PDF").enabled, "the format itself is always a choice");
    }

    #[test]
    fn the_chosen_segments_are_marked_active() {
        let mut c = choices(Format::Jpeg, true);
        c.scale = 2.0;
        let p = panel(&METRICS, &c);
        assert!(find(&p, "JPEG").active && !find(&p, "PNG").active);
        assert!(find(&p, "200%").active && !find(&p, "100%").active);
        assert!(find(&p, "High").active, "quality 90 is High");
    }

    #[test]
    fn a_closed_document_disables_every_choice_and_the_export_button() {
        let p = panel(&METRICS, &choices(Format::Png, false));
        assert!(p.buttons.iter().all(|b| !b.enabled), "{p:?}");
        assert_eq!(hit(&p, 650.0, p.buttons.last().map_or(0.0, |b| b.y as f32 + 1.0)), None);
    }

    #[test]
    fn a_click_on_an_enabled_segment_or_the_export_button_returns_its_action() {
        let p = panel(&METRICS, &choices(Format::Png, true));
        let pdf = find(&p, "PDF");
        assert_eq!(hit(&p, pdf.x as f32 + 1.0, pdf.y as f32 + 1.0), Some(Action::ChooseFormat(Format::Pdf)));
        let export = *p.buttons.last().expect("the export button");
        assert_eq!(export.action, Action::ExportNow);
        assert_eq!(export.label, "Export PNG");
        assert_eq!(hit(&p, export.x as f32 + 2.0, export.y as f32 + 2.0), Some(Action::ExportNow));
        assert_eq!(hit(&p, 10.0, 10.0), None, "outside the panel");
    }

    #[test]
    fn every_button_sits_inside_the_panel_width() {
        let p = panel(&METRICS, &choices(Format::Jpeg, true));
        for b in &p.buttons {
            assert!(b.x >= METRICS.left && b.x + b.w <= METRICS.left + METRICS.width, "{b:?}");
        }
        assert_eq!(p.headings.iter().map(|h| h.1).collect::<Vec<_>>(), ["Format", "JPEG Quality", "Size"]);
    }
}
