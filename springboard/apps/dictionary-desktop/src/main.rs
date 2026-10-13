//! Dictionary: looks words up in the StarDict dictionaries installed in the app's `dictionaries` folder, and in the
//! system's `/usr/share/stardict` folder. Type a word to see its suggestions, Return to look it up, or Look Up
//! Clipboard. The `look-up` intent returns a definition to other callers.

use bifrost_app::intents::{self, Intent, IntentResult, Param, ParamType};
use bifrost_app::control::{RpcError, INVALID_PARAMS};
use bifrost_app::kit::canvas::{Canvas, RectF};
use bifrost_app::kit::ui::{Ui, Weight, CONTENT_BG, FIELD_BG, LABEL, LABEL_2, MAC_BODY_PT, MAC_FOOTNOTE_PT, MAC_SUBHEAD_PT};
use bifrost_app::kit::{Damage, Scroller};
use bifrost_app::widgets;
use bifrost_app::{App, AppMenu, AppMenuItem, Command, Ctx, Input, KeyInput, TouchKind};
use bifrost_core::json::Json;
use bifrost_ui::stardict::{load_library, Hit, Library};
use std::path::Path;

const MENU_LOOK_UP_CLIPBOARD: u32 = 1;
const MENU_COPY_DEFINITION: u32 = 2;
const MENU_CLEAR: u32 = 3;

/// Most suggestions listed under the search field.
const SUGGESTIONS: usize = 12;

/// Layout in points: the search field, the list below it and the footer line.
const FIELD_TOP: f32 = 12.0;
const FIELD_H: f32 = 32.0;
const PAD: f32 = 16.0;
const LIST_TOP: f32 = 56.0;
const FOOTER_H: f32 = 28.0;
/// Height of one suggestion row, in points.
const ROW_H: f32 = 28.0;
/// Line advance of wrapped text, as a multiple of its size.
const LEADING: f32 = 1.35;

/// The lookup on screen: the word and what each dictionary says about it.
struct Shown {
    word: String,
    hits: Vec<Hit>,
}

/// One drawn line: its text, font size, weight, colour and the vertical space it takes, in points.
struct Line {
    text: String,
    pt: f32,
    weight: Weight,
    colour: u32,
    h: f32,
}

struct DictionaryApp {
    library: Option<Library>,
    /// Where the app's own dictionaries are kept, shown in the empty state.
    folder: String,
    query: String,
    /// The lookup on screen. `None` while the query is being typed: the suggestions show instead.
    shown: Option<Shown>,
    suggestions: Vec<String>,
    scroll: Scroller,
}

/// Pointer position in points: input is in pixels, and the layout is in points.
fn to_points(ctx: &Ctx, x: f32, y: f32) -> (f32, f32) {
    let scale = ctx.ui.pt(1.0).max(0.001);
    (x / scale, y / scale)
}

impl DictionaryApp {
    fn new() -> DictionaryApp {
        DictionaryApp {
            library: None,
            folder: String::new(),
            query: String::new(),
            shown: None,
            suggestions: Vec::new(),
            scroll: Scroller::new(),
        }
    }

    /// The query changed: drop the lookup on screen and list the words that start with what is typed.
    fn edited(&mut self) -> bool {
        self.shown = None;
        self.suggestions = self.library.as_ref().map(|l| l.suggest(&self.query, SUGGESTIONS)).unwrap_or_default();
        self.scroll.jump_top();
        true
    }

    /// Looks `word` up and shows the answer. Returns true when the screen needs a frame.
    fn look_up(&mut self, word: &str) -> bool {
        let word = word.trim();
        if word.is_empty() {
            return false;
        }
        let hits = self.library.as_ref().map(|l| l.lookup(word)).unwrap_or_default();
        self.query = word.to_string();
        self.suggestions.clear();
        self.shown = Some(Shown { word: word.to_string(), hits });
        self.scroll.jump_top();
        true
    }

