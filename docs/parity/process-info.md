# Process records and process events

Mirrors the process-event side of the reference endpoint-security framework (process execution, fork and exit) as
far as procfs text allows.

Code: `springboard/core/src/process_info.rs` (`parse_stat`, `parse_cmdline`, `parent_chain`, `snapshot_events`).
Tests: `springboard/core/tests/parity_sdk-complete-45.rs`, headless: no file is read.

## What it does

- `parse_stat` reads a `/proc/<pid>/stat` line: pid, command name (the text between the first `(` and the last `)`),
  state and parent pid.
- `parse_cmdline` splits a `/proc/<pid>/cmdline` buffer on its NUL bytes.
- `parent_chain` walks from a process to init. It stops at a missing parent or a repeated one.
- `snapshot_events` compares two process tables and reports exits, then starts. A pid reused by another command
  reports an exit and a start.

## Limits

Snapshots show the table, not the calls. A fork appears as a start with its parent, and an exec keeps the pid, so
it is not reported. Arguments, environment and file descriptors are not read.

## What is wired

Nothing reads `/proc` yet. The shell still reports only its own app launches and terminations
(`springboard/sdk/src/workspace.rs`).

## What needs the lab machine

Live `/proc` reads and the event stream of real processes are checked on Midget only.
