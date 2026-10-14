//! Mail rules: criteria over a message's headers and actions to run when they match. `evaluate` returns the actions
//! for one message in rule order; a rule with `Stop` ends the evaluation. Rules are stored one per line.

use crate::message::{address_of, Summary};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    From,
    To,
    Cc,
    Subject,
    /// To or Cc.
    AnyRecipient,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Test {
    Contains,
    DoesNotContain,
    Is,
    BeginsWith,
    EndsWith,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Criterion {
    pub field: Field,
    pub test: Test,
    pub value: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    MoveTo(String),
    MarkRead,
    Flag,
    Delete,
    /// Stop evaluating later rules.
    Stop,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    pub name: String,
    pub enabled: bool,
    /// All criteria must match when true, any one when false.
    pub all: bool,
    pub criteria: Vec<Criterion>,
    pub actions: Vec<Action>,
}

impl Criterion {
    pub fn matches(&self, s: &Summary) -> bool {
        let fields: Vec<&str> = match self.field {
            Field::From => vec![&s.from],
            Field::To => vec![&s.to],
            Field::Cc => vec![&s.cc],
            Field::Subject => vec![&s.subject],
            Field::AnyRecipient => vec![&s.to, &s.cc],
        };
        let want = self.value.to_lowercase();
        let hit = |text: &str| {
            let text = text.to_lowercase();
            // an address matches on its bare form too, so "Is" works for `Ann <ann@x.is>`
            let bare = address_of(&text).to_string();
            match self.test {
                Test::Contains | Test::DoesNotContain => text.contains(&want),
                Test::Is => text == want || bare == want,
                Test::BeginsWith => text.starts_with(&want),
                Test::EndsWith => text.ends_with(&want) || bare.ends_with(&want),
            }
        };
        let any = fields.iter().any(|f| hit(f));
        if self.test == Test::DoesNotContain { !any } else { any }
    }
}

impl Rule {
    pub fn matches(&self, s: &Summary) -> bool {
        if !self.enabled || self.criteria.is_empty() {
            return false;
        }
        if self.all {
            self.criteria.iter().all(|c| c.matches(s))
        } else {
            self.criteria.iter().any(|c| c.matches(s))
        }
    }
}

/// The actions to run on a message, in rule order, up to and including the first `Stop`.
pub fn evaluate(rules: &[Rule], s: &Summary) -> Vec<Action> {
    let mut out = vec![];
    for rule in rules.iter().filter(|r| r.matches(s)) {
        for a in &rule.actions {
            out.push(a.clone());
            if *a == Action::Stop {
                return out;
            }
        }
    }
    out
}

fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('|', "\\p").replace(';', "\\s").replace('\n', " ")
}