    /// The first line of the clipboard, looked up.
    fn look_up_clipboard(&mut self, ctx: &Ctx) -> bool {
        let text = ctx.pasteboard_string().unwrap_or_default();
        let word = text.lines().next().unwrap_or("").to_string();
        self.look_up(&word)
    }

    /// Copies the definitions on screen as plain text.
    fn copy_definition(&self, ctx: &Ctx) {
        let Some(shown) = &self.shown else { return };
        let text = shown.hits.iter().map(|h| format!("{}\n{}", h.dictionary, h.text)).collect::<Vec<_>>().join("\n\n");
        ctx.set_pasteboard_string(&text);
    }

    /// The text that explains an empty answer.
    fn no_answer(&self, word: &str) -> String {
        let Some(lib) = &self.library else { return String::new() };
        if lib.is_empty() {
            format!("No dictionary is installed yet. Copy StarDict dictionaries (.ifo, .idx and .dict files) into {}.", self.folder)
        } else if lib.contains_word(word) {
            format!("\u{201c}{word}\u{201d} is in the system word list, but no installed dictionary defines it.")
        } else {
            format!("No definition for \u{201c}{word}\u{201d}.")
        }
    }

    /// The lines of the list under the search field, in order, for a `width` points wide list.
    fn lines(&self, ui: &Ui, width: f32) -> Vec<Line> {
        let mut out = Vec::new();
        let body = |text: String, colour: u32| Line { text, pt: MAC_BODY_PT, weight: Weight::Regular, colour, h: MAC_BODY_PT * LEADING };
        let message = |out: &mut Vec<Line>, text: &str, colour: u32| {
            for line in ui.wrap(text, MAC_BODY_PT, Weight::Regular, ui.pt(width - 2.0 * PAD)) {
                out.push(body(line, colour));
            }
        };
        if let Some(shown) = &self.shown {
            if shown.hits.is_empty() {
                message(&mut out, &self.no_answer(&shown.word), LABEL_2.now());
            }
            for hit in &shown.hits {
                out.push(Line { text: hit.dictionary.clone(), pt: MAC_SUBHEAD_PT, weight: Weight::Semibold, colour: LABEL_2.now(), h: 24.0 });
                for paragraph in hit.text.split('\n') {
                    if paragraph.is_empty() {
                        out.push(Line { text: String::new(), pt: MAC_BODY_PT, weight: Weight::Regular, colour: LABEL.now(), h: 8.0 });
                        continue;
                    }
                    for line in ui.wrap(paragraph, MAC_BODY_PT, Weight::Regular, ui.pt(width - 2.0 * PAD)) {
                        out.push(body(line, LABEL.now()));
                    }
                }
                out.push(Line { text: String::new(), pt: MAC_BODY_PT, weight: Weight::Regular, colour: LABEL.now(), h: 18.0 });
            }
        } else if !self.suggestions.is_empty() {
            for word in &self.suggestions {
                out.push(Line { text: word.clone(), pt: MAC_BODY_PT, weight: Weight::Regular, colour: LABEL.now(), h: ROW_H });
            }
        } else if self.query.trim().is_empty() {
            message(&mut out, "Type a word to look it up.", LABEL_2.now());
        } else {
            message(&mut out, &format!("No words start with \u{201c}{}\u{201d}.", self.query.trim()), LABEL_2.now());
        }
        out
    }

    fn touch(&mut self, ctx: &Ctx, kind: TouchKind, x: f32, y: f32, t: f64) -> bool {
        let (_, py) = to_points(ctx, x, y);
        match kind {
            TouchKind::Down => {
                self.scroll.down(py, t);
                false
            }
            TouchKind::Move => {
                self.scroll.moved(py, t);
                true
            }
            // Scroller::up reports a tap (no drag): a suggestion row looks its word up
            TouchKind::Up => {
                if self.scroll.up(py, t) {
                    self.tap_row(py)
                } else {
                    true
                }
            }
            TouchKind::Cancel => {
                self.scroll.up(py, t);
                true
            }
        }
    }

