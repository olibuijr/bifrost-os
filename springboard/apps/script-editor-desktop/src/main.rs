//! Script Editor: edits shell scripts, runs them through the system shell and shows what they print. Documents are
//! opened and saved through the shell's panels; the File menu keeps the recent scripts.

mod buffer;
mod document;
mod runner;
mod view;

use bifrost_app::controls::{Node, Rect as ControlRect, Role};
use bifrost_app::document::{is_menu_id, DocId, DocumentController};
use bifrost_app::kit::canvas::{Canvas, RectF};
use bifrost_app::kit::ui::{
    Ui, Weight, CAPSULE_BG, CARD_BG, DETAIL_BG, LABEL, LABEL_2, LABEL_3, MAC_BODY_PT, MAC_CALLOUT_PT,
    MAC_FOOTNOTE_PT, MAC_HEADLINE_PT, SEPARATOR_C, SYSTEM_RED,
};
use bifrost_app::kit::{clipboard, Damage, Surface};
use bifrost_app::layout::traffic_lights_trailing;
use bifrost_app::proto::CHORD_FLAG;
use bifrost_app::widgets;
use bifrost_app::{App, AppMenu, AppMenuItem, Command, Commands, Ctx, DocumentEvent, Input, KeyInput, TouchKind};
use document::ScriptDocument;
use runner::{Outcome, Report, Run};
use view::{Button, Layout, R};

/// Menu item ids of the Script menu (outside the document controller's range).
const MENU_RUN: u32 = 0x5301;
const MENU_STOP: u32 = 0x5302;

/// Seconds between wake-ups while a script runs, so its result shows without waiting for input.
const POLL_WAKE_SECS: f64 = 0.1;

// key codes the shell sends for keys without text
const KEY_BACKSPACE: u32 = 8;
const KEY_TAB: u32 = 9;
const KEY_RETURN: u32 = 13;
const KEY_END: u32 = 35;
const KEY_HOME: u32 = 36;
const KEY_LEFT: u32 = 37;
const KEY_UP: u32 = 38;
const KEY_RIGHT: u32 = 39;
const KEY_DOWN: u32 = 40;
const KEY_DELETE: u32 = 46;

struct ScriptEditorApp {
    docs: DocumentController<ScriptDocument>,
    /// The script running now, if any.
    run: Option<Run>,
    /// The result of the last finished run.
    last: Option<Report>,
    /// The window size in points, as of the last configure or draw.
    size: (f32, f32),
}

impl ScriptEditorApp {
    fn new() -> Self {
        ScriptEditorApp { docs: DocumentController::new(), run: None, last: None, size: (0.0, 0.0) }
    }

    fn layout_pt(&self) -> Layout {
        view::layout(self.size.0, self.size.1, traffic_lights_trailing())
    }

    /// Runs `f` on the active document. Returns its id, or `None` when no document is open.
    fn with_active(&mut self, f: impl FnOnce(&mut ScriptDocument)) -> Option<DocId> {
        let id = self.docs.active()?;
        f(self.docs.get_mut(id)?);
        Some(id)
    }

    /// Changes the active document and marks it edited. False when no document is open.
    fn edit(&mut self, f: impl FnOnce(&mut ScriptDocument)) -> bool {
        match self.with_active(f) {
            Some(id) => {
                self.docs.mark_edited(id);
                true
            }
            None => false,
        }
    }

    /// The text of the active document, or nothing when no document is open.
    fn active_text(&self) -> String {
        self.docs.active().and_then(|id| self.docs.get(id)).map(|d| d.buffer.text()).unwrap_or_default()
    }

    /// A key without text: caret movement (which is not an edit) or an editing key. Returns whether anything changed
    /// on screen.
    fn key_code(&mut self, code: u32) -> bool {
        match code {
            KEY_BACKSPACE => self.edit(|d| d.buffer.backspace()),
            KEY_DELETE => self.edit(|d| d.buffer.delete()),
            KEY_RETURN => self.edit(|d| d.buffer.newline()),
            KEY_TAB => self.edit(|d| d.buffer.insert(buffer::TAB)),
            KEY_LEFT => self.with_active(|d| d.buffer.left()).is_some(),
            KEY_RIGHT => self.with_active(|d| d.buffer.right()).is_some(),
            KEY_UP => self.with_active(|d| d.buffer.up()).is_some(),
            KEY_DOWN => self.with_active(|d| d.buffer.down()).is_some(),
            KEY_HOME => self.with_active(|d| d.buffer.home()).is_some(),
            KEY_END => self.with_active(|d| d.buffer.end()).is_some(),
            _ => false,
        }
    }

