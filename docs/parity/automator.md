# Automator parity

Official reference: https://developer.apple.com/documentation/automator (macOS 10.4 and later; the AppleScript action
class is deprecated).

## What was built

Two pure Rust models, std only, with no unsafe code and no new crates. Neither one runs anything.

- `springboard/ui/src/workflow.rs` (`bifrost_ui::workflow`). Mirrors the workflow and controller part of the reference
  (`AMWorkflow`, `AMWorkflowController` and the error cases in `AMError`).
  - `Workflow` holds ordered `Step`s with an `enabled` flag and a `needs_input` flag. It keeps the results of the last
    run and exposes the workflow output.
  - Each step's output is the input of the next step. The first step receives the run input.
  - `WorkflowController` offers `run` (all steps), `step` (one step, then paused), `resume`, `stop` (only from paused),
    `reset` and `can_run`. `RunState` reports Idle, Paused, Finished, Stopped or Failed.
  - `WorkflowError` has `NoEnabledSteps`, `InsufficientInput { step }` and `StepFailed { step, reason }`.
  - Steps are closures `FnMut(&str) -> Result<String, String>`, so a caller decides what a step does.
- `springboard/core/src/script_io.rs` (`core::script_io`). Mirrors the I/O part of `AMShellScriptAction`:
  `inputFieldSeparator`, `outputFieldSeparator` and `remapLineEndings`, as `ScriptIo` with `encode_input` and
  `decode_output`. It spawns no process.

## Official API names mirrored

- `AMWorkflow`: ordered actions, input, output, results per run (`Workflow`).
- `AMWorkflowController`: `run`, `step`, `resume` (the run method on a paused workflow), `stop`, `reset`, `canRun`
  (`can_run`). `pause` is the state a step leaves behind, because execution is synchronous.
- `AMError` codes: `workflowNoEnabledActionsError`, `actionInsufficientDataError`, `actionExecutionError`
  (`WorkflowError`).
- `AMShellScriptAction`: `inputFieldSeparator`, `outputFieldSeparator`, `remapLineEndings` (`ScriptIo`).

Choices that are not in the official reference: the default separators (newline) and the "no remapping" default in
`ScriptIo`. Execution is synchronous, so `isRunning` and the controller delegate are not modelled. The reference does
not say what happens to a paused run on other calls, so `run` on a paused workflow restarts it.

## REA evidence

- `ev_a34202871a969c8341f73586cb24c93d7dc9782b14d30adf09f095d151b2859d`: rendered Automator landing page
  (https://developer.apple.com/documentation/automator), taken this session. Its text lists the action, workflow,
  error and deprecated sections, and the three bundle locations.
- `ev_92931747df2475eb5c7c1609143dbc235c17a2ebaeacf97a4e6864461e6a6ed8`: earlier evidence recorded in the matrix row.

Official pages read with `docc.py`: `automator`, `automator/amshellscriptaction`, `automator/amworkflowcontroller`,
`automator/amerror`.

## Remaining gaps

- Nothing runs a script step. A shell, Perl or Python runtime and the process wiring for `ScriptIo` do not exist.
- The Shortcuts app does not build a `Workflow` or call `WorkflowController`. Its actions are still logged, not run.
- No variables between steps (`valueForVariable`).
- No action bundle format or install scan.
- No delegate, no `isRunning`, and no run-by-path API or workflow file format.
- No error code table beyond the three cases.
- Shortcuts does not list or run intents from other apps.

## Tests

Unit tests are in `springboard/ui/src/workflow.rs` (8 behaviour tests) and `springboard/core/src/script_io.rs` (7
behaviour tests). They were not built or run in this stage; the build stage runs them.
