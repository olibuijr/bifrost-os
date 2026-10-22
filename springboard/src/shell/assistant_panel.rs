//! The assistant panel: a text field with an answer area under it, drawn over the desktop with the Search card's
//! material (`quicksearch::mac_glass`). A request goes to the backend on a worker thread through `assistant::ask`,
//! and the reply comes back on a channel that `receive` drains on each tick. An intent call is only ever shown as a
//! confirmation card; Run hands it to `assistant::execute` on another worker, and Cancel drops it.
//!
//! The panel's rules live in [`Panel`] and need no screen: nothing is sent while the backend is off or the field is
//! empty, a reply is an answer, a failure or one card, and a card runs once.

use super::assistant::{self, Backend, Call, Catalogue, Connection, Error, Gate, Reply, Settings};
use super::keyboard::Out;
use super::quicksearch::mac_glass;
use super::textfield::TextField;
use super::ui::{Ui, Weight, LABEL, WHITE};
use super::Shell;
use crate::gfx::{rgba, Surface};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::Duration;

/// The Anthropic model the panel asks, the default of the Assistant app.
pub const MODEL: &str = "claude-sonnet-5-5";
/// The keychain app store that holds the API key item (`assistant_client::KEY_SERVICE` / `KEY_ACCOUNT`).
pub const KEY_APP: &str = "is.olibuijr.assistant";
/// Longest question the field takes.
const QUERY_MAX: usize = 512;
/// How long a confirmed call may take to launch its app and answer.
const RUN_WAIT: Duration = Duration::from_secs(10);

const TOP_PT: f32 = 60.0;
const CARD_W_PT: f32 = 560.0;
const MIN_W_PT: f32 = 240.0;
const FIELD_PT: f32 = 52.0;
const FIELD_TEXT_PT: f32 = 22.0;
const FIELD_X_PT: f32 = 24.0;
const FIELD_CLEAR_PT: f32 = 24.0;
const GAP_PT: f32 = 8.0;
const PAD_PT: f32 = 16.0;
const RADIUS_PT: f32 = 13.0;
const BODY_PT: f32 = 13.0;
const LINE_PT: f32 = 18.0;
/// The answer area never grows past this height; longer answers scroll.
const BODY_MAX_PT: f32 = 300.0;
const BUTTON_H_PT: f32 = 28.0;
const RUN_W_PT: f32 = 72.0;
const CANCEL_W_PT: f32 = 84.0;
const BUTTON_GAP_PT: f32 = 8.0;
const BUTTON_LABEL_PT: f32 = 13.0;

/// What the panel shows under the field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    /// nothing asked yet (or the last panel was closed): the field alone
    Idle,
    /// a request is with the backend or a confirmed call is running
    Waiting,
    /// a plain answer, or the result of a run
    Answer(String),
    /// one intent call waiting for Run or Cancel, shown as its card text
    Card(String),
    /// a request, reply or run that failed, with the reason
    Failed(String),
}

/// A result from a worker thread.
pub enum Msg {
    Reply(Result<Reply, Error>),
    Ran(Result<Option<String>, String>),
}

/// Where a tap landed on the panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tap {
    /// outside the panel: it closes
    Outside,
    /// inside, on the field, the answer or empty space
    Inside,
    /// the card's Run button
    Run,
    /// the card's Cancel button
    Cancel,
}

/// The panel's geometry for one frame: all in px.
struct Layout {
    x: f32,
    y: f32,
    w: f32,
    field_h: f32,
    /// the answer area (y, h), when there is one
    body: Option<(f32, f32)>,
    /// the lines shown, after the scroll offset
    lines: Vec<String>,
    /// the Run and Cancel buttons (x, y, w, h), on a card
    run: Option<(f32, f32, f32, f32)>,
    cancel: Option<(f32, f32, f32, f32)>,
}