    /// Starts the active document's text as a script. Nothing starts while a run is still going.
    fn run_script(&mut self) -> bool {
        if self.run.is_some() {
            return false;
        }
        let script = self.active_text();
        if self.docs.active().is_none() {
            return false;
        }
        self.run = Some(Run::start(&script, runner::home_dir()));
        self.last = None;
        true
    }

    fn stop_script(&mut self) -> bool {
        match &self.run {
            Some(run) => run.stop(),
            None => false,
        }
    }

    /// Takes the result of a run that has finished. True when the window needs a redraw.
    fn finish_run_if_done(&mut self) -> bool {
        let Some(run) = &self.run else { return false };
        let Some(report) = run.poll() else { return false };
        self.run = None;
        self.last = Some(report);
        true
    }

    /// The status bar's state text.
    fn status_text(&self) -> String {
        if self.run.is_some() {
            "Running\u{2026}".to_string()
        } else if let Some(report) = &self.last {
            runner::status_line(&report.outcome)
        } else {
            "Ready".to_string()
        }
    }

    /// Puts the caret where a touch landed in the editor.
    fn place_caret(&mut self, ui: &Ui, lay: &Layout, px: f32, py: f32) -> bool {
        let shown = ((py - lay.editor.y - view::TEXT_PAD) / view::LINE_H).floor().max(0.0) as usize;
        let x = px - (lay.editor.x + view::GUTTER_W + view::TEXT_PAD);
        self.with_active(|doc| {
            let row = (doc.scroll + shown).min(doc.buffer.line_count() - 1);
            let col = column_at(ui, &doc.buffer.lines()[row], x);
            doc.buffer.place(row, col);
        })
        .is_some()
    }

    fn draw_window(&mut self, ui: &Ui, c: &mut dyn Canvas, width_px: u32, height_px: u32) {
        let p = ui.pt(1.0);
        let lay = view::layout(width_px as f32 / p, height_px as f32 / p, traffic_lights_trailing());
        self.size = (lay.width, lay.height);
        let visible = lay.editor_lines();
        // the caret's line stays on screen
        if let Some(id) = self.docs.active() {
            if let Some(doc) = self.docs.get_mut(id) {
                doc.scroll = view::scroll_to_row(doc.scroll, doc.buffer.row(), visible);
            }
        }
        fill(c, ui, R { x: 0.0, y: 0.0, w: lay.width, h: lay.height }, DETAIL_BG.now());
        fill(c, ui, R { x: 0.0, y: 0.0, w: lay.width, h: view::BAR_H }, CARD_BG.now());
        fill(c, ui, R { x: 0.0, y: view::BAR_H - 1.0, w: lay.width, h: 1.0 }, SEPARATOR_C.now());
        self.draw_buttons(ui, c, &lay);
        self.draw_title(ui, c, &lay);
        self.draw_editor(ui, c, &lay, visible);
        fill(c, ui, R { x: 0.0, y: lay.output.y, w: lay.width, h: 1.0 }, SEPARATOR_C.now());
        self.draw_output(ui, c, &lay);
        self.draw_status(ui, c, &lay);
    }

    fn draw_buttons(&self, ui: &Ui, c: &mut dyn Canvas, lay: &Layout) {
        let running = self.run.is_some();
        for (rect, label, enabled) in [(lay.run, "Run", !running), (lay.stop, "Stop", running)] {
            fill_round(c, ui, rect, view::BUTTON_RADIUS, CAPSULE_BG.now());
            let color = if enabled { LABEL.now() } else { LABEL_3.now() };
            let w = width_pt(ui, label, MAC_BODY_PT, Weight::Regular);
            let y = rect.y + (rect.h - MAC_BODY_PT * 1.2) / 2.0;
            text_at(c, ui, label, rect.x + (rect.w - w) / 2.0, y, MAC_BODY_PT, color);
        }
    }

