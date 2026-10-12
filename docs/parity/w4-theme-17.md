# Theme 17: per-session input controller (wave 4)

Closes nine gap rows across three frameworks (InputMethodKit, Game Controller, Core Text) with three modules.
All code is pure Rust, std only, with host unit tests. Nothing is wired into the shell or apps yet, and nothing has
run on the VM or Midget, so every row closes at `partial`.

## What was built

### `springboard/ui/src/input_method.rs` (new)

The input method side of text input: the server, one controller per client input session, the composition the
controller owns, and event routing.

- `Registration`: the record an input method ships with (controller class, optional delegate class, ordered input
  modes). `validate()` refuses a blank controller, no modes, blank or duplicate mode ids.
- `Server`: checks the registration once in `new`, then holds one `InputController` per client id (`connect`,
  `disconnect`, `controller`, `clients`, `shutdown`). A second connection for the same client is refused.
- `InputController`: per-session `State` (active flag and mode; `set_mode` accepts only registered modes and commits
  a pending composition first), the `Composition` (original keystrokes, composed text, caret, client offset of the
  marked text), and a `Candidates` list reused from `candidates.rs`.
- `Event` (key with `KeyCombo` and text, command as `EditCommand`, pointer with client byte offset) and `Outcome`
  (`Consumed`, `PassThrough`, `Committed(text)`, `Cancelled(original)`) describe the routing contract with the client.
  Return commits, Escape cancels, digit keys pick a visible candidate, Up and Down move the candidate selection,
  Delete takes back one keystroke, a click outside the marked text commits it, and other commands commit it first.

The conversion engine is not part of the model. `set_composition` takes its output. Without an engine, typed text
during a composition is shown as typed (identity conversion).

### `springboard/ui/src/gamepad.rs` (extended)

- `ControllerInput`: wraps the existing `Gamepad` state. `feed` runs every registered callback (`on_change`) and
  queues the change. `poll` returns queued changes in order, `pending` counts them, and `capture` copies the state
  as a snapshot. The queue holds `QUEUE_LIMIT` (256) changes and drops the oldest first.

### `springboard/ui/src/vertical_text.rs` (new)

Vertical text layout for one column, pure geometry with the glyph advances supplied by the caller:

- `Forms`: `Vertical` (upright, one em), `Horizontal` (turned a quarter turn, advance tall), `HorizontalInVertical`
  (a run of such glyphs sits centred in one em box when it fits, and falls back to sideways when it does not).
- `layout_column` returns the boxes in input order and the column length. `column_lefts` lays out columns right to
  left.

## Official API names mirrored

Apple documentation, read with `tools/parity/docc.py` and rendered with REA:

- InputMethodKit: `IMKInputController` (per-session controller, composition, candidates, cancel and commit),
  `IMKServer`, `IMKServerInput` (inputText, didCommand, handle, commitComposition, composedString, originalString,
  candidates), `IMKMouseHandling`, `IMKStateSetting`, the Info dictionary keys (controller class, delegate class,
  mode dictionary).
  - https://developer.apple.com/documentation/inputmethodkit
  - https://developer.apple.com/documentation/inputmethodkit/imkinputcontroller
  - https://developer.apple.com/documentation/inputmethodkit/imkserverinput
- Game Controller: `GCController` (controllers and input profile), `GCPhysicalInputProfile` (`valueDidChangeHandler`,
  `lastEventTimestamp`, `capture()`), `GCControllerDidConnect` (connection notifications). The new `ControllerInput`
  covers the polling and callback half; discovery is in `core/src/gamepad.rs`.
  - https://developer.apple.com/documentation/gamecontroller
  - https://developer.apple.com/documentation/gamecontroller/gccontroller
  - https://developer.apple.com/documentation/gamecontroller/gcphysicalinputprofile
- Core Text: `kCTVerticalFormsAttributeName` (glyph orientation) and `kCTHorizontalInVerticalFormsAttributeName`
  (tate-chu-yoko, horizontal numerals in vertical text).
  - https://developer.apple.com/documentation/coretext/kctverticalformsattributename
  - https://developer.apple.com/documentation/coretext/kcthorizontalinverticalformsattributename

## REA evidence

- InputMethodKit landing page: `ev_8f7e3a70d66c907432bd1c9ef32e27e1ed83044625198b72c3c5a70223ef47e9`
- Game Controller landing page: `ev_fc4c77604094c917fd4b7ac98d13aef7ab250dc34eb541a2859cba772f1b17c4`
- kCTVerticalFormsAttributeName page: `ev_41b9a98dbca488b68664ec94642af0c2ab29b45cfbfe781b3720ad8b16776685`

## Matrix rows closed

Each status is `partial`. Paths are the BifrOSt code that now covers the row.

| Framework slug | Feature (as in data/<slug>.json) | Status | BifrOSt path |
|---|---|---|---|
| inputmethodkit | Per-session input controller | partial | springboard/ui/src/input_method.rs (InputController) |
| inputmethodkit | Input method state | partial | springboard/ui/src/input_method.rs (State, set_mode) |
| inputmethodkit | Input method registration and Info dictionary | partial | springboard/ui/src/input_method.rs (Registration, validate) |
| inputmethodkit | Mouse handling for input methods | partial | springboard/ui/src/input_method.rs (Event::Mouse, mouse) |
| inputmethodkit | Key binding and text events to the input method | partial | springboard/ui/src/input_method.rs (Event, InputController::handle) |
| inputmethodkit | Composed, original string and candidate queries | partial | springboard/ui/src/input_method.rs (composed_string, original_string, candidates) |
| inputmethodkit | Input method server and connections | partial | springboard/ui/src/input_method.rs (Server) |
| game-controller | Input events (polling and callbacks) | partial | springboard/ui/src/gamepad.rs (ControllerInput) |
| core-text | Vertical text and horizontal-in-vertical | partial | springboard/ui/src/vertical_text.rs (layout_column, column_lefts) |

## Gaps that remain

- No compositor protocol: the shell does not serve an input method server, and no client can connect to `Server`.
  `Registration` is built in code; there is no info-record file reader.
- No conversion engine and no candidate panel drawing. `candidates.rs` still draws nothing.
- Mouse events are only routed in the model. No pointer hook from the shell reaches `InputController::handle`.
- `ControllerInput` is not fed by the shell's evdev decoder (`springboard/src/bare/input.rs`), so apps get no
  controller events yet.
- Vertical layout takes advances from the caller. It has no glyph lookup for vertical forms, no line breaking in
  vertical text, and no drawing.

## Tests

- `input_method.rs`: 9 tests (registration validation, server connections, inactive pass-through, return and escape,
  typing during a composition, candidate digit and arrow selection, mouse caret and commit, command handling and mode switch).
- `gamepad.rs`: 5 new tests (callback order, polling order, dead-zone silence, snapshot, queue limit).
- `vertical_text.rs`: 7 tests (upright stacking, sideways glyph, centred group, wide group fallback, mixed indices,
  empty and bad sizes, column order).

Expected values were worked out by hand from the rules above. Nothing was built or run, as the task requires.