pub struct Panel {
    shown: bool,
    field: TextField,
    phase: Phase,
    gate: Gate,
    /// earlier turns, kept only while history is on
    turns: Vec<(bool, String)>,
    history: bool,
    /// lines scrolled off the top of the answer
    skip: usize,
    /// the answer that arrived and is not yet handed to the speech service (see `take_answer`)
    to_speak: Option<String>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
}

impl Panel {
    pub fn new(ui: &Ui) -> Self {
        let (tx, rx) = channel();
        Panel {
            shown: false,
            field: TextField::new(ui, "Ask the assistant", FIELD_TEXT_PT, false, QUERY_MAX),
            phase: Phase::Idle,
            gate: Gate::default(),
            turns: Vec::new(),
            history: false,
            skip: 0,
            to_speak: None,
            tx,
            rx,
        }
    }

    pub fn is_shown(&self) -> bool {
        self.shown
    }

    pub fn phase(&self) -> &Phase {
        &self.phase
    }

    pub fn waiting(&self) -> bool {
        self.phase == Phase::Waiting
    }

    pub fn text(&self) -> &str {
        self.field.text()
    }

    pub fn field_mut(&mut self) -> &mut TextField {
        &mut self.field
    }

    /// Shows the empty panel. An earlier answer stays on screen until the next question.
    pub fn open(&mut self) {
        self.shown = true;
    }

    /// Closes the panel. A card waiting for Run is dropped unrun.
    pub fn hide(&mut self) {
        self.shown = false;
        self.gate.cancel();
        self.phase = Phase::Idle;
        self.skip = 0;
    }

    /// Shows a failure in the answer area (for example, no API key is saved). Nothing is sent.
    pub fn fail(&mut self, reason: &str) {
        self.phase = Phase::Failed(reason.to_string());
        self.skip = 0;
    }

    /// Starts a request for the field's text. Returns the messages to send, or None when nothing may be sent: the
    /// panel is closed, the assistant is off, or the field is empty. With the backend Off the request is still taken:
    /// it is matched on the device against the allowed intents (`assistant::ask`) and nothing leaves the machine.
    pub fn begin(&mut self, ui: &Ui, settings: &Settings) -> Option<Vec<(bool, String)>> {
        if !self.shown || !settings.enabled || self.waiting() {
            return None;
        }
        let text = self.field.text().trim().to_string();
        if text.is_empty() {
            return None;
        }
        self.history = settings.history;
        let messages = assistant::turn_messages(self.history, &self.turns, &text);
        if self.history {
            self.turns.push((true, text));
        }
        self.field.set_text(ui, "");
        self.gate.cancel();
        self.phase = Phase::Waiting;
        self.skip = 0;
        Some(messages)
    }

    /// Takes the confirmed card's call (the Run button or Enter). None when no card waits.
    pub fn run(&mut self) -> Option<Call> {
        let call = self.gate.confirm()?;
        self.phase = Phase::Waiting;
        self.skip = 0;
        Some(call)
    }

    /// Drops the waiting card. Nothing runs.
    pub fn cancel(&mut self) {
        if matches!(self.phase, Phase::Card(_)) {
            self.gate.cancel();
            self.phase = Phase::Idle;
            self.skip = 0;
        }
    }

    /// Applies one worker result: an answer, a card, or a failure.
    pub fn apply(&mut self, msg: Msg) {
        self.skip = 0;
        match msg {
            // a question (missing parameter, or which app) is shown like an answer; the person's reply comes next
            Msg::Reply(Ok(Reply::Answer(text) | Reply::Ask(text))) => {
                if self.history {
                    self.turns.push((false, text.clone()));
                }
                self.to_speak = Some(text.clone());
                self.phase = Phase::Answer(text);
            }
            Msg::Reply(Ok(Reply::Intent(call))) => {
                let card = self.gate.propose(call);
                self.phase = Phase::Card(card);
            }
            Msg::Reply(Err(assistant::Error::NoBackend)) => {
                self.phase = Phase::Failed("I could not match that to an action. Choose a backend in Settings > Assistant for open questions.".into());
            }
            Msg::Reply(Err(e)) => self.phase = Phase::Failed(e.to_string()),
            Msg::Ran(Ok(Some(text))) => {
                self.to_speak = Some(text.clone());
                self.phase = Phase::Answer(text);
            }
            Msg::Ran(Ok(None)) => self.phase = Phase::Answer("Done.".into()),
            Msg::Ran(Err(e)) => self.phase = Phase::Failed(e),
        }
    }