    /// The document's name, centred in the top band between the buttons and the right edge.
    fn draw_title(&self, ui: &Ui, c: &mut dyn Canvas, lay: &Layout) {
        let Some(id) = self.docs.active() else { return };
        let Some(name) = self.docs.display_name(id) else { return };
        let title = if self.docs.is_edited(id) { format!("{name} (edited)") } else { name };
        let max_w = (lay.width - 2.0 * (lay.stop.x + lay.stop.w)).max(0.0);
        let fitted = ui.fit(&title, MAC_HEADLINE_PT, ui.pt(max_w));
        let w = width_pt(ui, &fitted, MAC_HEADLINE_PT, Weight::Regular);
        let y = (view::BAR_H - MAC_HEADLINE_PT * 1.2) / 2.0;
        text_at(c, ui, &fitted, (lay.width - w) / 2.0, y, MAC_HEADLINE_PT, LABEL.now());
    }

    fn draw_editor(&self, ui: &Ui, c: &mut dyn Canvas, lay: &Layout, visible: usize) {
        let Some(doc) = self.docs.active().and_then(|id| self.docs.get(id)) else { return };
        let lines = doc.buffer.lines();
        let (row, col) = (doc.buffer.row(), doc.buffer.col());
        let scroll = doc.scroll;
        let end = (scroll + visible).min(lines.len());
        let area = lay.editor;
        c.push_clip(rect_px(ui, area), 0.0);
        for i in scroll..end {
            let shown = i - scroll;
            let (x, y) = lay.text_origin(shown);
            let number = (i + 1).to_string();
            let number_w = width_pt(ui, &number, MAC_FOOTNOTE_PT, Weight::Regular);
            let gutter_right = area.x + view::GUTTER_W - view::TEXT_PAD / 2.0;
            text_at(c, ui, &number, gutter_right - number_w, y + 2.0, MAC_FOOTNOTE_PT, LABEL_3.now());
            text_at(c, ui, &lines[i], x, y, MAC_BODY_PT, LABEL.now());
            if i == row {
                let prefix: String = lines[i].chars().take(col).collect();
                let caret_x = x + width_pt(ui, &prefix, MAC_BODY_PT, Weight::Regular);
                c.fill_rect(
                    RectF::new(ui.pt(caret_x), ui.pt(y), ui.pt(1.0).max(1.0), ui.pt(view::LINE_H - 2.0)),
                    LABEL.now(),
                );
            }
        }
        c.pop_clip();
    }

    fn draw_output(&self, ui: &Ui, c: &mut dyn Canvas, lay: &Layout) {
        let o = lay.output;
        fill(c, ui, o, CARD_BG.now());
        let x = o.x + view::TEXT_PAD + 4.0;
        text_at(c, ui, "Output", x, o.y + 6.0, MAC_FOOTNOTE_PT, LABEL_2.now());
        let Some(report) = &self.last else { return };
        let mut lines = runner::output_lines(report);
        if let Outcome::Failed(message) = &report.outcome {
            lines.push((true, message.clone()));
        }
        let start = view::output_tail_start(lines.len(), lay.output_lines());
        c.push_clip(rect_px(ui, o), 0.0);
        for (i, (is_error, line)) in lines[start..].iter().enumerate() {
            let y = o.y + view::OUTPUT_HEAD_H + i as f32 * view::OUTPUT_LINE_H;
            let color = if *is_error { SYSTEM_RED } else { LABEL.now() };
            text_at(c, ui, line, x, y, MAC_CALLOUT_PT, color);
        }
        c.pop_clip();
    }

    fn draw_status(&self, ui: &Ui, c: &mut dyn Canvas, lay: &Layout) {
        let s = lay.status;
        fill(c, ui, s, CARD_BG.now());
        let y = s.y + (view::STATUS_H - MAC_FOOTNOTE_PT * 1.2) / 2.0;
        let position = match self.docs.active().and_then(|id| self.docs.get(id)) {
            Some(doc) => format!("Ln {}, Col {}", doc.buffer.row() + 1, doc.buffer.col() + 1),
            None => String::new(),
        };
        text_at(c, ui, &position, view::TEXT_PAD + 4.0, y, MAC_FOOTNOTE_PT, LABEL_2.now());
        let state = self.status_text();
        let w = width_pt(ui, &state, MAC_FOOTNOTE_PT, Weight::Regular);
        text_at(c, ui, &state, s.w - view::TEXT_PAD - w - 4.0, y, MAC_FOOTNOTE_PT, LABEL_2.now());
    }
}

