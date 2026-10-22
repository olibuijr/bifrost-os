//! Switch Control in the shell. While the setting is on, the two switch keys (`switch_panels::SwitchKeys`, Space and Enter
//! unless rebound) step through an on-screen panel of controls: "next" moves the highlight (the first press shows the panel,
//! and after the last button the highlight goes on to the next panel), "select" performs the action of the highlighted
//! button. The panels come from the template in `PANELS_FILE` when it parses, else from the one that ships
//! (`switch_panels::BUILTIN`). Escape hides the panel and scanning starts over.
//!
//! Auto scanning: with `a11y_switch_auto_ms` set (500 to 10000 ms), the highlight steps on by itself every interval once the
//! panel is up, as if "next" were pressed, and a press of either switch restarts the interval. After `AUTO_PASSES` rounds
//! over all the panels with no press the panel hides again, so an idle scan does not run for ever ([`SwitchControl::auto_tick`]).
//!
//! The shell's input loop offers every key to [`Shell::switch_control_key`] first; a key that is a switch (and its release)
//! is used up, any other key is left alone. Nothing acts on the lock screen or with the screen off. Geometry is pure
//! (`panel`, `row_rect`) and tested on the host; the `Shell` methods at the bottom are the glue.

use super::switch_panels::{Outcome, Panel, PanelScanner, PanelSet, SwitchKeys};
use super::switch_scan::Switch;
use crate::gfx::Surface;
use crate::shell::prefs::Prefs;
use crate::shell::ui::{Ui, Weight, CARD_BG, LABEL, LINK, MAC_BODY_PT, MAC_CALLOUT_PT, MAC_TITLE3_PT, SECONDARY, SEPARATOR, WHITE};
use crate::shell::{assistive_touch, Drawn, Effect, Shell};

/// The pref that holds the key codes of the two switches (`SwitchKeys::save`).
pub const KEYS_PREF: &str = "a11y_switch_keys";
/// The pref that holds the auto-scan interval in milliseconds; absent or 0 keeps scanning manual (`parse_auto_ms`).
pub const AUTO_PREF: &str = "a11y_switch_auto_ms";
/// The shortest and longest auto-scan interval accepted, in milliseconds.
pub const AUTO_MIN_MS: u64 = 500;
pub const AUTO_MAX_MS: u64 = 10_000;
/// Auto scanning gives up after this many rounds over every button of every panel with no press.
pub const AUTO_PASSES: usize = 2;
/// A panel template that replaces the built-in panels when it exists and parses.
pub const PANELS_FILE: &str = "/etc/bifrost/accessibility/switch-panels.txt";

/// Panel width, the side padding, the title row, one button row, the footer hint, in points.
const PANEL_PT: f32 = 320.0;
const PAD_PT: f32 = 16.0;
const TITLE_PT: f32 = 40.0;
const ROW_PT: f32 = 36.0;
const FOOT_PT: f32 = 36.0;
/// The highlight plate sits this far in from the panel edge, with this corner radius.
const PLATE_INSET_PT: f32 = 8.0;
const PLATE_RADIUS_PT: f32 = 9.0;
const RADIUS_PT: f32 = 13.0;
/// Top of the panel as a share of the screen height.
const TOP_SHARE: f32 = 0.30;
const HINT: &str = "Next moves the highlight, Select chooses";

/// x, y, w, h in px.
type Rect = (f32, f32, f32, f32);

/// The panel rectangle for `n` buttons. `s` is one point in pixels; the panel is centred, its top at `TOP_SHARE` of the height.
pub fn panel(s: f32, w: f32, h: f32, n: usize) -> Rect {
    let pw = PANEL_PT * s;
    let ph = (TITLE_PT + n as f32 * ROW_PT + FOOT_PT) * s;
    ((w - pw) / 2.0, h * TOP_SHARE, pw, ph)
}

/// The rectangle of button row `i`.
pub fn row_rect(s: f32, w: f32, h: f32, n: usize, i: usize) -> Rect {
    let (x, y, pw, _) = panel(s, w, h, n);
    (x, y + (TITLE_PT + i as f32 * ROW_PT) * s, pw, ROW_PT * s)
}

