//! Touch on the Terminal's grid: a drag scrolls history, a tap brings the
//! keyboard back, a long press then drag selects cells (row-major, as bifrost
//! terminals do) and the editing menu copies the selection to the shell
//! clipboard or pastes into the shell.

use super::clipboard;
use super::terminal::profile_table::detect_urls;
use super::edit::Action;
use super::editmenu::EditMenu;
use super::textfield::LONG_PRESS_MS;
use super::ui::Ui;
use crate::gfx::Surface;
use crate::scene::STATUS_PT;
use crate::term::{Selection, Terminal};
use termcore::mouse::MouseEvent;

/// Finger travel that turns a press into a drag.
const SLOP_PT: f32 = 10.0;
/// Longest gap between taps on the same cell that still counts as a double or triple tap.
const MULTI_TAP_MS: f64 = 350.0;

#[derive(Clone, Copy, Debug)]
struct Press {
    x: f32,
    y: f32,
    t: f64,
    /// where the last scroll step was taken from, and the leftover px
    last_y: f32,
    carry: f32,
    moved: bool,
    long: bool,
    /// the cell under the finger when it landed
    cell: (usize, usize),
}

#[derive(Default)]
pub struct TermTouch {
    press: Option<Press>,
    menu: Option<EditMenu>,
    /// a finger holding the menu
    menu_touch: bool,
    /// the last tap: its cell, when it landed and how many taps in a row it is (1..=3)
    last_tap: Option<((usize, usize), f64, u8)>,
    /// the web link a single tap landed on; the shell opens it with `take_link`
    link: Option<String>,
}

impl TermTouch {
    pub fn menu(&self) -> Option<&EditMenu> {
        self.menu.as_ref()
    }

    /// The web link the last plain tap landed on, once: a tap on a link opens it.
    pub fn take_link(&mut self) -> Option<String> {
        self.link.take()
    }

    pub fn down(&mut self, ui: &Ui, term: &mut Terminal, x: f32, y: f32, t: f64) {
        if let Some(m) = &mut self.menu {
            if m.contains(ui, x, y) {
                m.press(x, y);
                self.menu_touch = true;
                return;
            }
        }
        self.menu = None;
        self.link = None;
        term.select(None);
        self.press = Some(Press { x, y, t, last_y: y, carry: 0.0, moved: false, long: false, cell: term.cell_at(x, y) });
    }

    /// Before a long press the finger scrolls (down reveals older lines);
    /// after one it extends the selection.
    pub fn moved(&mut self, ui: &Ui, term: &mut Terminal, x: f32, y: f32) {
        if self.menu_touch {
            return;
        }
        let Some(p) = &mut self.press else { return };
        if p.long {
            term.select(Some(Selection { anchor: p.cell, head: term.cell_at(x, y) }));
            return;
        }
        if !p.moved && (x - p.x).hypot(y - p.y) > ui.pt(SLOP_PT) {
            p.moved = true;
        }
        let px = p.carry + y - p.last_y;
        let lines = (px / term.line_px()).trunc();
        term.scroll(lines as i32);
        p.last_y = y;
        p.carry = px - lines * term.line_px();
    }

    /// The finger lifted; true for a plain tap (the keyboard should return).
    /// A long press ends in the menu: Copy over a dragged selection, Paste
    /// when the clipboard has text.
    pub fn up(&mut self, ui: &Ui, term: &mut Terminal, x: f32, y: f32, screen_w: f32) -> bool {
        if self.menu_touch {
            self.menu_touch = false;
            if let Some(action) = self.menu.as_mut().and_then(|m| m.release(x, y)) {
                self.menu = None;
                self.act(action, term);
            }
            return false;
        }
        let Some(p) = self.press.take() else { return false };
        if p.long {
            self.show_menu(ui, term, p.cell, screen_w);
            return false;
        }
        if !p.moved {
            self.tap(term, p.cell, p.t);
        }
        !p.moved
    }

    /// Counts taps on one cell: a second one selects its word, a third its line (as Terminal does).
    /// A tap on another cell, or after the window, starts again at one.
    fn tap(&mut self, term: &mut Terminal, cell: (usize, usize), t: f64) {
        // A program that asked for the mouse (and is allowed it) gets the click; the tap then selects and opens nothing.
        if term.report_mouse(MouseEvent::Press, cell) {
            term.report_mouse(MouseEvent::Release, cell);
            self.last_tap = None;
            return;
        }
        let count = match self.last_tap {
            Some((last, at, n)) if last == cell && t - at < MULTI_TAP_MS => n % 3 + 1,
            _ => 1,
        };
        self.last_tap = Some((cell, t, count));
        match count {
            1 => self.link = link_at(term, cell),
            2 => term.select_word(cell),
            3 => term.select_line(cell.1),
            _ => {}
        }
    }

