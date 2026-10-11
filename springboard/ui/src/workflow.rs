//! An ordered workflow of steps (`NativeWorkflow`) and its controller (`NativeWorkflowController`). The output of each
//! step is the input of the next. A run can start from scratch, go one step at a time, resume, stop and reset.
//!
//! Execution is synchronous: a step runs to completion inside the call that starts it, so no "running" state can be
//! observed from outside. A caller that wants to pause between steps calls [`WorkflowController::step`] instead of
//! [`WorkflowController::run`].

/// Why a run failed (the workflow error codes that a run of steps can produce).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkflowError {
    /// Every step is disabled, or there are no steps (`workflowNoEnabledActionsError`).
    NoEnabledSteps,
    /// A step needs input and the input is empty (`actionInsufficientDataError`).
    InsufficientInput { step: usize },
    /// A step reported a failure with a reason (`actionExecutionError`).
    StepFailed { step: usize, reason: String },
}

type StepWork = Box<dyn FnMut(&str) -> Result<String, String>>;

/// One step: a name, whether it runs, whether it needs input, and the work it does on its input.
pub struct Step {
    pub name: String,
    pub enabled: bool,
    pub needs_input: bool,
    work: StepWork,
}

impl Step {
    pub fn new(name: &str, work: impl FnMut(&str) -> Result<String, String> + 'static) -> Self {
        Step { name: name.into(), enabled: true, needs_input: false, work: Box::new(work) }
    }
}

/// Ordered steps. The results of the last run are kept until the next run or a reset.
#[derive(Default)]
pub struct Workflow {
    steps: Vec<Step>,
    results: Vec<String>,
}

