//! Spoken commands for driving the interface by voice: a phrase grammar that turns a recognized utterance into a
//! command, numbered and named targets on the screen, and the resolution of a click command against those targets.
//! The speech engine that hears the phrase is a separate service; this module only reads text.

/// Labels drawn over the screen so a target can be named by its number or its name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Overlay {
    Numbers,
    Names,
    Grid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// Activates the target with this name, or the numbered target when the text is a number.
    Click(String),
    /// Types text into the focused control.
    Dictate(String),
    /// Presses a named key or shortcut, in lower case, such as `return` or `select all`.
    Key(String),
    Scroll(Direction),
    Back,
    Show(Overlay),
    Hide(Overlay),
    /// The phrase is not a command.
    Unknown,
}

/// Key names a phrase may use on its own (`undo`) or after `press` (`press return`).
const KEYS: &[&str] = &[
    "return", "enter", "escape", "tab", "space", "delete", "undo", "redo", "select all", "copy", "cut", "paste",
];

/// Reads one utterance. Matching ignores case, extra spaces and final punctuation. Text that the user says is kept as
/// spoken, so dictated words keep their capitals.
pub fn parse(utterance: &str) -> Command {
    let text = utterance.trim().trim_end_matches(|c: char| matches!(c, '.' | '!' | '?')).trim();
    let whole = collapse(&text.to_ascii_lowercase());
    if KEYS.contains(&whole.as_str()) {
        return Command::Key(whole);
    }
    let (verb, rest) = match text.find(char::is_whitespace) {
        Some(i) => (&text[..i], text[i..].trim()),
        None => (text, ""),
    };
    let rest_key = collapse(&rest.to_ascii_lowercase());
    match verb.to_ascii_lowercase().as_str() {
        "click" | "tap" => with_text(rest, Command::Click),
        "press" => {
            if KEYS.contains(&rest_key.as_str()) {
                Command::Key(rest_key)
            } else {
                with_text(rest, Command::Click)
            }
        }
        "dictate" => with_text(rest, Command::Dictate),
        "scroll" => match rest_key.as_str() {
            "up" => Command::Scroll(Direction::Up),
            "down" => Command::Scroll(Direction::Down),
            "left" => Command::Scroll(Direction::Left),
            "right" => Command::Scroll(Direction::Right),
            _ => Command::Unknown,
        },
        "go" if rest_key == "back" => Command::Back,
        "show" => overlay(&rest_key).map(Command::Show).unwrap_or(Command::Unknown),
        "hide" => overlay(&rest_key).map(Command::Hide).unwrap_or(Command::Unknown),
        _ => Command::Unknown,
    }
}

/// A control on the screen that a command can reach.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub id: u32,
    pub name: String,
    /// Only actionable targets get a number and can be clicked by name.
    pub actionable: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resolution {
    Matched(u32),
    /// More than one target has this name; the caller asks the user which one to use.
    Ambiguous(Vec<u32>),
    NotFound,
}

/// Numbers the actionable targets in order, starting at 1, as the numbers overlay draws them. Returns `(number, id)`
/// pairs.
pub fn number_targets(targets: &[Target]) -> Vec<(u32, u32)> {
    targets
        .iter()
        .filter(|t| t.actionable)
        .enumerate()
        .map(|(i, t)| ((i + 1) as u32, t.id))
        .collect()
}

/// Finds the target a command names. A click text that is a number selects the target with that number; any other
/// text selects the actionable targets whose names match, ignoring case and spacing.
pub fn resolve(command: &Command, targets: &[Target]) -> Resolution {
    let text = match command {
        Command::Click(text) => text,
        _ => return Resolution::NotFound,
    };
    let wanted = collapse(&text.to_ascii_lowercase());
    if wanted.is_empty() {
        return Resolution::NotFound;
    }
    if let Ok(n) = wanted.parse::<u32>() {
        if let Some((_, id)) = number_targets(targets).into_iter().find(|(num, _)| *num == n) {
            return Resolution::Matched(id);
        }
    }
    let hits: Vec<u32> = targets
        .iter()
        .filter(|t| t.actionable && collapse(&t.name.to_ascii_lowercase()) == wanted)
        .map(|t| t.id)
        .collect();
    match hits.len() {
        0 => Resolution::NotFound,
        1 => Resolution::Matched(hits[0]),
        _ => Resolution::Ambiguous(hits),
    }
}

/// Joins the words of a phrase with single spaces.
fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn overlay(name: &str) -> Option<Overlay> {
    match name {
        "numbers" => Some(Overlay::Numbers),
        "names" => Some(Overlay::Names),
        "grid" => Some(Overlay::Grid),
        _ => None,
    }
}

fn with_text(rest: &str, make: fn(String) -> Command) -> Command {
    if rest.is_empty() {
        Command::Unknown
    } else {
        make(rest.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(id: u32, name: &str, actionable: bool) -> Target {
        Target { id, name: name.to_string(), actionable }
    }

    #[test]
    fn overlay_phrases_show_and_hide() {
        assert_eq!(parse("Show numbers."), Command::Show(Overlay::Numbers));
        assert_eq!(parse("hide grid"), Command::Hide(Overlay::Grid));
        assert_eq!(parse("show sounds"), Command::Unknown);
    }

    #[test]
    fn click_takes_the_rest_of_the_phrase() {
        assert_eq!(parse("Click Save File"), Command::Click("Save File".to_string()));
        assert_eq!(parse("tap"), Command::Unknown);
        assert_eq!(parse("click   "), Command::Unknown);
    }

    #[test]
    fn press_names_a_key_or_a_target() {
        assert_eq!(parse("press return"), Command::Key("return".to_string()));
        assert_eq!(parse("Press Export"), Command::Click("Export".to_string()));
        assert_eq!(parse("Undo"), Command::Key("undo".to_string()));
        assert_eq!(parse("select  all"), Command::Key("select all".to_string()));
    }

    #[test]
    fn dictation_keeps_the_spoken_words() {
        assert_eq!(parse("Dictate Hello World!"), Command::Dictate("Hello World".to_string()));
        assert_eq!(parse("dictate"), Command::Unknown);
    }

    #[test]
    fn scroll_takes_a_direction_and_back_is_a_command() {
        assert_eq!(parse("Scroll LEFT"), Command::Scroll(Direction::Left));
        assert_eq!(parse("scroll sideways"), Command::Unknown);
        assert_eq!(parse("go back"), Command::Back);
    }

    #[test]
    fn a_name_matches_actionable_targets_only() {
        let targets = vec![
            target(1, "Save", true),
            target(2, "Cancel", true),
            target(3, "save", true),
            target(4, "Save", false),
        ];
        assert_eq!(resolve(&Command::Click("SAVE".to_string()), &targets), Resolution::Ambiguous(vec![1, 3]));
        assert_eq!(resolve(&Command::Click("cancel".to_string()), &targets), Resolution::Matched(2));
        assert_eq!(resolve(&Command::Click("Delete".to_string()), &targets), Resolution::NotFound);
    }

    #[test]
    fn numbers_count_only_actionable_targets() {
        let targets = vec![
            target(10, "Open", true),
            target(11, "Label", false),
            target(12, "Close", true),
            target(13, "Help", true),
        ];
        assert_eq!(number_targets(&targets), vec![(1, 10), (2, 12), (3, 13)]);
        assert_eq!(resolve(&Command::Click("3".to_string()), &targets), Resolution::Matched(13));
        assert_eq!(resolve(&Command::Click("4".to_string()), &targets), Resolution::NotFound);
        assert_eq!(resolve(&Command::Back, &targets), Resolution::NotFound);
    }
}