    /// A tap at `py` (points) on the suggestion list looks that word up.
    fn tap_row(&mut self, py: f32) -> bool {
        if self.shown.is_some() || py < LIST_TOP {
            return true;
        }
        let row = ((py - LIST_TOP + self.scroll.offset()) / ROW_H).floor();
        if row < 0.0 {
            return true;
        }
        match self.suggestions.get(row as usize).cloned() {
            Some(word) => self.look_up(&word),
            None => true,
        }
    }

    fn key(&mut self, key: &KeyInput) -> bool {
        match key {
            KeyInput::Text(text) => {
                for c in text.chars().filter(|c| !c.is_control()) {
                    self.query.push(c);
                }
                self.edited()
            }
            KeyInput::Code(8) => {
                if self.query.pop().is_none() {
                    return false;
                }
                self.edited()
            }
            KeyInput::Code(13) => {
                let word = self.query.clone();
                self.look_up(&word)
            }
            KeyInput::Code(27) => {
                if self.query.is_empty() && self.shown.is_none() {
                    return false;
                }
                self.query.clear();
                self.edited()
            }
            _ => false,
        }
    }

    fn menu(&self) -> Vec<AppMenu> {
        let has_answer = self.shown.as_ref().is_some_and(|s| !s.hits.is_empty());
        let items = vec![
            AppMenuItem::new(MENU_LOOK_UP_CLIPBOARD, "Look Up Clipboard").key("cmd+l"),
            AppMenuItem::new(MENU_COPY_DEFINITION, "Copy Definition").enabled(has_answer),
            AppMenuItem::separator(),
            AppMenuItem::new(MENU_CLEAR, "Clear Search").enabled(!self.query.is_empty() || self.shown.is_some()),
        ];
        vec![AppMenu { title: "Dictionary".into(), items }]
    }
}

impl App for DictionaryApp {
    fn id(&self) -> &str {
        "is.olibuijr.dictionary.desktop"
    }

    fn configure(&mut self, ctx: &Ctx) {
        if self.library.is_none() {
            self.folder = ctx.data_dir.join("dictionaries").display().to_string();
            self.library = Some(load_library(&ctx.data_dir));
        }
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        match ev {
            Input::Touch { kind, x, y, t, .. } => self.touch(ctx, kind, x, y, t),
            Input::Key(key) => self.key(&key),
            Input::Command(Command::Copy) => {
                self.copy_definition(ctx);
                false
            }
            Input::Command(Command::Paste) => {
                let text = ctx.pasteboard_string().unwrap_or_default();
                let line = text.lines().next().unwrap_or("").to_string();
                self.query.push_str(&line);
                self.edited()
            }
            _ => false,
        }
    }

    fn main_menu(&self, _ctx: &Ctx) -> Vec<AppMenu> {
        self.menu()
    }

    fn menu_action(&mut self, ctx: &Ctx, id: u32) -> bool {
        match id {
            MENU_LOOK_UP_CLIPBOARD => self.look_up_clipboard(ctx),
            MENU_COPY_DEFINITION => {
                self.copy_definition(ctx);
                false
            }
            MENU_CLEAR => {
                self.query.clear();
                self.edited()
            }
            _ => false,
        }
    }

    fn intents(&self) -> Vec<Intent> {
        vec![Intent::new("look-up", "Look Up", "Looks a word up in the installed dictionaries and returns its definition.").param(Param::new("word", "Word", ParamType::String))]
    }

    fn perform_intent(&mut self, _ctx: Option<&Ctx>, data_dir: &Path, id: &str, params: &Json) -> Result<IntentResult, RpcError> {
        if id != "look-up" {
            return Err(RpcError::new(INVALID_PARAMS, format!("no intent {id}")));
        }
        let word = intents::str_param(params, "word")?.trim().to_string();
        let hits = load_library(data_dir).lookup(&word);
        let Some(first) = hits.first() else {
            return Err(intents::failed(format!("no definition for {word}")));
        };
        let text = first.text.clone();
        Ok(IntentResult::value(Json::Str(text.clone())).with_dialog(&format!("{word}: {text}")))
    }