    /// The answer that arrived since the last call, once: the shell hands it to the speech service when the setting is
    /// on. A failure or a card is never an answer, so it is never spoken.
    pub fn take_answer(&mut self) -> Option<String> {
        self.to_speak.take()
    }

    /// Applies every result that has arrived. True when the screen must be drawn again.
    pub fn receive(&mut self) -> bool {
        let mut changed = false;
        while let Ok(msg) = self.rx.try_recv() {
            self.apply(msg);
            changed = true;
        }
        changed
    }

    /// Moves the answer by `by` lines (negative goes back up). The draw clamps it.
    pub fn scroll(&mut self, by: isize) {
        self.skip = (self.skip as isize + by).max(0) as usize;
    }

    /// Sends one request on a worker thread; its reply arrives through `receive`.
    pub fn ask_async(&self, settings: Settings, connection: Connection, messages: Vec<(bool, String)>, catalogue: Catalogue) {
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let reply = assistant::ask(&settings, &connection, &messages, &catalogue);
            let _ = tx.send(Msg::Reply(reply));
        });
    }

    /// Runs a confirmed call on a worker thread; the result arrives through `receive`.
    pub fn run_async(&self, call: Call) {
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let result = assistant::execute(&call, RUN_WAIT);
            let _ = tx.send(Msg::Ran(result));
        });
    }

    /// Where the panel is drawn, and what is on it, for a screen `w` wide.
    fn layout(&self, ui: &Ui, w: i32) -> Layout {
        let cw = ui.pt(CARD_W_PT).min(w as f32 - ui.pt(2.0 * PAD_PT)).max(ui.pt(MIN_W_PT));
        let x = (w as f32 - cw) / 2.0;
        let y = ui.pt(TOP_PT);
        let field_h = ui.pt(FIELD_PT);
        let pad = ui.pt(PAD_PT);
        let gap = ui.pt(GAP_PT);
        let line_h = ui.pt(LINE_PT);
        let max_lines = (((ui.pt(BODY_MAX_PT) - 2.0 * pad) / line_h).floor() as usize).max(1);
        let text_w = cw - 2.0 * pad;
        let text: Option<String> = match &self.phase {
            Phase::Idle => None,
            Phase::Waiting => Some("Thinking…".into()),
            Phase::Answer(t) | Phase::Failed(t) | Phase::Card(t) => Some(t.clone()),
        };
        let Some(text) = text else {
            return Layout { x, y, w: cw, field_h, body: None, lines: Vec::new(), run: None, cancel: None };
        };
        let wrapped = ui.wrap(&text, BODY_PT, Weight::Regular, text_w);
        let skip = self.skip.min(wrapped.len().saturating_sub(max_lines));
        let lines: Vec<String> = wrapped.into_iter().skip(skip).take(max_lines).collect();
        let button_h = ui.pt(BUTTON_H_PT);
        let card = matches!(self.phase, Phase::Card(_));
        let text_h = lines.len() as f32 * line_h;
        let body_h = if card { pad + text_h + gap + button_h + pad } else { pad + text_h + pad };
        let by = y + field_h + gap;
        let (mut run, mut cancel) = (None, None);
        if card {
            let by_btn = by + pad + text_h + gap;
            let right = x + cw - pad;
            let cancel_w = ui.pt(CANCEL_W_PT);
            let run_w = ui.pt(RUN_W_PT);
            let cancel_x = right - cancel_w;
            let run_x = cancel_x - ui.pt(BUTTON_GAP_PT) - run_w;
            run = Some((run_x, by_btn, run_w, button_h));
            cancel = Some((cancel_x, by_btn, cancel_w, button_h));
        }
        Layout { x, y, w: cw, field_h, body: Some((by, body_h)), lines, run, cancel }
    }

    /// Where a tap at (x, y) lands.
    pub fn tap(&self, ui: &Ui, w: i32, x: f32, y: f32) -> Tap {
        if !self.shown {
            return Tap::Outside;
        }
        let l = self.layout(ui, w);
        let bottom = l.body.map_or(l.y + l.field_h, |(by, bh)| by + bh);
        let inside = x >= l.x && x < l.x + l.w && y >= l.y && y < bottom;
        let within = |r: Option<(f32, f32, f32, f32)>| r.is_some_and(|(bx, by, bw, bh)| x >= bx && x < bx + bw && y >= by && y < by + bh);
        if within(l.run) {
            Tap::Run
        } else if within(l.cancel) {
            Tap::Cancel
        } else if inside {
            Tap::Inside
        } else {
            Tap::Outside
        }
    }

    /// The rounded outline of the panel for the desktop's overlay surface: its rect and a coverage mask, as Search's.
    pub fn desk_overlay(&self, ui: &Ui, w: i32) -> Option<((usize, usize, usize, usize), Vec<u8>)> {
        if !self.shown {
            return None;
        }
        let l = self.layout(ui, w);
        let bottom = l.body.map_or(l.y + l.field_h, |(by, bh)| by + bh);
        let (x, y) = (l.x.max(0.0) as usize, l.y.max(0.0) as usize);
        let (ow, oh) = ((l.w.ceil() as usize).max(1), ((bottom - l.y).ceil() as usize).max(1));
        Some(((x, y, ow, oh), crate::gfx::rrect_mask(ow, oh, ui.pt(RADIUS_PT))))
    }

    /// Draws the panel: the field capsule, then the answer or card under it on its own glass.
    pub fn draw(&self, s: &mut Surface, ui: &Ui, w: i32, t: f64) {
        if !self.shown {
            return;
        }
        let l = self.layout(ui, w);
        let pad = ui.pt(PAD_PT);
        let fh = l.field_h;
        mac_glass(s, l.x, l.y, l.w, fh, fh / 2.0, 255);
        if let Some((by, bh)) = l.body {
            mac_glass(s, l.x, by, l.w, bh, ui.pt(RADIUS_PT), 255);
        }
        let text_x = l.x + ui.pt(FIELD_X_PT);
        let text_w = l.w - ui.pt(FIELD_X_PT + FIELD_CLEAR_PT);
        self.field.draw(s, ui, (text_x, l.y, text_w, fh), t);
        if let Some((by, _)) = l.body {
            let line_h = ui.pt(LINE_PT);
            for (i, line) in l.lines.iter().enumerate() {
                let img = ui.text(line, BODY_PT, Weight::Regular, WHITE);
                s.blit_colored(&img, (l.x + pad) as i32, (by + pad + i as f32 * line_h) as i32, LABEL.now(), 255);
            }
            for (rect, label) in [(l.run, "Run"), (l.cancel, "Cancel")] {
                let Some((bx, by, bw, bh)) = rect else { continue };
                let fill = if label == "Run" { super::ui::LINK } else { rgba(255, 255, 255, 46) };
                s.fill_rrect(bx, by, bw, bh, bh / 2.0, fill);
                let img = ui.text(label, BUTTON_LABEL_PT, Weight::Semibold, WHITE);
                s.blit(&img, (bx + (bw - img.w as f32) / 2.0) as i32, (by + (bh - img.h as f32) / 2.0) as i32, 255, 255);
            }
        }
    }
}