fn unesc(s: &str) -> String {
    let mut out = String::new();
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next() {
                Some('p') => out.push('|'),
                Some('s') => out.push(';'),
                Some(o) => out.push(o),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn split_esc(s: &str, sep: char) -> Vec<String> {
    // splits on an unescaped separator; the separators never occur raw inside escaped text
    if s.is_empty() { vec![] } else { s.split(sep).map(str::to_string).collect() }
}

impl Rule {
    /// `name|enabled|all|field:test:value;...|action[:arg];...`
    pub fn to_line(&self) -> String {
        let field = |f: Field| match f {
            Field::From => "from",
            Field::To => "to",
            Field::Cc => "cc",
            Field::Subject => "subject",
            Field::AnyRecipient => "recipient",
        };
        let test = |t: Test| match t {
            Test::Contains => "contains",
            Test::DoesNotContain => "not",
            Test::Is => "is",
            Test::BeginsWith => "begins",
            Test::EndsWith => "ends",
        };
        let criteria: Vec<String> =
            self.criteria.iter().map(|c| format!("{}:{}:{}", field(c.field), test(c.test), esc(&c.value))).collect();
        let actions: Vec<String> = self
            .actions
            .iter()
            .map(|a| match a {
                Action::MoveTo(m) => format!("move:{}", esc(m)),
                Action::MarkRead => "read".into(),
                Action::Flag => "flag".into(),
                Action::Delete => "delete".into(),
                Action::Stop => "stop".into(),
            })
            .collect();
        format!(
            "{}|{}|{}|{}|{}",
            esc(&self.name),
            self.enabled as u8,
            self.all as u8,
            criteria.join(";"),
            actions.join(";")
        )
    }

    pub fn from_line(line: &str) -> Option<Rule> {
        let f: Vec<&str> = line.split('|').collect();
        if f.len() != 5 {
            return None;
        }
        let mut criteria = vec![];
        for c in split_esc(f[3], ';') {
            let p: Vec<&str> = c.splitn(3, ':').collect();
            if p.len() != 3 {
                return None;
            }
            let field = match p[0] {
                "from" => Field::From,
                "to" => Field::To,
                "cc" => Field::Cc,
                "subject" => Field::Subject,
                "recipient" => Field::AnyRecipient,
                _ => return None,
            };
            let test = match p[1] {
                "contains" => Test::Contains,
                "not" => Test::DoesNotContain,
                "is" => Test::Is,
                "begins" => Test::BeginsWith,
                "ends" => Test::EndsWith,
                _ => return None,
            };
            criteria.push(Criterion { field, test, value: unesc(p[2]) });
        }
        let mut actions = vec![];
        for a in split_esc(f[4], ';') {
            actions.push(match a.split_once(':') {
                Some(("move", m)) => Action::MoveTo(unesc(m)),
                None if a == "read" => Action::MarkRead,
                None if a == "flag" => Action::Flag,
                None if a == "delete" => Action::Delete,
                None if a == "stop" => Action::Stop,
                _ => return None,
            });
        }
        Some(Rule { name: unesc(f[0]), enabled: f[1] == "1", all: f[2] == "1", criteria, actions })
    }
}

/// Stores rules, one per line.
pub fn to_text(rules: &[Rule]) -> String {
    rules.iter().map(|r| r.to_line() + "\n").collect()
}

/// Reads stored rules; unreadable lines are skipped.
pub fn from_text(text: &str) -> Vec<Rule> {
    text.lines().filter_map(Rule::from_line).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(from: &str, subject: &str) -> Summary {
        Summary { from: from.into(), subject: subject.into(), to: "me@x.is".into(), ..Summary::default() }
    }

    fn rule(c: Vec<Criterion>, a: Vec<Action>) -> Rule {
        Rule { name: "r".into(), enabled: true, all: true, criteria: c, actions: a }
    }

    fn crit(field: Field, test: Test, value: &str) -> Criterion {
        Criterion { field, test, value: value.into() }
    }

    #[test]
    fn criteria_tests() {
        let m = msg("Ann <ann@shop.is>", "Your order #12");
        assert!(crit(Field::From, Test::Contains, "SHOP").matches(&m));
        assert!(crit(Field::From, Test::Is, "ann@shop.is").matches(&m));
        assert!(crit(Field::From, Test::EndsWith, "@shop.is").matches(&m));
        assert!(crit(Field::Subject, Test::BeginsWith, "your").matches(&m));
        assert!(crit(Field::Subject, Test::DoesNotContain, "invoice").matches(&m));
        assert!(!crit(Field::Subject, Test::DoesNotContain, "order").matches(&m));
        assert!(crit(Field::AnyRecipient, Test::Is, "me@x.is").matches(&m));
    }

    #[test]
    fn all_versus_any_and_disabled() {
        let m = msg("a@b.is", "hello");
        let cs = vec![crit(Field::From, Test::Contains, "a@b"), crit(Field::Subject, Test::Contains, "nope")];
        let mut r = rule(cs, vec![Action::Flag]);
        assert!(!r.matches(&m));
        r.all = false;
        assert!(r.matches(&m));
        r.enabled = false;
        assert!(!r.matches(&m));
        assert!(!rule(vec![], vec![Action::Flag]).matches(&m));
    }

    #[test]
    fn evaluation_order_and_stop() {
        let m = msg("news@shop.is", "Sale");
        let c = vec![crit(Field::From, Test::Contains, "shop")];
        let rules = vec![
            rule(c.clone(), vec![Action::MoveTo("Promos".into()), Action::Stop, Action::Flag]),
            rule(c.clone(), vec![Action::Delete]),
        ];
        assert_eq!(evaluate(&rules, &m), vec![Action::MoveTo("Promos".into()), Action::Stop]);
        let rules = vec![rule(c.clone(), vec![Action::MarkRead]), rule(c, vec![Action::Flag])];
        assert_eq!(evaluate(&rules, &m), vec![Action::MarkRead, Action::Flag]);
        assert!(evaluate(&rules, &msg("x@y.is", "z")).is_empty());
    }

    #[test]
    fn text_round_trip_with_awkward_characters() {
        let mut r = rule(
            vec![crit(Field::Subject, Test::Contains, "a|b;c:d\\e"), crit(Field::Cc, Test::Is, "x@y.is")],
            vec![Action::MoveTo("Work;Q4".into()), Action::MarkRead, Action::Stop],
        );
        r.name = "Name | with ; chars".into();
        r.all = false;
        let text = to_text(&[r.clone(), r.clone()]);
        assert_eq!(from_text(&text), vec![r.clone(), r]);
        assert!(from_text("garbage\n\n").is_empty());
    }
}