/// The auto-scan interval a pref value names: 0 (manual) for a missing, malformed or zero value, else the value clamped to
/// `AUTO_MIN_MS..=AUTO_MAX_MS`.
pub fn parse_auto_ms(value: Option<&str>) -> u64 {
    match value.and_then(|v| v.trim().parse::<u64>().ok()) {
        None | Some(0) => 0,
        Some(ms) => ms.clamp(AUTO_MIN_MS, AUTO_MAX_MS),
    }
}

/// The scanner over the panels: which panel is up, where the highlight is, and whether the panel is on screen.
#[derive(Clone, Debug)]
pub struct SwitchControl {
    panels: PanelSet,
    keys: SwitchKeys,
    index: usize,
    scanner: PanelScanner,
    cursor: Option<usize>,
    shown: bool,
    /// The volume before the Mute button muted it, so the next press restores it.
    muted_from: Option<f32>,
    /// Auto scanning: the interval in ms (0 for manual), when the highlight last moved or a switch was last pressed, and
    /// the steps taken by the timer since the last press.
    auto_ms: u64,
    last_step: f64,
    auto_steps: usize,
}

impl SwitchControl {
    /// A control over `panels` (the built-in ones when the set has none) with the switches on `keys`.
    pub fn new(panels: PanelSet, keys: SwitchKeys) -> SwitchControl {
        let panels = if panels.panels().is_empty() { PanelSet::builtin() } else { panels };
        let scanner = PanelScanner::new(panels.panels()[0].clone());
        SwitchControl { panels, keys, index: 0, scanner, cursor: None, shown: false, muted_from: None, auto_ms: 0, last_step: 0.0, auto_steps: 0 }
    }

    /// The control for the saved switch keys and the template that applies (see `PANELS_FILE`).
    pub fn from_prefs(prefs: &Prefs) -> SwitchControl {
        let keys = SwitchKeys::load(prefs.get(KEYS_PREF).unwrap_or(""));
        let panels = match std::fs::read_to_string(PANELS_FILE) {
            Ok(text) => super::switch_panels::parse(&text).unwrap_or_else(|e| {
                eprintln!("springboard: {PANELS_FILE}: line {}: {}", e.line, e.message);
                PanelSet::default()
            }),
            Err(_) => PanelSet::default(),
        };
        SwitchControl::new(panels, keys).with_auto(parse_auto_ms(prefs.get(AUTO_PREF)))
    }

    /// The same control scanning by itself every `ms` milliseconds (0 keeps it manual; other values are clamped as
    /// `parse_auto_ms` does).
    pub fn with_auto(mut self, ms: u64) -> SwitchControl {
        self.auto_ms = parse_auto_ms(Some(&ms.to_string()));
        self
    }

    /// The auto-scan interval in milliseconds, 0 when scanning is manual.
    pub fn auto_ms(&self) -> u64 {
        self.auto_ms
    }

    /// The switch a key stands for, if it is bound.
    pub fn switch_for(&self, code: u16) -> Option<Switch> {
        self.keys.switch_for(code)
    }

    /// The panel the highlight is in.
    pub fn panel(&self) -> &Panel {
        self.scanner.panel()
    }

    /// The index of the highlighted button in the panel, None before the first step.
    pub fn highlighted(&self) -> Option<usize> {
        self.cursor
    }

    pub fn shown(&self) -> bool {
        self.shown
    }

    /// A switch press. "Next" moves the highlight and shows the panel; after the last button of a panel it goes on to the
    /// first button of the next panel (and wraps after the last panel). "Select" answers the action of the highlighted button.
    pub fn press(&mut self, switch: Switch) -> Outcome {
        let out = match switch {
            Switch::Next => {
                let was_scanning = self.cursor.is_some();
                let mut out = self.scanner.press(Switch::Next);
                if was_scanning && out == Outcome::Moved(0) && self.panels.panels().len() > 1 {
                    self.index = (self.index + 1) % self.panels.panels().len();
                    self.scanner = PanelScanner::new(self.panels.panels()[self.index].clone());
                    out = self.scanner.press(Switch::Next);
                }
                self.shown = true;
                out
            }
            Switch::Select => self.scanner.press(Switch::Select),
        };
        if let Outcome::Moved(i) = out {
            self.cursor = Some(i);
        }
        out
    }

    /// A switch press at time `t` (ms): restarts the auto-scan interval and its round count, then acts as [`SwitchControl::press`].
    pub fn press_at(&mut self, switch: Switch, t: f64) -> Outcome {
        self.last_step = t;
        self.auto_steps = 0;
        self.press(switch)
    }