/// The column of `line` whose left edge is nearest to `x` points from the start of the text.
fn column_at(ui: &Ui, line: &str, x: f32) -> usize {
    let count = line.chars().count();
    let mut best = (0usize, f32::MAX);
    for col in 0..=count {
        let prefix: String = line.chars().take(col).collect();
        let distance = (width_pt(ui, &prefix, MAC_BODY_PT, Weight::Regular) - x).abs();
        if distance < best.1 {
            best = (col, distance);
        }
    }
    best.0
}

fn rect_px(ui: &Ui, r: R) -> RectF {
    RectF::new(ui.pt(r.x), ui.pt(r.y), ui.pt(r.w), ui.pt(r.h))
}

fn fill(c: &mut dyn Canvas, ui: &Ui, r: R, color: u32) {
    c.fill_rect(rect_px(ui, r), color);
}

fn fill_round(c: &mut dyn Canvas, ui: &Ui, r: R, radius: f32, color: u32) {
    c.fill_rrect(rect_px(ui, r), ui.pt(radius), color);
}

/// The width of `s` in points at `pt`.
fn width_pt(ui: &Ui, s: &str, pt: f32, weight: Weight) -> f32 {
    if s.is_empty() {
        return 0.0;
    }
    ui.measure(s, pt, weight) / ui.pt(1.0)
}

/// Draws `s` with its top-left corner at (`x`, `y`) points.
fn text_at(c: &mut dyn Canvas, ui: &Ui, s: &str, x: f32, y: f32, pt: f32, color: u32) {
    if s.is_empty() {
        return;
    }
    let img = ui.text(s, pt, Weight::Regular, color);
    widgets::blit(c, &img, ui.pt(x) as i32, ui.pt(y) as i32);
}

impl App for ScriptEditorApp {
    fn id(&self) -> &str {
        "is.olibuijr.scripteditor.desktop"
    }

    fn configure(&mut self, ctx: &Ctx) {
        let p = ctx.ui.pt(1.0);
        self.size = (ctx.cfg.w as f32 / p, ctx.cfg.h as f32 / p);
        if self.docs.is_empty() {
            self.docs.new_document();
        }
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        match ev {
            Input::Command(Command::Paste) => match clipboard::text() {
                Some(text) => self.edit(|d| d.buffer.insert(&text)),
                None => false,
            },
            Input::Command(_) => false,
            Input::Key(KeyInput::Text(text)) => self.edit(|d| d.buffer.insert(&text)),
            Input::Key(KeyInput::Code(code)) => {
                // chords belong to the menus, which the shell routes as menu actions
                if code & CHORD_FLAG != 0 {
                    return false;
                }
                self.key_code(code)
            }
            Input::Touch { kind: TouchKind::Down, x, y, .. } => {
                let p = ctx.ui.pt(1.0);
                let (px, py) = (x / p, y / p);
                let lay = self.layout_pt();
                match lay.button_at(px, py) {
                    Some(Button::Run) => self.run_script(),
                    Some(Button::Stop) => self.stop_script(),
                    None if lay.editor.contains(px, py) => self.place_caret(&ctx.ui, &lay, px, py),
                    None => false,
                }
            }
            _ => false,
        }
    }

    /// Paste is enabled only when the clipboard has text.
    fn commands(&self, _ctx: &Ctx) -> Commands {
        Commands::default().with(Command::Paste, clipboard::text().is_some())
    }

