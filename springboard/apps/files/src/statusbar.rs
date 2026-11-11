//! The status bar: path bar, running job, notice, item count and selection.
use crate::*;

/// The status bar's height when shown (View > Show Status Bar, ⌘/): one row with the path and the item count.
pub(crate) const FOOTER_H: f32 = 28.;

/// The status bar's text: the subheadline style, one step under the window's body text.
const STATUS_TEXT_PT: f32 = bifrost_app::kit::ui::MAC_SUBHEAD_PT;

/// "1 item", "3 items".
pub(crate) fn items_text(n: usize) -> String {
    if n == 1 { "1 item".into() } else { format!("{n} items") }
}

/// The least width the path bar keeps while a notice is shown (points).
const NOTICE_PATH_MIN: f32 = 120.;

/// The width of the status at the right end of a status bar `bar_w` points wide. It is two fifths of the bar, 320 at
/// most; a notice `notice_w` points wide that does not fit there takes what it needs from the path bar, down to
/// `NOTICE_PATH_MIN` of path, so the reason at its end can be read.
pub(crate) fn status_width(bar_w: f32, notice_w: Option<f32>) -> f32 {
    let usual = (bar_w * 0.4).min(320.);
    match notice_w {
        // 36 is the bar's margins: 12 before the path, 12 between the two, 12 after the status
        Some(n) => n.max(usual).min((bar_w - 36. - NOTICE_PATH_MIN).max(usual)),
        None => usual,
    }
}

impl Files {
    /// The status bar: path bar and, on the same row, a running job, else a failed action, else the item count.
    pub(crate) fn paint_footer(&mut self, ui: &Ui, s: &mut Surface, visible: &[usize]) {
        let (w, h) = (self.w, self.h);
        let top = self.bottom();
        rect(s, ui, self.side(), top, w - self.side(), FOOTER_H, CARD_BG.now());
        rect(s, ui, self.side(), top, w - self.side(), 1., SEPARATOR_C.now());
        // one row: the path on the left, the status on the right (the path takes what the status leaves)
        // a notice wider than the usual status takes its room from the path bar (measured in pixels, plus a point of slack)
        let shown_notice = self.notice.as_deref().filter(|_| !self.dragging && self.work.is_none());
        let notice_w = shown_notice.map(|n| ui.measure(n, STATUS_TEXT_PT, Weight::Regular) / ui.pt(1.).max(0.01) + 2.);
        let status_w = status_width(w - self.side(), notice_w);
        let line_y = top + (FOOTER_H - 14.) / 2.;
        let items = self.path_items();
        let refs: Vec<(&str, &str)> = items.iter().map(|(l, g, _)| (l.as_str(), *g)).collect();
        self.path_segs = widgets::path_bar(s, ui, Rect { x: self.side() + 12., y: top + (FOOTER_H - 20.) / 2., w: (w - self.side() - 36. - status_w).max(0.), h: 20. }, &refs);
        // The status:  a running job, else a failed action, else the item count and selection.
        if self.dragging {
            // a drag from another app copies its items; a drag of this window's items moves them unless Option copies
            // the dragged items, not the selection: a folder that sprang open under the drag has none selected
            let own = if self.fx.drag_paths.is_empty() { self.selection.len() } else { self.fx.drag_paths.len() };
            // another Files window's items move here as this window's own do (`DragOut::sibling` says when they copy)
            let (n, copying) = match self.dnd.sibling {
                Some(copy) => (own, copy),
                None if self.dnd.foreign.is_some() => (self.dnd.foreign_items, true),
                None => (own, self.fx.copy_drag),
            };
            let line = match &self.drop_target {
                Some(target) => format!("{} {} to {}", if copying { "Copy" } else { "Move" }, if n == 1 { "1 item".to_string() } else { format!("{n} items") }, target.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Computer".into())),
                None => "Drag onto a folder or a sidebar place to move; Option copies".into(),
            };
            text(s, ui, &line, STATUS_TEXT_PT, LABEL.now(), w - 12. - status_w, line_y, status_w);
        } else if let Some(work) = &self.work {
            let pct = (work.progress.fraction() * 100.).round() as u32;
            let line = if work.progress.current.is_empty() {
                format!("{} · {}% · Esc to cancel", work.title, pct)
            } else {
                format!("{} {} · {}% · Esc to cancel", work.title, work.progress.current, pct)
            };
            text(s, ui, &line, STATUS_TEXT_PT, LABEL.now(), w - 12. - status_w, line_y, status_w);
            rect(s, ui, self.side(), h - 2., (w - self.side()) * work.progress.fraction(), 2., accent_now());
        } else if let Some(notice) = &self.notice {
            text(s, ui, notice, STATUS_TEXT_PT, LABEL.now(), w - 12. - status_w, line_y, status_w);
        } else {
            let mut status = match (&self.hits, self.search.is_some()) {
                (Some(_), true) => format!("{} found so far, searching…", visible.len()),
                (Some(_), false) => {
                    // the walk stops at the result cap, so a full list is the first hits, not all of them
                    let place = self.scope_label(self.effective_scope());
                    if visible.len() >= RESULT_CAP { format!("First {} found in {place}", visible.len()) } else { format!("{} found in {place}", visible.len()) }
                }
                (None, _) => items_text(visible.len()),
            };
            if self.selection.len() > 1 {
                status.push_str(&format!(", {} selected", self.selection.len()));
            } else if let Some(e) = self.focus_entry() {
                status.push_str(", 1 selected");
                if !e.is_dir {
                    status.push_str(&format!(" · {}", fs::size_text(e.size)));
                }
            }
            text(s, ui, &status, STATUS_TEXT_PT, LABEL_2.now(), w - 12. - status_w, line_y, status_w);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_status_is_two_fifths_of_the_bar_and_320_at_most() {
        assert_eq!(status_width(500., None), 200.);
        assert_eq!(status_width(800., None), 320.);
        assert_eq!(status_width(2000., None), 320.);
        assert_eq!(status_width(800., Some(100.)), 320., "a short notice leaves the status where it is");
    }

    #[test]
    fn a_long_notice_takes_its_width_from_the_path_bar_but_leaves_some_path() {
        assert_eq!(status_width(800., Some(450.)), 450., "the whole notice fits");
        assert_eq!(status_width(800., Some(5000.)), 800. - 36. - NOTICE_PATH_MIN, "the path keeps its least width");
        assert_eq!(status_width(300., Some(5000.)), 144.);
        // a bar too narrow for the notice and the least path keeps the usual split
        assert_eq!(status_width(200., Some(5000.)), 80.);
    }
}
