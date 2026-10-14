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

/// One condition of a user-defined smart folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rule {
    /// the note has this `#tag` (lower case)
    Tag(String),
    /// edited within the last this many days
    Edited(i64),
    /// created within the last this many days
    Created(i64),
    /// has an attached picture
    Attachments,
    /// has a checklist line
    Checklist,
    Pinned,
    /// lives in this folder
    Folder(String),
}

impl Rule {
    /// The rule as one word of the filter syntax: `tag:work`, `edited:7`, `created:30`, `attachments`,
    /// `checklist`, `pinned`, `folder:Name`.
    pub fn token(&self) -> String {
        match self {
            Rule::Tag(t) => format!("tag:{t}"),
            Rule::Edited(d) => format!("edited:{d}"),
            Rule::Created(d) => format!("created:{d}"),
            Rule::Attachments => "attachments".into(),
            Rule::Checklist => "checklist".into(),
            Rule::Pinned => "pinned".into(),
            Rule::Folder(f) => format!("folder:{f}"),
        }
    }

    /// Reads one word of the filter syntax; None for an unknown word or a bad number.
    pub fn parse(word: &str) -> Option<Rule> {
        let lower = word.to_lowercase();
        match lower.as_str() {
            "attachments" => return Some(Rule::Attachments),
            "checklist" => return Some(Rule::Checklist),
            "pinned" => return Some(Rule::Pinned),
            _ => {}
        }
        let (key, value) = word.split_once(':')?;
        if value.is_empty() {
            return None;
        }
        match key.to_lowercase().as_str() {
            "tag" => Some(Rule::Tag(value.trim_start_matches('#').to_lowercase())),
            "edited" => value.parse().ok().filter(|d| *d > 0).map(Rule::Edited),
            "created" => value.parse().ok().filter(|d| *d > 0).map(Rule::Created),
            "folder" => Some(Rule::Folder(value.to_string())),
            _ => None,
        }
    }
}

/// A smart folder the user made (File > New Smart Folder): a name and conditions that all must hold, or any of them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Filter {
    pub name: String,
    pub any: bool,
    pub rules: Vec<Rule>,
}

/// The note facts a [`Filter`] reads besides the note itself.
pub struct Context<'a> {
    pub pinned: bool,
    /// the folder the note is in ("" is none)
    pub folder: &'a str,
    /// the current time, Unix seconds
    pub now: i64,
}

impl Filter {
    /// Reads `Name | rules` (or just rules, which then name the folder). The rules are space-separated words of
    /// [`Rule::parse`]; the word `any` switches from all-of to any-of. Err carries the first word that is not understood.
    pub fn parse(definition: &str) -> Result<Filter, String> {
        let (name, rules) = match definition.split_once('|') {
            Some((n, r)) => (n.trim(), r),
            None => ("", definition),
        };
        let mut any = false;
        let mut parsed = vec![];
        for word in rules.split_whitespace() {
            if word.eq_ignore_ascii_case("any") {
                any = true;
            } else if word.eq_ignore_ascii_case("all") {
                any = false;
            } else {
                parsed.push(Rule::parse(word).ok_or_else(|| word.to_string())?);
            }
        }
        if parsed.is_empty() {
            return Err(String::new());
        }
        let name: String = if name.is_empty() { rules.trim().to_string() } else { name.to_string() };
        let name: String = name.chars().filter(|c| !c.is_control()).take(40).collect();
        Ok(Filter { name, any, rules: parsed })
    }

    /// The rules as the text `parse` reads back.
    pub fn definition(&self) -> String {
        let mut words: Vec<String> = self.rules.iter().map(Rule::token).collect();
        if self.any {
            words.push("any".into());
        }
        format!("{} | {}", self.name, words.join(" "))
    }

    /// Whether `note` satisfies the conditions.
    pub fn matches(&self, note: &Note, cx: &Context) -> bool {
        let test = |rule: &Rule| match rule {
            Rule::Tag(t) => super::store::tags(&note.text).contains(t),
            Rule::Edited(d) => note.modified >= cx.now - d * 86_400,
            Rule::Created(d) => super::store::created(note) >= cx.now - d * 86_400,
            Rule::Attachments => note.text.contains("!["),
            Rule::Checklist => note.checklist().1 > 0,
            Rule::Pinned => cx.pinned,
            Rule::Folder(f) => cx.folder == f,
        };
        if self.any {
            self.rules.iter().any(test)
        } else {
            self.rules.iter().all(test)
        }
    }
}

