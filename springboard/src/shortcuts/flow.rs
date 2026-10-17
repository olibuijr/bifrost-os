//! Running a shortcut's steps with variables and blocks.
//!
//! Every step has an output, the text it produced; the next step sees it as `{Output}`, or as `input` when it is
//! performed. A step that produces nothing (Wi-Fi, a timer) leaves the previous output as it was. `Set Variable` keeps
//! the output under a name and any later text can use it as `{Name}`. `If` runs its steps when a test on the output
//! holds (and the steps after `Otherwise` when it does not); `Repeat` runs its steps a number of times. Blocks are flat in
//! the list: an opener, its steps, and an end. A block with no end runs to the end of the list, and an end with no
//! opener is skipped, so a list edited by hand never stops the run.
//!
//! The run is pure: the steps that touch the system go to a `Host`, which each program (the window, the command line)
//! implements for what it can reach. A step the host cannot do stops the shortcut, and the steps after it are reported
//! as not run.

use super::{Action, MAX_REPEAT, MAX_VAR};

/// The most steps one run performs, counting every pass through a Repeat. A runaway nest of repeats stops here.
pub const MAX_STEPS: usize = 200;

/// The variable a Repeat with Each pass sets to the line it is on.
pub const ITEM_VAR: &str = "Item";

/// What was said about a step that was not run.
const NOT_RUN: &str = "not run, an earlier step stopped the shortcut";
/// What a Stop step reports: the shortcut ends there, as a failed step does.
const STOPPED: &str = "the shortcut stopped here";

/// One step of a run: its plan line and what happened to it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    pub line: String,
    pub outcome: Result<String, String>,
}

/// What a host did for a step: a line saying so, and the output the step produced (`None` keeps the previous output).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Done {
    pub message: String,
    pub output: Option<String>,
}

impl Done {
    /// A step that was done and produced no output.
    pub fn said(message: &str) -> Done {
        Done { message: message.into(), output: None }
    }

    /// A step that was done and produced `output`.
    pub fn with_output(message: &str, output: &str) -> Done {
        Done { message: message.into(), output: Some(output.into()) }
    }
}

/// What a run needs from the program that runs it.
pub trait Host {
    /// Performs a step that touches the system. `action` has its text filled in (`{Name}` is gone); `input` is the
    /// previous output. `Err` says why the step could not be done, and stops the shortcut.
    fn perform(&mut self, action: &Action, input: &str) -> Result<Done, String>;
}

/// A piece of text with variables: plain text, or the name between braces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part<'a> {
    Text(&'a str),
    Var(&'a str),
}

/// Whether `s` can name a variable: letters, digits, spaces and underscores, no spaces at the ends, at most `MAX_VAR`
/// characters.
pub fn is_var_name(s: &str) -> bool {
    !s.is_empty()
        && s.chars().count() <= MAX_VAR
        && s.trim() == s
        && s.chars().all(|c| c.is_alphanumeric() || c == ' ' || c == '_')
}

/// Splits `text` into plain text and `{Name}` variables. A brace pair that does not hold a valid name stays plain text.
pub fn parts(text: &str) -> Vec<Part<'_>> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let (mut start, mut i) = (0, 0);
    while i < bytes.len() {
        if bytes[i] == b'{' {
            if let Some(len) = text[i + 1..].find('}') {
                let name = &text[i + 1..i + 1 + len];
                if is_var_name(name) {
                    if start < i {
                        out.push(Part::Text(&text[start..i]));
                    }
                    out.push(Part::Var(name));
                    i += len + 2;
                    start = i;
                    continue;
                }
            }
        }
        i += 1;
    }
    if start < text.len() {
        out.push(Part::Text(&text[start..]));
    }
    out
}

/// `text` with each `{Name}` replaced by that variable, or by `output` for `{Output}` when no variable has that name. A
/// name nothing defines stays as typed, so a mistake shows in the result.
pub fn expand(text: &str, vars: &[(String, String)], output: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for part in parts(text) {
        match part {
            Part::Text(t) => out.push_str(t),
            Part::Var(name) => match vars.iter().find(|(n, _)| n == name) {
                Some((_, value)) => out.push_str(value),
                None if name == "Output" => out.push_str(output),
                None => {
                    out.push('{');
                    out.push_str(name);
                    out.push('}');
                }
            },
        }
    }
    out
}