    fn tick(&mut self, _ctx: &Ctx, t: f64) -> bool {
        if self.scroll.animating() {
            self.scroll.tick(t);
            return true;
        }
        false
    }

    fn next_wake(&self, t: f64) -> Option<f64> {
        self.scroll.animating().then_some(t + 16.0)
    }

    fn draws_canvas(&self) -> bool {
        true
    }

    fn draw_canvas(&mut self, ctx: &Ctx, canvas: &mut dyn Canvas, _t: f64) -> Damage {
        let (cw, ch) = canvas.size();
        let ui = &ctx.ui;
        let scale = ui.pt(1.0).max(0.001);
        let width_pt = cw as f32 / scale;
        let height_pt = ch as f32 / scale;
        let view_h = (height_pt - LIST_TOP - FOOTER_H).max(0.0);

        canvas.fill_rect(RectF::new(0.0, 0.0, cw as f32, ch as f32), CONTENT_BG.now());

        // the search field: a capsule with the query, or the placeholder while it is empty
        let field = RectF::new(ui.pt(PAD), ui.pt(FIELD_TOP), ui.pt(width_pt - 2.0 * PAD), ui.pt(FIELD_H));
        canvas.fill_rrect(field, ui.pt(FIELD_H / 2.0), FIELD_BG.now());
        let (text, colour) = if self.query.is_empty() { ("Search Dictionary", LABEL_2.now()) } else { (self.query.as_str(), LABEL.now()) };
        let fitted = ui.fit(text, MAC_BODY_PT, field.w - ui.pt(PAD));
        let img = ui.text(&fitted, MAC_BODY_PT, Weight::Regular, colour);
        widgets::blit(canvas, &img, (field.x + ui.pt(PAD / 2.0)) as i32, (field.y + (field.h - img.h as f32) / 2.0) as i32);

        // the list, clipped under the field and above the footer
        let lines = self.lines(ui, width_pt);
        let content_h: f32 = lines.iter().map(|l| l.h).sum();
        self.scroll.set_max(content_h - view_h);
        let offset = self.scroll.offset();
        let clip = RectF::new(0.0, ui.pt(LIST_TOP), cw as f32, ui.pt(view_h));
        canvas.push_clip(clip, 0.0);
        let mut y = LIST_TOP - offset;
        for line in &lines {
            if y + line.h > LIST_TOP && y < LIST_TOP + view_h && !line.text.is_empty() {
                let img = ui.text(&line.text, line.pt, line.weight, line.colour);
                let top = ui.pt(y) + (ui.pt(line.h) - img.h as f32) / 2.0;
                widgets::blit(canvas, &img, ui.pt(PAD) as i32, top as i32);
            }
            y += line.h;
        }
        canvas.pop_clip();

        // the footer: what is installed, and any dictionary that could not be opened
        let footer = match &self.library {
            Some(lib) if !lib.is_empty() => {
                let names = lib.dictionaries.len();
                let mut text = format!("{names} dictionar{} · {} words", if names == 1 { "y" } else { "ies" }, lib.word_count());
                if !lib.problems.is_empty() {
                    text.push_str(&format!(" · {} could not be opened", lib.problems.len()));
                }
                text
            }
            _ => "No dictionaries installed".to_string(),
        };
        let img = ui.text(&ui.fit(&footer, MAC_FOOTNOTE_PT, ui.pt(width_pt - 2.0 * PAD)), MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now());
        widgets::blit(canvas, &img, ui.pt(PAD) as i32, (ui.pt(height_pt - FOOTER_H) + (ui.pt(FOOTER_H) - img.h as f32) / 2.0) as i32);

        let mut d = Damage::new(cw as usize, ch as usize);
        d.all();
        d
    }

    /// The shared-memory path: the same routine on the CPU `Surface`, which implements `Canvas`.
    fn draw(&mut self, ctx: &Ctx, s: &mut bifrost_app::kit::Surface, t: f64) -> Damage {
        self.draw_canvas(ctx, s, t)
    }
}

fn main() {
    bifrost_app::run(DictionaryApp::new());
}
