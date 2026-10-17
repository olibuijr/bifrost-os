//! Mail rules (Mailbox > Rules): a rule names what a message must contain (the sender, the recipients, the subject) and what
//! happens to it (move it to a mailbox, mark it read, flag it in a colour). Rules run on each message that arrives in an
//! inbox and on request over the shown mailbox (Mailbox > Rules > Apply Rules).

use crate::flags::FlagColor;
use crate::message::Summary;
use bifrost_core::json::Json;

/// What a rule does to a message that matches it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Moves the message to the mailbox with this name.
    MoveTo(String),
    MarkRead,
    /// Flags the message in this colour.
    Flag(FlagColor),
}

impl Action {
    /// True when the action takes the message out of the inbox or reads it, so its arrival needs no notification.
    pub fn is_quiet(&self) -> bool {
        matches!(self, Action::MoveTo(_) | Action::MarkRead)
    }
}

/// A rule: every criterion that is not empty must match (without case), and the rule needs at least one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Rule {
    pub name: String,
    /// Text the From header must contain.
    pub from: String,
    /// Text the To or Cc header must contain.
    pub to: String,
    /// Text the Subject must contain.
    pub subject: String,
    pub actions: Vec<Action>,
}

fn contains(haystack: &str, needle: &str) -> bool {
    needle.trim().is_empty() || haystack.to_lowercase().contains(&needle.trim().to_lowercase())
}

impl Rule {
    /// True when the message meets every criterion the rule has. A rule with no criteria matches nothing.
    pub fn matches(&self, summary: &Summary) -> bool {
        let criteria = [&self.from, &self.to, &self.subject];
        if criteria.iter().all(|c| c.trim().is_empty()) {
            return false;
        }
        contains(&summary.from, &self.from)
            && (self.to.trim().is_empty() || contains(&summary.to, &self.to) || contains(&summary.cc, &self.to))
            && contains(&summary.subject, &self.subject)
    }

    pub fn to_json(&self) -> Json {
        let actions = self
            .actions
            .iter()
            .map(|a| match a {
                Action::MoveTo(name) => Json::Obj(vec![("move".into(), Json::Str(name.clone()))]),
                Action::MarkRead => Json::Obj(vec![("read".into(), Json::Bool(true))]),
                Action::Flag(color) => Json::Obj(vec![("flag".into(), Json::Str(color.label().to_string()))]),
            })
            .collect();
        Json::Obj(vec![
            ("name".into(), Json::Str(self.name.clone())),
            ("from".into(), Json::Str(self.from.clone())),
            ("to".into(), Json::Str(self.to.clone())),
            ("subject".into(), Json::Str(self.subject.clone())),
            ("actions".into(), Json::Arr(actions)),
        ])
    }

    /// Reads a rule back. A rule with no name or no action is dropped (None).
    pub fn from_json(json: &Json) -> Option<Rule> {
        let text = |key: &str| json.get(key).and_then(Json::as_str).unwrap_or("").to_string();
        let mut actions = Vec::new();
        for item in json.get("actions").and_then(Json::as_array).unwrap_or(&[]) {
            if let Some(name) = item.get("move").and_then(Json::as_str) {
                actions.push(Action::MoveTo(name.to_string()));
            } else if item.get("read").is_some() {
                actions.push(Action::MarkRead);
            } else if let Some(color) = item.get("flag").and_then(Json::as_str).and_then(color_named) {
                actions.push(Action::Flag(color));
            }
        }
        let rule = Rule { name: text("name"), from: text("from"), to: text("to"), subject: text("subject"), actions };
        (!rule.name.trim().is_empty() && !rule.actions.is_empty()).then_some(rule)
    }
}

/// The flag colour with this name (without case).
pub fn color_named(name: &str) -> Option<FlagColor> {
    FlagColor::ALL.into_iter().find(|c| c.label().eq_ignore_ascii_case(name.trim()))
}

/// The actions the form of a new rule asks for: the mailbox to move to, `yes` to mark read, a colour name to flag. All
/// three empty is an error.
pub fn actions_from(move_to: &str, mark_read: &str, flag: &str) -> Result<Vec<Action>, String> {
    let mut actions = Vec::new();
    if !move_to.trim().is_empty() {
        actions.push(Action::MoveTo(move_to.trim().to_string()));
    }
    match mark_read.trim().to_lowercase().as_str() {
        "" | "no" => {}
        "yes" | "y" | "true" => actions.push(Action::MarkRead),
        other => return Err(format!("Mark as read takes yes or no, not {other}")),
    }
    if !flag.trim().is_empty() {
        match color_named(flag) {
            Some(color) => actions.push(Action::Flag(color)),
            None => return Err(format!("{} is not a flag colour", flag.trim())),
        }
    }
    if actions.is_empty() {
        return Err("Choose what the rule does: a mailbox to move to, mark as read, or a flag colour".into());
    }
    Ok(actions)
}

