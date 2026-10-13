# Screenshot tool (round 75, slug `screenshot-tool`)

The screenshot toolbar (Shift-Cmd-5), its capture modes, and `screencapture -i -w`, which uses the same overlay.
Written by a code-only agent: nothing was built or run. The lead builds and checks on Midget.

## Evidence used

- Reference man page, read with `man -l`: `~/Projects/macos-ref/extract/ref27/usr/share/man/man1/screencapture.1`
  (options `-i -w -W -o -S -T -J -U -V -C -k`; `-i` cancels on Escape, Space toggles selection and window, Control sends
  to the clipboard).
- The reference Screenshot.app bundle (`System/Applications/Utilities/Screenshot.app`) is listed in `ref27-index.tsv` but
  is not extracted under `ref27/` yet. Its Info.plist, `en.lproj` labels and the ScreenshotControls extension are NOT
  measured. Re-check after the lead's extraction; toolbar labels in the code are placeholders until then.
- Existing code: `springboard/src/shell/screenshot.rs` (capture animation, thumbnail), `springboard/src/shell/hwkeys.rs`
  (shortcuts), `springboard/cmd/src/screencapture.rs` (CLI over the control port).

## Rows

| Feature | Reference | BifrOSt path | Done in code | Needs Midget check |
|---|---|---|---|---|
| Shift-Cmd-5 toolbar | Screenshot.app toolbar | `shell/hwkeys.rs` (`Shortcut::Capture`, `KEY_5`), `shell/capture.rs` (`Session::toolbar`) | Chord opens the session; Escape closes; toolbar buttons: Selection, Window, Screen, Timer (Off, 5 s, 10 s), Pointer, Cancel. Keyboard Shortcuts lists it as "Screenshot Toolbar" and can switch it off | Label size and position against the bar; the chord reaches the shell with Terminal in front |
| Selection overlay for a region | `-i` selection mode | `shell/capture.rs` (`Drag`, `Session::press/moved/release`, `draw`) | Drag over the screen; the rest is dimmed; edge and `W × H` readout; a click under 4 pt takes nothing | Live dim and readout while dragging |
| Window pick with shadow | `-w`, `-o` turns the shadow off | `shell/capture.rs` (`window_at`, `with_shadow`), `shell/mod.rs` (`capture_frames`) | The frontmost desk window under the pointer, with 24 pt of shadow clipped to the screen. Minimized windows are skipped | Window highlight while hovering; shadow matches what the desk draws |
| Timer | toolbar Options, `-T` | `shell/capture.rs` (`Countdown`, `next_timer`), `shell/mod.rs` (`capture_touch`, `finish_capture`, tick, `next_wake`) | 5 s and 10 s countdown with the numeral drawn on screen, then the capture; the screen ignores touches while it runs | Numeral size; countdown wakes once a second |
| Show pointer | `-C` (cursor) and the toolbar's options | `shell/capture.rs` (`draw_pointer`), `shell/mod.rs` (`take_capture`) | With Pointer on, a simple arrow is drawn into the picture at the last pointer position | The arrow is a placeholder glyph, not the desk cursor image |
| Save location | Desktop by default | `shell/mod.rs` (`save_capture`) | Photos directory as the existing screenshot path (unchanged) | Confirm Photos shows the capture |
| Floating thumbnail: drag and mark up | thumbnail corner, click to mark up | `shell/screenshot.rs` (`Overlay::grab`, `drag_to`, `release`, `thumb_rect`), `shell/mod.rs` (`shot_touch`) | A drag of more than 6 pt moves the thumbnail and keeps it on screen; a tap still opens Markup | Drag feel; the thumbnail stays where it was dropped during the hold |
| `screencapture -i` | selection mode | `bare/control.rs` (`Request::Capture`), `bin/shell.rs`, `cmd/src/screencapture.rs` | Sends `capture select`; the shell opens the session and replies `ok` | End to end on Midget; the CLI returns before the capture is taken |
| `screencapture -w` | window mode | same | Sends `capture window` | Same as above |
| `screencapture -v` (video) | `-v -V -G -g -k` | not built | Still refused with a message | Blocked: see Gaps |

## Migrations and deletions

- `take_screenshot` is now `take_capture(region, pointer)` plus `save_capture`. The chord, the toolbar and the control port
  share one save path. The old body moved into `save_capture` unchanged.
- The thumbnail's tap and drag share `shot_touch`. A drag is no longer treated as a tap.
- `screencapture` no longer refuses `-i` and `-w`. The usage string changed.

## Gaps

- **Recording (`-v`, `-V`, Shift-Cmd-5 Record) is not built.** The tree has no video encoder and no PipeWire or screen
  recording path: `springboard/src` has only still-image encoders (`pngenc`, `cg/imageio`). Recording needs a new
  pipeline (frame source, H.264 or similar encoder, MOV container) and is a separate task.
- `apps/screenshot-desktop` (`bifrost-screenshot`) is a window-less model crate with its own `Options`, `Target`, `Mode`
  and file naming. The shell cannot import it (crate graph), so it duplicates `shell/capture.rs`. The lead should decide
  whether to delete the app's model or make it the owner of the naming. Not changed here.
- `screencapture -o` (no shadow), `-S` (screen instead of window), `-W` (start in window mode), `-J` (start style) and the
  Control key (clipboard) are refused or missing. The toolbar does not yet take Space to toggle selection and window modes.
- `-i` and `-w` return before the capture is taken. The file lands in Photos, not at a path the caller names. The control
  port cannot reply later, and a caller-named path would let a local client write files as the user.
- `-T` on the command line is refused with `-i` and `-w`. The toolbar's own timer is the way to delay them.
- No text labels are checked against the reference strings (see the Evidence section).

## Shared-file edits the lead must merge with care

- `springboard/src/shell/mod.rs`: `pub mod capture`, the `capture` field and its init, routing in `event`, tick and
  `next_wake`, the draw call, the `take_screenshot` split, `capture_touch`, `finish_capture`, `capture_request`, and
  `shot_touch`. The `shot_touch` change is inside the screenshot section.
- `springboard/src/shell/hwkeys.rs`: `KEY_5`, `Shortcut::Capture` (variant, `out`, mapping, `hw_shortcut` arms), and a test.
- `springboard/src/shell/kb_input.rs`: `BINDINGS` grows from 10 to 11 entries (Screenshot Toolbar). The settings bit for it
  is bit 10.
- `springboard/src/bare/control.rs` and `springboard/src/bin/shell.rs`: the `capture` control request and its handler.

## Tests

Unit tests are in `shell/capture.rs`, `shell/screenshot.rs` (the thumbnail drag), `shell/hwkeys.rs` (the chord),
`shell/kb_input.rs` (bindings cover every switchable shortcut), `bare/control.rs` (parse), and
`cmd/src/screencapture.rs` (options, interactive requests). None were run, per the round rules.

## Matrix entries (for the lead; not edited here)

- `os/apps` `path:/System/Applications/Utilities/Screenshot.app`: partial (toolbar, selection, window, screen, timer, pointer,
  thumbnail drag). Recording missing.
- `os/cli` `path:/usr/sbin/screencapture`: partial (`-i`, `-w`, `-c`, `-R`, `-T`, `-x`). `-v` and the other options missing.
- `data/screencapturekit.json` single-frame screenshot entry: the toolbar is app-facing only through the shell. No SDK
  call takes a capture yet.
