//! Switch Control panels: the on-screen button panels a switch user steps through, and the keys that act as switches.
//!
//! A panel template is plain text, so a layout can be shipped, edited or shared without code:
//!
//! ```text
//! # comment
//! panel home "Home"
//!   button "Home" action=system.home
//!   button "Notifications" action=system.notifications
//! ```
//!
//! `parse` checks the template (unique panel ids, at least one button per panel, actions made of `a-z 0-9 . _ -`) and
//! `PanelSet::builtin` is the template that ships. A [`PanelScanner`] walks one panel's buttons with the shared
//! [`Scan`](super::switch_scan::Scan): a next switch moves the highlight, a select switch answers with the action of
//! the highlighted button for the shell to perform. [`SwitchKeys`] maps hardware key codes to the two switches.
//! Pure model: no drawing, no input device and no clock.

use super::switch_scan::{Scan, Switch};

/// evdev code of the space bar, the default "next" switch.
pub const KEY_SPACE: u16 = 57;
/// evdev code of Enter, the default "select" switch.
pub const KEY_ENTER: u16 = 28;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PanelButton {
    pub label: String,
    /// What the shell performs when the button is selected, for example `system.home`.
    pub action: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Panel {
    pub id: String,
    pub title: String,
    pub buttons: Vec<PanelButton>,
}

/// Why a template was refused: the 1-based line and what is wrong with it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    pub line: usize,
    pub message: String,
}

/// The template that ships: a system panel and a media panel.
pub const BUILTIN: &str = "\
# Switch Control panels that ship with the system
panel system \"System\"
  button \"Home\" action=system.home
  button \"Notifications\" action=system.notifications
  button \"Control Centre\" action=system.control-centre
  button \"Volume Up\" action=system.volume-up
  button \"Volume Down\" action=system.volume-down
  button \"Lock\" action=system.lock
panel media \"Media\"
  button \"Play or Pause\" action=media.play-pause
  button \"Next Track\" action=media.next
  button \"Previous Track\" action=media.previous
  button \"Mute\" action=media.mute
";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PanelSet {
    panels: Vec<Panel>,
}

fn valid_id(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-'))
}

/// A double-quoted string at the start of `s`; returns it and the rest. No escapes: a quote ends the string.
fn quoted(s: &str) -> Option<(&str, &str)> {
    let s = s.strip_prefix('"')?;
    let end = s.find('"')?;
    Some((&s[..end], &s[end + 1..]))
}

/// Reads a template. Blank lines and `#` comments are skipped.
pub fn parse(text: &str) -> Result<PanelSet, ParseError> {
    let mut panels: Vec<Panel> = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line_no = index + 1;
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let err = |message: &str| ParseError { line: line_no, message: message.to_string() };
        if let Some(rest) = line.strip_prefix("panel ") {
            let rest = rest.trim_start();
            let (id, rest) = rest.split_once(char::is_whitespace).ok_or_else(|| err("a panel needs an id and a quoted title"))?;
            if !valid_id(id) {
                return Err(err("a panel id uses only a-z 0-9 . _ -"));
            }
            if panels.iter().any(|p| p.id == id) {
                return Err(err("this panel id is used twice"));
            }
            let (title, tail) = quoted(rest.trim_start()).ok_or_else(|| err("the panel title is in double quotes"))?;
            if title.is_empty() || !tail.trim().is_empty() {
                return Err(err("a panel line is `panel <id> \"<title>\"`"));
            }
            panels.push(Panel { id: id.to_string(), title: title.to_string(), buttons: Vec::new() });
        } else if let Some(rest) = line.strip_prefix("button ") {
            let panel = panels.last_mut().ok_or_else(|| err("a button comes after a panel line"))?;
            let (label, tail) = quoted(rest.trim_start()).ok_or_else(|| err("the button label is in double quotes"))?;
            let action = tail.trim().strip_prefix("action=").ok_or_else(|| err("a button line ends with action=<name>"))?;
            if label.is_empty() || !valid_id(action) {
                return Err(err("a button needs a label and an action of a-z 0-9 . _ -"));
            }
            panel.buttons.push(PanelButton { label: label.to_string(), action: action.to_string() });
        } else {
            return Err(err("a line is `panel ...` or `button ...`"));
        }
    }
    if let Some(p) = panels.iter().find(|p| p.buttons.is_empty()) {
        return Err(ParseError { line: 0, message: format!("panel {} has no buttons", p.id) });
    }
    Ok(PanelSet { panels })
}

impl PanelSet {
    /// The template that ships.
    pub fn builtin() -> PanelSet {
        parse(BUILTIN).expect("the built-in panel template is valid")
    }

    pub fn panels(&self) -> &[Panel] {
        &self.panels
    }

    pub fn get(&self, id: &str) -> Option<&Panel> {
        self.panels.iter().find(|p| p.id == id)
    }

    /// The template text that `parse` reads back to the same set.
    pub fn format(&self) -> String {
        let mut out = String::new();
        for p in &self.panels {
            out.push_str(&format!("panel {} \"{}\"\n", p.id, p.title));
            for b in &p.buttons {
                out.push_str(&format!("  button \"{}\" action={}\n", b.label, b.action));
            }
        }
        out
    }
}

