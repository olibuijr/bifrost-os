# Task 17: process and system taps

Implemented in `springboard/audio/server/src/tap.rs`: `TapDescription` (processes or global-except, Source/Mono/Stereo
mixdown, Unmuted/Muted/MutedWhenTapped, private), `Tap` (preallocated cycle accumulator and ring; `accumulate`,
`finish_cycle`, `read_input` never allocate or lock), `TapRegistry` (create needs `audio-capture` else
`PermissionDenied`; private taps visible to the creator only; `process_exited` cleanup; `indicator_visible`;
`is_muted(pid)` for the mixer). `springboard/ui/src/process_tap.rs` (kit model) was already complete and is unchanged.

## Left
- Wire into the server: the mixer calls `accumulate_all(pid, frames)` per client then `finish_all()` in ProcessInput,
  and skips clients where `is_muted(pid)`; the private aggregate (task 11) reads `Tap::read_input`; the permission
  check supplies `has_capture`; the menu bar polls `indicator_visible`/`active_owners`; Logger category `tap`.

## Midget verification
1. Play a known WAV in Music; create a process tap (Unmuted) feeding a private aggregate; record the aggregate
   input to WAV; null-test against the source.
2. Create the tap with Muted; confirm Music's speaker output is silent while the recording still matches.
3. Without `audio-capture` creation must fail with PermissionDenied; the indicator shows only while a tap runs.
