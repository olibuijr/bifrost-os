# Screen recording (parity round 75)

Slice: screen recording, the screenshot toolbar's Record buttons, the menu bar stop item and `screencapture -v` / `-V`.

## What is built

| Part | BifrOSt path |
|---|---|
| Frame source: the shell draws the screen as it does for a screenshot, at 10 frames a second, cropped to the area | `springboard/src/shell/screenrec.rs` (`Shell::recording_tick`, `Session::push`, `crop_pixels`) |
| Encoder choice: system `ffmpeg` on the PATH (raw RGBA on stdin, H.264 in `.mov`), else the built-in Motion JPEG AVI writer | `screenrec.rs` (`choose`, `find_in_path`, `ffmpeg_args`, `AviWriter`, `encode`) |
| Encoder thread with a bounded queue; a frame the encoder cannot take is skipped, not waited for | `screenrec.rs` (`Session::start`, `Session::finish`) |
| Screenshot toolbar: Record Selection and Record Screen buttons; the chosen region or screen starts a recording after the timer | `springboard/src/shell/capture.rs` (`Tool::Record`, `Session::record`), `shell/mod.rs` (`finish_capture`) |
| Control port: `rec start [screen or x,y,w,h] [file]`, `rec stop`, `rec status`; refused while the screen is locked | `springboard/src/bare/control.rs` (`Request::RecStart`, `parse_area`), `springboard/src/bin/shell.rs` |
| Menu bar: red stop item beside Control Center while recording; its menu has Stop Recording | `springboard/src/shell/desktop_bar.rs` (`Menu::Recording`, `Action::RecordStop`) |
| `screencapture -v` (until stopped) and `-V seconds` (timed), with `-R` and a file operand | `springboard/cmd/src/screencapture.rs` (`record`, `ask`) |

Default file: `~/Desktop/Screen Recording YYYY-MM-DD at HH.MM.SS.mov` (`.avi` when the built-in writer is used).

## Parity rows (ScreenCaptureKit)

- Stream screen content to an app: still Partial. The shell records to a file; no app receives a frame stream.
- Permission for screen recording: the toolbar, the control port and `screencapture` start recordings without the per-app
  screen recording permission (they are system actions). Apps still have no permission flow.
- Clip buffering and recording editor: not started.

## Gaps and decisions

- PipeWire screencast is not used. The shell's own frame is the source, so there is no portal frontend and no
  compositor-side capture for recording.
- gstreamer is not a second encoder. Without ffmpeg the built-in AVI writer is used (Motion JPEG, no audio).
- Audio: with ffmpeg and the Options menu's Microphone on, the audio service's take is muxed in (see `recording-polish.md`).
  `-g` and `-k` remain refused.
- Window recording (the toolbar's Window pick) is not offered as a recording target.
- `screencapture -v` without `-V` returns when the recording ends. Ctrl-C ends the client only; the recording continues
  until stopped from the menu bar or with `rec stop`.
- `springboard/apps/screenshot-desktop` (model only) still has its own `Mode::Recording` and `recording_name`. The shell's
  toolbar is the working path; the app model is a superseded copy to merge or delete in a later round.

## Evidence

- Reference: the macOS 27 `screencapture` flags (`-v`, `-V`, `-R`, `-T`, `-c`) as already listed in the cmd tool. No REA
  evidence was collected this round, and the Screenshot.app bundle was not inspected.
- Code only, not built or run on Midget (the lead builds). Unit tests: `screenrec.rs` (ffmpeg arguments, executable
  lookup, area rules, RGBA bytes, AVI layout and index, session frame timing), `capture.rs` (record buttons),
  `control.rs` (`rec` parsing), `cmd/src/screencapture.rs` (`-v`, `-V`).

## Still needs Midget checking

- Record Selection and Record Screen from Shift-Cmd-5; the stop item and its menu; a `.mov` that plays on the TV and on Midget.
- The built-in AVI writer on a machine without ffmpeg (VLC and mpv should open it).
- Frame rate under load: the shell draws each frame in full, so a busy screen may skip frames.
