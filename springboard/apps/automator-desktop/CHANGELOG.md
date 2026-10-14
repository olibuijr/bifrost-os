# Changelog

## 0.1.1 — 2026-10-14

- Schedule workflow execution and script input work through the shared dispatch queues.

## 0.1.0

- First version: a workflow is a sequence of actions (Find Files, Rename, Copy, Move, Transform, Combine, Resize,
  Convert, Run Script). Each action's output feeds the next one as files or text.
- Workflows run in the background with a log; errors stop the run at the failing step.
- Save and open workflows as `.automator` files through the shell's save and open panels.
