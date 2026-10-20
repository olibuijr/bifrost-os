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
| Screenshot chords and toolbar | Screenshot.app toolbar, Shift-Cmd-3 and 4 | `shell/hwkeys.rs` (`Shortcut::Capture`, `CaptureScreen`, `CaptureSelection`, `KEY_3`..`KEY_5`), `shell/capture.rs` (`Session::toolbar`, `bar_rect`, `tool_icon`) | Shift-Cmd-3 saves the whole screen at once; Shift-Cmd-4 opens the crosshair without a toolbar and Space turns it into a window pick; Shift-Cmd-5 opens the toolbar: icon buttons (Entire Screen, Window, Selection, Record Screen, Record Selection, Options, Close) with their names as tooltips, centred above the Dock (derived from the Dock frame, any screen size). Keyboard Shortcuts lists the three chords as one switch, "Screenshots" | Icon shapes against the reference; tooltip delay; the chords reach the shell with Terminal in front |
| Selection overlay for a region | `-i` selection mode | `shell/capture.rs` (`Drag`, `Session::press/moved/release`, `draw`) | Drag over the screen; the rest is dimmed; edge and `W × H` readout; a click under 4 pt takes nothing | Live dim and readout while dragging |
| Window pick with shadow | `-w`, `-o` turns the shadow off | `shell/capture.rs` (`window_at`, `with_shadow`, `window_image`), `shell/capture_flow.rs` (`finish_grab`) | The frontmost desk window under the pointer, cut out of the composed screen on a transparent ground with its drop shadow and rounded corners, saved as an RGBA PNG. Gap: the cut-out is of the composed screen, so another window in front of part of it shows in the picture (the reference takes the window's own buffer) | Window highlight while hovering; shadow matches the reference |
| Timer | toolbar Options, `-T` | `shell/capture.rs` (`Countdown`, `next_timer`), `shell/mod.rs` (`capture_touch`, `finish_capture`, tick, `next_wake`) | 5 s and 10 s countdown with the numeral drawn on screen, then the capture; the screen ignores touches while it runs | Numeral size; countdown wakes once a second |
| Show pointer | `-C` (cursor) and the toolbar's options | `shell/capture.rs` (`draw_pointer`), `shell/mod.rs` (`take_capture`) | With Pointer on, a simple arrow is drawn into the picture at the last pointer position | The arrow is a placeholder glyph, not the desk cursor image |
| Save location | Desktop by default | `shell/capture_flow.rs` (`capture_dir`, `save_capture`), `shell/capture.rs` (`SaveTo`), `shell/dock_stack.rs` (`pick_home`) | `~/Desktop` of the session account (installed account first, then `$HOME`, never the filesystem root); Options > Save To cycles Desktop, Documents, Pictures; the file is chowned to the account when the shell runs as root; the folder is shared with recordings | Confirm the file on the Desktop and its owner |
| Floating thumbnail: drag and mark up | thumbnail corner, click to mark up | `shell/screenshot.rs` (`Overlay`, `bounds`, `draw_at`), `shell/capture_flow.rs` (`shot_layer`), `shell/desktop_bar.rs` (`desktop_chrome_frames`, id 9) | Bottom right; an own overlay surface above the windows, removed from the screen when it has slid away (about 5 s); a drag of more than 6 pt moves the thumbnail, a click opens Markup. Gap: dragging the file out into another app is not built | Nothing left on screen afterwards; drag feel |
| `screencapture -i` | selection mode | `bare/control.rs` (`Request::Capture`), `bin/shell.rs`, `cmd/src/screencapture.rs` | Sends `capture select`; the shell opens the session and replies `ok` | End to end on Midget; the CLI returns before the capture is taken |
| `screencapture -w` | window mode | same | Sends `capture window` | Same as above |
| Record Screen and Record Selection | toolbar Record buttons, `screencapture -v -V` | `shell/screenrec.rs` (`recording_begin`, `plan_source`, `Session::start_composed`), `shell/screenrec_composed.rs` (pacing, cropping, encoder feed, owner), `shell/screenrec_wl.rs` (screencopy client), `shell/desktop_bar.rs` (`Menu::Recording`), `shell/desk.rs` (`desk_bar_hidden`) | Under the window server a worker thread reads the composed output (wallpaper, windows, menu bar, Dock) through `wlr-screencopy` into a pool of reused shm buffers, cropped to the selection (any output size and scale), at the display rate up to 30 frames a second; ffmpeg reads the buffers as they are (`bgr0`), the built-in writer is the fallback encoder. The shell's frame loop only looks at the worker every 100 ms. Frames are dropped, never waited for, when no buffer is free, the encoder is behind or a deadline passed; the dropped count is logged when the recording ends and shown by `rec status`; the movie keeps real time by repeating the frame that ends a gap. The red recording indicator stays in the menu bar for the whole recording (the bar does not hide over a full-screen window while recording). The recording ends when the screen locks. The file goes to the folder pictures go to (Desktop of the account, `pick_home`) and belongs to the account (ffmpeg runs as the account, or the file is chowned), not to root. If the composed output cannot be read (no screencopy global, the Vulkan renderer, a refused frame) nothing starts and a notification says so; the shell's own frame is never recorded under the window server. Without a window server (tablet, test fixtures) the shell's own frames are recorded as before | The movie shows wallpaper and windows; the indicator; the Vulkan renderer refuses with a message; owner of the file on the Desktop; dropped count under load |
| `screencapture -v` (video) | `-v -V -G -g -k` | `cmd/src/screencapture.rs`, `bare/control.rs` (`rec start`) | `-v` and `-V` start and stop the same recording through the control port (`-G -g -k` still refused) | Same checks as the Record buttons |

## Migrations and deletions

- E2E 2026-10-20 (SC1 to SC4): under the window server the shell's own frame is not the screen, so on the desktop a capture
  takes its overlays off the screen, waits `SETTLE_MS`, asks `bifrost-apps --screenshot` (the grabber the lab tooling uses)
  for the composed output and crops it (`shell/capture_flow.rs`). Without the window server (tablet, test fixtures) the
  picture is still the shell's own frame. The capture session and the thumbnail are chrome surfaces 8 and 9. The
  countdown now runs on the shell's millisecond clock (it counted milliseconds as seconds).

- Recording (2026-10-20): `Shell::recording_start` took the shell's own frames, which under the window server hold only the
  menu bar and the Dock on black. It now records the composed output through a screencopy client (`shell/screenrec_wl.rs`,
  own thread and connection) and keeps the shell-frame path only for a shell without a window server. `ffmpeg_args` takes the
  input pixel format (`rgba` for shell frames, `bgr0` for screencopy buffers). `Session::start` is unchanged for
  `services/screen_stream.rs`; `start_owned` and `start_composed` are new. `recording_dir` is `capture_dir` (one folder for
  pictures and recordings).
- `take_screenshot` is now `take_capture(region, pointer)` plus `save_capture`. The chord, the toolbar and the control port
  share one save path. The old body moved into `save_capture` unchanged.
- The thumbnail's tap and drag share `shot_touch`. A drag is no longer treated as a tap.
- `screencapture` no longer refuses `-i` and `-w`. The usage string changed.

## Gaps

- **Recording gaps.** The window server answers screencopy with shm buffers only (no dma-buf) and treats
  `copy_with_damage` as a plain copy, so each frame makes the window server draw the scene again and read it back; the
  recorder is therefore capped at 30 frames a second (`COMPOSED_MAX_FPS`). A damage-driven, dma-buf capture needs the window
  server's screencopy to be extended first. The pointer is drawn only when Options > Show Pointer is on and the DRM backend
  paints it. System audio is not recorded (microphone only, as before). Any local client that can reach the window server
  socket can still bind screencopy: the window server does not yet limit the global to the session's account. The control
  port's `rec start` takes a caller-named file, written as the account.
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
`cmd/src/screencapture.rs` (options, interactive requests), and for recording in `shell/screenrec_composed.rs` (timeline
pacing, deadlines, crop at any output size and scale, pixel conversion, fallback decision and messages, file owner and
folders, the encoder over a gap) and `shell/tests.rs` (the indicator and the menu bar over a full-screen window). None were
run, per the round rules.

## Matrix entries (for the lead; not edited here)

- `os/apps` `path:/System/Applications/Utilities/Screenshot.app`: partial (toolbar, selection, window, screen, timer, pointer,
  thumbnail drag, composed-screen recording).
- `os/cli` `path:/usr/sbin/screencapture`: partial (`-i`, `-w`, `-c`, `-R`, `-T`, `-x`). `-v` and `-V` record the composed screen; the other options missing.
- `data/screencapturekit.json` single-frame screenshot entry: the toolbar is app-facing only through the shell. No SDK
  call takes a capture yet.