/// The line the Rules pane shows for a rule: its name, what it looks for, and what it does.
pub fn describe(rule: &Rule) -> String {
    let mut looks = Vec::new();
    for (label, text) in [("from", &rule.from), ("to", &rule.to), ("subject", &rule.subject)] {
        if !text.trim().is_empty() {
            looks.push(format!("{label} contains {}", text.trim()));
        }
    }
    let does: Vec<String> = rule
        .actions
        .iter()
        .map(|a| match a {
            Action::MoveTo(name) => format!("move to {name}"),
            Action::MarkRead => "mark read".to_string(),
            Action::Flag(color) => format!("flag {}", color.label()),
        })
        .collect();
    format!("{}: {} then {}", rule.name, looks.join(", "), does.join(", "))
}

/// What happens to one message: the actions of every rule that matches it, in rule order. Once an action moves the
/// message the rules after it are not run (the message is no longer here).
pub fn outcome(rules: &[Rule], summary: &Summary) -> Vec<Action> {
    let mut actions = Vec::new();
    for rule in rules.iter().filter(|r| r.matches(summary)) {
        for action in &rule.actions {
            actions.push(action.clone());
            if matches!(action, Action::MoveTo(_)) {
                return actions;
            }
        }
    }
    actions
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary() -> Summary {
        Summary::from_headers("From: Ana Smith <ana@shop.example>\r\nTo: me@example.com\r\nCc: team@example.com\r\nSubject: Your order has shipped\r\n\r\n")
    }

    fn rule(from: &str, to: &str, subject: &str) -> Rule {
        Rule { name: "r".into(), from: from.into(), to: to.into(), subject: subject.into(), actions: vec![Action::MarkRead] }
    }

    #[test]
    fn every_given_criterion_must_match_without_case() {
        assert!(rule("SHOP.example", "", "").matches(&summary()));
        assert!(rule("shop", "", "order").matches(&summary()));
        assert!(!rule("shop", "", "invoice").matches(&summary()));
        assert!(rule("", "team@", "").matches(&summary()), "the Cc header counts as a recipient");
        assert!(!rule("", "other@", "").matches(&summary()));
    }

    #[test]
    fn a_rule_without_criteria_matches_nothing() {
        assert!(!rule("", "  ", "").matches(&summary()));
    }

    #[test]
    fn the_form_builds_actions_and_refuses_an_empty_or_unknown_one() {
        assert_eq!(
            actions_from("Receipts", "yes", "green").unwrap(),
            vec![Action::MoveTo("Receipts".into()), Action::MarkRead, Action::Flag(FlagColor::Green)]
        );
        assert_eq!(actions_from("", "no", "").unwrap_err(), "Choose what the rule does: a mailbox to move to, mark as read, or a flag colour");
        assert!(actions_from("", "maybe", "").unwrap_err().contains("yes or no"));
        assert_eq!(actions_from("", "", "pink").unwrap_err(), "pink is not a flag colour");
    }

    #[test]
    fn the_outcome_stops_at_a_move() {
        let rules = vec![
            Rule { actions: vec![Action::Flag(FlagColor::Red), Action::MoveTo("Shops".into()), Action::MarkRead], ..rule("shop", "", "") },
            Rule { actions: vec![Action::MarkRead], ..rule("", "", "order") },
        ];
        assert_eq!(outcome(&rules, &summary()), vec![Action::Flag(FlagColor::Red), Action::MoveTo("Shops".into())]);
        assert!(outcome(&rules, &Summary::from_headers("Subject: hello\r\n\r\n")).is_empty());
    }

    #[test]
    fn quiet_actions_are_the_ones_that_hide_or_read_a_message() {
        assert!(Action::MoveTo("x".into()).is_quiet() && Action::MarkRead.is_quiet());
        assert!(!Action::Flag(FlagColor::Blue).is_quiet());
    }

    #[test]
    fn rules_survive_a_round_trip_through_json() {
        let original = Rule { name: "Shops".into(), actions: vec![Action::MoveTo("Shops".into()), Action::MarkRead, Action::Flag(FlagColor::Purple)], ..rule("shop", "me@", "order") };
        let text = original.to_json().encode();
        let back = Rule::from_json(&bifrost_core::json::parse(&text).unwrap()).unwrap();
        assert_eq!(back, original);
        assert!(Rule::from_json(&bifrost_core::json::parse(r#"{"name":"x","actions":[]}"#).unwrap()).is_none());
        assert!(Rule::from_json(&bifrost_core::json::parse(r#"{"actions":[{"read":true}]}"#).unwrap()).is_none());
    }
}
