# 008 — Phase 6: complete application lifecycle and recovery

Status: planned. Depends on: 003, 005, 007. Parent: `001`.

## Problem (verified)

- `docs/app-manager.md` and `HANDOFF.md` record these as unfinished: persistent crash restoration, app memory-warning callbacks, and occlusion-based suspension. Unfocused visible desktop apps are not frozen.
- `bare/app_manager.rs` samples resources every five seconds on a worker. There are no memory-pressure events to apps.
- Focus adjusts SDK and native priorities (`shell/desk.rs`, Wayland focus handling).

## Steps

1. Separate focus, visibility, occlusion, and suspension as four states. Visible unfocused desktop windows keep running.
2. Add bounded memory-pressure events from cgroup `memory.pressure` or `memory.events`. Send a cache-release request before any forced termination when possible. Rate-limit warnings to avoid storms. Never call app callbacks on the rendering critical path.
3. Add versioned state checkpoints: `version` field, deliberate durable write (003 strict writer), and recovery from corrupt or incompatible state by discarding the checkpoint with a log entry.
4. Restoration: a real app restores its document, selection, or navigation after a controlled termination. Define separately what is restored after app crash, shell restart, and whole-system rollback.
5. Bounded background-task leases with expiry. Expired work is cancelled and reported.
6. Termination reasons recorded (exit, signal, OOM, cgroup kill, quit request).
7. Crash-loop policy: after N crashes in a window, stop relaunching automatically and show a user action. Never reopen an endlessly crashing app automatically.

## Acceptance

- Controlled termination restores the document or navigation state of a real app.
- A memory warning releases measurable cache state (report before and after counts).
- Visible background apps keep updating.
- Expired background work is cancelled.
- Repeated crashes stop safely and do not relaunch.
- Lifecycle transitions and restoration failure paths are tested with isolated processes. Do not disrupt a physical device to test them.