    fn act(&mut self, action: Action, term: &mut Terminal) {
        match action {
            Action::Copy => {
                if let Some(text) = term.selected_text() {
                    clipboard::set_text(&text);
                }
                term.select(None);
            }
            Action::Paste => {
                if let Some(text) = clipboard::text() {
                    term.paste(&text);
                }
            }
            Action::Cut | Action::Select | Action::SelectAll => {}
        }
    }

    fn show_menu(&mut self, ui: &Ui, term: &mut Terminal, cell: (usize, usize), screen_w: f32) {
        let mut actions = Vec::new();
        let anchor = match term.selection().filter(|s| s.anchor != s.head) {
            Some(sel) => {
                actions.push(Action::Copy);
                let ((x0, y0), _) = sel.ordered();
                let (start, end) = sel.span_in_row(y0, term.cols()).unwrap_or((x0, x0 + 1));
                let (x, y, cw, ch) = term.cell_rect(start, y0);
                (x, y, (end - start) as f32 * cw, ch)
            }
            None => {
                term.select(None);
                term.cell_rect(cell.0, cell.1)
            }
        };
        if clipboard::global().as_ref().is_some_and(|c| c.has_text()) {
            actions.push(Action::Paste);
        }
        self.menu = (!actions.is_empty()).then(|| EditMenu::new(ui, &actions, anchor, screen_w, ui.pt(STATUS_PT)));
    }

    /// Fires a pending long press: the cell under the finger is selected
    /// and dragging now extends it. True when the screen changed.
    pub fn tick(&mut self, term: &mut Terminal, t: f64) -> bool {
        let Some(p) = &mut self.press else { return false };
        if p.moved || p.long || t - p.t < crate::shell::accessibility::long_press_ms(LONG_PRESS_MS) {
            return false;
        }
        p.long = true;
        term.select(Some(Selection { anchor: p.cell, head: p.cell }));
        true
    }

    /// When `tick` is next due for a pending long press.
    pub fn next_wake(&self) -> Option<f64> {
        self.press.filter(|p| !p.moved && !p.long).map(|p| p.t + crate::shell::accessibility::long_press_ms(LONG_PRESS_MS))
    }

    /// Typing dismisses the menu (the terminal drops its selection itself).
    pub fn dismiss(&mut self) {
        self.menu = None;
        self.menu_touch = false;
    }

    pub fn draw(&self, s: &mut Surface, ui: &Ui) {
        if let Some(m) = &self.menu {
            m.draw(s, ui);
        }
    }
}

