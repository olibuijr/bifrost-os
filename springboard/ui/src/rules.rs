//! Rule system: rules with a predicate over the system's facts and an action that asserts or retracts a fact, evaluated
//! in order of salience until no rule on the agenda matches.
//!
//! A fact is a name with a membership grade in 0..=1 (1 is fully true, values between are fuzzy). `evaluate` takes the
//! agenda, the rules not yet fired, picks the matching rule with the highest salience (ties go to the rule added first),
//! fires it, and starts again, so a fact asserted by one rule can enable another. Each rule fires at most once until
//! `reset`. A rule whose predicate is true is not fired while a higher-salience rule is still matching. Changing a fact
//! from outside the system (`assert_fact`, `retract_fact`) evaluates again at once.
//!
//! Decision: the fact grade is clamped to 0..=1 on every change, so a rule that asserts a grade of 0.8 twice gives 1.0,
//! and a grade that falls to 0 or below removes the fact. Grades of facts that are not present read as 0.

use std::collections::BTreeMap;

/// What a fired rule does to the fact set.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// Raises the grade of the fact by this amount, adding the fact if it is absent.
    Assert { fact: String, grade: f64 },
    /// Lowers the grade of the fact by this amount, removing the fact when the grade reaches 0. A grade of
    /// `f64::INFINITY` removes the fact at once.
    Retract { fact: String, grade: f64 },
}

/// A rule: fires its action once, the first time its predicate holds during an evaluation.
pub struct Rule {
    predicate: Box<dyn Fn(&RuleSystem) -> bool>,
    action: Action,
    /// Higher values are considered first. Equal values keep the order the rules were added in.
    pub salience: i32,
}

impl Rule {
    pub fn new(predicate: impl Fn(&RuleSystem) -> bool + 'static, action: Action) -> Self {
        Rule { predicate: Box::new(predicate), action, salience: 0 }
    }

    pub fn with_salience(mut self, salience: i32) -> Self {
        self.salience = salience;
        self
    }

    pub fn action(&self) -> &Action {
        &self.action
    }
}

/// A set of rules, the facts they read and write, and the agenda that says which rules may still fire.
#[derive(Default)]
pub struct RuleSystem {
    rules: Vec<Rule>,
    agenda: Vec<usize>,
    executed: Vec<usize>,
    facts: BTreeMap<String, f64>,
}

fn clamp_grade(g: f64) -> f64 {
    if g.is_nan() {
        0.0
    } else {
        g.clamp(0.0, 1.0)
    }
}

impl RuleSystem {
    pub fn new() -> Self {
        RuleSystem::default()
    }

    /// Adds a rule to the end of the list and to the agenda.
    pub fn add_rule(&mut self, rule: Rule) {
        self.agenda.push(self.rules.len());
        self.rules.push(rule);
    }

    /// Removes every rule, and the agenda and executed list with them. Facts are kept.
    pub fn remove_all_rules(&mut self) {
        self.rules.clear();
        self.agenda.clear();
        self.executed.clear();
    }

    pub fn rule_count(&self) -> usize {
        self.rules.len()
    }

    /// Rules that have not fired yet, in the order `evaluate` will consider them.
    pub fn agenda(&self) -> Vec<&Rule> {
        self.agenda.iter().map(|&i| &self.rules[i]).collect()
    }

    /// Rules that have fired since the last `reset`, in the order they fired.
    pub fn executed(&self) -> Vec<&Rule> {
        self.executed.iter().map(|&i| &self.rules[i]).collect()
    }

    /// The membership grade of a fact, 0 when the fact is absent.
    pub fn grade(&self, fact: &str) -> f64 {
        self.facts.get(fact).copied().unwrap_or(0.0)
    }

    pub fn has_fact(&self, fact: &str) -> bool {
        self.facts.contains_key(fact)
    }

    /// Names of the facts currently held, in name order.
    pub fn facts(&self) -> Vec<&str> {
        self.facts.keys().map(|k| k.as_str()).collect()
    }

    /// The lowest grade among the named facts (fuzzy AND). Returns 0 for an empty list.
    pub fn minimum_grade(&self, facts: &[&str]) -> f64 {
        if facts.is_empty() {
            return 0.0;
        }
        facts.iter().map(|f| self.grade(f)).fold(f64::INFINITY, f64::min)
    }

    /// The highest grade among the named facts (fuzzy OR). Returns 0 for an empty list.
    pub fn maximum_grade(&self, facts: &[&str]) -> f64 {
        facts.iter().map(|f| self.grade(f)).fold(0.0, f64::max)
    }

    /// Adds a fact with grade 1, then evaluates.
    pub fn assert_fact(&mut self, fact: &str) {
        self.assert_fact_grade(fact, 1.0);
    }

    /// Raises the grade of a fact by `grade`, adding the fact if needed, then evaluates.
    pub fn assert_fact_grade(&mut self, fact: &str, grade: f64) {
        self.apply(&Action::Assert { fact: fact.to_string(), grade });
        self.evaluate();
    }

    /// Removes a fact outright, then evaluates.
    pub fn retract_fact(&mut self, fact: &str) {
        self.facts.remove(fact);
        self.evaluate();
    }

    /// Lowers the grade of a fact by `grade`, removing it at 0 or below, then evaluates.
    pub fn retract_fact_grade(&mut self, fact: &str, grade: f64) {
        self.apply(&Action::Retract { fact: fact.to_string(), grade });
        self.evaluate();
    }

