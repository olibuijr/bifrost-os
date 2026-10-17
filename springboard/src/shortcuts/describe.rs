//! Describe a Shortcut: draft a shortcut from a plain sentence, such as "turn off Wi-Fi, then start a timer for 10
//! minutes". There is no language model: the sentence is cut into clauses at "and", "then", commas and full stops, and
//! each clause is matched against the device steps the Shortcuts app offers (Wi-Fi, Low Power Mode, Do Not Disturb,
//! brightness, screenshot, timer, open an app or a folder, a notification, copy text). A clause that names none of them
//! is not guessed at: it is returned in `ignored` so the person sees what was left out and can add the step by hand.
//!
//! The draft is a normal `Shortcut`, so it is stored, edited and run like any other; nothing runs by itself.
//!
//! ```ignore
//! let d = describe::draft("Turn off Wi-Fi and low power mode, then open Notes");
//! // d.shortcut.actions: Wifi(false), LowPower(false), OpenApp("Notes")
//! ```

use super::{clean_name, Action, Shortcut, APP_CHOICES, COPY_CHOICES, FOLDER_CHOICES, MAX_ACTIONS, NOTE_CHOICES};

/// What a description turned into.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Draft {
    /// The drafted shortcut: named after the first words of the description, with the steps that were understood.
    pub shortcut: Shortcut,
    /// One plan line per drafted step, in order.
    pub understood: Vec<String>,
    /// The clauses that became no step: not recognised, or past the step limit.
    pub ignored: Vec<String>,
}

/// The lower-case clauses of a description.
fn clauses(description: &str) -> Vec<String> {
    let mut text = description.to_lowercase();
    for separator in [" and then ", " then ", " and ", ",", ";", "."] {
        text = text.replace(separator, "|");
    }
    text.split('|').map(|c| c.trim().to_string()).filter(|c| !c.is_empty()).collect()
}

/// The runs of letters and digits of a clause.
fn tokens(clause: &str) -> Vec<String> {
    clause.split(|c: char| !c.is_alphanumeric()).filter(|t| !t.is_empty()).map(str::to_string).collect()
}

/// Whether the clause says on or off; `None` when it says neither.
fn polarity(tokens: &[String]) -> Option<bool> {
    let any = |words: &[&str]| tokens.iter().any(|t| words.contains(&t.as_str()));
    if any(&["off", "disable", "disabled"]) {
        Some(false)
    } else if any(&["on", "enable", "enabled"]) {
        Some(true)
    } else {
        None
    }
}

/// The step a clause asks for. `last_toggle` is the on/off of the clause before when that was a switch, so "turn off
/// Wi-Fi and low power mode" turns both off; a clause that is not a switch clears it.
fn clause_action(clause: &str, last_toggle: &mut Option<bool>) -> Option<Action> {
    let t = tokens(clause);
    let has = |w: &str| t.iter().any(|x| x == w);
    let mentions = |s: &str| clause.contains(s);

    let switch = if mentions("wi-fi") || mentions("wifi") || mentions("wi fi") {
        Some(0)
    } else if mentions("low power") {
        Some(1)
    } else if mentions("do not disturb") || has("dnd") {
        Some(2)
    } else {
        None
    };
    if let Some(kind) = switch {
        let on = polarity(&t).or(*last_toggle).unwrap_or(true);
        *last_toggle = Some(on);
        return Some(match kind {
            0 => Action::Wifi(on),
            1 => Action::LowPower(on),
            _ => Action::Dnd(on),
        });
    }
    *last_toggle = None;

    let number = t.iter().find_map(|x| x.parse::<u32>().ok());
    if mentions("brightness") {
        return Some(Action::Brightness(number.unwrap_or(50).min(100) as u8));
    }
    if mentions("screenshot") || mentions("screen shot") {
        return Some(Action::Screenshot);
    }
    if has("timer") {
        let unit = if has("hour") || has("hours") { 60 } else { 1 };
        return Some(Action::Timer(number.map_or(5, |n| n.saturating_mul(unit).clamp(1, 180))));
    }
    if mentions("notif") || has("notify") || has("remind") {
        let note = NOTE_CHOICES.iter().find(|n| clause.contains(&n.to_lowercase()))?;
        return Some(Action::Notify((*note).to_string()));
    }
    if has("copy") {
        let text = COPY_CHOICES.iter().find(|c| clause.contains(&c.to_lowercase()))?;
        return Some(Action::CopyText((*text).to_string()));
    }
    if has("open") || has("show") || has("launch") {
        let folder = FOLDER_CHOICES.iter().find(|f| clause.contains(&f.to_lowercase())).copied();
        let app = APP_CHOICES.iter().find(|a| clause.contains(&a.to_lowercase())).copied();
        return match (has("folder"), folder, app) {
            (true, Some(f), _) => Some(Action::OpenFolder(f.to_string())),
            (_, _, Some(a)) => Some(Action::OpenApp(a.to_string())),
            (_, Some(f), None) => Some(Action::OpenFolder(f.to_string())),
            _ => None,
        };
    }
    None
}

