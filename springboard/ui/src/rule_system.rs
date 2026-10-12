//! Rule systems for game logic: named rules over a set of numeric facts. A rule has a predicate that tests the
//! facts and an action that changes them. `RuleSystem::evaluate` runs sweeps over the rules, highest salience
//! first (equal salience keeps insertion order), and fires each rule whose predicate holds. It stops after a
//! sweep that fires nothing. A rule fires at most once per evaluation, so evaluation always ends.

use std::collections::BTreeMap;

/// Named numeric facts that rules test and change. Missing facts read as absent, or as 0.0 through `value`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Facts {
    values: BTreeMap<String, f64>,
}

impl Facts {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&mut self, key: &str, value: f64) {
        self.values.insert(key.to_string(), value);
    }

    /// The value of `key`, or `None` when it was never set.
    pub fn get(&self, key: &str) -> Option<f64> {
        self.values.get(key).copied()
    }

    /// The value of `key`, or 0.0 when it was never set.
    pub fn value(&self, key: &str) -> f64 {
        self.get(key).unwrap_or(0.0)
    }

    /// Adds `delta` to `key`. A missing fact counts as 0.0 first.
    pub fn add(&mut self, key: &str, delta: f64) {
        let next = self.value(key) + delta;
        self.set(key, next);
    }
}

/// A rule: fires its action when its predicate holds.
pub struct Rule {
    name: String,
    salience: i32,
    when: Box<dyn Fn(&Facts) -> bool>,
    then: Box<dyn Fn(&mut Facts)>,
}

impl Rule {
    /// A rule named `name`. Higher `salience` is tried first.
    pub fn new(
        name: &str,
        salience: i32,
        when: impl Fn(&Facts) -> bool + 'static,
        then: impl Fn(&mut Facts) + 'static,
    ) -> Self {
        Self {
            name: name.to_string(),
            salience,
            when: Box::new(when),
            then: Box::new(then),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn salience(&self) -> i32 {
        self.salience
    }
}

/// An ordered set of rules.
#[derive(Default)]
pub struct RuleSystem {
    rules: Vec<Rule>,
}

impl RuleSystem {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, rule: Rule) {
        self.rules.push(rule);
    }

    pub fn len(&self) -> usize {
        self.rules.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// Runs the rules against `facts` and returns the names of the rules that fired, in firing order.
    pub fn evaluate(&self, facts: &mut Facts) -> Vec<String> {
        let mut order: Vec<usize> = (0..self.rules.len()).collect();
        // sort_by_key is stable, so equal salience keeps insertion order.
        order.sort_by_key(|&i| std::cmp::Reverse(self.rules[i].salience));
        let mut fired = vec![false; self.rules.len()];
        let mut log = Vec::new();
        loop {
            let mut any = false;
            for &i in &order {
                if fired[i] {
                    continue;
                }
                let rule = &self.rules[i];
                if (rule.when)(&*facts) {
                    (rule.then)(&mut *facts);
                    fired[i] = true;
                    log.push(rule.name.clone());
                    any = true;
                }
            }
            if !any {
                break;
            }
        }
        log
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game_rules() -> RuleSystem {
        let mut rs = RuleSystem::new();
        rs.add(Rule::new(
            "alarm",
            10,
            |f: &Facts| f.value("health") < 5.0,
            |f: &mut Facts| f.set("alarm", 1.0),
        ));
        rs.add(Rule::new(
            "heal",
            5,
            |f: &Facts| f.value("alarm") == 1.0,
            |f: &mut Facts| f.add("health", 10.0),
        ));
        rs.add(Rule::new(
            "ok",
            1,
            |f: &Facts| f.value("health") >= 10.0,
            |f: &mut Facts| f.set("state", 2.0),
        ));
        rs
    }

    #[test]
    fn rules_chain_through_the_facts_they_change() {
        // health 3: alarm fires (3 < 5) and sets alarm = 1; heal then adds 10 to health (13); ok sees 13 >= 10.
        let rs = game_rules();
        let mut facts = Facts::new();
        facts.set("health", 3.0);
        let fired = rs.evaluate(&mut facts);
        assert_eq!(fired, vec!["alarm", "heal", "ok"]);
        assert_eq!(facts.value("health"), 13.0);
        assert_eq!(facts.value("alarm"), 1.0);
        assert_eq!(facts.value("state"), 2.0);
    }

    #[test]
    fn no_predicate_holding_means_nothing_fires() {
        // health 7: alarm needs < 5, heal needs alarm == 1, ok needs >= 10. None hold.
        let rs = game_rules();
        let mut facts = Facts::new();
        facts.set("health", 7.0);
        assert!(rs.evaluate(&mut facts).is_empty());
        assert_eq!(facts.value("health"), 7.0);
        assert_eq!(facts.get("alarm"), None);
    }

    #[test]
    fn a_rule_that_always_holds_fires_only_once() {
        let mut rs = RuleSystem::new();
        rs.add(Rule::new("tick", 0, |_f: &Facts| true, |f: &mut Facts| f.add("ticks", 1.0)));
        let mut facts = Facts::new();
        assert_eq!(rs.evaluate(&mut facts), vec!["tick"]);
        assert_eq!(facts.value("ticks"), 1.0);
    }

    #[test]
    fn higher_salience_fires_first_and_ties_keep_insertion_order() {
        let mut rs = RuleSystem::new();
        rs.add(Rule::new("low", 1, |_f: &Facts| true, |_f: &mut Facts| {}));
        rs.add(Rule::new("high", 9, |_f: &Facts| true, |_f: &mut Facts| {}));
        rs.add(Rule::new("tie_a", 4, |_f: &Facts| true, |_f: &mut Facts| {}));
        rs.add(Rule::new("tie_b", 4, |_f: &Facts| true, |_f: &mut Facts| {}));
        let fired = rs.evaluate(&mut Facts::new());
        assert_eq!(fired, vec!["high", "tie_a", "tie_b", "low"]);
        assert_eq!(rs.len(), 4);
    }

    #[test]
    fn a_later_sweep_picks_up_what_an_earlier_sweep_enabled() {
        // Sweep 1: "use" (salience 5) sees flag 0 and is skipped. "set" (salience 0) fires and sets flag 1.
        // Sweep 2: "use" now sees flag 1 and fires.
        let mut rs = RuleSystem::new();
        rs.add(Rule::new(
            "use",
            5,
            |f: &Facts| f.value("flag") == 1.0,
            |f: &mut Facts| f.set("done", 1.0),
        ));
        rs.add(Rule::new(
            "set",
            0,
            |f: &Facts| f.value("flag") == 0.0,
            |f: &mut Facts| f.set("flag", 1.0),
        ));
        let mut facts = Facts::new();
        assert_eq!(rs.evaluate(&mut facts), vec!["set", "use"]);
        assert_eq!(facts.value("done"), 1.0);
    }

    #[test]
    fn facts_report_missing_keys_and_accumulate_adds() {
        let mut facts = Facts::new();
        assert_eq!(facts.get("x"), None);
        assert_eq!(facts.value("x"), 0.0);
        facts.add("x", 2.5);
        facts.add("x", 1.5);
        assert_eq!(facts.get("x"), Some(4.0));
    }
}
