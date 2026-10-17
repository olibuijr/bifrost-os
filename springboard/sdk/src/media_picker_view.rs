//! The media picker as a list an app or the shell draws: one row per visible item with its title, a subtitle, whether
//! it is chosen and whether it can be chosen. The model is `bifrost_ui::media_picker`; this module only shapes the rows
//! and the footer count, and draws nothing.

use bifrost_ui::media_picker::{Error, Kind, MediaPicker, PickItem};

/// One row of the picker list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub id: u64,
    pub title: String,
    /// Artist and album joined with a middle dot, or the kind name when both are empty.
    pub subtitle: String,
    pub chosen: bool,
    /// False for items listed but not playable; they are drawn dimmed and ignore taps.
    pub enabled: bool,
}

/// The name shown for a kind when an item has no artist or album.
pub fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Song => "Song",
        Kind::Album => "Album",
        Kind::Playlist => "Playlist",
    }
}

fn subtitle(item: &PickItem) -> String {
    let parts: Vec<&str> = [item.artist.as_str(), item.album.as_str()].into_iter().filter(|s| !s.is_empty()).collect();
    if parts.is_empty() {
        kind_name(item.kind).to_string()
    } else {
        parts.join(" \u{b7} ")
    }
}

/// The rows in the order the picker lists them.
pub fn rows(picker: &MediaPicker, items: &[PickItem]) -> Vec<Row> {
    picker
        .visible()
        .into_iter()
        .filter_map(|id| items.iter().find(|i| i.id == id).map(|item| (id, item)))
        .map(|(id, item)| Row {
            id,
            title: item.title.clone(),
            subtitle: subtitle(item),
            chosen: picker.chosen().contains(&id),
            enabled: picker.can_choose(id),
        })
        .collect()
}

/// The footer text: how many are chosen and how many more the rule allows.
pub fn footer(picker: &MediaPicker) -> String {
    let chosen = picker.chosen().len();
    match picker.remaining() {
        Some(0) => format!("{chosen} selected"),
        Some(left) => format!("{chosen} selected, {left} more allowed"),
        None => format!("{chosen} selected"),
    }
}

/// A tap on a row: toggles the item. A tap on a row that cannot be chosen returns its error and changes nothing.
pub fn tap(picker: &mut MediaPicker, id: u64) -> Result<bool, Error> {
    picker.toggle(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_ui::media_picker::Selection;

    fn item(id: u64, kind: Kind, title: &str, playable: bool) -> PickItem {
        PickItem {
            id,
            kind,
            title: title.to_string(),
            artist: "Ann".to_string(),
            album: String::new(),
            playable,
        }
    }

    fn fixture() -> (Vec<PickItem>, MediaPicker) {
        let items = vec![
            item(1, Kind::Song, "Beta", true),
            item(2, Kind::Song, "Alpha", false),
            item(3, Kind::Album, "Gamma", true),
        ];
        let picker = MediaPicker::new(items.clone(), vec![Kind::Song], Selection::Many { max: Some(1) });
        (items, picker)
    }

    #[test]
    fn rows_list_only_accepted_kinds_sorted_by_title_with_playability() {
        let (items, picker) = fixture();
        let rows = rows(&picker, &items);
        let titles: Vec<&str> = rows.iter().map(|r| r.title.as_str()).collect();
        assert_eq!(titles, vec!["Alpha", "Beta"], "the album is not listed");
        assert!(!rows[0].enabled, "an unplayable item stays listed but disabled");
        assert!(rows[1].enabled);
        assert_eq!(rows[1].subtitle, "Ann");
    }

    #[test]
    fn a_tap_marks_the_row_and_the_footer_counts_it() {
        let (items, mut picker) = fixture();
        assert_eq!(tap(&mut picker, 1), Ok(true));
        let rows = rows(&picker, &items);
        assert!(rows.iter().find(|r| r.id == 1).is_some_and(|r| r.chosen));
        assert_eq!(footer(&picker), "1 selected");
    }

    #[test]
    fn a_tap_on_an_unplayable_row_is_refused_and_changes_nothing() {
        let (_, mut picker) = fixture();
        assert_eq!(tap(&mut picker, 2), Err(Error::NotPlayable));
        assert!(picker.chosen().is_empty());
    }

    #[test]
    fn a_subtitle_falls_back_to_the_kind_name() {
        let mut bare = item(9, Kind::Playlist, "Mix", true);
        bare.artist.clear();
        assert_eq!(subtitle(&bare), "Playlist");
    }
}