impl Shell {
    /// The assistant's settings as the prefs hold them now.
    pub(super) fn assistant_settings(&self) -> Settings {
        Settings::load(&self.prefs)
    }

    /// The API key from the keychain item the Assistant app also uses, if one is saved.
    fn assistant_api_key(&self) -> Option<String> {
        let store = crate::keychain::Store::open(&super::apphost::root(), KEY_APP).ok()?;
        String::from_utf8(store.get(assistant_client::KEY_SERVICE, assistant_client::KEY_ACCOUNT).ok()?).ok()
    }

    /// Cmd+Shift+A: opens the panel. With the assistant off, the Settings page for it opens instead.
    pub(super) fn assistant_hotkey(&mut self, t: f64) -> Vec<super::Effect> {
        let settings = self.assistant_settings();
        if settings.chord == assistant::Chord::Off {
            return vec![];
        }
        if !settings.enabled {
            return self.open_settings_page("Assistant", t);
        }
        self.overview_dismiss(t);
        self.assistant.open();
        self.dirty = true;
        vec![]
    }

    /// Hides the panel (Escape, or a tap outside it).
    pub(super) fn assistant_hide(&mut self) {
        self.assistant.hide();
        self.dirty = true;
    }

    /// A key typed while the panel is open: Enter sends or runs, arrows scroll, the rest edits the field.
    pub(super) fn assistant_key(&mut self, out: Out, t: f64) -> Vec<super::Effect> {
        self.dirty = true;
        match out {
            Out::Hide | Out::Named("Escape") => self.assistant_hide(),
            Out::Enter => {
                if let Some(call) = self.assistant.run() {
                    self.assistant.run_async(call);
                } else {
                    self.assistant_send();
                }
            }
            Out::Named("Up") => self.assistant.scroll(-1),
            Out::Named("Down") => self.assistant.scroll(1),
            other => {
                self.assistant.field_mut().edit(&self.ui, &other, t);
            }
        }
        vec![]
    }

