# G1: SDK direct path, input and frame (`sdk-wl-input-frame`)

Scope: `springboard/sdk/src/wl/input.rs`, `springboard/sdk/src/wl/frame.rs` and the `presented()` extension of
`springboard/sdk/src/display_link.rs`. Contract: `docs/graphics-architecture.md`, "SDK" and "Canvas, DisplayList".

## What is implemented

### `wl/input.rs`: `wl_seat` pointer and keyboard

- `Seat` is the pointer and keyboard state of one seat. Its methods are pure (no socket, no clock except the event
  time), so the state machine is host-tested.
- Pointer to the SDK's `Input` events, the same as the touch path the shell already uses:
  - left button down, drag, up: `Input::Touch` with `TouchKind::Down`, `Move`, `Up` (id 0);
  - leaving the window during a drag: `TouchKind::Cancel`;
  - right button press: `Input::Context { x, y }`;
  - hover (motion with no button): no event, the position is kept (`pointer_position`).
- Scrolling has no `Input` variant, so it goes out as `ScrollEvent` (`take_scrolls`):
  - `Delta` for continuous scroll in surface pixels (`wl_pointer.axis`);
  - `Detent` for wheel steps from `axis_value120` (120 units per notch, `steps = value120 / 120`);
  - `Stop` from `wl_pointer.axis_stop`, the start of kinetic scrolling (the app feeds its `bifrost_ui::scroll::Scroller`).
- Keyboard: `wl_keyboard.keymap` is read to its end (read-only, at most 1 MiB) and kept as text
  (`keymap_text`); `enter`/`leave`; `key` (evdev codes) and `modifiers` (xkb bit order: Shift 1, Lock 2, Control 4,
  Mod1 = Alt 8, Mod4 = Super 64); `repeat_info`.
- Key codes use the shell's app protocol (`src/shell/apphost.rs`): printable keys are `KeyInput::Text`; Backspace 8,
  Tab 9, Enter 13, Escape 1, Home 36, Left 37, Up 38, Right 39, Down 40, End 35, PageUp 33, PageDown 34, Delete 46.
  Ctrl, Alt or Meta with a printable key is `KeyInput::chord(evdev, CHORD_*)`, the same chord Undo uses.
- Key repeat: the compositor sends no repeat events, so the seat keeps the held key. `next_repeat_at()` says when the
  event loop should wake; `repeat_due(now)` returns one repeat per call, at the compositor's rate after its delay.
  Only text and editing keys repeat; chords and Escape do not. Losing keyboard focus stops it.
- Dispatch glue: `impl Dispatch<WlPointer, (), D>` and `impl Dispatch<WlKeyboard, (), D>` for `Seat`, where
  `D: SeatHandler`. The connection's state implements `SeatHandler` (`fn seat(&mut self) -> &mut Seat`) and delegates
  with `wayland_client::delegate_dispatch!`. `bind_devices(seat, capabilities, qh)` asks for the pointer and keyboard the
  seat advertises.
- Event time follows the shell's rule (`event_time`): the event's age is taken from the monotonic clock in ms.

### `wl/frame.rs`: one frame of the direct path

- `FrameLoop::draw_frame(sink, draw)`:
  1. the app's drawing goes into a CPU `Surface` (`App::draw` still takes a `Surface`);
  2. if the app reports no damage (and the root exists), the frame is skipped;
  3. the surface becomes one `Image` op of a window-sized `DisplayList` (the list keeps its own copy);
  4. `sink.acquire` gives a free backing store and its contents id; `bifrost_2d::Renderer::draw` renders the list into it;
  5. `frame_transaction` builds one `bifrost_layers::Transaction` (the first frame creates and roots the layer, a resize
     sets its bounds, every frame binds the store); `sink.commit` binds the store, waits on the submission and commits.
- `FrameSink` is the seam to the connection and the backing-store pool (owners `sdk-wl-connection` and
  `sdk-wl-backing`), so this module needs neither.
- `Pacer` wraps `DisplayLink`: `next_frame(host_time)` reports a refresh once and skips it while `MAX_IN_FLIGHT` (2)
  frames are not yet shown. `presented(host_time, refresh_ns)` decrements the in-flight count and moves the clock's
  phase (below).
- `presented_host_time(hi, lo, nsec)` converts the layer protocol's `presented` event (seconds in two 32-bit halves,
  nanoseconds) to host nanoseconds.

### `display_link.rs`: `DisplayLink::presented(host_time, refresh_ns)`

- Re-anchors the refresh grid so a refresh begins at the time a frame was shown. Ticks after that report the moved
  grid. `refresh_ns` (when non-zero) replaces the period. A stopped link stays stopped (only the period changes).
- Indices already reported are kept, so no refresh is reported twice. Existing API and tests are unchanged; five tests
  added (grid moves, grid aligned, period taken, stopped link, no underflow).

## Contract changes

None to the public signatures of the skeleton. The skeleton's `input.rs` and `frame.rs` had no API (doc comments
only), so the API above is new. It depends on the other owners' names through two seams, not through their files:

- `FrameSink` (in `frame.rs`): `acquire(w, h) -> (ContentsId, Texture)` and `commit(contents, texture, &Submission, &Transaction)`.
- `SeatHandler` (in `input.rs`): the connection's state provides the `Seat`.

## Verification

- `cd springboard && CARGO_TARGET_DIR=/home/olibuijr/Projects/bifrost-os/springboard/target-agents cargo check --offline -j 2 -p bifrost-app --tests`
  passes with no warnings from `sdk/src/wl/` or `display_link.rs`.
- `python3 tools/check-names.py` reports no hits.
- Tests are written with hand-worked expected values (input: key table, modifier bits, chord encoding, touch
  sequences, scroll, repeat timing, keymap read limit; frame: transaction shape, pacer in-flight skip, presentation
  realignment; display link: grid moves). They were not run by this agent (the rules allow check only); run them with
  `cargo test -p bifrost-app --lib wl::` and `cargo test -p bifrost-app --lib display_link` on the lead's build host.
- On Midget (Intel HD 530, Mesa ANV, Vulkan 1.4): once the connection and backing owners land, run an SDK app with
  `BIFROST_DIRECT=1`, click and drag a control with `bifrost pc midget tap-window APP X Y`, type into a field, hold a key
  and check the repeat, scroll a list with the two-finger gesture and check the kinetic `Stop`, and read the frame
  timings from the bench (`tools/bench.py --midget`). Until the window server's layer protocol exists, the frame path
  cannot be exercised on Midget.

## todo_left

- Keyboard layout: printable keys use a built-in US table. The keymap text is kept (`keymap_text`); the xkb pass (a
  keysym and text lookup from the compositor's keymap, with dead keys and the Icelandic layout) is not done. Adding
  `xkbcommon` needs a Cargo.toml edit, which this change does not make.
- Scroll events: `ScrollEvent` is outside `sdk::Input`. Adding a scroll variant to `runtime::Input` is a change of the
  SDK's public enum and is left to the lead.
- Frame: `App::draw` takes a `Surface`, so the whole surface is copied into the display list and redrawn every frame.
  Damage is not passed to `Renderer::draw` (it takes `None`). The Canvas-based `App::draw` (which records only the
  changed layer) comes with the layer-tree SDK views.
- Frame: the `presented` feedback is counted per frame with no sequence check; a dropped frame is not detected
  beyond the pacer's count.
- No GPU test in this crate yet (the `gpu-tests` feature lives in `bifrost-gpu`). The frame's GPU path is covered by
  review and the Midget check above, not by a test.
