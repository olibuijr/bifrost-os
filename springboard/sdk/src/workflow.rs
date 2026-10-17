//! Workflows: an ordered list of steps where each step gets the previous step's output as its input, can save its
//! output under a name, and can read the names saved so far. A [`WorkflowRun`] is the controller: it runs the whole
//! workflow, runs one step at a time, pauses before the next step, stops, and resets to the start. This is the model
//! behind the Automator workflow engine (`AMWorkflow` run, pause, step, stop and reset).
//!
//! The steps are plain closures, so a workflow can be built in code, and an app can also run a list of built-in
//! actions by wrapping each one in a step. A step that fails ends the run in [`RunState::Failed`] with the step's
//! message; the variables saved so far stay readable. Steps run on the calling thread, one after another.

use std::collections::BTreeMap;

/// A value passed between steps.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// No value: the input of the first step when the workflow has none, or the output of a step that returns none.
    Nothing,
    Text(String),
    Number(f64),
    List(Vec<Value>),
}

impl Value {
    /// The text form of the value: the text itself, a number without a trailing `.0`, a list joined by newlines.
    pub fn as_text(&self) -> String {
        match self {
            Value::Nothing => String::new(),
            Value::Text(t) => t.clone(),
            Value::Number(n) => {
                if n.fract() == 0.0 && n.abs() < 1e15 {
                    format!("{}", *n as i64)
                } else {
                    format!("{n}")
                }
            }
            Value::List(items) => items.iter().map(Value::as_text).collect::<Vec<_>>().join("\n"),
        }
    }
}

/// The values saved by steps, keyed by name.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Variables {
    values: BTreeMap<String, Value>,
}

impl Variables {
    /// Saves `value` under `name`, replacing an earlier value of that name.
    pub fn set(&mut self, name: &str, value: Value) {
        self.values.insert(name.to_string(), value);
    }

    /// The value saved under `name`.
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.values.get(name)
    }

    /// The names saved so far, in order.
    pub fn names(&self) -> Vec<&str> {
        self.values.keys().map(String::as_str).collect()
    }
}

/// Why a step failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StepError {
    pub message: String,
}

impl StepError {
    pub fn new(message: impl Into<String>) -> StepError {
        StepError { message: message.into() }
    }
}

type StepFn = Box<dyn Fn(&Value, &mut Variables) -> Result<Value, StepError>>;

/// One step: a name for the log, the closure, and the variable its output is saved under (if any).
pub struct Step {
    name: String,
    save_as: Option<String>,
    run: StepFn,
}

impl Step {
    /// A step that runs `run`. It gets the previous output and the saved variables, and returns its output.
    pub fn new(
        name: &str,
        run: impl Fn(&Value, &mut Variables) -> Result<Value, StepError> + 'static,
    ) -> Step {
        Step { name: name.to_string(), save_as: None, run: Box::new(run) }
    }

    /// Saves this step's output under `variable` as well as passing it on.
    pub fn saving_as(mut self, variable: &str) -> Step {
        self.save_as = Some(variable.to_string());
        self
    }

    /// The step's name.
    pub fn name(&self) -> &str {
        &self.name
    }
}

/// Where a run stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunState {
    /// Not started, or reset.
    Ready,
    /// Paused before the next step. [`WorkflowRun::resume`] or [`WorkflowRun::step`] continues it.
    Paused,
    /// Every step ran.
    Finished,
    /// A step failed; the message names the step.
    Failed(String),
    /// Stopped by the caller. The run does not continue; [`WorkflowRun::reset`] starts it again.
    Stopped,
}

/// A workflow and its controller.
pub struct WorkflowRun {
    steps: Vec<Step>,
    input: Value,
    next: usize,
    last_output: Value,
    variables: Variables,
    state: RunState,
}

impl WorkflowRun {
    /// A run of `steps` that starts with `input` for the first step.
    pub fn new(steps: Vec<Step>, input: Value) -> WorkflowRun {
        WorkflowRun {
            steps,
            last_output: input.clone(),
            input,
            next: 0,
            variables: Variables::default(),
            state: RunState::Ready,
        }
    }

    /// The state of the run.
    pub fn state(&self) -> &RunState {
        &self.state
    }