    /// Sends the field's question. Nothing leaves the machine unless the backend is chosen and the key is saved.
    fn assistant_send(&mut self) {
        let settings = self.assistant_settings();
        let Some(messages) = self.assistant.begin(&self.ui, &settings) else { return };
        // with the backend Off the request is matched on the device and no key is needed
        let api_key = if settings.backend == Backend::Off { Some(String::new()) } else { self.assistant_api_key() };
        let Some(api_key) = api_key else {
            self.assistant.fail("No API key is saved for the assistant. Add it to the keychain item is.olibuijr.assistant / anthropic-api-key.");
            return;
        };
        let installed = super::apphost::installed();
        let apps: Vec<(&str, &str, &[super::intents::ManifestIntent])> = installed.iter().map(|a| (a.manifest.id.as_str(), a.manifest.name.as_str(), a.manifest.intents.as_slice())).collect();
        let catalogue = assistant::build(apps, &settings);
        let connection = Connection { api_key, model: MODEL.into() };
        self.assistant.ask_async(settings, connection, messages, catalogue);
    }

    /// Takes the worker results that arrived and redraws when there are some. True when a frame is needed.
    pub(super) fn assistant_tick(&mut self) -> bool {
        let changed = self.assistant.receive();
        if let Some(answer) = self.assistant.take_answer() {
            if self.assistant_settings().speak {
                self.assistant_speak(&answer);
            }
        }
        changed
    }

    /// Reads an answer aloud through the speech service, under the shell's own speaker key. The answer is on screen
    /// either way, so a missing engine is not reported here.
    fn assistant_speak(&mut self, answer: &str) {
        use super::services::speech::{clip, SERVICE, SHELL_SPEAKER};
        let body = bifrost_core::json::Json::Obj(vec![("text".to_string(), bifrost_core::json::Json::Str(clip(answer).to_string()))]).encode();
        let _ = self.service_call(SHELL_SPEAKER, SERVICE, "speak", &body, 0.0);
    }

    /// Touches while the panel is open belong to it: a tap outside closes it, and Run and Cancel act on the card.
    pub(super) fn assistant_event(&mut self, ev: super::Event) -> Option<Vec<super::Effect>> {
        if !self.assistant.is_shown() {
            return None;
        }
        let super::Event::Up(x, y) = ev else {
            return Some(vec![]);
        };
        self.dirty = true;
        match self.assistant.tap(&self.ui, self.w, x, y) {
            Tap::Outside => self.assistant_hide(),
            Tap::Run => {
                if let Some(call) = self.assistant.run() {
                    self.assistant.run_async(call);
                }
            }
            Tap::Cancel => self.assistant.cancel(),
            Tap::Inside => {}
        }
        Some(vec![])
    }

