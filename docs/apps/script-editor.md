# Script Editor

Script Editor (`springboard/apps/script-editor-desktop`, crate `bifrost-script-editor`, version 0.1.0) edits shell
scripts, runs them through the system shell and shows what they print. It is a full-size window (no title row) built
on the SDK.

## What it does

- **Editing.** A plain text editor for one script per document. Typing, Return (the new line keeps the indentation
  of the line it splits), Tab (inserts four spaces), Delete, Backspace, the arrow keys, Home and End. A click in the
  text puts the caret at the nearest character. Paste inserts the clipboard text. The line-number gutter and the
  caret's position (`Ln`, `Col`) are shown in the status bar. The editor scrolls to keep the caret in view.
- **Running.** Run (Script > Run, Command-R, or the Run button) starts the document's text in `/bin/sh -c` with the
  home directory as the working directory. The shell runs on a worker thread, so the window keeps drawing. Stop
  (Script > Stop, or the Stop button) sends SIGTERM to the shell.
- **Output.** The pane under the editor shows standard output, then standard error in red, and the newest lines when
  the output is longer than the pane. Output is kept to 256 KiB per stream; a longer stream is cut and marked
  `[output truncated]`. A run that could not start shows the reason in the pane.
- **Status.** The status bar shows `Running...` while a script works, then `Exited with status N`, `Stopped` after
  Stop, or the failure message.
- **Documents.** The File menu comes from the SDK document controller: New, Open, Open Recent, Close, Save, Save As,
  Revert, and the rest of the standard set. Scripts open and save as `public.shell-script`, and plain text files open
  as well. The controller keeps the recent scripts list (ten entries), so Open Recent lists the scripts used last.
  The window title shows the document name, with `(edited)` after an unsaved change.

## Layout

The window is one column: a 52 pt top band (traffic lights, Run and Stop, the document name), the editor (60 % of
the height above the status bar), a hairline, the output pane, and the status bar. Buttons use a 6 pt corner.

| File | Role |
|---|---|
| `src/buffer.rs` | The text as lines, the caret, and the editing operations. Pure, unit tested. |
| `src/runner.rs` | Runs a script in `/bin/sh` on a worker thread; output clipping, status text, output lines. Unit tested. |
| `src/document.rs` | `ScriptDocument`: reads and writes the script bytes for the document controller. Unit tested. |
| `src/view.rs` | Window geometry in points (buttons, panes, scrolling). Unit tested without a canvas. |
| `src/main.rs` | The `App`: input, menus, controls for accessibility, drawing with the SDK canvas. |

## Gaps

- No syntax colouring, search, replace, undo, or line wrapping: long lines scroll out of the pane's clip.
- Selection is not implemented: Cut and Copy are not offered, and the caret is the only cursor.
- Stop signals the shell only; a script's own child processes may keep the output open until they exit.
- No timeout: a script that never ends runs until Stop.
- The output pane shows standard output and standard error as two blocks, not interleaved in the order written.
- The working directory is always the home directory, not the document's folder.
- The icon is a generated placeholder (dark tile, green prompt).
