# Automator

Automator is a workflow builder for BifrOSt PC. A workflow is a sequence of actions. Each action takes the output of
the step before it (a list of files, or text) and produces its own output for the next one. The run keeps a log, and
it stops at the first step that fails.

Crate: `springboard/apps/automator-desktop` (`bifrost-automator` 0.1.0, app id `is.olibuijr.automator.desktop`).
Window: full-size, with the Golden Gate toolbar band and three columns.

## Window

- **Library** (left): the actions, grouped as Files, Text, Images and Shell. A click adds the action to the end of the
  sequence and selects it.
- **Sequence** (middle): the steps in order, numbered, with their values under each name. Click a step to select it.
- **Editor** (right, top): the selected step's fields and a hint for the action. A choice field (for example
  Subfolders, or Operation) changes to its next choice on a click. A text field takes the focus on a click; typing
  goes to it, Backspace deletes, Tab or Enter moves to the next field, and Escape leaves it.
- **Log** (right, bottom): the lines of the last run. Errors are in red.
- **Toolbar**: New, Open, Save, Move Up, Move Down, Remove, Run. Edits are disabled while a run is in progress.

## Actions

| Action | Input | Fields | Output |
|---|---|---|---|
| Find Files | none | Folder, Name contains (text, or `*.ext`), Subfolders (yes or no) | the matching files, sorted |
| Rename | files | Name template: `{name}` (no extension), `{ext}`, `{n}` (position from 1) | the renamed files |
| Copy | files | Destination folder (created if missing) | the copies |
| Move | files | Destination folder | the moved files |
| Transform Text | text | Operation (upper, lower, trim, sort lines, unique lines, replace), Find, Replace with | the text |
| Combine Text | text | Separator (`\n` for a new line) | the joined text |
| Resize Images | image files | Width, Height (0 keeps the proportions) | PNG files named `name-WxH.png` beside the originals |
| Convert Images | image files | Format (png or bmp) | the converted files beside the originals |
| Run Shell Script | any | Script (run with `sh -c`) | the script's standard output as text |

A step that gets files also accepts text: each non-blank line is read as a path. So a Find Files step can feed
Rename after a script step prints a list of paths.

Rules that keep the files safe:

- Rename, Copy, Move, Resize and Convert never replace an existing file. A target that exists stops the run.
- Move falls back to copy and delete when a rename cannot cross to another disk.
- A Convert step leaves a picture alone when it is already in the chosen format.
- Find Files does not follow symbolic links into subfolders.

## Files

A workflow is saved as a `.automator` file: a JSON object with `format` (`automator-workflow`), `version` (1), `name`
and `steps` (each step has `action` and `values`). Save and Open use the shell's save and open panels. The workflow
takes the name of the file it was saved to. A file from a newer version is refused.

## Code

- `src/model.rs`: kinds, fields, steps, values, the file format, and `execute`, the runner.
- `src/actions.rs`: what each action does. File work and the shell step.
- `src/imageio.rs`: decoding through the SDK's `ImageSource`, and PNG and BMP writing.
- `src/window.rs`: state, geometry, taps and keys, the run thread and `poll`.
- `src/draw.rs`: painting and the accessibility nodes.
- `src/main.rs`: the `App` implementation and the document panels.

Tests are unit tests in each module. The action tests write under `target/automator-tests` in the crate.

## Limits of this version

- A run cannot be stopped once it has started. A script that never exits keeps the run going.
- Values are edited with the keyboard only. There is no drag and drop, no undo, and no file picker inside a field.
- The sequence shows as many steps as fit in the column. Longer workflows need a taller window.
- Images are read and written as PNG and BMP. Other formats can be decoded (what the shell can show) but not written.
