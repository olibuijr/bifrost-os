//! Switch Control in the shell. While the setting is on, the two switch keys (`switch_panels::SwitchKeys`, Space and Enter
//! unless rebound) step through an on-screen panel of controls: "next" moves the highlight (the first press shows the panel,
//! and after the last button the highlight goes on to the next panel), "select" performs the action of the highlighted
//! button. The panels come from the template in `PANELS_FILE` when it parses, else from the one that ships
//! (`switch_panels::BUILTIN`). Escape hides the panel and scanning starts over.
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
}

impl SwitchControl {
    /// A control over `panels` (the built-in ones when the set has none) with the switches on `keys`.
    pub fn new(panels: PanelSet, keys: SwitchKeys) -> SwitchControl {
        let panels = if panels.panels().is_empty() { PanelSet::builtin() } else { panels };
        let scanner = PanelScanner::new(panels.panels()[0].clone());
        SwitchControl { panels, keys, index: 0, scanner, cursor: None, shown: false, muted_from: None }
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
        SwitchControl::new(panels, keys)
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
        let outcome = control.press(switch);
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