/// A name for the draft: the first five words of the description, capitalised, without closing punctuation.
fn name_for(description: &str) -> String {
    let words: Vec<&str> = description.split_whitespace().take(5).collect();
    let joined = words.join(" ");
    let mut chars = joined.chars();
    let titled = match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    };
    let name = clean_name(titled.trim_end_matches(|c: char| ".,;:!?".contains(c)));
    if name.is_empty() {
        "New Shortcut".to_string()
    } else {
        name
    }
}

/// Drafts a shortcut from `description`.
pub fn draft(description: &str) -> Draft {
    let mut actions: Vec<Action> = Vec::new();
    let mut ignored: Vec<String> = Vec::new();
    let mut last_toggle = None;
    for clause in clauses(description) {
        match clause_action(&clause, &mut last_toggle) {
            Some(action) if actions.len() < MAX_ACTIONS => actions.push(action),
            _ => ignored.push(clause),
        }
    }
    let understood = actions.iter().map(Action::line).collect();
    Draft { shortcut: Shortcut::new(&name_for(description), 0, actions), understood, ignored }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shortcuts::{parse, serialize};

    fn actions(description: &str) -> Vec<Action> {
        draft(description).shortcut.actions
    }

    #[test]
    fn clauses_become_steps_in_order() {
        assert_eq!(actions("Turn off Wi-Fi, then start a timer for 10 minutes"), vec![Action::Wifi(false), Action::Timer(10)]);
        assert_eq!(
            actions("take a screenshot and then open Notes"),
            vec![Action::Screenshot, Action::OpenApp("Notes".into())]
        );
    }

    #[test]
    fn a_switch_without_on_or_off_follows_the_clause_before() {
        assert_eq!(actions("turn off wi-fi and low power mode"), vec![Action::Wifi(false), Action::LowPower(false)]);
        assert_eq!(actions("turn on do not disturb"), vec![Action::Dnd(true)]);
        // a clause in between ends the carry-over, and with nothing said the switch turns on
        assert_eq!(actions("turn off wi-fi, take a screenshot, low power mode"), vec![Action::Wifi(false), Action::Screenshot, Action::LowPower(true)]);
    }

    #[test]
    fn numbers_are_kept_inside_the_steps_limits() {
        assert_eq!(actions("set brightness to 150"), vec![Action::Brightness(100)]);
        assert_eq!(actions("set the brightness"), vec![Action::Brightness(50)]);
        assert_eq!(actions("start a timer"), vec![Action::Timer(5)]);
        assert_eq!(actions("timer for 2 hours"), vec![Action::Timer(120)]);
        assert_eq!(actions("timer for 9 hours"), vec![Action::Timer(180)]);
        assert_eq!(actions("timer for 0 minutes"), vec![Action::Timer(1)]);
    }

    #[test]
    fn an_app_a_folder_and_a_name_with_both_readings() {
        assert_eq!(actions("open music"), vec![Action::OpenApp("Music".into())]);
        assert_eq!(actions("open the music folder"), vec![Action::OpenFolder("Music".into())]);
        assert_eq!(actions("show downloads"), vec![Action::OpenFolder("Downloads".into())]);
    }

    #[test]
    fn notifications_and_copied_text_come_from_the_fixed_choices() {
        assert_eq!(actions("show a notification that says dinner is ready"), vec![Action::Notify("Dinner is ready".into())]);
        assert_eq!(actions("copy on my way"), vec![Action::CopyText("On my way".into())]);
        let d = draft("notify me about the weather");
        assert!(d.shortcut.actions.is_empty());
        assert_eq!(d.ignored, vec!["notify me about the weather".to_string()]);
    }

    #[test]
    fn what_is_not_understood_is_reported_not_guessed() {
        let d = draft("Bake a cake and send it, then take a screenshot");
        assert_eq!(d.shortcut.actions, vec![Action::Screenshot]);
        assert_eq!(d.ignored, vec!["bake a cake".to_string(), "send it".to_string()]);
        assert_eq!(d.understood, vec!["Take Screenshot".to_string()]);
    }

    #[test]
    fn the_name_comes_from_the_first_words() {
        assert_eq!(draft("turn off wi-fi and low power mode, then open Notes").shortcut.name, "Turn off wi-fi and low");
        assert_eq!(draft("").shortcut.name, "New Shortcut");
        assert_eq!(draft("take a screenshot.").shortcut.name, "Take a screenshot");
    }

    #[test]
    fn steps_stop_at_the_limit_and_the_rest_is_ignored() {
        let text = vec!["take a screenshot"; MAX_ACTIONS + 3].join(", ");
        let d = draft(&text);
        assert_eq!(d.shortcut.actions.len(), MAX_ACTIONS);
        assert_eq!(d.ignored.len(), 3);
    }

    #[test]
    fn a_draft_is_stored_like_any_shortcut() {
        let d = draft("turn off wi-fi, then start a timer for 10 minutes");
        assert_eq!(parse(&serialize(&[d.shortcut.clone()])), vec![d.shortcut]);
    }
}
