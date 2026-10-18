//! An attribute graph: values that depend on other values and are recomputed only when something they read changed.
//!
//! An *input* holds a value that code sets. A *rule* computes a value from other attributes by reading them through a
//! [`Reader`]; the reads are recorded, so the graph knows what each rule depends on without being told, and the set of
//! dependencies follows the rule (a rule that reads `b` only when a flag is set stops depending on `b` when the flag
//! clears). This is what keeps a view tree cheap to update: setting an input marks only what depends on it, nothing is
//! recomputed until a value is asked for, and a rule whose inputs changed but whose result did not (the parity of a
//! number, say) stops the update there, so what reads that result is not recomputed either.
//!
//! Rules are created after the attributes they read, so the graph cannot contain a cycle. Single-threaded.

use std::any::Any;
use std::marker::PhantomData;

/// A handle to a value in a [`Graph`].
pub struct Attr<T> {
    id: usize,
    _value: PhantomData<fn() -> T>,
}

impl<T> Clone for Attr<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for Attr<T> {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    /// The value is current.
    Clean,
    /// Something this value depends on, further back, was set: it must be checked before use.
    Check,
    /// A direct dependency changed: the rule has to run again.
    Dirty,
}

type Rule = Box<dyn Fn(&mut Reader<'_>) -> Box<dyn Any>>;

struct Node {
    value: Option<Box<dyn Any>>,
    equal: fn(&dyn Any, &dyn Any) -> bool,
    rule: Option<Rule>,
    /// (dependency, its version when this node last read it)
    deps: Vec<(usize, u64)>,
    dependents: Vec<usize>,
    version: u64,
    state: State,
}

fn equal_values<T: PartialEq + 'static>(a: &dyn Any, b: &dyn Any) -> bool {
    a.downcast_ref::<T>() == b.downcast_ref::<T>()
}

#[derive(Default)]
pub struct Graph {
    nodes: Vec<Node>,
    evals: u64,
}

/// What a rule reads its inputs through. Every read is recorded as a dependency of the rule.
pub struct Reader<'g> {
    graph: &'g mut Graph,
    deps: Vec<(usize, u64)>,
}

impl Reader<'_> {
    pub fn read<T: Clone + 'static>(&mut self, attr: Attr<T>) -> T {
        let value = self.graph.get(attr);
        let version = self.graph.nodes[attr.id].version;
        if !self.deps.iter().any(|(d, _)| *d == attr.id) {
            self.deps.push((attr.id, version));
        }
        value
    }
}

impl Graph {
    pub fn new() -> Graph {
        Graph::default()
    }

