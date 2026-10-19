//! Voice Control: spoken command words run against the element map, and spoken shortcut titles run app shortcuts
//! ("run Daily Summary" opens the app that declares the shortcut and sends it the intent, as Search does).
//!
//! The parser is pure. The whisper recognizer (`bifrost_ui::whisper`) reports a transcript; only a final one is a command, and a
//! hypothesis the element may still revise is ignored. A command is a verb and a name: "press Save", "click Save" or
//! "tap Save" presses the element called Save, the same way `ax press` does. The shell glue is `Shell::voice_press`,
//! which resolves the name in the element map. The recognizer service (`services::recognizer`) feeds this parser while
//! the `voice_control` preference is `1`: it holds the session, and the effects a press returns go out with the next
//! shell tick. The switch is Settings > Accessibility > Voice Control (the `voice_control` preference).
//! Exactly one recognition runs: the recognizer service's own Voice Control step (`services::recognizer`, which holds the
//! consent check and the retry pause). The shell only consumes what it heard (`Shell::voice_consume`).
//!
//! "show numbers", "show names" and "show grid" label the front window (`voice_overlay`); "hide numbers" and kin remove the
//! label. With numbers shown "press 3" presses the third label; with names shown (or numbers) "press Save" presses the
//! visible element of that name; with the grid shown "press 5" narrows the grid to its fifth cell and "click here" clicks its
//! centre.

use crate::shell::{Effect, Shell};
use bifrost_proto::AnnouncePriority;
use bifrost_ui::voice_commands::Overlay;
use bifrost_ui::whisper::Transcript;

/// One command a spoken transcript asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// Press the element whose name is `name` (the words after the verb, trimmed and lower-cased).
    Press(String),
    /// Run the app shortcut whose title is `name` (lower-cased, without a leading "shortcut").
    Run(String),
    /// Draw the labels or the grid over the front window.
    Show(Overlay),
    /// Remove that overlay.
    Hide(Overlay),
    /// Click the centre of the grid as it stands.
    ClickHere,
}

/// Verbs that press an element, in the words the person may say.
const PRESS_VERBS: [&str; 3] = ["press", "click", "tap"];
/// The verb that runs an app shortcut.
const RUN_VERB: &str = "run";

/// The command in a transcript. None for a hypothesis (not final), a blank transcript, a verb it does not know, or a verb
/// with no name after it. Trailing punctuation the recognizer adds ("Save.") is dropped.
pub fn parse(transcript: &Transcript) -> Option<Command> {
    if !transcript.is_final {
        return None;
    }
    let words = transcript.text.trim().trim_end_matches(['.', '!', '?']).trim();
    let (verb, name) = words.split_once(char::is_whitespace)?;
    let verb = verb.to_lowercase();
    let name = name.trim();
    if verb == "show" || verb == "hide" {
        // the overlay vocabulary is the shared grammar's (`bifrost_ui::voice_commands`)
        return match bifrost_ui::voice_commands::parse(words) {
            bifrost_ui::voice_commands::Command::Show(o) => Some(Command::Show(o)),
            bifrost_ui::voice_commands::Command::Hide(o) => Some(Command::Hide(o)),
            _ => None,
        };
    }
    if PRESS_VERBS[1..].contains(&verb.as_str()) && name.eq_ignore_ascii_case("here") {
        return Some(Command::ClickHere);
    }
    if PRESS_VERBS.contains(&verb.as_str()) {
        return (!name.is_empty()).then(|| Command::Press(name.to_lowercase()));
    }
    if verb == RUN_VERB {
        let lower = name.to_lowercase();
        // "run shortcut" alone names nothing: the word is the grammar, not a title
        let title = if lower == "shortcut" { "" } else { lower.strip_prefix("shortcut ").unwrap_or(&lower).trim() };
        return (!title.is_empty()).then(|| Command::Run(title.to_string()));
    }
    None
}

/// Which shortcut a spoken title names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    /// The index of the one shortcut with this title.
    One(usize),
    /// More than one shortcut has this title; nothing runs.
    Many,
    None,
}

/// The shortcut whose title equals `spoken`, ignoring case and extra spaces. Several equal titles are `Many`.
pub fn pick_title(titles: &[&str], spoken: &str) -> Pick {
    let fold = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
    let wanted = fold(spoken);
    if wanted.is_empty() {
        return Pick::None;
    }
    let mut hits = titles.iter().enumerate().filter(|(_, t)| fold(t) == wanted).map(|(i, _)| i);
    match (hits.next(), hits.next()) {
        (Some(i), None) => Pick::One(i),
        (Some(_), Some(_)) => Pick::Many,
        _ => Pick::None,
    }
}

impl Shell {
    /// Whether Voice Control is on (Settings > Accessibility > Touch > Voice Control).
    pub(in crate::shell) fn voice_control_on(&self) -> bool {
        self.settings.as_ref().map_or_else(|| super::AccessibilitySettings::load(&self.prefs).voice_control, |st| st.accessibility().voice_control)
    }

    /// Runs the final transcripts the recognizer service heard while Voice Control is on, as commands, and returns their
    /// effects. A command that cannot run says why on the announcement banner; a sentence that is no command is ignored.
    /// The shell starts, polls and stops nothing itself: the one recognition lives in `services::recognizer`.
    pub(in crate::shell) fn voice_consume(&mut self, heard: &[Transcript], t: f64) -> Vec<Effect> {
        let mut effects = Vec::new();
        for transcript in heard {
            match self.voice_press(transcript, t) {
                Some(Ok((_, fx))) => {
                    effects.extend(fx);
                    self.dirty = true;
                }
                Some(Err(why)) => {
                    self.announce.post(&why, AnnouncePriority::High);
                    self.dirty = true;
                }
                None => {}
            }
        }
        effects
    }