/// How deep each step sits in blocks, for indenting the editor rows: an opener is at the depth of the steps around
/// it, its steps one deeper, and Otherwise and the end are level with the opener.
pub fn depths(actions: &[Action]) -> Vec<usize> {
    let mut depth = 0usize;
    actions
        .iter()
        .map(|a| match a {
            Action::If(_) | Action::Repeat(_) | Action::RepeatEach => {
                depth += 1;
                depth - 1
            }
            Action::EndIf | Action::EndRepeat => {
                depth = depth.saturating_sub(1);
                depth
            }
            Action::Otherwise => depth.saturating_sub(1),
            _ => depth,
        })
        .collect()
}

fn is_marker(a: &Action) -> bool {
    matches!(a, Action::Otherwise | Action::EndIf | Action::EndRepeat)
}

/// The Otherwise and the end of the block opened at `open`. The end is `actions.len()` when the block never closes.
fn block(actions: &[Action], open: usize) -> (Option<usize>, usize) {
    let (mut depth, mut otherwise) = (0usize, None);
    for (j, a) in actions.iter().enumerate().skip(open + 1) {
        match a {
            Action::If(_) | Action::Repeat(_) | Action::RepeatEach => depth += 1,
            Action::EndIf | Action::EndRepeat => {
                if depth == 0 {
                    return (otherwise, j);
                }
                depth -= 1;
            }
            Action::Otherwise if depth == 0 && otherwise.is_none() => otherwise = Some(j),
            _ => {}
        }
    }
    (otherwise, actions.len())
}

/// Removes step `k`. An If or Repeat takes its whole block with it, so no end is left without its opener. Returns how
/// many steps went.
pub fn remove_step(actions: &mut Vec<Action>, k: usize) -> usize {
    if k >= actions.len() {
        return 0;
    }
    let end = match actions[k] {
        Action::If(_) | Action::Repeat(_) | Action::RepeatEach => block(actions, k).1.min(actions.len() - 1),
        _ => k,
    };
    actions.drain(k..=end).count()
}

struct Run<'a> {
    actions: &'a [Action],
    host: &'a mut dyn Host,
    vars: Vec<(String, String)>,
    output: String,
    steps: Vec<Step>,
    budget: usize,
    failed_at: Option<usize>,
}