    /// The most steps the timer takes without a press: `AUTO_PASSES` rounds over every button of every panel.
    fn auto_limit(&self) -> usize {
        AUTO_PASSES * self.panels.panels().iter().map(|p| p.buttons.len()).sum::<usize>()
    }

    /// Auto scanning at time `t` (ms). With the panel up, a highlight on a button and the interval passed since the last
    /// step or press, the highlight moves on as a "next" press would. Once the timer has made [`AUTO_PASSES`] rounds with no
    /// press the panel hides and scanning starts over. True when anything changed (the frame needs repainting).
    pub fn auto_tick(&mut self, t: f64) -> bool {
        if self.auto_ms == 0 || !self.shown || self.cursor.is_none() || t - self.last_step < self.auto_ms as f64 {
            return false;
        }
        self.last_step = t;
        self.auto_steps += 1;
        if self.auto_steps > self.auto_limit() {
            self.hide();
        } else {
            self.press(Switch::Next);
        }
        true
    }

    /// Takes the panel away and starts scanning over from the first panel.
    fn hide(&mut self) {
        self.shown = false;
        self.cursor = None;
        self.index = 0;
        self.auto_steps = 0;
        self.scanner = PanelScanner::new(self.panels.panels()[0].clone());
    }

    pub fn draw(&self, s: &mut Surface, ui: &Ui, w: i32, h: i32) {
        let sc = ui.pt(1.0);
        let (wf, hf) = (w as f32, h as f32);
        let current = self.panel();
        let n = current.buttons.len();
        let (x, y, pw, ph) = panel(sc, wf, hf, n);
        s.fill_rrect(x, y, pw, ph, ui.pt(RADIUS_PT), CARD_BG.now());
        s.stroke_rrect(x, y, pw, ph, ui.pt(RADIUS_PT), 1.0, SEPARATOR);

        let title = ui.text(&current.title, MAC_TITLE3_PT, Weight::Semibold, WHITE);
        s.blit_colored(&title, (x + PAD_PT * sc) as i32, (y + (TITLE_PT * sc - title.h as f32) / 2.0) as i32, LABEL.now(), 255);

        for (i, b) in current.buttons.iter().enumerate() {
            let (rx, ry, rw, rh) = row_rect(sc, wf, hf, n, i);
            let on = self.cursor == Some(i);
            if on {
                let inset = PLATE_INSET_PT * sc;
                s.fill_rrect(rx + inset, ry, rw - 2.0 * inset, rh, ui.pt(PLATE_RADIUS_PT), LINK);
            }
            let label = ui.text(&b.label, MAC_BODY_PT, Weight::Regular, WHITE);
            let color = if on { WHITE } else { LABEL.now() };
            s.blit_colored(&label, (rx + PAD_PT * sc) as i32, (ry + (rh - label.h as f32) / 2.0) as i32, color, 255);
        }

        let foot_y = y + (TITLE_PT + n as f32 * ROW_PT) * sc;
        let foot = ui.text(HINT, MAC_CALLOUT_PT, Weight::Regular, WHITE);
        s.blit_colored(&foot, (x + (pw - foot.w as f32) / 2.0) as i32, (foot_y + (FOOT_PT * sc - foot.h as f32) / 2.0) as i32, SECONDARY, 255);
    }
}

impl Shell {
    /// Whether the switch panel is on screen.
    pub(in crate::shell) fn switch_panel_shown(&self) -> bool {
        self.switch_control.as_ref().is_some_and(SwitchControl::shown) && self.switch_control_on()
    }

    /// The open panel, for the frame to draw.
    pub(in crate::shell) fn switch_panel(&self) -> Option<&SwitchControl> {
        self.switch_control.as_ref().filter(|c| c.shown())
    }

    /// Requests a redraw of the whole frame, the panel's state having changed.
    fn switch_panel_changed(&mut self) {
        self.drawn = Drawn::Nothing;
        self.dirty = true;
    }