/// What a switch press did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The highlight moved to this button.
    Moved(usize),
    /// A button was selected: perform this action.
    Activate(String),
    /// Nothing happened (select with nothing highlighted).
    Nothing,
}

/// Steps through one panel's buttons.
#[derive(Clone, Debug)]
pub struct PanelScanner {
    panel: Panel,
    scan: Scan,
}

impl PanelScanner {
    pub fn new(panel: Panel) -> PanelScanner {
        let scan = Scan::new(panel.buttons.len());
        PanelScanner { panel, scan }
    }

    pub fn panel(&self) -> &Panel {
        &self.panel
    }

    /// The button the highlight is on.
    pub fn highlighted(&self) -> Option<&PanelButton> {
        self.scan.highlighted().and_then(|i| self.panel.buttons.get(i))
    }

    pub fn press(&mut self, switch: Switch) -> Outcome {
        match switch {
            Switch::Next => self.scan.press(Switch::Next).map_or(Outcome::Nothing, Outcome::Moved),
            Switch::Select => match self.scan.press(Switch::Select).and_then(|i| self.panel.buttons.get(i)) {
                Some(b) => Outcome::Activate(b.action.clone()),
                None => Outcome::Nothing,
            },
        }
    }
}

/// Which hardware keys act as the two switches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SwitchKeys {
    next: u16,
    select: u16,
}

impl Default for SwitchKeys {
    fn default() -> Self {
        SwitchKeys { next: KEY_SPACE, select: KEY_ENTER }
    }
}

impl SwitchKeys {
    /// Rebinds the switches. The two keys must differ.
    pub fn bind(&mut self, next: u16, select: u16) -> Result<(), &'static str> {
        if next == select {
            return Err("the next and select switches need different keys");
        }
        self.next = next;
        self.select = select;
        Ok(())
    }

    /// The switch a key press stands for, if the key is bound.
    pub fn switch_for(&self, code: u16) -> Option<Switch> {
        if code == self.next {
            Some(Switch::Next)
        } else if code == self.select {
            Some(Switch::Select)
        } else {
            None
        }
    }

    /// `next=<code>,select=<code>`, for the settings file.
    pub fn save(&self) -> String {
        format!("next={},select={}", self.next, self.select)
    }

    /// Reads the form `save` writes; anything else gives the defaults.
    pub fn load(text: &str) -> SwitchKeys {
        let mut keys = SwitchKeys::default();
        let (mut next, mut select) = (None, None);
        for part in text.split(',') {
            match part.trim().split_once('=') {
                Some(("next", v)) => next = v.parse::<u16>().ok(),
                Some(("select", v)) => select = v.parse::<u16>().ok(),
                _ => {}
            }
        }
        if let (Some(n), Some(s)) = (next, select) {
            let _ = keys.bind(n, s);
        }
        keys
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_built_in_template_parses_and_round_trips() {
        let set = PanelSet::builtin();
        assert_eq!(set.panels().len(), 2);
        assert_eq!(set.get("system").unwrap().buttons[0].action, "system.home");
        assert_eq!(parse(&set.format()).unwrap(), set);
    }

    #[test]
    fn bad_templates_name_their_line() {
        assert_eq!(parse("button \"x\" action=a\n").unwrap_err().line, 1);
        assert_eq!(parse("panel a \"A\"\n  button \"x\" action=A!\n").unwrap_err().line, 2);
        assert_eq!(parse("panel a \"A\"\npanel a \"B\"\n").unwrap_err().line, 2);
        assert_eq!(parse("panel a \"A\"\n  frob\n").unwrap_err().line, 2);
        assert_eq!(parse("panel a A\n").unwrap_err().line, 1);
        let empty = parse("panel a \"A\"\n").unwrap_err();
        assert!(empty.message.contains("no buttons"));
        assert!(parse("# only a comment\n\n").unwrap().panels().is_empty());
    }

    #[test]
    fn a_scanner_moves_then_activates_the_highlighted_button() {
        let set = PanelSet::builtin();
        let mut s = PanelScanner::new(set.get("media").unwrap().clone());
        assert_eq!(s.press(Switch::Select), Outcome::Nothing, "nothing is highlighted yet");
        assert_eq!(s.press(Switch::Next), Outcome::Moved(0));
        assert_eq!(s.press(Switch::Next), Outcome::Moved(1));
        assert_eq!(s.highlighted().unwrap().label, "Next Track");
        assert_eq!(s.press(Switch::Select), Outcome::Activate("media.next".into()));
        for _ in 0..3 {
            s.press(Switch::Next);
        }
        assert_eq!(s.highlighted().unwrap().label, "Play or Pause", "the highlight wraps");
    }

    #[test]
    fn keys_map_to_switches_and_save_and_load() {
        let mut keys = SwitchKeys::default();
        assert_eq!(keys.switch_for(KEY_SPACE), Some(Switch::Next));
        assert_eq!(keys.switch_for(KEY_ENTER), Some(Switch::Select));
        assert_eq!(keys.switch_for(30), None);
        assert!(keys.bind(30, 30).is_err());
        keys.bind(30, 31).unwrap();
        assert_eq!(SwitchKeys::load(&keys.save()), keys);
        assert_eq!(SwitchKeys::load("garbage"), SwitchKeys::default());
        assert_eq!(SwitchKeys::load("next=5,select=5"), SwitchKeys::default());
    }
}