    fn push<T: PartialEq + 'static>(&mut self, value: Option<Box<dyn Any>>, rule: Option<Rule>) -> Attr<T> {
        let state = if rule.is_some() { State::Dirty } else { State::Clean };
        self.nodes.push(Node { value, equal: equal_values::<T>, rule, deps: Vec::new(), dependents: Vec::new(), version: 0, state });
        Attr { id: self.nodes.len() - 1, _value: PhantomData }
    }

    /// A value that code sets with [`Graph::set`].
    pub fn input<T: PartialEq + 'static>(&mut self, value: T) -> Attr<T> {
        self.push(Some(Box::new(value)), None)
    }

    /// A value computed by `rule` from the attributes it reads. It first runs when the value is first asked for.
    pub fn rule<T: PartialEq + 'static>(&mut self, rule: impl Fn(&mut Reader<'_>) -> T + 'static) -> Attr<T> {
        let boxed: Rule = Box::new(move |r: &mut Reader<'_>| -> Box<dyn Any> { Box::new(rule(r)) });
        self.push(None, Some(boxed))
    }

    /// Sets an input. Returns false, and marks nothing, when the value is equal to the current one.
    ///
    /// # Panics
    /// When `attr` is a rule: only inputs are set.
    pub fn set<T: PartialEq + 'static>(&mut self, attr: Attr<T>, value: T) -> bool {
        let node = &mut self.nodes[attr.id];
        assert!(node.rule.is_none(), "only an input can be set");
        if let Some(old) = &node.value {
            if (node.equal)(old.as_ref(), &value as &dyn Any) {
                return false;
            }
        }
        node.value = Some(Box::new(value));
        node.version += 1;
        for d in node.dependents.clone() {
            self.nodes[d].state = State::Dirty;
            self.mark_check(d);
        }
        true
    }

    fn mark_check(&mut self, id: usize) {
        for d in self.nodes[id].dependents.clone() {
            if self.nodes[d].state == State::Clean {
                self.nodes[d].state = State::Check;
                self.mark_check(d);
            }
        }
    }

    /// The current value, running the rules that are out of date first.
    pub fn get<T: Clone + 'static>(&mut self, attr: Attr<T>) -> T {
        self.refresh(attr.id);
        self.nodes[attr.id]
            .value
            .as_ref()
            .and_then(|v| v.downcast_ref::<T>())
            .expect("an attribute is read with the type it was made with")
            .clone()
    }

    fn refresh(&mut self, id: usize) {
        match self.nodes[id].state {
            State::Clean => return,
            State::Check => {
                let deps = self.nodes[id].deps.clone();
                let mut changed = false;
                for (dep, seen) in deps {
                    self.refresh(dep);
                    if self.nodes[dep].version != seen {
                        changed = true;
                        break;
                    }
                }
                if !changed {
                    self.nodes[id].state = State::Clean;
                    return;
                }
            }
            State::Dirty => {}
        }
        self.recompute(id);
    }

    fn recompute(&mut self, id: usize) {
        let rule = self.nodes[id].rule.take().expect("only rules are recomputed");
        let mut reader = Reader { graph: self, deps: Vec::new() };
        let new = rule(&mut reader);
        let deps = reader.deps;
        self.nodes[id].rule = Some(rule);
        self.evals += 1;
        let old: Vec<usize> = self.nodes[id].deps.iter().map(|(d, _)| *d).collect();
        for d in old {
            self.nodes[d].dependents.retain(|x| *x != id);
        }
        for (d, _) in &deps {
            self.nodes[*d].dependents.push(id);
        }
        let node = &mut self.nodes[id];
        node.deps = deps;
        let same = node.value.as_ref().map_or(false, |old| (node.equal)(old.as_ref(), new.as_ref()));
        if !same {
            node.value = Some(new);
            node.version += 1;
        }
        node.state = State::Clean;
    }

    /// How many times a rule has run.
    pub fn eval_count(&self) -> u64 {
        self.evals
    }

    /// Whether the value may be out of date, so the next read runs a rule or checks the ones it depends on.
    pub fn is_stale<T>(&self, attr: Attr<T>) -> bool {
        self.nodes[attr.id].state != State::Clean
    }

    /// How many times the value has changed (an input set to a different value, a rule that gave a different result).
    pub fn version<T>(&self, attr: Attr<T>) -> u64 {
        self.nodes[attr.id].version
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_diamond_recomputes_each_rule_once_per_change() {
        let mut g = Graph::new();
        let a = g.input(1i32);
        let b = g.rule(move |r| r.read(a) + 1);
        let c = g.rule(move |r| r.read(a) * 2);
        let d = g.rule(move |r| r.read(b) + r.read(c));
        assert_eq!(g.get(d), 4);
        assert_eq!(g.eval_count(), 3);
        assert!(g.set(a, 10));
        assert_eq!(g.get(d), 11 + 20);
        assert_eq!(g.eval_count(), 6);
        assert_eq!(g.get(d), 31);
        assert_eq!(g.eval_count(), 6, "a current value is not recomputed");
    }

    #[test]
    fn setting_an_equal_value_changes_nothing() {
        let mut g = Graph::new();
        let a = g.input(5i32);
        let b = g.rule(move |r| r.read(a));
        g.get(b);
        assert!(!g.set(a, 5));
        assert!(!g.is_stale(b));
        assert_eq!(g.version(a), 0);
    }

    #[test]
    fn nothing_runs_until_a_value_is_asked_for() {
        let mut g = Graph::new();
        let a = g.input(1i32);
        let b = g.rule(move |r| r.read(a) + 1);
        assert_eq!(g.eval_count(), 0);
        assert_eq!(g.get(b), 2);
        g.set(a, 2);
        g.set(a, 3);
        assert!(g.is_stale(b));
        assert_eq!(g.eval_count(), 1);
        assert_eq!(g.get(b), 4);
        assert_eq!(g.eval_count(), 2, "two sets, one recompute");
    }

    #[test]
    fn an_unchanged_result_stops_the_update() {
        let mut g = Graph::new();
        let x = g.input(2i32);
        let parity = g.rule(move |r| r.read(x) % 2);
        let label = g.rule(move |r| if r.read(parity) == 0 { "even".to_string() } else { "odd".to_string() });
        assert_eq!(g.get(label), "even");
        assert_eq!(g.eval_count(), 2);
        g.set(x, 4);
        assert_eq!(g.get(label), "even");
        assert_eq!(g.eval_count(), 3, "parity ran again; label did not");
        g.set(x, 5);
        assert_eq!(g.get(label), "odd");
        assert_eq!(g.eval_count(), 5);
    }

    #[test]
    fn dependencies_follow_what_the_rule_reads() {
        let mut g = Graph::new();
        let flag = g.input(true);
        let a = g.input(1i32);
        let b = g.input(100i32);
        let pick = g.rule(move |r| if r.read(flag) { r.read(a) } else { r.read(b) });
        assert_eq!(g.get(pick), 1);
        g.set(b, 200);
        assert!(!g.is_stale(pick), "b is not read while the flag is set");
        assert_eq!(g.get(pick), 1);
        assert_eq!(g.eval_count(), 1);
        g.set(flag, false);
        assert_eq!(g.get(pick), 200);
        g.set(a, 2);
        assert!(!g.is_stale(pick), "a is no longer read");
        g.set(b, 300);
        assert_eq!(g.get(pick), 300);
    }

    #[test]
    fn a_list_of_rows_updates_only_the_rows_that_changed() {
        let mut g = Graph::new();
        let theme = g.input("light".to_string());
        let items: Vec<Attr<i32>> = (0..5).map(|i| g.input(i)).collect();
        let rows: Vec<Attr<String>> = items.iter().map(|item| {
            let item = *item;
            g.rule(move |r| format!("{}:{}", r.read(theme), r.read(item)))
        }).collect();
        let all = rows.clone();
        let summary = g.rule(move |r| all.iter().map(|row| r.read(*row)).collect::<Vec<_>>().join(","));
        assert_eq!(g.get(summary), "light:0,light:1,light:2,light:3,light:4");
        assert_eq!(g.eval_count(), 6);
        g.set(items[2], 20);
        assert_eq!(g.get(summary), "light:0,light:1,light:20,light:3,light:4");
        assert_eq!(g.eval_count(), 8, "one row and the summary");
        g.set(theme, "dark".to_string());
        assert!(g.get(summary).starts_with("dark:0"));
        assert_eq!(g.eval_count(), 14, "every row and the summary");
    }

    #[test]
    #[should_panic(expected = "only an input can be set")]
    fn a_rule_cannot_be_set() {
        let mut g = Graph::new();
        let a = g.input(1i32);
        let b = g.rule(move |r| r.read(a));
        g.set(b, 3);
    }
}