impl Run<'_> {
    fn expand(&self, text: &str) -> String {
        expand(text, &self.vars, &self.output)
    }

    fn fail(&mut self, i: usize, why: &str) {
        self.steps.push(Step { line: self.actions[i].line(), outcome: Err(why.into()) });
        self.failed_at = Some(i);
    }

    /// Sets variable `name` to `value`, replacing its value when it is already set.
    fn set_var(&mut self, name: &str, value: String) {
        match self.vars.iter_mut().find(|(n, _)| n == name) {
            Some(slot) => slot.1 = value,
            None => self.vars.push((name.to_string(), value)),
        }
    }

    /// Runs the steps from `from` up to (not including) `to`, stopping at the first one that fails.
    fn range(&mut self, from: usize, to: usize) {
        let actions = self.actions;
        let mut i = from;
        while i < to && self.failed_at.is_none() {
            if self.budget == 0 {
                self.fail(i, "stopped, the shortcut ran too many steps");
                return;
            }
            self.budget -= 1;
            let a = &actions[i];
            match a {
                Action::If(cond) => {
                    let (otherwise, end) = block(actions, i);
                    let end = end.min(to);
                    let holds = cond.map_text(|t| self.expand(t)).holds(&self.output);
                    self.steps.push(Step { line: a.line(), outcome: Ok(if holds { "true" } else { "false" }.to_string()) });
                    let split = otherwise.map(|o| o.min(end));
                    if holds {
                        self.range(i + 1, split.unwrap_or(end));
                    } else if let Some(o) = split {
                        self.range(o + 1, end);
                    }
                    i = end + 1;
                }
                Action::Repeat(n) => {
                    let (_, end) = block(actions, i);
                    let end = end.min(to);
                    self.steps.push(Step { line: a.line(), outcome: Ok(a.value()) });
                    for _ in 0..*n {
                        if self.failed_at.is_some() {
                            break;
                        }
                        self.range(i + 1, end);
                    }
                    i = end + 1;
                }
                Action::RepeatEach => {
                    let (_, end) = block(actions, i);
                    let end = end.min(to);
                    // one pass per line of the previous output, at most MAX_REPEAT of them
                    let lines: Vec<String> = self
                        .output
                        .lines()
                        .map(str::trim)
                        .filter(|l| !l.is_empty())
                        .take(MAX_REPEAT as usize)
                        .map(str::to_string)
                        .collect();
                    let count = if lines.len() == 1 { "1 line".to_string() } else { format!("{} lines", lines.len()) };
                    self.steps.push(Step { line: a.line(), outcome: Ok(count) });
                    for line in lines {
                        if self.failed_at.is_some() {
                            break;
                        }
                        self.set_var(ITEM_VAR, line);
                        self.range(i + 1, end);
                    }
                    i = end + 1;
                }
                Action::Stop => {
                    self.steps.push(Step { line: a.line(), outcome: Err(STOPPED.into()) });
                    self.failed_at = Some(i);
                    i += 1;
                }
                Action::Otherwise | Action::EndIf | Action::EndRepeat => i += 1,
                Action::SetVariable(name) => {
                    let value = self.output.clone();
                    let shown: String = value.chars().take(40).collect();
                    match self.vars.iter_mut().find(|(n, _)| n == name) {
                        Some(slot) => slot.1 = value,
                        None => self.vars.push((name.clone(), value)),
                    }
                    self.steps.push(Step { line: a.line(), outcome: Ok(format!("{name} = {shown}")) });
                    i += 1;
                }
                Action::Text(t) => {
                    let out = self.expand(t);
                    self.output = out.clone();
                    self.steps.push(Step { line: a.line(), outcome: Ok(out) });
                    i += 1;
                }
                other => {
                    let ready = other.map_text(|t| self.expand(t));
                    match self.host.perform(&ready, &self.output) {
                        Ok(done) => {
                            if let Some(out) = done.output {
                                self.output = out;
                            }
                            self.steps.push(Step { line: a.line(), outcome: Ok(done.message) });
                        }
                        Err(why) => {
                            self.steps.push(Step { line: a.line(), outcome: Err(why) });
                            self.failed_at = Some(i);
                        }
                    }
                    i += 1;
                }
            }
        }
    }
}

/// Runs `actions` through `host` and reports each step that ran, in order, with the steps of a block once per pass. The
/// first step that cannot be done stops the run; every step after it in the list is reported as not run.
pub fn run(actions: &[Action], host: &mut dyn Host) -> Vec<Step> {
    run_with_input(actions, host, "")
}