const FILE: &str = ".springboard-notes-smart";

/// The saved smart folders, one per line as `name<TAB>definition-rules`.
pub fn load(dir: &std::path::Path) -> Vec<Filter> {
    std::fs::read_to_string(dir.join(FILE))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| {
            let (name, rules) = line.split_once('\t')?;
            Filter::parse(&format!("{name} | {rules}")).ok()
        })
        .collect()
}

pub fn save(dir: &std::path::Path, filters: &[Filter]) -> std::io::Result<()> {
    let body: String = filters
        .iter()
        .map(|f| {
            let definition = f.definition();
            let rules = definition.split_once(" | ").map_or("", |(_, r)| r);
            format!("{}\t{}\n", f.name.replace(['\t', '\n', '|'], " "), rules)
        })
        .collect();
    springboard::shell::fsutil::write_atomic(&dir.join(FILE), body)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cx(pinned: bool, folder: &str, now: i64) -> Context<'_> {
        Context { pinned, folder, now }
    }

    #[test]
    fn a_definition_names_the_folder_and_lists_its_rules() {
        let f = Filter::parse("Work | tag:#Work edited:7 pinned").unwrap();
        assert_eq!(f.name, "Work");
        assert!(!f.any);
        assert_eq!(f.rules, vec![Rule::Tag("work".into()), Rule::Edited(7), Rule::Pinned]);
        let bare = Filter::parse("checklist attachments any").unwrap();
        assert_eq!(bare.name, "checklist attachments any", "the rules name an unnamed folder");
        assert!(bare.any);
    }

    #[test]
    fn unknown_words_and_empty_rules_are_refused() {
        assert_eq!(Filter::parse("N | color:red"), Err("color:red".to_string()));
        assert_eq!(Filter::parse("N | edited:0"), Err("edited:0".to_string()));
        assert_eq!(Filter::parse("N | edited:x"), Err("edited:x".to_string()));
        assert_eq!(Filter::parse("N |"), Err(String::new()));
        assert_eq!(Filter::parse(""), Err(String::new()));
    }

    #[test]
    fn all_of_and_any_of_conditions() {
        let n = Note { id: "1".into(), text: "Plan #work\n- [ ] a".into(), modified: 1000 };
        let all = Filter::parse("W | tag:work checklist").unwrap();
        assert!(all.matches(&n, &cx(false, "", 1000)));
        let miss = Filter::parse("W | tag:work pinned").unwrap();
        assert!(!miss.matches(&n, &cx(false, "", 1000)));
        let any = Filter::parse("W | tag:work pinned any").unwrap();
        assert!(any.matches(&n, &cx(false, "", 1000)));
        assert!(!Filter::parse("W | tag:home attachments any").unwrap().matches(&n, &cx(false, "", 1000)));
    }

    #[test]
    fn date_rules_count_days_back_from_now() {
        let day = 86_400;
        let n = Note { id: "1".into(), text: "x".into(), modified: 10 * day };
        assert!(Filter::parse("E | edited:2").unwrap().matches(&n, &cx(false, "", 11 * day)));
        assert!(!Filter::parse("E | edited:2").unwrap().matches(&n, &cx(false, "", 13 * day)));
        // an id that is not a creation time falls back to the last change
        assert!(Filter::parse("C | created:2").unwrap().matches(&n, &cx(false, "", 11 * day)));
    }

    #[test]
    fn folder_and_pin_rules_read_the_context() {
        let n = Note { id: "1".into(), text: "x".into(), modified: 0 };
        assert!(Filter::parse("F | folder:Work").unwrap().matches(&n, &cx(false, "Work", 0)));
        assert!(!Filter::parse("F | folder:Work").unwrap().matches(&n, &cx(false, "Home", 0)));
        assert!(Filter::parse("P | pinned").unwrap().matches(&n, &cx(true, "", 0)));
    }

    #[test]
    fn saved_filters_survive_a_reload() {
        let dir = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-notes")).join(format!("{}-smart", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(load(&dir).is_empty());
        let filters = vec![Filter::parse("Work | tag:work pinned any").unwrap(), Filter::parse("Recent | edited:3").unwrap()];
        save(&dir, &filters).unwrap();
        assert_eq!(load(&dir), filters);
        let _ = std::fs::remove_dir_all(&dir);
    }

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
