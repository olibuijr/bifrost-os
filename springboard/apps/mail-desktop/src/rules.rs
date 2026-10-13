//! Mail rules (Message > New Rule, Apply Rules). A rule has one condition on a header of the message (From, To, Subject,
//! or any of the three contains some text) and one action. Rules run in list order; the first one that matches a
//! message decides what happens to it. They are kept in the app's data directory, one rule a line, fields split by tabs.

use crate::message::Summary;
use std::path::Path;

/// File name of the rules in the app's data directory.
pub const FILE: &str = "rules.txt";

/// Which header a rule looks at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    From,
    To,
    Subject,
    Any,
}

impl Part {
    pub fn name(self) -> &'static str {
        match self {
            Part::From => "from",
            Part::To => "to",
            Part::Subject => "subject",
            Part::Any => "any",
        }
    }

    pub fn parse(text: &str) -> Option<Part> {
        match text.trim().to_ascii_lowercase().as_str() {
            "from" => Some(Part::From),
            "to" => Some(Part::To),
            "subject" => Some(Part::Subject),
            "any" | "" => Some(Part::Any),
            _ => None,
        }
    }
}

/// What a rule does to a message that matches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    MoveTo(String),
    Archive,
    Trash,
    Junk,
    MarkRead,
    Flag,
}

impl Action {
    /// The text a rule is typed and stored as: `move:Work`, `archive`, `trash`, `junk`, `read` or `flag`.
    pub fn text(&self) -> String {
        match self {
            Action::MoveTo(name) => format!("move:{name}"),
            Action::Archive => "archive".into(),
            Action::Trash => "trash".into(),
            Action::Junk => "junk".into(),
            Action::MarkRead => "read".into(),
            Action::Flag => "flag".into(),
        }
    }

    pub fn parse(text: &str) -> Option<Action> {
        let text = text.trim();
        if let Some((head, name)) = text.split_once(':') {
            let name = name.trim();
            return (head.trim().eq_ignore_ascii_case("move") && !name.is_empty()).then(|| Action::MoveTo(name.to_string()));
        }
        match text.to_ascii_lowercase().as_str() {
            "archive" => Some(Action::Archive),
            "trash" | "delete" => Some(Action::Trash),
            "junk" => Some(Action::Junk),
            "read" | "mark read" => Some(Action::MarkRead),
            "flag" => Some(Action::Flag),
            _ => None,
        }
    }

    /// The words the menu shows for this action.
    pub fn label(&self) -> String {
        match self {
            Action::MoveTo(name) => format!("Move to {name}"),
            Action::Archive => "Archive".into(),
            Action::Trash => "Move to Trash".into(),
            Action::Junk => "Move to Junk".into(),
            Action::MarkRead => "Mark as Read".into(),
            Action::Flag => "Flag".into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    pub name: String,
    pub part: Part,
    /// The text to look for, without regard to case.
    pub contains: String,
    pub action: Action,
}

/// Tabs and line breaks would break the file format, so they become spaces.
fn clean(text: &str) -> String {
    text.chars().map(|c| if c == '\t' || c == '\r' || c == '\n' { ' ' } else { c }).collect::<String>().trim().to_string()
}

impl Rule {
    /// A rule from what the New Rule sheet holds. `part` and `action` are typed as words (see [`Part::parse`] and
    /// [`Action::parse`]).
    pub fn from_form(name: &str, part: &str, contains: &str, action: &str) -> Result<Rule, String> {
        let contains = clean(contains);
        if contains.is_empty() {
            return Err("Enter the text the message must contain".into());
        }
        let part = Part::parse(part).ok_or("The condition is from, to, subject or any")?;
        let action = Action::parse(action).ok_or("The action is move:Mailbox, archive, trash, junk, read or flag")?;
        let name = match clean(name) {
            n if n.is_empty() => format!("{} contains {contains}", part.name()),
            n => n,
        };
        Ok(Rule { name, part, contains, action })
    }

    /// True when the message matches the rule's condition.
    pub fn matches(&self, summary: &Summary) -> bool {
        let needle = self.contains.to_lowercase();
        let has = |text: &str| text.to_lowercase().contains(&needle);
        match self.part {
            Part::From => has(&summary.from),
            Part::To => has(&summary.to),
            Part::Subject => has(&summary.subject),
            Part::Any => has(&summary.from) || has(&summary.to) || has(&summary.subject),
        }
    }
}

/// The action of the first rule that matches the message, if any.
pub fn decide<'a>(rules: &'a [Rule], summary: &Summary) -> Option<&'a Action> {
    rules.iter().find(|r| r.matches(summary)).map(|r| &r.action)
}