    /// The index of the next step to run.
    pub fn next_index(&self) -> usize {
        self.next
    }

    /// The output of the last step that ran, or the input when none has.
    pub fn output(&self) -> &Value {
        &self.last_output
    }

    /// The variables saved so far.
    pub fn variables(&self) -> &Variables {
        &self.variables
    }

    /// Pauses a ready run before its next step. Between calls a run is never in the middle of a step, so a pause
    /// always lands between steps. A run that has finished, failed or stopped is left as it is.
    pub fn pause(&mut self) {
        if self.state == RunState::Ready {
            self.state = RunState::Paused;
        }
    }

    /// Stops the run. Nothing else runs until [`WorkflowRun::reset`].
    pub fn stop(&mut self) {
        if matches!(self.state, RunState::Ready | RunState::Paused) {
            self.state = RunState::Stopped;
        }
    }

    /// Continues a paused run from the step it paused before, up to the end, a failure or the next pause request.
    pub fn resume(&mut self) -> &RunState {
        if self.state == RunState::Paused {
            self.state = RunState::Ready;
        }
        self.run_until_pause()
    }

    /// Runs the workflow to its end or to a failure. A paused run does not move: call [`WorkflowRun::resume`] or
    /// [`WorkflowRun::step`] for it. A stopped, failed or finished run stays as it is.
    pub fn run(&mut self) -> &RunState {
        self.run_until_pause()
    }

    /// Runs exactly one step, then pauses before the next one. A finished, failed or stopped run is left as it is.
    pub fn step(&mut self) -> &RunState {
        if !matches!(self.state, RunState::Ready | RunState::Paused) {
            return &self.state;
        }
        self.state = RunState::Ready;
        if self.next >= self.steps.len() {
            self.state = RunState::Finished;
            return &self.state;
        }
        self.run_one();
        if self.state == RunState::Ready && self.next < self.steps.len() {
            self.state = RunState::Paused;
        }
        &self.state
    }

    /// Starts again from the first step, with the original input. Saved variables are cleared.
    pub fn reset(&mut self) {
        self.next = 0;
        self.last_output = self.input.clone();
        self.variables = Variables::default();
        self.state = RunState::Ready;
    }

    /// Runs steps from a ready run until it finishes or a step fails. A run in any other state is returned as is.
    fn run_until_pause(&mut self) -> &RunState {
        if self.state != RunState::Ready {
            return &self.state;
        }
        while self.next < self.steps.len() {
            if !self.run_one() {
                return &self.state;
            }
        }
        self.state = RunState::Finished;
        &self.state
    }

