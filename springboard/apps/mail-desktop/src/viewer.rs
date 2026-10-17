//! Viewer windows: a message opened in a window of its own. Double-click on a message row, or File > New Viewer Window,
//! asks the window for a [`Snapshot`] of the message; the app opens an SDK window (`bifrost_app::app_windows`) and a
//! [`Viewer`] draws the snapshot there. A viewer holds no connection of its own: it reads the text the main window
//! already fetched, so it works from a copy and does not change the mailbox.

use crate::draw::{fit, put, wrap, BODY_LINE_PT, FOOT_LINE_PT, READER_PAD_PT, TITLE_LINE_PT};
use bifrost_app::kit::ui::{CONTENT_BG, LABEL, LABEL_2, MAC_BODY_PT, MAC_FOOTNOTE_PT, MAC_TITLE1_PT, SEPARATOR_C, SPACE_M, SPACE_S};
use bifrost_app::kit::{Scroller, Surface, TextCache, Ui, Weight};
use bifrost_app::app_windows::{WindowId, WindowSpec};
use bifrost_app::TouchKind;

/// The window size in points for a viewer. Viewers open at this size and the shell lets the user resize them.
pub const VIEWER_W: f32 = 560.0;
pub const VIEWER_H: f32 = 640.0;

/// Two taps on the same message within this many seconds count as a double-click.
pub const DOUBLE_TAP_SECS: f64 = 0.4;

/// The part of a message a viewer shows, copied when the viewer opens.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub uid: u32,
    pub subject: String,
    pub from: String,
    pub to: String,
    pub date: String,
    /// The readable text of the message (HTML already turned into text, as the reader shows it).
    pub body: String,
    /// Names of the files of the message, with their sizes as the reader writes them.
    pub files: Vec<String>,
    /// False while the body has not been fetched yet; the viewer says so.
    pub loaded: bool,
}

impl Snapshot {
    /// The window title: the subject, or a placeholder when the message has none.
    pub fn title(&self) -> String {
        if self.subject.trim().is_empty() {
            "(no subject)".to_string()
        } else {
            self.subject.clone()
        }
    }

    /// The window spec the app opens the viewer with.
    pub fn spec(&self) -> WindowSpec {
        WindowSpec::new(&self.title(), VIEWER_W, VIEWER_H)
    }
}

/// One open viewer: its SDK window, what it shows, and its scroll position.
pub struct Viewer {
    pub id: WindowId,
    pub snapshot: Snapshot,
    pub scroll: Scroller,
    labels: TextCache,
    /// The body wrapped at the width it was last laid out at.
    laid: Option<(f32, Vec<String>)>,
}

impl Viewer {
    pub fn new(id: WindowId, snapshot: Snapshot) -> Viewer {
        Viewer { id, snapshot, scroll: Scroller::new(), labels: TextCache::default(), laid: None }
    }

    /// Scrolls the text with a touch. Returns true when the window needs a new frame.
    pub fn touch(&mut self, kind: TouchKind, y: f32, t: f64) -> bool {
        match kind {
            TouchKind::Down => {
                self.scroll.down(y, t);
                false
            }
            TouchKind::Move => {
                self.scroll.moved(y, t);
                true
            }
            TouchKind::Up | TouchKind::Cancel => {
                self.scroll.up(y, t);
                true
            }
        }
    }

    /// Paints the viewer into `s`: the header lines, the files, and the body below them.
    pub fn draw(&mut self, s: &mut Surface, ui: &Ui) {
        let (w, h) = (s.w as f32, s.h as f32);
        let pad = ui.pt(READER_PAD_PT);
        let text_w = (w - 2.0 * pad).max(0.0);
        let x = pad;
        s.fill_rect(0, 0, s.w, s.h, CONTENT_BG.now());

        let snap = self.snapshot.clone();
        let offset = self.scroll.offset();
        let mut y = pad - offset;

        let title = snap.title();
        for line in wrap(ui, &title, text_w, MAC_TITLE1_PT, Weight::Semibold) {
            put(s, &mut self.labels, ui, &line, MAC_TITLE1_PT, Weight::Semibold, LABEL.now(), x, y);
            y += ui.pt(TITLE_LINE_PT);
        }
        y += ui.pt(SPACE_S);
        for (label, value) in [("From", snap.from.as_str()), ("To", snap.to.as_str()), ("Date", snap.date.as_str())] {
            if value.is_empty() {
                continue;
            }
            let text = fit(ui, &format!("{label}: {value}"), MAC_FOOTNOTE_PT, Weight::Regular, text_w);
            put(s, &mut self.labels, ui, &text, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), x, y);
            y += ui.pt(FOOT_LINE_PT);
        }
        y += ui.pt(SPACE_M);
        s.fill_rect(x as usize, y.max(0.0) as usize, text_w.max(0.0) as usize, (ui.pt(1.0) as usize).max(1), SEPARATOR_C.now());
        y += ui.pt(SPACE_M);

        for name in &snap.files {
            let text = fit(ui, name, MAC_FOOTNOTE_PT, Weight::Regular, text_w);
            put(s, &mut self.labels, ui, &text, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), x, y);
            y += ui.pt(FOOT_LINE_PT);
        }
        if !snap.files.is_empty() {
            y += ui.pt(SPACE_M);
        }

        if !snap.loaded {
            put(s, &mut self.labels, ui, "Loading...", MAC_BODY_PT, Weight::Regular, LABEL_2.now(), x, y);
            self.scroll.set_max(0.0);
            return;
        }

        let fresh = matches!(&self.laid, Some((width, _)) if (width - text_w).abs() < 0.5);
        if !fresh {
            let lines = wrap(ui, &snap.body, text_w, MAC_BODY_PT, Weight::Regular);
            self.laid = Some((text_w, lines));
        }
        let line_h = ui.pt(BODY_LINE_PT);
        let top_of_body = y;
        if let Some((_, lines)) = &self.laid {
            for line in lines.iter() {
                if y >= -line_h && y <= h {
                    put(s, &mut self.labels, ui, line, MAC_BODY_PT, Weight::Regular, LABEL.now(), x, y);
                }
                y += line_h;
            }
        }
        // the scroll range is the body's height past the bottom of the window
        let content = y - top_of_body + offset;
        self.scroll.set_max((content - h + pad).max(0.0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_without_a_subject_is_titled_no_subject() {
        let snap = Snapshot { subject: "  ".into(), ..Snapshot::default() };
        assert_eq!(snap.title(), "(no subject)");
        let snap = Snapshot { subject: "Lunch".into(), ..Snapshot::default() };
        assert_eq!(snap.title(), "Lunch");
    }

    #[test]
    fn the_viewer_window_opens_at_the_viewer_size() {
        let spec = Snapshot { subject: "Lunch".into(), ..Snapshot::default() }.spec();
        assert_eq!(spec.title, "Lunch");
        assert_eq!((spec.width, spec.height), (VIEWER_W, VIEWER_H));
    }

    #[test]
    fn a_viewer_keeps_its_window_and_the_text_it_was_given() {
        let snap = Snapshot { uid: 7, body: "hi".into(), loaded: true, ..Snapshot::default() };
        let viewer = Viewer::new(WindowId(1), snap.clone());
        assert_eq!(viewer.id, WindowId(1));
        assert_eq!(viewer.snapshot, snap);
    }
}