    /// The per-frame step of auto scanning (see `SwitchControl::auto_tick`). True when the highlight moved or the panel hid.
    /// Does nothing while the setting is off, the screen is off or locked, or the panel has never been opened.
    pub(in crate::shell) fn switch_control_tick(&mut self, t: f64) -> bool {
        if self.switch_control.is_none() || !self.switch_control_on() || !self.screen_on || self.is_locked() {
            return false;
        }
        let moved = self.switch_control.as_mut().is_some_and(|c| c.auto_tick(t));
        if moved {
            self.switch_panel_changed();
        }
        moved
    }

    /// Escape hides the panel and starts scanning over. True when it was up (the key is then used up).
    pub(in crate::shell) fn switch_panel_escape(&mut self) -> bool {
        if !self.switch_panel_shown() {
            return false;
        }
        self.switch_control = None;
        self.switch_panel_changed();
        true
    }

    /// Offers a key to Switch Control. Returns the effects when the key was a switch (it is then used up, and so is its
    /// release), None when the key is not for Switch Control: the setting is off, the screen is off or locked, or the key
    /// is not bound to a switch.
    pub fn switch_control_key(&mut self, code: u16, down: bool, t: f64) -> Option<Vec<Effect>> {
        if !self.switch_control_on() {
            if self.switch_control.take().is_some() {
                self.switch_panel_changed();
            }
            return None;
        }
        if !self.screen_on || self.is_locked() {
            return None;
        }
        if self.switch_control.is_none() {
            self.switch_control = Some(SwitchControl::from_prefs(&self.prefs));
        }
        let control = self.switch_control.as_mut()?;
        let switch = control.switch_for(code)?;
        if !down {
            return Some(vec![]);
        }
        let outcome = control.press_at(switch, t);
        self.last_input = t;
        match outcome {
            Outcome::Moved(_) => {
                self.switch_panel_changed();
                Some(vec![])
            }
            Outcome::Activate(action) => {
                let fx = self.switch_action(&action, t);
                self.switch_panel_changed();
                Some(fx)
            }
            Outcome::Nothing => Some(vec![]),
        }
    }

    /// Performs the action a panel button names (`system.*` are the AssistiveTouch actions, `media.*` the transport keys).
    fn switch_action(&mut self, action: &str, t: f64) -> Vec<Effect> {
        use assistive_touch::Action;
        let fx = match action {
            "system.home" => self.assistive_action(Action::Home, t),
            "system.notifications" => self.assistive_action(Action::NotificationCenter, t),
            "system.control-centre" => self.assistive_action(Action::ControlCenter, t),
            "system.volume-up" => self.assistive_action(Action::VolumeUp, t),
            "system.volume-down" => self.assistive_action(Action::VolumeDown, t),
            "system.lock" => {
                self.switch_control = None;
                self.assistive_action(Action::Lock, t)
            }
            // KEY_PLAYPAUSE, KEY_NEXTSONG, KEY_PREVIOUSSONG: the owner of the Now Playing session gets the command
            "media.play-pause" => {
                crate::shell::services::now_playing::media_key(164);
                vec![]
            }
            "media.next" => {
                crate::shell::services::now_playing::media_key(163);
                vec![]
            }
            "media.previous" => {
                crate::shell::services::now_playing::media_key(165);
                vec![]
            }
            "media.mute" => self.switch_mute(t),
            _ => vec![],
        };
        self.dirty = true;
        fx
    }

