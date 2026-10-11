//! Contact card model: the rows a contact viewer shows for one contact and what each row can do. The viewer opens in one
//! of three kinds: an existing contact, an unknown contact (a name only, for example from a message), or a new contact
//! being filled in. The model picks the title, keeps the displayed keys, and says which rows offer a call or an email.
//! It draws nothing and owns no contacts; the caller builds an [`Entry`] from its store.

/// What a card row holds. The name is the entry's title and is never a row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Key {
    Company,
    Phone,
    Email,
    Address,
    Note,
}

/// Every key, in the order a card shows them.
pub const ALL_KEYS: [Key; 5] = [Key::Company, Key::Phone, Key::Email, Key::Address, Key::Note];

/// One labelled value on a contact, such as the home phone number.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    pub key: Key,
    pub label: String,
    pub value: String,
}

/// A contact as the viewer and the picker see it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Entry {
    pub id: u64,
    pub name: String,
    pub fields: Vec<Field>,
}

impl Entry {
    /// The non-empty values under `key`, in the order the contact gives them.
    pub fn values(&self, key: Key) -> Vec<&Field> {
        self.fields.iter().filter(|f| f.key == key && !f.value.trim().is_empty()).collect()
    }

    /// True when the contact has at least one value under `key`.
    pub fn has(&self, key: Key) -> bool {
        !self.values(key).is_empty()
    }
}

/// The kind of card: which contact it shows, or a blank one to fill in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Existing,
    Unknown,
    New,
}

/// What a row can do when the user activates it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    None,
    Call,
    Email,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub key: Key,
    pub label: String,
    pub value: String,
    pub action: Action,
}

/// How a card is shown: its kind, the fallback title, the line under the name, the keys to show and the actions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Card {
    pub kind: Kind,
    /// Shown as the title when the entry has no name.
    pub alternate_name: String,
    /// Shown under the name.
    pub message: String,
    /// The keys to show. `None` shows every key.
    pub displayed: Option<Vec<Key>>,
    pub allows_editing: bool,
    /// When false, phone and email rows offer no call or email action.
    pub allows_actions: bool,
}

impl Card {
    /// A card of `kind` that shows every key, allows editing and offers actions.
    pub fn new(kind: Kind) -> Card {
        Card {
            kind,
            alternate_name: String::new(),
            message: String::new(),
            displayed: None,
            allows_editing: true,
            allows_actions: true,
        }
    }

    /// The title: the entry's name, else the alternate name, else a placeholder for the kind.
    pub fn title(&self, entry: &Entry) -> String {
        let name = entry.name.trim();
        if !name.is_empty() {
            return name.to_string();
        }
        let alternate = self.alternate_name.trim();
        if !alternate.is_empty() {
            return alternate.to_string();
        }
        let placeholder = match self.kind {
            Kind::New => "New Contact",
            Kind::Unknown => "Unknown Contact",
            Kind::Existing => "No Name",
        };
        placeholder.to_string()
    }

    /// True when the card shows `key`.
    pub fn shows(&self, key: Key) -> bool {
        self.displayed.as_ref().map_or(true, |keys| keys.contains(&key))
    }

    /// The rows in card order: shown keys only, empty values skipped.
    pub fn rows(&self, entry: &Entry) -> Vec<Row> {
        let mut rows = Vec::new();
        for key in ALL_KEYS {
            if !self.shows(key) {
                continue;
            }
            for field in entry.values(key) {
                let action = match (key, self.allows_actions) {
                    (Key::Phone, true) => Action::Call,
                    (Key::Email, true) => Action::Email,
                    _ => Action::None,
                };
                rows.push(Row { key, label: field.label.clone(), value: field.value.trim().to_string(), action });
            }
        }
        rows
    }
}

/// The index of the row a highlight names: the first row under `key` whose label is `identifier`, or the first row
/// under `key` when `identifier` is `None`.
pub fn highlight_row(rows: &[Row], key: Key, identifier: Option<&str>) -> Option<usize> {
    rows.iter().position(|r| r.key == key && identifier.map_or(true, |id| r.label == id))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(key: Key, label: &str, value: &str) -> Field {
        Field { key, label: label.to_string(), value: value.to_string() }
    }

    fn anna() -> Entry {
        Entry {
            id: 7,
            name: "Anna Lind".to_string(),
            fields: vec![
                field(Key::Email, "work", "anna@example.com"),
                field(Key::Phone, "mobile", "555-1111"),
                field(Key::Phone, "home", "   "),
                field(Key::Phone, "work", "555-2222"),
                field(Key::Company, "", "Lind ehf"),
            ],
        }
    }

    #[test]
    fn title_falls_back_from_name_to_alternate_to_placeholder() {
        let mut card = Card::new(Kind::Existing);
        assert_eq!(card.title(&anna()), "Anna Lind");
        let nameless = Entry { id: 1, name: "  ".to_string(), fields: vec![] };
        assert_eq!(card.title(&nameless), "No Name");
        card.alternate_name = "Mum".to_string();
        assert_eq!(card.title(&nameless), "Mum");
        assert_eq!(Card::new(Kind::New).title(&nameless), "New Contact");
        assert_eq!(Card::new(Kind::Unknown).title(&nameless), "Unknown Contact");
    }

    #[test]
    fn rows_follow_card_order_and_skip_empty_values() {
        let rows = Card::new(Kind::Existing).rows(&anna());
        let order: Vec<(Key, &str)> = rows.iter().map(|r| (r.key, r.value.as_str())).collect();
        assert_eq!(
            order,
            vec![
                (Key::Company, "Lind ehf"),
                (Key::Phone, "555-1111"),
                (Key::Phone, "555-2222"),
                (Key::Email, "anna@example.com"),
            ]
        );
    }

    #[test]
    fn displayed_keys_limit_the_rows() {
        let mut card = Card::new(Kind::Existing);
        card.displayed = Some(vec![Key::Email]);
        let rows = card.rows(&anna());
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].key, Key::Email);
        assert!(!card.shows(Key::Phone));
    }

    #[test]
    fn phone_and_email_rows_offer_actions_only_when_allowed() {
        let mut card = Card::new(Kind::Existing);
        let rows = card.rows(&anna());
        assert_eq!(rows[1].action, Action::Call);
        assert_eq!(rows[3].action, Action::Email);
        assert_eq!(rows[0].action, Action::None);
        card.allows_actions = false;
        assert!(card.rows(&anna()).iter().all(|r| r.action == Action::None));
    }

    #[test]
    fn highlight_finds_row_by_label_or_first_of_key() {
        let rows = Card::new(Kind::Existing).rows(&anna());
        assert_eq!(highlight_row(&rows, Key::Phone, None), Some(1));
        assert_eq!(highlight_row(&rows, Key::Phone, Some("work")), Some(2));
        assert_eq!(highlight_row(&rows, Key::Phone, Some("fax")), None);
        assert_eq!(highlight_row(&rows, Key::Note, None), None);
    }

    #[test]
    fn entry_values_ignore_blank_values() {
        let e = anna();
        assert_eq!(e.values(Key::Phone).len(), 2);
        assert!(e.has(Key::Email));
        assert!(!e.has(Key::Address));
    }
}
