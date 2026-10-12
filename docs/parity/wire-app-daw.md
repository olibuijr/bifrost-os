# Wire: DAW app (MIDI and device parameters), 0.2.6

Branch `wire/app-daw`. Scope: `springboard/apps/daw/` only (plus this note). Models reused from
`springboard/ui/src` through `bifrost_app::controls` (the `bifrost-ui` crate): `midi_events`, `plugin_host`.

## What is now wired

| Matrix slug | Feature (data row) | BifrOSt path | Status after this round |
|---|---|---|---|
| core-midi | MIDI Messages (MIDI 1.0 and 2.0 / UMP construction) | `springboard/apps/daw/src/midi.rs` (`Recorder`, `schedule`) over `springboard/ui/src/midi_events.rs` | MIDI 1.0 channel messages in and out. No UMP, no MIDI 2.0. Stays Partial. |
| audio-unit | MIDI input and note data | `springboard/apps/daw/src/midi.rs` (`Recorder::bytes`, `Recorder::event`) over `springboard/ui/src/midi_events.rs` | Note input from MIDI bytes or events. No device input yet. Stays Partial. |
| audio-unit | Parameters and presets | `springboard/apps/daw/src/params.rs` (`tree_for`, `params_from_tree`, `save_preset`, `apply_preset`, `presets_to_json`, `presets_from_json`) over `springboard/ui/src/plugin_host.rs` | Every instrument and effect spec is a parameter tree. Presets save, apply and round-trip through JSON. Stays Partial. |
| (none) | MIDI clock pulses | `springboard/apps/daw/src/midi.rs` (`clock_pulses`, `CLOCK`, `START`, `CONTINUE`, `STOP`) | No matching matrix row. A BifrOSt addition for sync, not sent to a device. |

Coverage notes: the `ParameterTree` clamps writes, `PresetStore` numbers user presets, and `params_from_tree` hands
the tree's values to the device engine (`devices::Params`). Units map to the plug-in unit model (Hz, dB, %, s).

## How a user reaches it

Honest status: this round wires the logic into the app's library (`daw::midi`, `daw::params`). **No window control
yet calls it.** The app's Rack view (`src/main.rs`, `draw_rack` and `Hit::RackParam`) is the place for a preset menu
and a MIDI-input toggle; that UI work is a gap below. A developer reaches the code through the `daw` library, and the
test file exercises every public function.

## How to verify (test PC)

Host verification only; the lead builds once.

- `cargo test -p bifrost-daw --test parity_wire` (in `springboard/`): 13 tests covering recording, sustain, running
  status, scheduling order, round-trip through the MIDI parser, clock pulses, spec-to-tree conversion, clamping, the
  engine hand-off, presets and JSON.
- Midget: there is no visible entry point, so nothing can be checked on Midget yet. After the Rack preset menu lands,
  verify with `bifrost pc midget ota` until `phase=accepted`, open DAW, go to Rack, and save and re-apply a preset.

## REA evidence

- `ev_5bff97e822468901403a53b7478768b29f8d96cbf96d122f485defa616797a50`: MIDI Messages page (rendered page).
- `ev_1a2776cda9dd233a4abe8d4693e2218f35bb01de48da9429fda90bc9ee10a112`: MIDI Services page.
- `ev_315686835938dbbe8ca013d12fc6e37afccb32e773af182314378b3db395b98f`: Audio Unit v3 plug-ins page.
- Official DocC fetched with `python3 -I tools/parity/docc.py coremidi/midi-messages`, `coremidi/midi-services` and
  `audiotoolbox/audio-unit-v3-plug-ins`.

## Remaining gaps

- **No UI entry point.** Rack preset menu, MIDI input toggle and clock display are not drawn or hit-tested yet.
- **MIDI endpoints and clients** (`springboard/core/src/midi_endpoints.rs`) are not used: `bifrost-core` is not
  reachable from the app crate, and no device is opened. Input sources and destinations are a gap.
- **Recorder input.** The shell does not deliver MIDI events to the DAW; `Recorder::bytes` and `Recorder::event` are
  ready for when it does.
- **Channels.** The note model has no MIDI channel, so recorded notes from different channels share a track.
- **Clock sync.** `clock_pulses` is not tied to the playback clock (`springboard/ui/src/media_timebase.rs`).
- **Persistence.** Presets are JSON through `presets_to_json`; they are not yet stored in the song file.
- **Controllers.** Only the sustain pedal is recorded; pitch bend, mod wheel and other controllers are ignored.
- **MIDI 2.0 / UMP** encoding and Standard MIDI File import are not implemented.
- **Version.** `springboard/apps/daw/Cargo.toml` and `manifest.toml` are at 0.2.6. `Cargo.lock` still records 0.2.5
  until the lead's build updates it.