/// Like [`run`], but the first step sees `input` as the previous output: the text a caller hands the shortcut (the files
/// dropped on its droplet, one path per line).
pub fn run_with_input(actions: &[Action], host: &mut dyn Host, input: &str) -> Vec<Step> {
    let mut r = Run { actions, host, vars: Vec::new(), output: input.to_string(), steps: Vec::new(), budget: MAX_STEPS, failed_at: None };
    r.range(0, actions.len());
    if let Some(f) = r.failed_at {
        for a in actions.iter().skip(f + 1).filter(|a| !is_marker(a)) {
            r.steps.push(Step { line: a.line(), outcome: Err(NOT_RUN.into()) });
        }
    }
    r.steps
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shortcuts::Cond;

    /// A host with canned answers that remembers what it was asked.
    #[derive(Default)]
    struct Fake {
        seen: Vec<String>,
    }

    impl Host for Fake {
        fn perform(&mut self, a: &Action, input: &str) -> Result<Done, String> {
            self.seen.push(format!("{} <{input}>", a.line()));
            match a {
                Action::Wifi(_) => Err("no radio".into()),
                Action::AskInput(_) => Ok(Done::with_output("asked", "Ada")),
                Action::GetUrl(_) => Ok(Done::with_output("fetched", "Example Domain")),
                Action::RunScript(s) => Ok(Done::with_output(&format!("ran {s}"), "up 3 days")),
                Action::ShowResult(t) => Ok(Done::said(t)),
                _ => Ok(Done::said("done")),
            }
        }
    }

    fn vars(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs.iter().map(|(n, v)| (n.to_string(), v.to_string())).collect()
    }

    fn outcomes(steps: &[Step]) -> Vec<String> {
        steps.iter().map(|s| format!("{} => {}", s.line, s.outcome.clone().unwrap_or_else(|e| format!("ERR {e}")))).collect()
    }

    #[test]
    fn text_splits_into_plain_parts_and_variables() {
        assert_eq!(parts("Hi {Name}!"), vec![Part::Text("Hi "), Part::Var("Name"), Part::Text("!")]);
        assert_eq!(parts("{A}{B}"), vec![Part::Var("A"), Part::Var("B")]);
        assert_eq!(parts("no braces"), vec![Part::Text("no braces")]);
        assert!(parts("").is_empty());
    }

    #[test]
    fn a_brace_pair_without_a_valid_name_stays_text() {
        for text in ["{}", "{ a }", "{a-b}", "{open", "close}", "{{x", "{é!}"] {
            assert_eq!(parts(text), vec![Part::Text(text)], "{text}");
        }
        assert_eq!(parts("{{x}"), vec![Part::Text("{"), Part::Var("x")], "the inner pair is the variable");
        assert_eq!(parts("日本{名前}"), vec![Part::Text("日本"), Part::Var("名前")], "names may use any letters");
        let long = format!("{{{}}}", "n".repeat(MAX_VAR + 1));
        assert_eq!(parts(&long), vec![Part::Text(long.as_str())]);
    }

    #[test]
    fn variables_and_the_output_fill_in() {
        let v = vars(&[("Name", "Ada"), ("Count", "3")]);
        assert_eq!(expand("Hi {Name}, {Count} left. {Output}", &v, "ok"), "Hi Ada, 3 left. ok");
        assert_eq!(expand("{Missing} {Output}", &[], "x"), "{Missing} x", "an unknown name stays as typed");
        assert_eq!(expand("{Output}", &vars(&[("Output", "mine")]), "x"), "mine", "a variable wins over the output");
        assert_eq!(expand("100% {Name};", &v, ""), "100% Ada;");
    }

    #[test]
    fn variable_names_follow_the_rule() {
        assert!(is_var_name("Name") && is_var_name("my var_2"));
        assert!(!is_var_name("") && !is_var_name(" a") && !is_var_name("a-b"));
        assert!(!is_var_name(&"n".repeat(MAX_VAR + 1)) && is_var_name(&"n".repeat(MAX_VAR)));
    }

    #[test]
    fn an_answer_flows_through_a_variable_into_the_result() {
        let list = vec![
            Action::AskInput("Name?".into()),
            Action::SetVariable("Name".into()),
            Action::Text("Hello, {Name}!".into()),
            Action::ShowResult("{Output}".into()),
        ];
        let mut host = Fake::default();
        let steps = run(&list, &mut host);
        assert_eq!(
            outcomes(&steps),
            vec![
                "Ask for Input: Name? => asked",
                "Set Variable: Name => Name = Ada",
                "Text: Hello, {Name}! => Hello, Ada!",
                "Show Result: {Output} => Hello, Ada!",
            ]
        );
        assert_eq!(host.seen.last().unwrap(), "Show Result: Hello, Ada! <Hello, Ada!>", "the host got the text filled in");
    }

    #[test]
    fn a_step_with_no_output_passes_the_previous_output_on() {
        let list = vec![Action::Text("keep".into()), Action::Dnd(true), Action::ShowResult("{Output}".into())];
        let mut host = Fake::default();
        let steps = run(&list, &mut host);
        assert_eq!(steps[2].outcome, Ok("keep".into()));
        assert_eq!(host.seen[0], "Set Do Not Disturb: On <keep>", "the host sees the previous output as its input");
    }

    #[test]
    fn a_variable_set_again_takes_the_new_value() {
        let list = vec![
            Action::Text("one".into()),
            Action::SetVariable("V".into()),
            Action::Text("two".into()),
            Action::SetVariable("V".into()),
            Action::Text("{V}".into()),
        ];
        let steps = run(&list, &mut Fake::default());
        assert_eq!(steps[4].outcome, Ok("two".into()));
    }

    #[test]
    fn if_runs_its_steps_when_the_test_holds_and_otherwise_when_it_does_not() {
        let list = |word: &str| {
            vec![
                Action::Text(word.into()),
                Action::If(Cond::Equals("yes".into())),
                Action::Text("then".into()),
                Action::Otherwise,
                Action::Text("else".into()),
                Action::EndIf,
                Action::Text("after".into()),
            ]
        };
        let yes = outcomes(&run(&list("yes"), &mut Fake::default()));
        assert_eq!(yes, vec!["Text: yes => yes", "If: is \"yes\" => true", "Text: then => then", "Text: after => after"]);
        let no = outcomes(&run(&list("no"), &mut Fake::default()));
        assert_eq!(no, vec!["Text: no => no", "If: is \"yes\" => false", "Text: else => else", "Text: after => after"]);
    }

    #[test]
    fn an_if_without_otherwise_skips_its_steps() {
        let list = vec![
            Action::If(Cond::NotEmpty),
            Action::Text("never".into()),
            Action::EndIf,
            Action::Text("after".into()),
        ];
        let steps = run(&list, &mut Fake::default());
        assert_eq!(outcomes(&steps), vec!["If: has a value => false", "Text: after => after"]);
    }

    #[test]
    fn an_if_test_can_use_a_variable() {
        let list = vec![
            Action::Text("abc".into()),
            Action::SetVariable("Key".into()),
            Action::Text("xxabcxx".into()),
            Action::If(Cond::Contains("{Key}".into())),
            Action::Text("found".into()),
            Action::EndIf,
        ];
        let steps = run(&list, &mut Fake::default());
        assert_eq!(steps.last().unwrap().outcome, Ok("found".into()));
    }

    #[test]
    fn repeat_runs_its_steps_that_many_times() {
        let list = vec![Action::Repeat(3), Action::Dnd(true), Action::EndRepeat, Action::Dnd(false)];
        let mut host = Fake::default();
        let steps = run(&list, &mut host);
        assert_eq!(host.seen.len(), 4);
        assert_eq!(steps[0].line, "Repeat: 3 times");
        assert_eq!(steps.len(), 5);
    }

    #[test]
    fn blocks_nest() {
        let list = vec![
            Action::Repeat(2),
            Action::Repeat(3),
            Action::Dnd(true),
            Action::EndRepeat,
            Action::If(Cond::Empty),
            Action::Text("empty".into()),
            Action::EndIf,
            Action::EndRepeat,
        ];
        let mut host = Fake::default();
        run(&list, &mut host);
        assert_eq!(host.seen.len(), 6, "three steps in each of two passes");
    }

    #[test]
    fn a_failing_step_stops_the_run_and_the_rest_is_not_run() {
        let list = vec![
            Action::Text("a".into()),
            Action::Wifi(true),
            Action::Repeat(2),
            Action::Timer(5),
            Action::EndRepeat,
            Action::Dnd(true),
        ];
        let mut host = Fake::default();
        let steps = run(&list, &mut host);
        assert_eq!(host.seen.len(), 1, "nothing after the failure is performed");
        assert_eq!(steps[1].outcome, Err("no radio".into()));
        let rest: Vec<&str> = steps[2..].iter().map(|s| s.line.as_str()).collect();
        assert_eq!(rest, vec!["Repeat: 2 times", "Start Timer: 5 min", "Set Do Not Disturb: On"], "the End Repeat marker is not listed");
        assert!(steps[2..].iter().all(|s| s.outcome == Err(NOT_RUN.to_string())));
    }

    #[test]
    fn a_failure_inside_a_repeat_stops_every_pass() {
        let list = vec![Action::Repeat(5), Action::Wifi(true), Action::EndRepeat, Action::Dnd(true)];
        let mut host = Fake::default();
        let steps = run(&list, &mut host);
        assert_eq!(host.seen.len(), 1);
        assert_eq!(steps.last().unwrap().line, "Set Do Not Disturb: On");
        assert!(steps.last().unwrap().outcome.is_err());
    }

    #[test]
    fn a_runaway_nest_of_repeats_stops_at_the_step_limit() {
        let list = vec![
            Action::Repeat(20),
            Action::Repeat(20),
            Action::Repeat(20),
            Action::Dnd(true),
            Action::EndRepeat,
            Action::EndRepeat,
            Action::EndRepeat,
        ];
        let mut host = Fake::default();
        let steps = run(&list, &mut host);
        assert!(host.seen.len() < MAX_STEPS);
        assert!(steps.len() <= MAX_STEPS + 5);
        let last = steps.last().unwrap();
        assert!(last.outcome.as_ref().unwrap_err().contains("too many steps"), "{last:?}");
    }

    #[test]
    fn a_block_with_no_end_runs_to_the_end_and_a_stray_end_is_skipped() {
        let open = vec![Action::If(Cond::Empty), Action::Text("in".into())];
        assert_eq!(outcomes(&run(&open, &mut Fake::default())), vec!["If: is empty => true", "Text: in => in"]);
        let stray = vec![Action::EndIf, Action::Otherwise, Action::Text("x".into()), Action::EndRepeat];
        assert_eq!(outcomes(&run(&stray, &mut Fake::default())), vec!["Text: x => x"]);
    }

    #[test]
    fn a_command_output_feeds_the_next_step() {
        let list = vec![
            Action::RunScript("uptime".into()),
            Action::If(Cond::Contains("days".into())),
            Action::ShowResult("Long run: {Output}".into()),
            Action::EndIf,
        ];
        let mut host = Fake::default();
        let steps = run(&list, &mut host);
        assert_eq!(steps.last().unwrap().outcome, Ok("Long run: up 3 days".into()));
    }

    #[test]
    fn rows_are_indented_by_their_blocks() {
        let list = vec![
            Action::Text("a".into()),
            Action::If(Cond::Empty),
            Action::Repeat(2),
            Action::Dnd(true),
            Action::EndRepeat,
            Action::Otherwise,
            Action::Dnd(false),
            Action::EndIf,
            Action::EndIf,
            Action::Dnd(true),
        ];
        assert_eq!(depths(&list), vec![0, 0, 1, 2, 1, 0, 1, 0, 0, 0], "a stray end never goes below zero");
    }

    #[test]
    fn removing_an_opener_removes_its_block_and_a_plain_step_only_itself() {
        let mut list = vec![
            Action::Text("a".into()),
            Action::If(Cond::Empty),
            Action::Repeat(2),
            Action::Dnd(true),
            Action::EndRepeat,
            Action::Otherwise,
            Action::Dnd(false),
            Action::EndIf,
            Action::Text("z".into()),
        ];
        assert_eq!(remove_step(&mut list, 0), 1);
        assert_eq!(remove_step(&mut list, 0), 7, "the If, both branches and the nested Repeat");
        assert_eq!(list, vec![Action::Text("z".into())]);
        assert_eq!(remove_step(&mut list, 5), 0, "past the end removes nothing");
    }

    #[test]
    fn removing_an_unclosed_opener_removes_what_follows_it() {
        let mut list = vec![Action::Text("a".into()), Action::Repeat(2), Action::Dnd(true)];
        assert_eq!(remove_step(&mut list, 1), 2);
        assert_eq!(list, vec![Action::Text("a".into())]);
    }

    #[test]
    fn a_stray_end_is_removed_alone() {
        let mut list = vec![Action::EndIf, Action::Dnd(true)];
        assert_eq!(remove_step(&mut list, 0), 1);
        assert_eq!(list, vec![Action::Dnd(true)]);
    }

    #[test]
    fn an_empty_list_runs_nothing() {
        assert!(run(&[], &mut Fake::default()).is_empty());
    }
}