    fn controls(&self, _ctx: &Ctx) -> Vec<Node> {
        let lay = self.layout_pt();
        let bounds = |r: R| ControlRect { x: r.x, y: r.y, w: r.w, h: r.h };
        vec![
            Node {
                id: 1,
                role: Role::Button,
                label: "Run".to_string(),
                value: String::new(),
                bounds: bounds(lay.run),
                enabled: self.run.is_none(),
            },
            Node {
                id: 2,
                role: Role::Button,
                label: "Stop".to_string(),
                value: String::new(),
                bounds: bounds(lay.stop),
                enabled: self.run.is_some(),
            },
        ]
    }

    fn main_menu(&self, _ctx: &Ctx) -> Vec<AppMenu> {
        let running = self.run.is_some();
        let script = AppMenu {
            title: "Script".to_string(),
            items: vec![
                AppMenuItem::new(MENU_RUN, "Run").key("cmd+r").enabled(!running),
                AppMenuItem::new(MENU_STOP, "Stop").enabled(running),
            ],
        };
        vec![self.docs.file_menu(), script]
    }

    fn menu_action(&mut self, ctx: &Ctx, id: u32) -> bool {
        if is_menu_id(id) {
            return self.docs.menu_action(ctx, id).is_some();
        }
        match id {
            MENU_RUN => self.run_script(),
            MENU_STOP => self.stop_script(),
            _ => false,
        }
    }

    fn document(&mut self, ctx: &Ctx, ev: DocumentEvent) -> bool {
        self.docs.handle(ctx, ev).is_some()
    }

    fn tick(&mut self, ctx: &Ctx, t: f64) -> bool {
        self.docs.tick(ctx, t);
        self.finish_run_if_done()
    }

    fn next_wake(&self, _t: f64) -> Option<f64> {
        self.docs.next_wake()
    }

    fn suspend(&mut self, ctx: &Ctx) {
        self.docs.suspend(ctx);
    }

    /// The window draws through `Canvas`, so the same routine serves the CPU `Surface` and the GPU canvas.
    fn draws_canvas(&self) -> bool {
        true
    }

    fn draw_canvas(&mut self, ctx: &Ctx, canvas: &mut dyn Canvas, _t: f64) -> Damage {
        let (w, h) = canvas.size();
        self.draw_window(&ctx.ui, canvas, w, h);
        let mut d = Damage::new(w as usize, h as usize);
        d.all();
        d
    }

    /// The shared-memory path: the same routine on the CPU `Surface`, which implements `Canvas`.
    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, t: f64) -> Damage {
        self.draw_canvas(ctx, s, t)
    }
}

fn main() {
    bifrost_app::run(ScriptEditorApp::new());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app_with(text: &str) -> ScriptEditorApp {
        let mut app = ScriptEditorApp::new();
        app.docs.new_document();
        app.edit(|d| d.buffer.insert(text));
        app
    }

    #[test]
    fn typing_changes_the_active_document_and_marks_it_edited() {
        let app = app_with("echo hi");
        assert_eq!(app.active_text(), "echo hi");
        let id = app.docs.active().unwrap();
        assert!(app.docs.is_edited(id));
    }

    #[test]
    fn editing_keys_change_the_text_and_arrows_only_move() {
        let mut app = app_with("ab\ncd");
        assert!(app.key_code(KEY_LEFT));
        assert!(app.key_code(KEY_BACKSPACE));
        assert_eq!(app.active_text(), "ab\nd");
        assert!(app.key_code(KEY_TAB));
        assert_eq!(app.active_text(), format!("ab\n{}d", buffer::TAB));
        // Return keeps the indentation the Tab added: the caret line is "    d" with the caret after the spaces
        assert!(app.key_code(KEY_RETURN));
        assert_eq!(app.active_text(), format!("ab\n{t}\n{t}d", t = buffer::TAB));
        assert!(!app.key_code(999));
    }

    #[test]
    fn the_status_says_ready_until_a_run_finishes() {
        let app = ScriptEditorApp::new();
        assert_eq!(app.status_text(), "Ready");
        let mut app = app;
        app.last = Some(Report { stdout: String::new(), stderr: String::new(), outcome: Outcome::Exited(2) });
        assert_eq!(app.status_text(), "Exited with status 2");
    }

    #[test]
    fn running_without_a_document_does_nothing() {
        let mut app = ScriptEditorApp::new();
        assert!(!app.run_script());
        assert!(app.run.is_none());
    }
}
