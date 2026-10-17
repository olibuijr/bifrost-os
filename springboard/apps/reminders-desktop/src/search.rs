//! Core Search index of the reminders. Each reminder with a title is indexed under its id, with its title and the first
//! lines of its notes as the description. Edited reminders are indexed again, deleted ones (and reminders whose title
//! was cleared) are removed. The app compares its reminders with what it indexed when the store generation changes.
use crate::model::Reminder;
use bifrost_app::quicksearch::{CSSearchableItem, CSSearchableItemAttributeSet};
use std::collections::HashMap;

/// Search domain of every reminder.
pub const SPOT_DOMAIN: &str = "reminders";
/// The most note characters a reminder indexes as its description.
const DESC_CHARS: usize = 240;

/// What a reminder is indexed with; a change of it means the item is indexed again.
pub fn signature(r: &Reminder) -> String {
    format!("{}\n{}", r.title, r.notes)
}

/// The searchable item of a reminder: its title, and its notes' first non-empty lines joined as the description.
pub fn index_item(r: &Reminder) -> CSSearchableItem {
    let description: String = r.notes.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" ").chars().take(DESC_CHARS).collect();
    let set = CSSearchableItemAttributeSet::new(r.title.trim()).content_description(&description);
    CSSearchableItem::new(&r.id.to_string(), SPOT_DOMAIN, set)
}

/// Compares `items` with `indexed` (the signature each id was last indexed with) and updates `indexed` to match.
/// Returns the items to index again and the ids to delete.
pub fn sync(indexed: &mut HashMap<u64, String>, items: &[Reminder]) -> (Vec<CSSearchableItem>, Vec<String>) {
    let mut current = HashMap::new();
    let mut changed = vec![];
    for r in items.iter().filter(|r| !r.title.trim().is_empty()) {
        let sig = signature(r);
        if indexed.get(&r.id) != Some(&sig) {
            changed.push(index_item(r));
        }
        current.insert(r.id, sig);
    }
    let gone: Vec<String> = indexed.keys().filter(|id| !current.contains_key(id)).map(u64::to_string).collect();
    *indexed = current;
    (changed, gone)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reminder(id: u64, title: &str, notes: &str) -> Reminder {
        let mut r = Reminder::new(id, title);
        r.notes = notes.to_string();
        r
    }

    #[test]
    fn the_item_has_the_title_and_the_first_note_lines() {
        let r = reminder(7, "  Call mum ", "\n  Ask about Sunday  \n\nBring the blue cake tin\nthird line");
        let item = index_item(&r);
        assert_eq!(item.unique_identifier, "7");
        assert_eq!(item.domain_identifier, SPOT_DOMAIN);
        assert_eq!(item.attribute_set.title, "Call mum");
        assert_eq!(item.attribute_set.content_description, "Ask about Sunday Bring the blue cake tin third line");
    }

    #[test]
    fn the_description_is_capped() {
        let item = index_item(&reminder(1, "Long", &"é".repeat(1000)));
        assert_eq!(item.attribute_set.content_description.chars().count(), DESC_CHARS);
    }

    #[test]
    fn sync_indexes_new_and_edited_reminders_and_deletes_removed_ones() {
        let mut indexed = HashMap::new();
        let mut items = vec![reminder(1, "Milk", ""), reminder(2, "Post", "stamps")];
        let (changed, gone) = sync(&mut indexed, &items);
        assert_eq!(changed.len(), 2);
        assert!(gone.is_empty());

        // nothing changed: nothing is sent
        let (changed, gone) = sync(&mut indexed, &items);
        assert!(changed.is_empty() && gone.is_empty());

        // an edit re-indexes that one; a deletion removes its id
        items[1].notes = "stamps and tape".into();
        items.remove(0);
        let (changed, gone) = sync(&mut indexed, &items);
        assert_eq!(changed.len(), 1);
        assert_eq!(changed[0].unique_identifier, "2");
        assert_eq!(gone, vec!["1".to_string()]);
    }

    #[test]
    fn a_cleared_title_is_not_indexed_and_leaves_the_index() {
        let mut indexed = HashMap::new();
        let mut items = vec![reminder(3, "Pay rent", "")];
        sync(&mut indexed, &items);
        items[0].title = "   ".into();
        let (changed, gone) = sync(&mut indexed, &items);
        assert!(changed.is_empty());
        assert_eq!(gone, vec!["3".to_string()]);
    }
}
