//! The Playback menu: the speed picker and the subtitle picker of the shell's media pickers, as menu rows.
//!
//! Pure: `menu` turns the movie's state into the rows, and `action` turns a chosen row id back into what to do. The
//! rows come from `springboard::media::pickers` (`SPEEDS`, `speed_label`), the same choices the music player uses.

use bifrost_app::{AppMenu, AppMenuItem};
use springboard::media::pickers::{speed_label, SPEEDS};

/// Row ids of the speed rows: `SPEED_BASE` plus the index in `SPEEDS`.
pub const SPEED_BASE: u32 = 100;
/// Row ids of the subtitle rows: `SUBTITLE_BASE` plus the picker row (0 is Off).
pub const SUBTITLE_BASE: u32 = 200;
/// The most subtitle rows the menu offers (the app menu holds 48 items in all).
pub const MAX_SUBTITLE_ROWS: usize = 24;

/// What a chosen row asks for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    /// Play at this rate.
    Speed(f32),
    /// Choose this subtitle picker row (0 is Off).
    Subtitles(usize),
}

/// The action of a menu row id, or `None` when the id is not a Playback row.
pub fn action(id: u32) -> Option<Action> {
    if let Some(i) = id.checked_sub(SPEED_BASE).filter(|i| (*i as usize) < SPEEDS.len()) {
        return Some(Action::Speed(SPEEDS[i as usize]));
    }
    if let Some(row) = id.checked_sub(SUBTITLE_BASE).filter(|r| (*r as usize) < MAX_SUBTITLE_ROWS) {
        return Some(Action::Subtitles(row as usize));
    }
    None
}

/// The Playback menu. `open` is whether a movie is open, `speed` the rate in force, `rows` the subtitle picker rows (Off
/// first) and `chosen` the row that is checked. The speed that matches `speed` is checked; the Subtitles submenu is
/// off when the movie has no subtitle file.
pub fn menu(open: bool, speed: f32, rows: &[String], chosen: usize) -> AppMenu {
    let mut items = vec![AppMenuItem::submenu("Speed").enabled(open)];
    for (i, s) in SPEEDS.iter().enumerate() {
        items.push(AppMenuItem::new(SPEED_BASE + i as u32, &speed_label(*s)).checked((speed - s).abs() < 0.01).enabled(open).depth(1));
    }
    // Off alone is no choice: the submenu is live only when a file can be picked
    items.push(AppMenuItem::submenu("Subtitles").enabled(open && rows.len() > 1));
    for (i, name) in rows.iter().take(MAX_SUBTITLE_ROWS).enumerate() {
        items.push(AppMenuItem::new(SUBTITLE_BASE + i as u32, name).checked(i == chosen).depth(1));
    }
    AppMenu { title: "Playback".into(), items }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(names: &[&str]) -> Vec<String> {
        std::iter::once("Off").chain(names.iter().copied()).map(String::from).collect()
    }

    #[test]
    fn speed_rows_decode_to_the_offered_speeds() {
        assert_eq!(action(SPEED_BASE), Some(Action::Speed(0.5)));
        assert_eq!(action(SPEED_BASE + 2), Some(Action::Speed(1.0)));
        assert_eq!(action(SPEED_BASE + SPEEDS.len() as u32 - 1), Some(Action::Speed(3.0)));
        assert_eq!(action(SPEED_BASE + SPEEDS.len() as u32), None, "one past the last speed");
    }

    #[test]
    fn subtitle_rows_decode_to_picker_rows_and_other_ids_are_not_ours() {
        assert_eq!(action(SUBTITLE_BASE), Some(Action::Subtitles(0)));
        assert_eq!(action(SUBTITLE_BASE + 3), Some(Action::Subtitles(3)));
        assert_eq!(action(SUBTITLE_BASE + MAX_SUBTITLE_ROWS as u32), None);
        assert_eq!(action(0), None);
        assert_eq!(action(1), None, "the Movie menu's ids are not Playback rows");
        assert_eq!(action(99), None);
    }

    #[test]
    fn the_menu_checks_the_speed_in_force_and_the_chosen_subtitle_row() {
        let m = menu(true, 1.25, &rows(&["en", "is"]), 2);
        assert_eq!(m.title, "Playback");
        let checked: Vec<&str> = m.items.iter().filter(|i| i.checked).map(|i| i.title.as_str()).collect();
        assert_eq!(checked, vec!["1.25x", "is"]);
        // every row sits under its submenu header
        assert!(m.items[0].submenu && m.items[1..=SPEEDS.len()].iter().all(|i| i.depth == 1 && i.id >= SPEED_BASE));
        let subs = m.items.iter().position(|i| i.submenu && i.title == "Subtitles").expect("the Subtitles submenu");
        assert_eq!(m.items[subs + 1].title, "Off");
        assert_eq!(m.items.len(), 1 + SPEEDS.len() + 1 + 3);
    }

    #[test]
    fn nothing_can_be_chosen_without_a_movie_and_subtitles_need_a_file() {
        let none = menu(false, 1.0, &[], 0);
        assert!(none.items.iter().filter(|i| i.id != 0).all(|i| !i.enabled), "no movie, no choices");
        let plain = menu(true, 1.0, &rows(&[]), 0);
        let subs = plain.items.iter().find(|i| i.submenu && i.title == "Subtitles").unwrap();
        assert!(!subs.enabled, "Off alone is not a choice");
        let with = menu(true, 1.0, &rows(&["en"]), 1);
        assert!(with.items.iter().find(|i| i.submenu && i.title == "Subtitles").unwrap().enabled);
    }
}
