//! Media picker model: the library list an app shows when the user chooses songs, albums or playlists to hand back to
//! the app. The app asks for a kind and a selection rule, and receives only the ids the user confirmed. It never reads
//! the library itself. The model draws nothing: the shell draws [`MediaPicker::visible`] and the marks from
//! [`MediaPicker::chosen`]. Items that cannot be played stay listed, but they cannot be chosen.

use crate::fuzzy;

/// The kind of library item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Song,
    Album,
    Playlist,
}

/// One library item shown in the picker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PickItem {
    pub id: u64,
    pub kind: Kind,
    pub title: String,
    pub artist: String,
    pub album: String,
    /// False for items the user cannot play (for example, not downloaded). They stay listed and disabled.
    pub playable: bool,
}

/// How many items the user may choose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Selection {
    /// Choosing an item replaces the previous choice.
    One,
    /// Any number of items, or at most `max` when it is given.
    Many { max: Option<usize> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// No item with that id is in the library.
    UnknownItem,
    /// The item is listed but cannot be played.
    NotPlayable,
    /// The item's kind is not one the app asked for.
    WrongKind,
    /// The selection already holds the most items the rule allows.
    LimitReached,
    /// `confirm` was called with nothing chosen.
    NothingSelected,
}

#[derive(Clone, Debug)]
pub struct MediaPicker {
    items: Vec<PickItem>,
    /// The kinds the app accepts. Empty means every kind.
    kinds: Vec<Kind>,
    selection: Selection,
    chosen: Vec<u64>,
    query: String,
}

impl MediaPicker {
    pub fn new(items: Vec<PickItem>, kinds: Vec<Kind>, selection: Selection) -> MediaPicker {
        MediaPicker { items, kinds, selection, chosen: Vec::new(), query: String::new() }
    }

    pub fn set_query(&mut self, query: &str) {
        self.query = query.trim().to_string();
    }

    fn item(&self, id: u64) -> Option<&PickItem> {
        self.items.iter().find(|i| i.id == id)
    }

    /// True when `id` is listed, playable and of an accepted kind.
    pub fn can_choose(&self, id: u64) -> bool {
        self.item(id).is_some_and(|i| i.playable && self.accepts(i.kind))
    }

    fn accepts(&self, kind: Kind) -> bool {
        self.kinds.is_empty() || self.kinds.contains(&kind)
    }

    /// The ids the list shows: every item sorted by title, or the matches for the query, best match first.
    pub fn visible(&self) -> Vec<u64> {
        if self.query.is_empty() {
            let mut all: Vec<&PickItem> = self.items.iter().collect();
            all.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()).then(a.id.cmp(&b.id)));
            return all.iter().map(|i| i.id).collect();
        }
        let texts: Vec<String> = self.items.iter().map(|i| format!("{} {} {}", i.title, i.artist, i.album)).collect();
        fuzzy::rank(&self.query, texts.iter().map(|s| s.as_str()))
            .into_iter()
            .map(|(i, _)| self.items[i].id)
            .collect()
    }

    /// Chooses `id` if it is not chosen, or drops it if it is. Returns true when the item is now chosen.
    pub fn toggle(&mut self, id: u64) -> Result<bool, Error> {
        let item = self.item(id).ok_or(Error::UnknownItem)?;
        if !item.playable {
            return Err(Error::NotPlayable);
        }
        if !self.accepts(item.kind) {
            return Err(Error::WrongKind);
        }
        if let Some(at) = self.chosen.iter().position(|&c| c == id) {
            self.chosen.remove(at);
            return Ok(false);
        }
        match self.selection {
            Selection::One => self.chosen = vec![id],
            Selection::Many { max: Some(max) } if self.chosen.len() >= max => return Err(Error::LimitReached),
            Selection::Many { .. } => self.chosen.push(id),
        }
        Ok(true)
    }

    /// The chosen ids in the order the user chose them.
    pub fn chosen(&self) -> &[u64] {
        &self.chosen
    }

    /// The choice handed back to the app. Fails when nothing is chosen.
    pub fn confirm(&self) -> Result<Vec<u64>, Error> {
        if self.chosen.is_empty() {
            return Err(Error::NothingSelected);
        }
        Ok(self.chosen.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: u64, kind: Kind, title: &str, playable: bool) -> PickItem {
        PickItem { id, kind, title: title.to_string(), artist: String::new(), album: String::new(), playable }
    }

    fn library() -> Vec<PickItem> {
        vec![
            item(1, Kind::Song, "Zebra Crossing", true),
            item(2, Kind::Song, "Blue Sky", true),
            item(3, Kind::Album, "Radio Days", true),
            item(4, Kind::Song, "Locked Track", false),
        ]
    }

    #[test]
    fn items_are_listed_by_title() {
        let p = MediaPicker::new(library(), vec![], Selection::One);
        assert_eq!(p.visible(), vec![2, 4, 3, 1]);
    }

    #[test]
    fn a_query_keeps_only_the_matches() {
        let mut p = MediaPicker::new(library(), vec![], Selection::One);
        p.set_query("rad");
        let shown = p.visible();
        assert!(shown.contains(&3), "{shown:?}");
        assert!(!shown.contains(&2), "{shown:?}");
    }

    #[test]
    fn many_items_toggle_on_and_off() {
        let mut p = MediaPicker::new(library(), vec![], Selection::Many { max: None });
        assert_eq!(p.toggle(1), Ok(true));
        assert_eq!(p.toggle(2), Ok(true));
        assert_eq!(p.toggle(1), Ok(false));
        assert_eq!(p.chosen(), &[2]);
        assert_eq!(p.confirm(), Ok(vec![2]));
    }

    #[test]
    fn a_single_choice_replaces_the_previous_one() {
        let mut p = MediaPicker::new(library(), vec![], Selection::One);
        p.toggle(1).unwrap();
        p.toggle(2).unwrap();
        assert_eq!(p.chosen(), &[2]);
    }

    #[test]
    fn the_limit_stops_further_choices() {
        let mut p = MediaPicker::new(library(), vec![], Selection::Many { max: Some(2) });
        p.toggle(1).unwrap();
        p.toggle(2).unwrap();
        assert_eq!(p.toggle(3), Err(Error::LimitReached));
        assert_eq!(p.chosen(), &[1, 2]);
    }

    #[test]
    fn an_unplayable_item_is_listed_but_refused() {
        let mut p = MediaPicker::new(library(), vec![], Selection::Many { max: None });
        assert!(p.visible().contains(&4));
        assert!(!p.can_choose(4));
        assert_eq!(p.toggle(4), Err(Error::NotPlayable));
    }

    #[test]
    fn only_the_requested_kinds_can_be_chosen() {
        let mut p = MediaPicker::new(library(), vec![Kind::Song], Selection::Many { max: None });
        assert_eq!(p.toggle(3), Err(Error::WrongKind));
        assert_eq!(p.toggle(1), Ok(true));
    }

    #[test]
    fn unknown_ids_and_an_empty_choice_are_refused() {
        let mut p = MediaPicker::new(library(), vec![], Selection::One);
        assert_eq!(p.toggle(99), Err(Error::UnknownItem));
        assert_eq!(p.confirm(), Err(Error::NothingSelected));
    }
}
