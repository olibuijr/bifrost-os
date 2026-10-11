//! The status bar: path bar, running job, notice, item count and selection.
use crate::*;

/// The status bar's height when shown (View > Show Status Bar, ⌘/): one row with the path and the item count.
pub(crate) const FOOTER_H: f32 = 28.;

/// "1 item", "3 items".
pub(crate) fn items_text(n: usize) -> String {
    if n == 1 { "1 item".into() } else { format!("{n} items") }
}

impl Files {
    /// The status bar: path bar and, on the same row, a running job, else a failed action, else the item count.
    pub(crate) fn paint_footer(&mut self, ui: &Ui, s: &mut Surface, visible: &[usize]) {
        let (w, h) = (self.w, self.h);
        let top = self.bottom();
        rect(s, ui, self.side(), top, w - self.side(), FOOTER_H, CARD_BG.now());
        rect(s, ui, self.side(), top, w - self.side(), 1., SEPARATOR_C.now());
        // one row: the path on the left, the status on the right (the path takes what the status leaves)
        let status_w = ((w - self.side()) * 0.4).min(320.);
        let line_y = top + (FOOTER_H - 14.) / 2.;
        let items = self.path_items();
        let refs: Vec<(&str, &str)> = items.iter().map(|(l, g, _)| (l.as_str(), *g)).collect();
        self.path_segs = widgets::path_bar(s, ui, Rect { x: self.side() + 12., y: top + (FOOTER_H - 20.) / 2., w: (w - self.side() - 36. - status_w).max(0.), h: 20. }, &refs);
        // The status:  a running job, else a failed action, else the item count and selection.
        if self.dragging {
            let n = self.selection.len();
            let line = match &self.drop_target {
                Some(target) => format!("{} {} to {}", if self.fx.copy_drag { "Copy" } else { "Move" }, if n == 1 { "1 item".to_string() } else { format!("{n} items") }, target.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Computer".into())),
                None => "Drag onto a folder or a sidebar place to move; Option copies".into(),
            };
            text(s, ui, &line, 11., LABEL.now(), w - 12. - status_w, line_y, status_w);
        } else if let Some(work) = &self.work {
            let pct = (work.progress.fraction() * 100.).round() as u32;
            let line = if work.progress.current.is_empty() {
                format!("{} · {}% · Esc to cancel", work.title, pct)
            } else {
                format!("{} {} · {}% · Esc to cancel", work.title, work.progress.current, pct)
            };
            text(s, ui, &line, 11., LABEL.now(), w - 12. - status_w, line_y, status_w);
            rect(s, ui, self.side(), h - 2., (w - self.side()) * work.progress.fraction(), 2., TINT);
        } else if let Some(notice) = &self.notice {
            text(s, ui, notice, 11., LABEL.now(), w - 12. - status_w, line_y, status_w);
        } else {
            let mut status = match (&self.hits, self.search.is_some()) {
                (Some(_), true) => format!("{} found so far, searching…", visible.len()),
                (Some(_), false) => format!("{} found in {}", visible.len(), self.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Computer".into())),
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
            text(s, ui, &status, 11., LABEL_2.now(), w - 12. - status_w, line_y, status_w);
        }
    }
}