/// The http(s) link under `cell`, if its row has one there.
fn link_at(term: &Terminal, (col, row): (usize, usize)) -> Option<String> {
    detect_urls(&term.row_text(row))
        .into_iter()
        .find(|(start, end, url)| (*start..*end).contains(&col) && (url.starts_with("http://") || url.starts_with("https://")))
        .map(|(_, _, url)| url)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ui() -> Ui {
        crate::shell::tests::ui()
    }

    fn font() -> &'static [u8] {
        Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/SFUIMono.ttf")).unwrap().into_boxed_slice())
    }

    fn terminal(cmd: &[&str]) -> Terminal {
        Terminal::spawn(font(), (0, 100, 1200, 800), 24.0, cmd).unwrap()
    }

    fn pump_until(t: &mut Terminal, want: &str) -> String {
        let start = std::time::Instant::now();
        while !t.viewport_text().contains(want) && start.elapsed().as_secs() < 5 {
            crate::bare::sys::wait(&[t.fd()], 50);
            t.pump();
        }
        t.viewport_text()
    }

    fn centre(term: &Terminal, col: usize, row: usize) -> (f32, f32) {
        let (x, y, w, h) = term.cell_rect(col, row);
        (x + w / 2.0, y + h / 2.0)
    }

    #[test]
    fn long_press_drag_selects_and_copy_fills_the_clipboard() {
        let _g = clipboard::tests::exclusive();
        let ui = ui();
        let mut term = terminal(&["/bin/sh", "-c", "printf 'first line\\nsecond   \\n'; read x"]);
        pump_until(&mut term, "second");
        let mut tt = TermTouch::default();
        let (x, y) = centre(&term, 0, 0);
        tt.down(&ui, &mut term, x, y, 1000.0);
        assert_eq!(tt.next_wake(), Some(1400.0));
        assert!(!tt.tick(&mut term, 1399.0));
        assert!(tt.tick(&mut term, 1400.0));
        assert_eq!(term.selection(), Some(Selection { anchor: (0, 0), head: (0, 0) }));
        let (x1, y1) = centre(&term, 3, 1);
        tt.moved(&ui, &mut term, x1, y1);
        assert_eq!(term.selection().map(|s| s.head), Some((3, 1)));
        assert!(!tt.up(&ui, &mut term, x1, y1, 1200.0), "a selection is not a tap");
        let menu = tt.menu().expect("Copy menu");
        let (mx, my, mw, mh) = menu.rect();
        assert_eq!(menu.hit(mx + 5.0, my + 5.0), Some(Action::Copy));
        assert_eq!(menu.hit(mx + mw - 5.0, my + 5.0), Some(Action::Copy), "nothing to paste yet");
        assert!(my + mh < term.cell_rect(0, 0).1, "above the first selected row");
        tt.down(&ui, &mut term, mx + 5.0, my + mh / 2.0, 2000.0);
        assert!(term.selection().is_some(), "pressing the menu keeps the selection");
        tt.up(&ui, &mut term, mx + 5.0, my + mh / 2.0, 1200.0);
        assert_eq!(clipboard::text().as_deref(), Some("first line\nseco"));
        assert!(term.selection().is_none() && tt.menu().is_none());
        let mut buf = vec![0u32; 1200 * 900];
        tt.draw(&mut Surface { w: 1200, h: 900, stride: 1200, px: &mut buf, y0: 0 }, &ui);
    }

    #[test]
    fn long_press_alone_offers_paste_and_pastes_into_the_shell() {
        let _g = clipboard::tests::exclusive();
        let ui = ui();
        let mut term = terminal(&["/bin/sh", "-c", "read x; echo got:$x; read y"]);
        let mut tt = TermTouch::default();
        let (x, y) = centre(&term, 4, 2);
        tt.down(&ui, &mut term, x, y, 0.0);
        tt.tick(&mut term, 500.0);
        assert!(!tt.up(&ui, &mut term, x, y, 1200.0));
        assert!(tt.menu().is_none() && term.selection().is_none(), "nothing on the clipboard: no menu");
        clipboard::set_text("echo hi\n");
        tt.down(&ui, &mut term, x, y, 1000.0);
        tt.tick(&mut term, 1500.0);
        tt.up(&ui, &mut term, x, y, 1200.0);
        let menu = tt.menu().expect("Paste menu");
        let (mx, my, mw, mh) = menu.rect();
        assert_eq!(menu.hit(mx + 5.0, my + 5.0), Some(Action::Paste));
        assert_eq!(menu.hit(mx + mw - 5.0, my + 5.0), Some(Action::Paste));
        assert!((mx + mw / 2.0 - x).abs() < 2.0, "centred on the pressed cell");
        tt.down(&ui, &mut term, mx + 5.0, my + mh / 2.0, 2000.0);
        tt.up(&ui, &mut term, mx + 5.0, my + mh / 2.0, 1200.0);
        assert!(pump_until(&mut term, "got:echo hi").contains("got:echo hi"));
        assert!(tt.menu().is_none());
    }

    #[test]
    fn double_and_triple_taps_select_the_word_and_the_line() {
        let ui = ui();
        let mut term = terminal(&["/bin/sh", "-c", "printf 'hello world\\n'; read x"]);
        pump_until(&mut term, "hello world");
        let mut tt = TermTouch::default();
        let (x, y) = centre(&term, 7, 0);
        tt.down(&ui, &mut term, x, y, 1000.0);
        assert!(tt.up(&ui, &mut term, x, y, 1100.0), "one tap still brings the keyboard back");
        assert_eq!(term.selection(), None, "one tap selects nothing");
        tt.down(&ui, &mut term, x, y, 1150.0);
        assert!(tt.up(&ui, &mut term, x, y, 1200.0));
        assert_eq!(term.selected_text().as_deref(), Some("world"), "double tap selects the word");
        tt.down(&ui, &mut term, x, y, 1300.0);
        tt.up(&ui, &mut term, x, y, 1350.0);
        assert_eq!(term.selected_text().as_deref(), Some("hello world"), "triple tap selects the line");
        let (far_x, _) = centre(&term, 0, 0);
        tt.down(&ui, &mut term, far_x, y, 1400.0);
        tt.up(&ui, &mut term, far_x, y, 1450.0);
        assert_eq!(term.selection(), None, "a tap elsewhere starts the count again");
        tt.down(&ui, &mut term, x, y, 1500.0);
        tt.up(&ui, &mut term, x, y, 1550.0);
        tt.down(&ui, &mut term, x, y, 3000.0);
        tt.up(&ui, &mut term, x, y, 3050.0);
        assert_eq!(term.selection(), None, "a slow second tap is not a double tap");
    }

    #[test]
    fn taps_and_scroll_drags_are_told_apart() {
        let ui = ui();
        let mut term = terminal(&["/bin/sh", "-c", "i=0; while [ $i -lt 80 ]; do i=$((i+1)); echo line$i; done; read x"]);
        pump_until(&mut term, "line80");
        let mut tt = TermTouch::default();
        tt.down(&ui, &mut term, 600.0, 300.0, 0.0);
        assert!(tt.up(&ui, &mut term, 602.0, 301.0, 1200.0), "a tap");
        tt.down(&ui, &mut term, 600.0, 300.0, 100.0);
        for i in 1..=10 {
            tt.moved(&ui, &mut term, 600.0, 300.0 + 40.0 * i as f32);
        }
        assert!(!term.viewport_text().contains("line80"), "scrolled back");
        assert!(!tt.tick(&mut term, 1000.0), "a drag never turns into a long press");
        assert_eq!(tt.next_wake(), None);
        assert!(!tt.up(&ui, &mut term, 600.0, 700.0, 1200.0));
        tt.dismiss();
        assert!(tt.menu().is_none());
    }

    #[test]
    fn a_tap_is_a_click_for_a_program_that_asked_for_the_mouse() {
        let ui = ui();
        let mut term = terminal(&[
            "/bin/sh",
            "-c",
            "/bin/stty raw -echo; printf '\\033[?1000h\\033[?1006h'; echo ready; /bin/head -c 18 | /bin/od -An -c; read x",
        ]);
        pump_until(&mut term, "ready");
        let mut tt = TermTouch::default();
        let (x, y) = centre(&term, 6, 0);
        tt.down(&ui, &mut term, x, y, 1000.0);
        assert!(tt.up(&ui, &mut term, x, y, 1100.0));
        tt.down(&ui, &mut term, x, y, 1150.0);
        tt.up(&ui, &mut term, x, y, 1200.0);
        assert_eq!(term.selection(), None, "a double tap selects nothing while the program has the mouse");
        let screen = pump_until(&mut term, "1   m");
        assert!(screen.contains("1   M") && screen.contains("1   m"), "press and release reached the program: {screen:?}");
    }

    #[test]
    fn with_mouse_reporting_off_a_tap_stays_with_the_terminal() {
        let ui = ui();
        let mut term = terminal(&["/bin/sh", "-c", "printf '\\033[?1000h\\033[?1006hhello world\\n'; read x"]);
        pump_until(&mut term, "hello world");
        term.set_mouse_reporting(false);
        let mut tt = TermTouch::default();
        let (x, y) = centre(&term, 1, 0);
        tt.down(&ui, &mut term, x, y, 1000.0);
        tt.up(&ui, &mut term, x, y, 1100.0);
        tt.down(&ui, &mut term, x, y, 1150.0);
        tt.up(&ui, &mut term, x, y, 1200.0);
        assert_eq!(term.selected_text().as_deref(), Some("hello"), "the double tap selects the word");
    }

    #[test]
    fn a_tap_on_a_link_takes_its_url_once() {
        let ui = ui();
        let mut term = terminal(&["/bin/sh", "-c", "printf 'see https://a.io/x.\\n'; read x"]);
        pump_until(&mut term, "https://");
        let mut tt = TermTouch::default();
        let (x, y) = centre(&term, 6, 0);
        tt.down(&ui, &mut term, x, y, 1000.0);
        assert!(tt.up(&ui, &mut term, x, y, 1200.0), "a tap still brings the keyboard back");
        assert_eq!(tt.take_link().as_deref(), Some("https://a.io/x"));
        assert_eq!(tt.take_link(), None, "taken once");
        let (x, y) = centre(&term, 1, 0);
        tt.down(&ui, &mut term, x, y, 2000.0);
        tt.up(&ui, &mut term, x, y, 2200.0);
        assert_eq!(tt.take_link(), None, "a tap on plain text is no link");
    }
}