    /// The panel over the desktop, as Search draws over it.
    pub(super) fn draw_assistant(&self, s: &mut Surface, t: f64) {
        self.assistant.draw(s, &self.ui, self.w, t);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::assistant::{Call, Chord};

    fn ui() -> Ui {
        crate::shell::tests::ui()
    }

    fn on() -> Settings {
        Settings { enabled: true, backend: Backend::Anthropic, ..Settings::default() }
    }

    fn shown(ui: &Ui) -> Panel {
        let mut p = Panel::new(ui);
        p.open();
        p
    }

    fn type_text(p: &mut Panel, ui: &Ui, text: &str) {
        for c in text.chars() {
            p.field_mut().edit(ui, &Out::Text(c.to_string()), 0.0);
        }
    }

    fn call() -> Call {
        Call {
            app_id: "is.olibuijr.notes".into(),
            app_name: "Notes".into(),
            intent_id: "create-note".into(),
            title: "Create note".into(),
            params: vec![("text".into(), "Groceries".into())],
            summary: String::new(),
        }
    }

    #[test]
    fn the_shortcut_is_cmd_shift_a_and_has_its_own_switch() {
        assert!(Chord::MetaShiftA.matches(30, true, false, false, true));
        assert_eq!(crate::shell::kb_input::binding_of(crate::shell::hwkeys::Shortcut::Assistant), Some(11));
        assert_eq!(crate::shell::kb_input::BINDINGS[11].1, "Shift+Cmd+A");
    }

    #[test]
    fn with_the_backend_off_the_request_stays_on_the_device() {
        let ui = ui();
        let mut p = shown(&ui);
        type_text(&mut p, &ui, "hello");
        let off = Settings { enabled: true, backend: Backend::Off, ..Settings::default() };
        let messages = p.begin(&ui, &off).expect("the request is taken for on-device matching");
        assert_eq!(messages, vec![(true, "hello".to_string())]);
        p.apply(Msg::Reply(Err(assistant::Error::NoBackend)));
        match p.phase() {
            Phase::Failed(text) => assert!(text.contains("Settings > Assistant"), "the panel says what to do: {text}"),
            _ => panic!("an unmatched request is a failure"),
        }
    }

    #[test]
    fn nothing_is_sent_while_the_assistant_is_disabled_or_closed_or_empty() {
        let ui = ui();
        let mut p = shown(&ui);
        type_text(&mut p, &ui, "hello");
        assert_eq!(p.begin(&ui, &Settings { enabled: false, ..on() }), None);
        assert_eq!(p.begin(&ui, &Settings { backend: Backend::Anthropic, ..Settings::default() }), None);
        let mut blank = shown(&ui);
        type_text(&mut blank, &ui, "   ");
        assert_eq!(blank.begin(&ui, &on()), None, "a blank question is not sent");
        let mut closed = Panel::new(&ui);
        type_text(&mut closed, &ui, "hello");
        assert_eq!(closed.begin(&ui, &on()), None, "a closed panel sends nothing");
    }

    #[test]
    fn typing_then_sending_waits_and_clears_the_field() {
        let ui = ui();
        let mut p = shown(&ui);
        type_text(&mut p, &ui, "What time is it");
        let messages = p.begin(&ui, &on()).expect("a request goes out");
        assert_eq!(messages, vec![(true, "What time is it".to_string())]);
        assert!(p.waiting());
        assert_eq!(p.text(), "");
        assert_eq!(p.begin(&ui, &on()), None, "one request at a time");
    }

    #[test]
    fn an_answer_replaces_the_waiting_state() {
        let ui = ui();
        let mut p = shown(&ui);
        type_text(&mut p, &ui, "Hi");
        p.begin(&ui, &on());
        p.apply(Msg::Reply(Ok(Reply::Answer("Hello.".into()))));
        assert_eq!(p.phase(), &Phase::Answer("Hello.".into()));
        assert!(!p.waiting());
    }

    #[test]
    fn an_intent_reply_is_a_card_that_runs_only_on_run() {
        let ui = ui();
        let mut p = shown(&ui);
        type_text(&mut p, &ui, "Note groceries");
        p.begin(&ui, &on());
        p.apply(Msg::Reply(Ok(Reply::Intent(call()))));
        assert_eq!(p.phase(), &Phase::Card("Create note 'Groceries' in Notes?".into()));
        assert!(!p.waiting(), "a card waits for the person; nothing runs yet");
        assert_eq!(p.run(), Some(call()), "Run hands the call out");
        assert!(p.waiting());
        assert_eq!(p.run(), None, "a card runs once");
    }

    #[test]
    fn cancel_drops_the_card_unrun() {
        let ui = ui();
        let mut p = shown(&ui);
        type_text(&mut p, &ui, "Note groceries");
        p.begin(&ui, &on());
        p.apply(Msg::Reply(Ok(Reply::Intent(call()))));
        p.cancel();
        assert_eq!(p.phase(), &Phase::Idle);
        assert_eq!(p.run(), None, "a cancelled card never runs");
    }

    #[test]
    fn a_run_result_shows_as_the_answer_or_a_failure() {
        let ui = ui();
        let mut p = shown(&ui);
        p.apply(Msg::Ran(Ok(None)));
        assert_eq!(p.phase(), &Phase::Answer("Done.".into()));
        p.apply(Msg::Ran(Err("The app did not answer".into())));
        assert_eq!(p.phase(), &Phase::Failed("The app did not answer".into()));
    }

    #[test]
    fn a_failed_reply_shows_its_reason() {
        let ui = ui();
        let mut p = shown(&ui);
        p.apply(Msg::Reply(Err(Error::Unreachable("no route".into()))));
        assert_eq!(p.phase(), &Phase::Failed("no route".into()));
    }

    #[test]
    fn history_off_keeps_no_turns_and_sends_only_the_new_question() {
        let ui = ui();
        let mut p = shown(&ui);
        type_text(&mut p, &ui, "first");
        p.begin(&ui, &on());
        p.apply(Msg::Reply(Ok(Reply::Answer("one".into()))));
        type_text(&mut p, &ui, "second");
        let messages = p.begin(&ui, &on()).unwrap();
        assert_eq!(messages, vec![(true, "second".to_string())]);
    }

    #[test]
    fn a_question_from_the_assistant_is_shown_and_kept_as_a_turn() {
        let ui = ui();
        let mut p = shown(&ui);
        let history = Settings { history: true, ..on() };
        type_text(&mut p, &ui, "make a note");
        p.begin(&ui, &history);
        p.apply(Msg::Reply(Ok(Reply::Ask("What text should I use to create note in Notes?".into()))));
        type_text(&mut p, &ui, "Groceries");
        let messages = p.begin(&ui, &history).unwrap();
        assert_eq!(messages[1], (false, "What text should I use to create note in Notes?".to_string()));
        assert_eq!(messages[2], (true, "Groceries".to_string()));
    }

    #[test]
    fn history_on_sends_the_earlier_turns_too() {
        let ui = ui();
        let mut p = shown(&ui);
        let history = Settings { history: true, ..on() };
        type_text(&mut p, &ui, "first");
        p.begin(&ui, &history);
        p.apply(Msg::Reply(Ok(Reply::Answer("one".into()))));
        type_text(&mut p, &ui, "second");
        let messages = p.begin(&ui, &history).unwrap();
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[2], (true, "second".to_string()));
    }