    /// Mute: the volume goes to zero, and the next press brings back what it was.
    fn switch_mute(&mut self, t: f64) -> Vec<Effect> {
        let restore = self.switch_control.as_mut().and_then(|c| c.muted_from.take());
        let level = match restore {
            Some(previous) => previous,
            None => {
                let now = self.cc.volume;
                if now > 0.0 {
                    if let Some(c) = self.switch_control.as_mut() {
                        c.muted_from = Some(now);
                    }
                }
                0.0
            }
        };
        self.cc.volume = level;
        self.audio_volume(level);
        if self.screen_on && !self.cc_active() {
            self.hud.show(level, t);
        }
        vec![Effect::Volume(level)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::accessibility::switch_panels::{KEY_ENTER, KEY_SPACE};
    use crate::shell::tests::{settle, shell, unlock};

    fn control() -> SwitchControl {
        SwitchControl::new(PanelSet::builtin(), SwitchKeys::default())
    }

    fn on(sh: &mut Shell) {
        sh.prefs.set("a11y_switch_control", "1").unwrap();
        unlock(sh);
    }

    #[test]
    fn the_first_next_shows_the_first_button_and_select_answers_its_action() {
        let mut c = control();
        assert!(!c.shown() && c.highlighted().is_none());
        assert_eq!(c.press(Switch::Select), Outcome::Nothing);
        assert_eq!(c.press(Switch::Next), Outcome::Moved(0));
        assert!(c.shown());
        assert_eq!(c.press(Switch::Next), Outcome::Moved(1));
        assert_eq!(c.highlighted(), Some(1));
        assert_eq!(c.press(Switch::Select), Outcome::Activate("system.notifications".into()));
    }

    #[test]
    fn after_the_last_button_the_highlight_goes_on_to_the_next_panel_and_wraps() {
        let mut c = control();
        for _ in 0..6 {
            c.press(Switch::Next);
        }
        assert_eq!(c.panel().id, "system");
        assert_eq!(c.press(Switch::Next), Outcome::Moved(0));
        assert_eq!(c.panel().id, "media");
        for _ in 0..3 {
            c.press(Switch::Next);
        }
        assert_eq!(c.press(Switch::Next), Outcome::Moved(0));
        assert_eq!(c.panel().id, "system", "after the last panel the first one comes round again");
    }

    #[test]
    fn a_single_panel_wraps_onto_itself() {
        let one = super::super::switch_panels::parse("panel a \"A\"\n  button \"x\" action=a.x\n  button \"y\" action=a.y\n").unwrap();
        let mut c = SwitchControl::new(one, SwitchKeys::default());
        assert_eq!(c.press(Switch::Next), Outcome::Moved(0));
        assert_eq!(c.press(Switch::Next), Outcome::Moved(1));
        assert_eq!(c.press(Switch::Next), Outcome::Moved(0));
        assert_eq!(c.panel().id, "a");
        assert_eq!(SwitchControl::new(PanelSet::default(), SwitchKeys::default()).panel().id, "system", "no panels gives the built-in ones");
    }

    #[test]
    fn the_auto_interval_is_read_from_a_pref_and_clamped() {
        assert_eq!(parse_auto_ms(None), 0);
        assert_eq!(parse_auto_ms(Some("")), 0);
        assert_eq!(parse_auto_ms(Some("soon")), 0);
        assert_eq!(parse_auto_ms(Some("0")), 0);
        assert_eq!(parse_auto_ms(Some("1500")), 1500);
        assert_eq!(parse_auto_ms(Some("10")), AUTO_MIN_MS);
        assert_eq!(parse_auto_ms(Some("999999")), AUTO_MAX_MS);
        assert_eq!(control().with_auto(2_000).auto_ms(), 2_000);
        assert_eq!(control().auto_ms(), 0, "scanning is manual unless asked");
    }

    #[test]
    fn auto_scanning_steps_on_its_own_after_the_interval() {
        let mut c = control().with_auto(1_000);
        assert!(!c.auto_tick(5_000.0), "nothing moves before the panel is up");
        c.press_at(Switch::Next, 0.0);
        assert_eq!(c.highlighted(), Some(0));
        assert!(!c.auto_tick(900.0), "the interval has not passed");
        assert!(c.auto_tick(1_000.0));
        assert_eq!(c.highlighted(), Some(1));
        assert!(!c.auto_tick(1_500.0), "the interval counts from the last step");
        assert!(c.auto_tick(2_000.0));
        assert_eq!(c.highlighted(), Some(2));
        // a press restarts the interval
        c.press_at(Switch::Next, 2_900.0);
        assert!(!c.auto_tick(3_500.0));
        assert!(c.auto_tick(3_900.0));
        assert_eq!(c.highlighted(), Some(4));
    }

    #[test]
    fn manual_scanning_never_moves_on_a_tick() {
        let mut c = control();
        c.press_at(Switch::Next, 0.0);
        assert!(!c.auto_tick(1_000_000.0));
        assert_eq!(c.highlighted(), Some(0));
    }

    #[test]
    fn auto_scanning_goes_on_into_the_next_panel_and_gives_up_after_two_rounds() {
        let mut c = control().with_auto(1_000);
        c.press_at(Switch::Next, 0.0);
        let mut t = 0.0;
        for step in 1..=20 {
            t += 1_000.0;
            assert!(c.auto_tick(t));
            assert!(c.shown(), "still scanning at step {step}");
            if step == 6 {
                assert_eq!(c.panel().id, "media", "the highlight went on to the next panel");
            }
        }
        t += 1_000.0;
        assert!(c.auto_tick(t), "the hiding is a change too");
        assert!(!c.shown() && c.highlighted().is_none());
        assert_eq!(c.panel().id, "system", "scanning starts over from the first panel");
        assert!(!c.auto_tick(t + 5_000.0), "nothing runs while the panel is down");
    }

    #[test]
    fn a_press_resets_the_count_of_idle_rounds() {
        let mut c = control().with_auto(1_000);
        c.press_at(Switch::Next, 0.0);
        for n in 1..=15 {
            assert!(c.auto_tick(n as f64 * 1_000.0));
        }
        c.press_at(Switch::Select, 15_500.0);
        for n in 1..=20 {
            assert!(c.auto_tick(15_500.0 + n as f64 * 1_000.0));
            assert!(c.shown());
        }
    }

    #[test]
    fn the_shell_ticks_auto_scanning_while_the_setting_is_on() {
        let mut sh = shell();
        sh.prefs.set(AUTO_PREF, "1000").unwrap();
        assert!(!sh.switch_control_tick(1_000.0), "no panel yet");
        on(&mut sh);
        assert!(!sh.switch_control_tick(1_500.0), "setting on but the panel was never opened");
        sh.switch_control_key(KEY_SPACE, true, 2000.0);
        assert_eq!(sh.switch_control.as_ref().unwrap().highlighted(), Some(0));
        assert!(!sh.switch_control_tick(2_500.0));
        assert!(sh.switch_control_tick(3_000.0));
        assert_eq!(sh.switch_control.as_ref().unwrap().highlighted(), Some(1));
        sh.prefs.set("a11y_switch_control", "0").unwrap();
        assert!(!sh.switch_control_tick(4_000.0), "nothing steps with the setting off");
    }

    #[test]
    fn the_panel_is_centred_and_its_rows_stack_under_the_title() {
        let (x, y, pw, ph) = panel(1.0, 400.0, 1000.0, 4);
        assert_eq!((x, pw), (40.0, PANEL_PT));
        assert_eq!(y, 300.0);
        assert_eq!(ph, TITLE_PT + 4.0 * ROW_PT + FOOT_PT);
        assert_eq!(row_rect(1.0, 400.0, 1000.0, 4, 2), (40.0, 300.0 + TITLE_PT + 2.0 * ROW_PT, PANEL_PT, ROW_PT));
        assert_eq!(panel(2.0, 800.0, 1000.0, 1).2, 2.0 * PANEL_PT);
    }

    #[test]
    fn the_switch_keys_are_left_alone_while_switch_control_is_off() {
        let mut sh = shell();
        unlock(&mut sh);
        assert_eq!(sh.switch_control_key(KEY_SPACE, true, 2000.0), None);
        assert!(!sh.switch_panel_shown());
    }

    #[test]
    fn the_keys_step_the_panel_and_a_release_is_used_up_with_its_press() {
        let mut sh = shell();
        on(&mut sh);
        assert_eq!(sh.switch_control_key(KEY_SPACE, true, 2000.0), Some(vec![]));
        assert!(sh.switch_panel_shown());
        assert_eq!(sh.switch_control.as_ref().unwrap().highlighted(), Some(0));
        assert_eq!(sh.switch_control_key(KEY_SPACE, false, 2010.0), Some(vec![]), "the release goes with the press");
        sh.switch_control_key(KEY_SPACE, true, 2100.0);
        assert_eq!(sh.switch_control.as_ref().unwrap().highlighted(), Some(1));
        assert_eq!(sh.switch_control_key(30, true, 2200.0), None, "a key that is not a switch is not taken");
        assert_eq!(sh.switch_control_key(30, false, 2210.0), None);
    }

    #[test]
    fn select_performs_the_highlighted_action_in_the_shell() {
        let mut sh = shell();
        on(&mut sh);
        sh.set_volume(0.5);
        for _ in 0..4 {
            sh.switch_control_key(KEY_SPACE, true, 2000.0);
        }
        assert_eq!(sh.switch_control.as_ref().unwrap().panel().buttons[3].action, "system.volume-up");
        assert_eq!(sh.switch_control_key(KEY_ENTER, true, 2100.0), Some(vec![Effect::Volume(9.0 / 16.0)]));
        assert_eq!(sh.volume(), 9.0 / 16.0);
        assert!(sh.switch_panel_shown(), "the panel stays up for the next choice");
    }

    #[test]
    fn select_on_a_system_button_opens_the_real_screen() {
        let mut sh = shell();
        on(&mut sh);
        for _ in 0..3 {
            sh.switch_control_key(KEY_SPACE, true, 2000.0);
        }
        assert_eq!(sh.switch_control.as_ref().unwrap().panel().buttons[2].action, "system.control-centre");
        sh.switch_control_key(KEY_ENTER, true, 3000.0);
        settle(&mut sh, 3000.0);
        assert!(sh.cc_active());
    }

    #[test]
    fn mute_sets_the_volume_to_zero_and_the_next_press_restores_it() {
        let mut sh = shell();
        on(&mut sh);
        sh.set_volume(0.5);
        for _ in 0..10 {
            sh.switch_control_key(KEY_SPACE, true, 2000.0);
        }
        let c = sh.switch_control.as_ref().unwrap();
        assert_eq!((c.panel().id.as_str(), c.panel().buttons[c.highlighted().unwrap()].action.as_str()), ("media", "media.mute"));
        assert_eq!(sh.switch_control_key(KEY_ENTER, true, 2200.0), Some(vec![Effect::Volume(0.0)]));
        assert_eq!(sh.volume(), 0.0);
        assert_eq!(sh.switch_control_key(KEY_ENTER, true, 2300.0), Some(vec![Effect::Volume(0.5)]));
        assert_eq!(sh.volume(), 0.5);
    }

    #[test]
    fn escape_hides_the_panel_and_scanning_starts_over() {
        let mut sh = shell();
        on(&mut sh);
        assert!(!sh.switch_panel_escape(), "nothing to hide yet");
        sh.switch_control_key(KEY_SPACE, true, 2000.0);
        sh.switch_control_key(KEY_SPACE, true, 2100.0);
        assert!(sh.switch_panel_escape());
        assert!(!sh.switch_panel_shown() && !sh.switch_panel_escape());
        sh.switch_control_key(KEY_SPACE, true, 2200.0);
        assert_eq!(sh.switch_control.as_ref().unwrap().highlighted(), Some(0), "the highlight starts on the first button again");
    }

    #[test]
    fn nothing_acts_on_the_lock_screen_or_with_the_setting_off_again() {
        let mut sh = shell();
        sh.prefs.set("a11y_switch_control", "1").unwrap();
        assert!(sh.is_locked());
        assert_eq!(sh.switch_control_key(KEY_SPACE, true, 1000.0), None, "the lock screen keeps its keys");
        unlock(&mut sh);
        sh.switch_control_key(KEY_SPACE, true, 2000.0);
        assert!(sh.switch_panel_shown());
        sh.prefs.set("a11y_switch_control", "0").unwrap();
        assert!(!sh.switch_panel_shown(), "turning the setting off takes the panel away");
        assert_eq!(sh.switch_control_key(KEY_SPACE, true, 3000.0), None);
        assert!(sh.switch_control.is_none());
    }

    #[test]
    fn the_panel_is_painted_over_the_frame_with_the_highlight_on_its_button() {
        let mut sh = shell();
        on(&mut sh);
        let mut buf = vec![0u32; 1200 * 1920];
        let draw = |sh: &mut Shell, buf: &mut Vec<u32>, t: f64| sh.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: buf, y0: 0 }, t).clone();
        draw(&mut sh, &mut buf, 2000.0);
        let sc = sh.ui.pt(1.0);
        let (rx, ry, rw, rh) = row_rect(sc, 1200.0, 1920.0, 6, 0);
        let spot = (ry + rh / 2.0) as usize * 1200 + (rx + rw - 12.0 * sc) as usize;
        let before = buf[spot];
        sh.switch_control_key(KEY_SPACE, true, 2100.0);
        let d = draw(&mut sh, &mut buf, 2200.0);
        assert!(d.is_all(), "the panel repaints the whole frame");
        assert_ne!(buf[spot], before, "the highlighted button's plate covers the frame");
        sh.switch_panel_escape();
        buf.iter_mut().for_each(|p| *p = 0);
        draw(&mut sh, &mut buf, 2300.0);
        assert_eq!(buf[spot], before, "and is gone again once the panel is hidden");
    }
}