    /// Restores the agenda to every rule, clears the executed list and removes every fact.
    pub fn reset(&mut self) {
        self.agenda = (0..self.rules.len()).collect();
        self.executed.clear();
        self.facts.clear();
    }

    /// Fires matching rules until none on the agenda matches. Each pass takes the highest-salience matching rule.
    pub fn evaluate(&mut self) {
        loop {
            {
                let rules = &self.rules;
                self.agenda.sort_by(|&a, &b| rules[b].salience.cmp(&rules[a].salience).then(a.cmp(&b)));
            }
            let view: &RuleSystem = &*self;
            let fired = view.agenda.iter().position(|&i| (view.rules[i].predicate)(view));
            let pos = match fired {
                Some(pos) => pos,
                None => break,
            };
            let idx = self.agenda.remove(pos);
            let action = self.rules[idx].action.clone();
            self.apply(&action);
            self.executed.push(idx);
        }
    }

    /// Changes the fact set for one action, without evaluating.
    fn apply(&mut self, action: &Action) {
        match action {
            Action::Assert { fact, grade } => {
                let g = clamp_grade(self.grade(fact) + grade);
                self.facts.insert(fact.clone(), g);
            }
            Action::Retract { fact, grade } => {
                let g = self.grade(fact) - grade;
                if g.is_nan() || g <= 0.0 {
                    self.facts.remove(fact);
                } else {
                    self.facts.insert(fact.clone(), clamp_grade(g));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_rule(when: &'static str, fact: &str, salience: i32) -> Rule {
        let fact = fact.to_string();
        Rule::new(move |s: &RuleSystem| s.has_fact(when), Action::Assert { fact, grade: 1.0 }).with_salience(salience)
    }

    #[test]
    fn a_rule_fires_once_when_its_predicate_holds() {
        let mut s = RuleSystem::new();
        s.add_rule(assert_rule("rain", "wet", 0));
        assert_eq!(s.agenda().len(), 1);
        s.evaluate();
        assert!(!s.has_fact("wet"), "no rain yet");
        s.assert_fact("rain");
        assert!(s.has_fact("wet"), "the rule fired on the assert");
        assert_eq!(s.executed().len(), 1);
        assert!(s.agenda().is_empty());
    }

    #[test]
    fn higher_salience_fires_first_and_ties_keep_insertion_order() {
        let mut s = RuleSystem::new();
        s.add_rule(assert_rule("go", "low", 1));
        s.add_rule(assert_rule("go", "high", 5));
        s.add_rule(assert_rule("go", "also-low", 1));
        s.assert_fact("go");
        let names: Vec<String> = s
            .executed()
            .iter()
            .map(|r| match r.action() {
                Action::Assert { fact, .. } => fact.clone(),
                Action::Retract { fact, .. } => fact.clone(),
            })
            .collect();
        assert_eq!(names, vec!["high", "low", "also-low"]);
    }

    #[test]
    fn a_fact_asserted_by_one_rule_enables_the_next() {
        let mut s = RuleSystem::new();
        s.add_rule(assert_rule("b", "c", 0));
        s.add_rule(assert_rule("a", "b", 0));
        s.assert_fact("a");
        assert!(s.has_fact("b"));
        assert!(s.has_fact("c"), "chained through the agenda");
        assert_eq!(s.facts(), vec!["a", "b", "c"]);
    }

    #[test]
    fn grades_accumulate_and_clamp_to_one() {
        let mut s = RuleSystem::new();
        s.assert_fact_grade("warm", 0.4);
        assert!((s.grade("warm") - 0.4).abs() < 1e-12);
        s.assert_fact_grade("warm", 0.8);
        assert_eq!(s.grade("warm"), 1.0, "clamped");
        s.retract_fact_grade("warm", 0.25);
        assert!((s.grade("warm") - 0.75).abs() < 1e-12);
        s.retract_fact_grade("warm", 2.0);
        assert!(!s.has_fact("warm"), "a grade at or below zero removes the fact");
        assert_eq!(s.grade("warm"), 0.0);
    }

    #[test]
    fn fuzzy_minimum_and_maximum_over_named_facts() {
        let mut s = RuleSystem::new();
        s.assert_fact_grade("a", 0.2);
        s.assert_fact_grade("b", 0.9);
        assert_eq!(s.minimum_grade(&["a", "b"]), 0.2);
        assert_eq!(s.maximum_grade(&["a", "b"]), 0.9);
        assert_eq!(s.minimum_grade(&["a", "missing"]), 0.0, "absent facts read as zero");
        assert_eq!(s.minimum_grade(&[]), 0.0);
    }

    #[test]
    fn reset_restores_the_agenda_and_clears_facts() {
        let mut s = RuleSystem::new();
        s.add_rule(assert_rule("x", "y", 0));
        s.assert_fact("x");
        assert!(s.agenda().is_empty());
        s.reset();
        assert_eq!(s.agenda().len(), 1, "the rule can fire again");
        assert!(s.executed().is_empty());
        assert!(s.facts().is_empty());
    }

    #[test]
    fn retract_action_removes_a_fact_and_can_fire_later_rules() {
        let mut s = RuleSystem::new();
        s.assert_fact("lit");
        s.add_rule(Rule::new(
            |s: &RuleSystem| s.has_fact("switch"),
            Action::Retract { fact: "lit".to_string(), grade: f64::INFINITY },
        ));
        s.assert_fact("switch");
        assert!(!s.has_fact("lit"));
        assert!(s.has_fact("switch"));
    }
}