    #[test]
    fn a_tap_outside_closes_and_buttons_act_on_the_card() {
        let ui = ui();
        let mut p = shown(&ui);
        type_text(&mut p, &ui, "Note groceries");
        p.begin(&ui, &on());
        p.apply(Msg::Reply(Ok(Reply::Intent(call()))));
        let w = 1200;
        let l = p.layout(&ui, w);
        let (rx, ry, rw, rh) = l.run.unwrap();
        assert_eq!(p.tap(&ui, w, rx + rw / 2.0, ry + rh / 2.0), Tap::Run);
        let (cx, cy, cw, ch) = l.cancel.unwrap();
        assert_eq!(p.tap(&ui, w, cx + cw / 2.0, cy + ch / 2.0), Tap::Cancel);
        assert_eq!(p.tap(&ui, w, 1.0, 1.0), Tap::Outside);
        assert_eq!(p.tap(&ui, w, l.x + 2.0, l.y + 2.0), Tap::Inside);
    }

    #[test]
    fn a_closed_panel_takes_no_taps_and_hiding_drops_a_card() {
        let ui = ui();
        let mut p = Panel::new(&ui);
        assert_eq!(p.tap(&ui, 1200, 600.0, 200.0), Tap::Outside);
        p.open();
        type_text(&mut p, &ui, "Note groceries");
        p.begin(&ui, &on());
        p.apply(Msg::Reply(Ok(Reply::Intent(call()))));
        p.hide();
        assert!(!p.is_shown());
        assert_eq!(p.run(), None, "hiding the panel drops its card");
    }