pub fn parse(text: &str) -> Vec<Rule> {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.split('\t');
            let name = parts.next()?;
            let part = Part::parse(parts.next()?)?;
            let contains = parts.next()?.to_string();
            let action = Action::parse(parts.next()?)?;
            (!contains.is_empty()).then(|| Rule { name: name.to_string(), part, contains, action })
        })
        .collect()
}

pub fn serialize(rules: &[Rule]) -> String {
    rules.iter().map(|r| format!("{}\t{}\t{}\t{}\n", clean(&r.name), r.part.name(), clean(&r.contains), clean(&r.action.text()))).collect()
}

/// Reads the rules. A missing or unreadable file is no rules.
pub fn load(path: &Path) -> Vec<Rule> {
    std::fs::read_to_string(path).map(|t| parse(&t)).unwrap_or_default()
}

pub fn save(path: &Path, rules: &[Rule]) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, serialize(rules)).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(from: &str, subject: &str) -> Summary {
        Summary { from: from.into(), to: "me@example.com".into(), subject: subject.into(), ..Summary::default() }
    }

    #[test]
    fn a_rule_matches_its_header_without_regard_to_case() {
        let rule = Rule::from_form("Shop", "from", "Shop.example", "move:Receipts").unwrap();
        assert!(rule.matches(&summary("Orders <orders@shop.example>", "Hi")));
        assert!(!rule.matches(&summary("Ana <ana@x.is>", "shop.example")));
        let any = Rule::from_form("", "any", "fundur", "flag").unwrap();
        assert!(any.matches(&summary("Ana", "Fundur á morgun")));
        assert_eq!(any.name, "any contains fundur");
    }

    #[test]
    fn the_first_matching_rule_decides() {
        let rules = vec![
            Rule::from_form("a", "subject", "invoice", "archive").unwrap(),
            Rule::from_form("b", "subject", "invoice", "trash").unwrap(),
        ];
        assert_eq!(decide(&rules, &summary("x", "Your invoice")), Some(&Action::Archive));
        assert_eq!(decide(&rules, &summary("x", "Hello")), None);
    }

    #[test]
    fn a_rule_needs_text_a_known_condition_and_a_known_action() {
        assert!(Rule::from_form("n", "from", "  ", "flag").is_err());
        assert!(Rule::from_form("n", "colour", "x", "flag").is_err());
        assert!(Rule::from_form("n", "from", "x", "explode").is_err());
        assert!(Rule::from_form("n", "from", "x", "move:").is_err());
    }

    #[test]
    fn rules_round_trip_through_the_file() {
        let rules = vec![
            Rule::from_form("News", "from", "news@", "read").unwrap(),
            Rule::from_form("Work", "to", "team@", "move:Work/Team").unwrap(),
        ];
        assert_eq!(parse(&serialize(&rules)), rules);
        assert!(parse("broken line\nonly\ttwo\n").is_empty());
    }

    #[test]
    fn tabs_in_typed_text_do_not_split_a_rule() {
        let rule = Rule::from_form("a\tb", "any", "x\ty", "flag").unwrap();
        assert_eq!(parse(&serialize(&[rule.clone()])), vec![Rule { name: "a b".into(), contains: "x y".into(), ..rule }]);
    }
}
