# Drag and drop: spring-loading, paste control, text drag and drop

Status: built on the `parity/team-p-drag-drop` branch; not yet verified on the tablet. The rows in
`docs/parity-matrix/ipados/data/uikit-drag-drop.json` stay `partial` until a tablet run confirms the behaviour.

Reference: the UIKit drag and drop topics (`uikit/uispringloadedinteraction`, `uikit/uipastecontrol`,
`uikit/uitextdragdelegate`, `uikit/uitextdropdelegate`), read with `python3 -I tools/parity/docc.py`.

## What was built

### Spring-loaded interactions

- `springboard/ui/src/spring_load.rs` (`bifrost_ui::spring_load`): the model. A drag that rests over a target for
  `Config::delay_ms` (default 700 ms) activates it once. Moving to another target restarts the timer; leaving the target
  cancels the hover; a target that refuses spring loading (the behavior's `shouldAllow` answer) never activates.
  `progress` gives a 0 to 1 ramp for the hover effect. Time is supplied by the caller in milliseconds.
- `springboard/sdk/src/spring_loaded.rs` (`bifrost_sdk::spring_loaded`): `SpringLoadedInteraction`, the app API. It is
  built with a handler (`init(activationHandler:)`) that runs with the activated target's id.

### Paste control

- `springboard/ui/src/paste_control.rs` (`bifrost_ui::paste_control`): `Configuration` (display mode icon and label,
  icon only or label only; the label; the accepted UTTypes) and `is_enabled`, true only while the pasteboard holds an item
  conforming to an accepted type. `Tap` is the proof that the person pressed the control.
- `springboard/sdk/src/paste_control.rs` (`bifrost_sdk::paste_control`): `is_enabled` and `paste`. `paste` reads only the
  items of the accepted types, and it takes a `Tap`, so the read follows a press. No access prompt is shown, because the
  press is the consent.

### Text drag and drop

- `springboard/ui/src/text_drag.rs` (`bifrost_ui::text_drag`): `Policy` (draggable, accepts drops, allows move on the same
  view), `drag_payload` (the selected text as `public.utf8-plain-text`), `propose` (the operation and insert point for a
  drop, with a move within the same view refused onto the dragged selection) and `apply_drop` (the resulting text and the
  caret). Byte indexes are snapped to character boundaries.

## How an app uses it

Spring-loading. Handle the drag updates of a destination and feed the target under the pointer:

```rust
let mut spring = bifrost_sdk::spring_loaded::SpringLoadedInteraction::with_default(|folder_id| open_folder(folder_id));
// on Dest Entered / Updated, with the element under the pointer and the behavior's answer for it
spring.drag_updated(hovered_folder, folder_accepts_spring, now_ms);
// from the frame timer while a drag runs, so a still pointer activates
spring.tick(now_ms);
// hover effect: spring.progress(now_ms) in 0.0..=1.0
// on Dest Exited or Ended
spring.finish();
```

Paste control. Create the configuration, enable the button from `is_enabled`, and paste from the tap:

```rust
let config = bifrost_ui::paste_control::Configuration::accepting(&[bifrost_sdk::pasteboard::STRING]);
let enabled = bifrost_sdk::paste_control::is_enabled(&NativePasteboard::general(), &config);
// on the press of the paste button
let items = bifrost_sdk::paste_control::paste(&NativePasteboard::general(), &config, tap);
```

Text drag and drop. A text view builds a `Policy` from its delegate-style settings and calls the functions from its drag
handlers:

```rust
let policy = bifrost_ui::text_drag::Policy::default();
let payload = text_drag::drag_payload(&policy, view.text(), view.editor.buf.selection());   // begin a drag
let proposal = text_drag::propose(&policy, view.text(), source_range, pointer_byte, allowed_mask); // Updated
let result = text_drag::apply_drop(view.text(), source_range, insert_at, &payload, operation); // Perform
```

## Gaps and integration notes

- The shell does not yet send spring-load ticks or an activation event to apps. Until it does, the app must call `tick`
  from its own frame timer.
- The hover effect is not drawn by a shell widget; the app draws it from `progress`.
- The paste control is a model, not a shell widget. The system look and placement are not built.
- `springboard/ui/src/textview.rs` does not call the text drag functions yet; a text view needs a `Policy` field and a
  drag start in its pointer handling.
- Operation bits: `drag_session::op` (`COPY` 1, `LINK` 2, `MOVE` 16) differs from the wire encoding
  (`proto::rich::dragop`, `MOVE` 4). The caller converts at the shell boundary.
- Attaching drag and drop interactions to views, touch lift on long press, drop proposals with progress and Split View
  pane targets are still missing (see the gaps of the parity row).