    #[test]
    fn a_long_answer_scrolls_and_the_scroll_stops_at_the_top() {
        let ui = ui();
        let mut p = shown(&ui);
        // long enough to wrap past the answer area (about 14 lines at this card width)
        let long = (0..400).map(|i| format!("line {i}")).collect::<Vec<_>>().join(" ");
        p.apply(Msg::Reply(Ok(Reply::Answer(long))));
        let top = p.layout(&ui, 1200).lines.first().cloned();
        p.scroll(3);
        assert_ne!(p.layout(&ui, 1200).lines.first().cloned(), top, "down moves the answer");
        p.scroll(-100);
        assert_eq!(p.layout(&ui, 1200).lines.first().cloned(), top, "up stops at the first line");
    }

    #[test]
    fn the_desktop_overlay_covers_the_field_and_the_answer() {
        let ui = ui();
        let mut p = shown(&ui);
        let field_only = p.desk_overlay(&ui, 1200).unwrap().0;
        p.apply(Msg::Reply(Ok(Reply::Answer("Hello.".into()))));
        let with_answer = p.desk_overlay(&ui, 1200).unwrap().0;
        assert!(with_answer.3 > field_only.3, "the answer adds height to the overlay");
        p.hide();
        assert!(p.desk_overlay(&ui, 1200).is_none());
    }

    #[test]
    fn the_worker_results_arrive_on_the_channel() {
        let ui = ui();
        let mut p = shown(&ui);
        p.tx.send(Msg::Reply(Ok(Reply::Answer("Later.".into())))).unwrap();
        assert!(p.receive(), "a result arrived, so a frame is needed");
        assert_eq!(p.phase(), &Phase::Answer("Later.".into()));
        assert!(!p.receive());
    }

    #[test]
    fn an_answer_is_handed_to_speech_once_and_a_failure_is_not() {
        let ui = ui();
        let mut p = shown(&ui);
        p.apply(Msg::Reply(Err(Error::Unreachable("no route".into()))));
        assert_eq!(p.take_answer(), None, "a failure is not spoken");
        p.apply(Msg::Reply(Ok(Reply::Answer("It is noon.".into()))));
        assert_eq!(p.take_answer(), Some("It is noon.".into()));
        assert_eq!(p.take_answer(), None, "an answer is spoken once");
        p.apply(Msg::Ran(Ok(Some("Saved.".into()))));
        assert_eq!(p.take_answer(), Some("Saved.".into()), "a run's reply is an answer too");
        p.apply(Msg::Ran(Err("The app did not answer".into())));
        assert_eq!(p.take_answer(), None);
    }
}