    /// Runs a final transcript as a Voice Control command. None when the transcript is not a command; otherwise the
    /// result of pressing the named element (its address and effects, or why it could not be pressed), as `ax press`
    /// answers it.
    pub(in crate::shell) fn voice_press(&mut self, transcript: &Transcript, t: f64) -> Option<Result<(String, Vec<Effect>), String>> {
        match parse(transcript)? {
            Command::Press(name) => Some(self.voice_press_named(&name, t)),
            Command::Run(title) => Some(self.voice_run(&title, t)),
            Command::Show(overlay) => Some(Ok((self.voice_show(overlay), Vec::new()))),
            Command::Hide(overlay) => Some(self.voice_hide(overlay).map(|answer| (answer, Vec::new()))),
            // "click here" is a click only while the grid is up; otherwise it names an element called Here
            Command::ClickHere => self.voice_click_here().or_else(|| Some(self.voice_press_named("here", t))),
        }
    }

    /// "press X": a number presses that label (or grid cell); a name presses the visible label of that name while numbers or
    /// names are shown; anything else is looked up in the whole element map, as `ax press` does.
    fn voice_press_named(&mut self, name: &str, t: f64) -> Result<(String, Vec<Effect>), String> {
        if let Ok(n) = name.parse::<u32>() {
            return self.voice_press_number(n, t);
        }
        match self.voice_press_label(name, t) {
            Some(answer) => answer,
            None => self.ax_press(name, t),
        }
    }

    /// Runs the installed app's shortcut titled `title`: opens the app and sends it the intent, as Search does. The
    /// answer names the app and the intent.
    fn voice_run(&mut self, title: &str, t: f64) -> Result<(String, Vec<Effect>), String> {
        let shortcuts = crate::shell::intents::runnable_shortcuts(&crate::shell::apphost::installed());
        let titles: Vec<&str> = shortcuts.iter().map(|s| s.intent.title.as_str()).collect();
        match pick_title(&titles, title) {
            Pick::None => Err(format!("no shortcut {title:?}")),
            Pick::Many => Err(format!("more than one shortcut is called {title:?}")),
            Pick::One(i) => {
                let (app_id, intent) = (shortcuts[i].app_id.clone(), shortcuts[i].intent.id.clone());
                let slot = crate::shell::apphost::slot_of(&app_id).ok_or_else(|| format!("{app_id} is not installed"))?;
                self.open(crate::shell::AppId::External(slot as u8), t);
                let answer = format!("{app_id}:{intent}");
                // the app may still be starting: retry the connection for a few seconds, off the UI thread
                crate::bare::sys::spawn_named("intent-retry", move || {
                    if let Err(e) = crate::shell::intents::run(&app_id, &intent, std::time::Duration::from_secs(8)) {
                        eprintln!("voice control: intent {intent} of {app_id}: {e}");
                    }
                });
                Ok((answer, Vec::new()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fin(text: &str) -> Transcript {
        Transcript { text: text.into(), is_final: true }
    }

    #[test]
    fn a_press_verb_names_the_element_to_press() {
        assert_eq!(parse(&fin("press Save")), Some(Command::Press("save".into())));
        assert_eq!(parse(&fin("Click Software Update")), Some(Command::Press("software update".into())));
        assert_eq!(parse(&fin("  tap   Close window  ")), Some(Command::Press("close window".into())));
    }

    #[test]
    fn trailing_punctuation_from_the_recognizer_is_dropped() {
        assert_eq!(parse(&fin("Press Save.")), Some(Command::Press("save".into())));
        assert_eq!(parse(&fin("tap Done!")), Some(Command::Press("done".into())));
    }

    #[test]
    fn run_names_an_app_shortcut_with_or_without_the_word_shortcut() {
        assert_eq!(parse(&fin("run Daily Summary")), Some(Command::Run("daily summary".into())));
        assert_eq!(parse(&fin("Run shortcut New Note.")), Some(Command::Run("new note".into())));
        assert_eq!(parse(&fin("run")), None);
        assert_eq!(parse(&fin("run shortcut ")), None);
        assert_eq!(parse(&fin("running late")), None, "the verb must be a whole word");
    }

    #[test]
    fn a_spoken_title_picks_one_shortcut_ignoring_case_and_spacing() {
        let titles = ["New Note", "Daily  Summary", "new note", "Timer"];
        assert_eq!(pick_title(&titles, "daily summary"), Pick::One(1));
        assert_eq!(pick_title(&titles, "TIMER"), Pick::One(3));
        assert_eq!(pick_title(&titles, "New Note"), Pick::Many, "two shortcuts share the title: ask which");
        assert_eq!(pick_title(&titles, "weather"), Pick::None);
        assert_eq!(pick_title(&titles, "  "), Pick::None);
        assert_eq!(pick_title(&[], "timer"), Pick::None);
    }

    #[test]
    fn a_hypothesis_is_never_a_command() {
        let partial = Transcript { text: "press Save".into(), is_final: false };
        assert_eq!(parse(&partial), None);
    }

    #[test]
    fn unknown_verbs_and_missing_names_are_not_commands() {
        assert_eq!(parse(&fin("open Settings")), None);
        assert_eq!(parse(&fin("press")), None, "a verb with no name");
        assert_eq!(parse(&fin("press   ")), None);
        assert_eq!(parse(&fin("")), None);
        assert_eq!(parse(&fin("pressing Save")), None, "the verb must be a whole word");
    }
}