impl Workflow {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn add(&mut self, step: Step) {
        self.steps.push(step);
    }
    /// Inserts at `index`; an index past the end appends.
    pub fn insert(&mut self, index: usize, step: Step) {
        let index = index.min(self.steps.len());
        self.steps.insert(index, step);
    }
    pub fn remove(&mut self, index: usize) -> Option<Step> {
        (index < self.steps.len()).then(|| self.steps.remove(index))
    }
    pub fn set_enabled(&mut self, index: usize, enabled: bool) {
        if let Some(step) = self.steps.get_mut(index) {
            step.enabled = enabled;
        }
    }
    pub fn steps(&self) -> &[Step] {
        &self.steps
    }
    /// The output of each step that ran in the last run, in order.
    pub fn results(&self) -> &[String] {
        &self.results
    }
    /// The output of the last step that ran (`output`). `None` before a run, or when the run produced nothing.
    pub fn output(&self) -> Option<&str> {
        self.results.last().map(String::as_str)
    }
    fn enabled_count(&self) -> usize {
        self.steps.iter().filter(|s| s.enabled).count()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunState {
    /// Nothing has run since the last reset.
    Idle,
    /// A single step ran and more enabled steps remain. Call `step` or `resume`.
    Paused,
    /// Every enabled step ran.
    Finished,
    /// The run was stopped while paused; the remaining steps will not run.
    Stopped,
    /// A step failed. The error is available from `error`.
    Failed,
}

/// Drives a [`Workflow`]: run it whole, one step at a time, or resume it, with stop and reset.
pub struct WorkflowController {
    workflow: Workflow,
    state: RunState,
    /// Index of the next step to consider; disabled steps are skipped.
    next: usize,
    input: String,
    error: Option<WorkflowError>,
}

impl WorkflowController {
    pub fn new(workflow: Workflow) -> Self {
        WorkflowController { workflow, state: RunState::Idle, next: 0, input: String::new(), error: None }
    }
    pub fn workflow(&self) -> &Workflow {
        &self.workflow
    }
    pub fn state(&self) -> RunState {
        self.state
    }
    pub fn error(&self) -> Option<&WorkflowError> {
        self.error.as_ref()
    }
    /// True when there is at least one enabled step for `run` or `step` to execute (`canRun`).
    pub fn can_run(&self) -> bool {
        self.workflow.enabled_count() > 0
    }
    /// Starts a fresh run with `input` and runs every enabled step. Results of an earlier run are cleared.
    pub fn run(&mut self, input: &str) -> Result<(), WorkflowError> {
        self.restart(input)?;
        while self.advance()? {}
        self.state = RunState::Finished;
        Ok(())
    }
    /// Starts a fresh run with `input`, or continues a paused run (then `input` is ignored), and runs exactly one
    /// step. The run is paused afterwards if enabled steps remain, otherwise finished.
    pub fn step(&mut self, input: &str) -> Result<(), WorkflowError> {
        if self.state != RunState::Paused {
            self.restart(input)?;
        }
        self.advance()?;
        self.state = if self.has_pending() { RunState::Paused } else { RunState::Finished };
        Ok(())
    }
    /// Runs the remaining steps of a paused run to the end (`run` on a paused workflow). Does nothing otherwise.
    pub fn resume(&mut self) -> Result<(), WorkflowError> {
        if self.state != RunState::Paused {
            return Ok(());
        }
        while self.advance()? {}
        self.state = RunState::Finished;
        Ok(())
    }
    /// Stops a paused run; its remaining steps never run. Does nothing in other states.
    pub fn stop(&mut self) {
        if self.state == RunState::Paused {
            self.state = RunState::Stopped;
        }
    }
    /// Clears the results, the input and any error, and returns to `Idle`.
    pub fn reset(&mut self) {
        self.workflow.results.clear();
        self.next = 0;
        self.input.clear();
        self.error = None;
        self.state = RunState::Idle;
    }

    fn restart(&mut self, input: &str) -> Result<(), WorkflowError> {
        self.workflow.results.clear();
        self.next = 0;
        self.input = input.into();
        self.error = None;
        if self.workflow.enabled_count() == 0 {
            return Err(self.fail(WorkflowError::NoEnabledSteps));
        }
        Ok(())
    }

    fn has_pending(&self) -> bool {
        self.workflow.steps[self.next..].iter().any(|s| s.enabled)
    }

    /// Runs the next enabled step. Returns `Ok(false)` when no enabled step is left.
    fn advance(&mut self) -> Result<bool, WorkflowError> {
        while self.next < self.workflow.steps.len() && !self.workflow.steps[self.next].enabled {
            self.next += 1;
        }
        if self.next == self.workflow.steps.len() {
            return Ok(false);
        }
        let index = self.next;
        let input = match self.workflow.results.last() {
            Some(previous) => previous.clone(),
            None => self.input.clone(),
        };
        if self.workflow.steps[index].needs_input && input.is_empty() {
            return Err(self.fail(WorkflowError::InsufficientInput { step: index }));
        }
        match (self.workflow.steps[index].work)(&input) {
            Ok(output) => {
                self.workflow.results.push(output);
                self.next += 1;
                Ok(true)
            }
            Err(reason) => Err(self.fail(WorkflowError::StepFailed { step: index, reason })),
        }
    }

    fn fail(&mut self, error: WorkflowError) -> WorkflowError {
        self.state = RunState::Failed;
        self.error = Some(error.clone());
        error
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, rc::Rc};

    fn counted(counter: &Rc<Cell<u32>>, out: &'static str) -> Step {
        let c = counter.clone();
        Step::new(out, move |_| {
            c.set(c.get() + 1);
            Ok(out.to_string())
        })
    }

    #[test]
    fn each_step_gets_the_previous_output() {
        let mut wf = Workflow::new();
        wf.add(Step::new("trim", |s| Ok(s.trim().to_string())));
        wf.add(Step::new("upper", |s| Ok(s.to_uppercase())));
        let mut c = WorkflowController::new(wf);
        c.run("  hi ").unwrap();
        assert_eq!(c.workflow().output(), Some("HI"));
        assert_eq!(c.workflow().results(), ["hi", "HI"]);
        assert_eq!(c.state(), RunState::Finished);
    }

    #[test]
    fn disabled_steps_never_run() {
        let ran = Rc::new(Cell::new(0));
        let mut wf = Workflow::new();
        wf.add(Step::new("first", |s| Ok(format!("{s}1"))));
        wf.add(counted(&ran, "skipped"));
        wf.add(Step::new("last", |s| Ok(format!("{s}2"))));
        wf.set_enabled(1, false);
        let mut c = WorkflowController::new(wf);
        c.run("x").unwrap();
        assert_eq!(ran.get(), 0);
        assert_eq!(c.workflow().output(), Some("x12"));
    }

    #[test]
    fn a_failing_step_stops_the_run_and_names_the_step() {
        let ran = Rc::new(Cell::new(0));
        let mut wf = Workflow::new();
        wf.add(Step::new("ok", |_| Ok("fine".into())));
        wf.add(Step::new("broken", |_| Err("disk full".into())));
        wf.add(counted(&ran, "after"));
        let mut c = WorkflowController::new(wf);
        let err = c.run("").unwrap_err();
        assert_eq!(err, WorkflowError::StepFailed { step: 1, reason: "disk full".into() });
        assert_eq!(c.error(), Some(&err));
        assert_eq!(c.state(), RunState::Failed);
        assert_eq!(c.workflow().results(), ["fine"]);
        assert_eq!(ran.get(), 0);
    }

    #[test]
    fn step_pauses_after_each_step_and_resume_finishes_the_run() {
        let ran = Rc::new(Cell::new(0));
        let mut wf = Workflow::new();
        wf.add(counted(&ran, "a"));
        wf.add(counted(&ran, "b"));
        wf.add(counted(&ran, "c"));
        let mut c = WorkflowController::new(wf);
        c.step("start").unwrap();
        assert_eq!((c.state(), ran.get()), (RunState::Paused, 1));
        c.step("ignored").unwrap();
        assert_eq!((c.state(), ran.get()), (RunState::Paused, 2));
        c.resume().unwrap();
        assert_eq!((c.state(), ran.get()), (RunState::Finished, 3));
        assert_eq!(c.workflow().output(), Some("c"));
    }

    #[test]
    fn stop_while_paused_keeps_the_remaining_steps_from_running() {
        let ran = Rc::new(Cell::new(0));
        let mut wf = Workflow::new();
        wf.add(counted(&ran, "a"));
        wf.add(counted(&ran, "b"));
        let mut c = WorkflowController::new(wf);
        c.step("").unwrap();
        c.stop();
        assert_eq!(c.state(), RunState::Stopped);
        c.resume().unwrap();
        assert_eq!((c.state(), ran.get()), (RunState::Stopped, 1));
    }

    #[test]
    fn a_workflow_without_enabled_steps_cannot_run() {
        let mut wf = Workflow::new();
        wf.add(Step::new("off", |s| Ok(s.to_string())));
        wf.set_enabled(0, false);
        let mut c = WorkflowController::new(wf);
        assert!(!c.can_run());
        assert_eq!(c.run("x"), Err(WorkflowError::NoEnabledSteps));
        assert_eq!(c.state(), RunState::Failed);
    }

    #[test]
    fn a_step_that_needs_input_rejects_an_empty_input() {
        let mut wf = Workflow::new();
        let mut step = Step::new("write", |s| Ok(s.to_uppercase()));
        step.needs_input = true;
        wf.add(step);
        let mut c = WorkflowController::new(wf);
        assert_eq!(c.run(""), Err(WorkflowError::InsufficientInput { step: 0 }));
        c.run("ok").unwrap();
        assert_eq!(c.workflow().output(), Some("OK"));
    }

    #[test]
    fn reset_clears_results_and_errors() {
        let mut wf = Workflow::new();
        wf.add(Step::new("boom", |_| Err("no".into())));
        let mut c = WorkflowController::new(wf);
        let _ = c.run("x");
        c.reset();
        assert_eq!(c.state(), RunState::Idle);
        assert!(c.error().is_none());
        assert!(c.workflow().results().is_empty());
        assert_eq!(c.workflow().output(), None);
    }
}
