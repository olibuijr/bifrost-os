//! Smart folders: the notes a predicate picks out (`bifrost_app::controls::fetch`), listed in the sidebar and the
//! list menu. A smart folder is a view over the notes, never a folder on disk: nothing is written, and a note keeps
//! the folder it is in. The predicates read three fields a note exposes: its text, its last change and its pin.
use super::store::Note;
use bifrost_app::controls::fetch::{Field, Op, Predicate, Record};

/// A week in Unix seconds: the window of "Edited This Week".
pub const WEEK_SECS: i64 = 7 * 24 * 60 * 60;

/// One smart folder. The order is the order the sidebar lists them in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Smart {
    /// Pinned notes, wherever they are.
    Pinned,
    /// Notes changed within the last week.
    Recent,
    /// Notes with a checklist line (`- [ ] ` or `- [x] `, either case).
    Checklists,
    /// Notes with an attached picture (an image line, `![...](...)`).
    Photos,
}

impl Smart {
    pub const ALL: [Smart; 4] = [Smart::Pinned, Smart::Recent, Smart::Checklists, Smart::Photos];

    pub fn label(self) -> &'static str {
        match self {
            Smart::Pinned => "Pinned",
            Smart::Recent => "Edited This Week",
            Smart::Checklists => "Checklists",
            Smart::Photos => "Photos",
        }
    }

    /// The symbol name the sidebar draws beside the label.
    pub fn symbol(self) -> &'static str {
        match self {
            Smart::Pinned => "bookmark",
            Smart::Recent => "clock",
            Smart::Checklists => "list.bullet",
            Smart::Photos => "doc",
        }
    }

    /// The name the restoration key uses (`smart:<key>`).
    pub fn key(self) -> &'static str {
        match self {
            Smart::Pinned => "pinned",
            Smart::Recent => "recent",
            Smart::Checklists => "checklists",
            Smart::Photos => "photos",
        }
    }

    pub fn from_key(key: &str) -> Option<Smart> {
        Smart::ALL.into_iter().find(|s| s.key() == key)
    }

    /// The fetch predicate of the folder. `cutoff` is the Unix time before which a note is not recent.
    pub fn predicate(self, cutoff: i64) -> Predicate {
        match self {
            Smart::Pinned => Predicate::compare("pinned", Op::Eq, Field::Bool(true)),
            Smart::Recent => Predicate::compare("modified", Op::Ge, Field::Int(cutoff)),
            Smart::Checklists => Predicate::contains("text", "- [ ] ").or(Predicate::contains("text", "- [x] ")),
            Smart::Photos => Predicate::contains("text", "!["),
        }
    }

    /// Whether `note` (pinned or not) belongs in the folder at `cutoff`.
    pub fn matches(self, note: &Note, pinned: bool, cutoff: i64) -> bool {
        self.predicate(cutoff).matches(&Fields { note, pinned })
    }
}

/// The fields of a note that the predicates read.
struct Fields<'a> {
    note: &'a Note,
    pinned: bool,
}

impl Record for Fields<'_> {
    fn field(&self, key: &str) -> Option<Field> {
        match key {
            "text" => Some(Field::Text(self.note.text.clone())),
            "modified" => Some(Field::Int(self.note.modified)),
            "pinned" => Some(Field::Bool(self.pinned)),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(text: &str, modified: i64) -> Note {
        Note { id: "1".into(), text: text.into(), modified }
    }

    #[test]
    fn checklists_need_a_checkbox_line_in_either_case() {
        assert!(Smart::Checklists.matches(&note("Shop\n- [ ] milk", 0), false, 0));
        assert!(Smart::Checklists.matches(&note("- [X] done", 0), false, 0));
        assert!(!Smart::Checklists.matches(&note("A plain line [ ] not a box", 0), false, 0));
    }

    #[test]
    fn recent_compares_the_last_change_with_the_cutoff() {
        let n = note("x", 1000);
        assert!(Smart::Recent.matches(&n, false, 900));
        assert!(Smart::Recent.matches(&n, false, 1000), "a change exactly at the cutoff counts");
        assert!(!Smart::Recent.matches(&n, false, 1001));
    }

    #[test]
    fn pinned_follows_the_pin_not_the_text() {
        let n = note("", 0);
        assert!(Smart::Pinned.matches(&n, true, 0));
        assert!(!Smart::Pinned.matches(&n, false, 0));
    }

    #[test]
    fn photos_need_an_image_line() {
        assert!(Smart::Photos.matches(&note("Trip\n![image](a-1.png)", 0), false, 0));
        assert!(!Smart::Photos.matches(&note("Trip", 0), false, 0));
    }

    #[test]
    fn keys_round_trip_and_unknown_keys_are_refused() {
        for s in Smart::ALL {
            assert_eq!(Smart::from_key(s.key()), Some(s));
        }
        assert_eq!(Smart::from_key("nope"), None);
        assert_eq!(Smart::Recent.label(), "Edited This Week");
    }
}