    /// Runs the step at `next`. Returns false when it failed (the state is then `Failed`).
    fn run_one(&mut self) -> bool {
        let index = self.next;
        let result = {
            let step = &self.steps[index];
            (step.run)(&self.last_output, &mut self.variables).map(|out| (step.save_as.clone(), out))
        };
        match result {
            Ok((save_as, out)) => {
                if let Some(name) = save_as {
                    self.variables.set(&name, out.clone());
                }
                self.last_output = out;
                self.next += 1;
                if self.next == self.steps.len() {
                    self.state = RunState::Finished;
                }
                true
            }
            Err(e) => {
                let name = self.steps[index].name().to_string();
                self.state = RunState::Failed(format!("{name}: {}", e.message));
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    fn upper() -> Step {
        Step::new("upper", |input, _| Ok(Value::Text(input.as_text().to_uppercase())))
    }

    fn exclaim() -> Step {
        Step::new("exclaim", |input, _| Ok(Value::Text(format!("{}!", input.as_text()))))
    }

    #[test]
    fn passes_each_output_to_the_next_step() {
        let mut run = WorkflowRun::new(vec![upper(), exclaim()], Value::Text("hi".into()));
        assert_eq!(run.run(), &RunState::Finished);
        assert_eq!(run.output(), &Value::Text("HI!".into()));
    }

    #[test]
    fn an_empty_workflow_finishes_with_its_input() {
        let mut run = WorkflowRun::new(vec![], Value::Number(4.0));
        assert_eq!(run.run(), &RunState::Finished);
        assert_eq!(run.output(), &Value::Number(4.0));
    }

    #[test]
    fn saves_named_outputs_and_steps_can_read_them() {
        let steps = vec![
            Step::new("count", |_, _| Ok(Value::Number(3.0))).saving_as("count"),
            Step::new("double", |_, vars| match vars.get("count") {
                Some(Value::Number(n)) => Ok(Value::Number(n * 2.0)),
                _ => Err(StepError::new("no count")),
            }),
        ];
        let mut run = WorkflowRun::new(steps, Value::Nothing);
        assert_eq!(run.run(), &RunState::Finished);
        assert_eq!(run.output(), &Value::Number(6.0));
        assert_eq!(run.variables().get("count"), Some(&Value::Number(3.0)));
        assert_eq!(run.variables().names(), vec!["count"]);
    }

    #[test]
    fn a_failed_step_ends_the_run_and_keeps_the_saved_variables() {
        let steps = vec![
            Step::new("ok", |_, _| Ok(Value::Text("a".into()))).saving_as("first"),
            Step::new("broken", |_, _| Err(StepError::new("disk full"))),
            exclaim(),
        ];
        let mut run = WorkflowRun::new(steps, Value::Nothing);
        assert_eq!(run.run(), &RunState::Failed("broken: disk full".into()));
        assert_eq!(run.next_index(), 1);
        assert_eq!(run.variables().get("first"), Some(&Value::Text("a".into())));
    }

    #[test]
    fn step_runs_one_step_and_pauses_before_the_next() {
        let calls = Rc::new(RefCell::new(Vec::new()));
        let log = |name: &'static str| {
            let c = calls.clone();
            Step::new(name, move |_, _| {
                c.borrow_mut().push(name);
                Ok(Value::Nothing)
            })
        };
        let mut run = WorkflowRun::new(vec![log("a"), log("b")], Value::Nothing);
        assert_eq!(run.step(), &RunState::Paused);
        assert_eq!(*calls.borrow(), vec!["a"]);
        assert_eq!(run.step(), &RunState::Finished);
        assert_eq!(*calls.borrow(), vec!["a", "b"]);
        assert_eq!(run.step(), &RunState::Finished);
        assert_eq!(*calls.borrow(), vec!["a", "b"]);
    }

    #[test]
    fn pause_before_run_stops_at_the_first_step_until_resumed() {
        let mut run = WorkflowRun::new(vec![upper(), exclaim()], Value::Text("x".into()));
        run.pause();
        assert_eq!(run.run(), &RunState::Paused);
        assert_eq!(run.next_index(), 0);
        assert_eq!(run.resume(), &RunState::Finished);
        assert_eq!(run.output(), &Value::Text("X!".into()));
    }

    #[test]
    fn stop_blocks_further_steps_until_reset() {
        let mut run = WorkflowRun::new(vec![upper(), exclaim()], Value::Text("x".into()));
        run.stop();
        assert_eq!(run.run(), &RunState::Stopped);
        assert_eq!(run.step(), &RunState::Stopped);
        run.reset();
        assert_eq!(run.state(), &RunState::Ready);
        assert_eq!(run.run(), &RunState::Finished);
        assert_eq!(run.output(), &Value::Text("X!".into()));
    }

    #[test]
    fn reset_clears_variables_and_restarts_from_the_input() {
        let steps = vec![Step::new("save", |input, _| Ok(input.clone())).saving_as("kept")];
        let mut run = WorkflowRun::new(steps, Value::Text("in".into()));
        run.run();
        assert!(run.variables().get("kept").is_some());
        run.reset();
        assert_eq!(run.next_index(), 0);
        assert_eq!(run.output(), &Value::Text("in".into()));
        assert!(run.variables().names().is_empty());
    }

    #[test]
    fn number_and_list_values_have_a_text_form() {
        assert_eq!(Value::Number(2.0).as_text(), "2");
        assert_eq!(Value::Number(2.5).as_text(), "2.5");
        assert_eq!(Value::List(vec![Value::Text("a".into()), Value::Number(1.0)]).as_text(), "a\n1");
        assert_eq!(Value::Nothing.as_text(), "");
    }
}
