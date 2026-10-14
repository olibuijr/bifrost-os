# Changelog

## 0.1.4 — 2026-10-14

- Mark the fixed test-window dimensions as a geometry fixture; production geometry is configured by the host.

## 0.1.3 — 2026-10-14

- Derive the initial content geometry from the host-provided window size instead of assuming a default resolution.

## 0.1.2 — 2026-10-14

- Add the generated app compliance harness.

## 0.1.1 — 2026-10-14

- Schedule workflow execution and script input work through the shared dispatch queues.

## 0.1.0

- First version: a workflow is a sequence of actions (Find Files, Rename, Copy, Move, Transform, Combine, Resize,
  Convert, Run Script). Each action's output feeds the next one as files or text.
- Workflows run in the background with a log; errors stop the run at the failing step.
- Save and open workflows as `.automator` files through the shell's save and open panels.
