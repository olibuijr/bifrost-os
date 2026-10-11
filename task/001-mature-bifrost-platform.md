# Mature the BifrOSt application platform and OS boundaries (overview)

Status: partially implemented. HTTPS hardening and initial baseline review are integrated; milestones 003–010 remain open.
Created: 2026-10-08. Revised: 2026-10-08 (after codebase inspection).
Owner: one implementation agent, working inline; do not delegate or spawn subagents.
Review: a separate agent reviews and merges each milestone.

## Objective

Turn the existing BifrOSt foundations into a stable application platform: apps build independently of the shell, communicate through versioned and bounded service APIs, access documents through explicit grants, survive lifecycle transitions, and inherit consistent input and accessibility behavior. Reduce the graphical shell's privilege and improve rendering based on measurements. Preserve working update recovery and independently packaged apps throughout the migration.

This is delivered as ordered milestones. Do not rewrite all subsystems at once. A design document alone never completes a milestone; scaffolding is not a completed phase.

## Milestones

| File | Scope | Depends on |
|---|---|---|
| `002-baseline-and-ownership.md` | Phase 0: baseline, dirty-tree ownership, security triage | none |
| `003-state-ownership-persistence.md` | Phase 1: owned catalogue, stable IDs, durable writes | 002 |
| `004-sdk-shell-decoupling.md` | Phase 2: SDK and apps off the shell crate | 003 |
| `005-app-service-protocol.md` | Phase 3: negotiated, bounded, typed app/service IPC | 003 |
| `006-document-grants.md` | Phase 4: brokered document access | 005 |
| `007-session-privilege-brokers.md` | Phase 5: non-root PC session via brokers | 005 (uses 006 boundaries) |
| `008-lifecycle-recovery.md` | Phase 6: focus/visibility, memory pressure, restoration, crash-loop | 003, 005, 007 |
| `009-text-input-accessibility.md` | Phase 7: grapheme editing, IME, accessibility tree | 004, 005 |
| `010-rendering-measurement.md` | Phase 8: instrumented, measured rendering work | 002 (baseline); code changes after 004 |
| `011-update-trust.md` | Phase 9: public-key update trust | investigation now; implementation after 005 and 007 |

Phase 9's investigation and the HMAC key triage (002, item 4) start immediately because they are a security finding. Milestones 003 to 011 each produce a reviewable checkpoint with its own exit report.

## Shared constraints

- Read `AGENTS.md`, the current-state portion of `HANDOFF.md`, and the bifrost, bifrost-dev, bifrost-boot, and bifrost-audio skills before implementation.
- Read `docs/codebase-cleanup.md`, `docs/bifrost-ui.md`, `docs/app-isolation.md`, `docs/app-manager.md`, and `docs/system-recovery.md`. For PC work also read `docs/bifrost-pc-workflow.md` and `docs/build-performance.md`.
- Paths under `springboard/` are relative to `springboard/`. Its docs are `springboard/docs/*.md`; the repo-root `docs/` is a separate tree.
- The Paperclip routine is paused. Work as one agent. Follow current repository instructions for checks and release routes.
- This task is a backlog specification. Do not erase machines, flash partitions, interrupt a physical session, or play sound. Device work follows the current explicit authorization. Record hardware-dependent acceptance separately when it cannot be performed safely.
- The working tree contains substantial pre-existing uncommitted work, including the PC desktop app ports owned by another agent. Do not reset, overwrite, stage indiscriminately, or absorb others' changes. Coordinate through `HANDOFF.md` and the ownership list in 002.
- PC presentation follows the user's macOS-style design. Use SF UI, named system colors, established list/navigation components, blurred materials and appropriate motion.
- Generic Linux services stay independent of presentation. Emulator success is not physical hardware acceptance.
- Compatibility: installed `.aap` apps built against SDK v1 must keep working until a documented removal, unless a milestone says so.

## Build and release routes

- PC iteration: optimized incremental `--profile fast --bin shell`, lld, Cargo timings. Full LTO is opt-in.
- Desktop changes: focused tests and previews.
- Full acceptance (`cargo test --workspace` plus previews) runs once per integrated batch before publication. Baseline reference: 2,799 tests passed, 4 ignored (`docs/codebase-cleanup.md`).
- PC shell: `bifrost pc publish` and Settings > Software Update. Apps: App Store packages, each with a CHANGELOG entry. No ISO rebuild for ordinary changes.
- Do not install an artifact for the wrong architecture or restart an in-use physical device without approval.

## Open decisions (record the answer in the milestone file before implementing)

1. Whether legacy plain-HTTP OTA stays supported (affects 011 and the HMAC key handling in 002).
2. Ownership of dirty `springboard/apps/*-desktop` paths (owned by the PC ports agent per `HANDOFF.md`).
3. Whether the items in `docs/codebase-cleanup.md` "Further structural work" (Settings split, HANDOFF shortening) are absorbed by 004 or handled separately.

## Final deliverables

Maintain `springboard/docs/platform-maturity.md` as the implementation record: architecture boundaries, API contracts, migration notes, compatibility matrix, measurements, unresolved hardware checks. Create it in 002 (Phase 0). Update the relevant SDK/UI/isolation/resource/recovery docs alongside each change. Keep `HANDOFF.md` focused on current state and next work.

Each milestone ends with an exit report: implemented behavior, checks run, compatibility impact, limitations, and the exact unfinished item if it is not complete. Mark the overall task done only when every milestone's acceptance is met.
