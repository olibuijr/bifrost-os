# Script menu and the scripting service

Code: `springboard/src/shell/scripts_menu.rs` (the menu extra), `services/scripting.rs` (the `scripting` service),
`bifrost_core::script::dictionary` (the dictionary reader).

## Script menu

- Lists the files in `~/.local/share/bifrost/scripts` in the menu bar. The menu extra shows only while the folder holds a
  script.
- Hidden files and folders are left out. A chosen name must be a plain file name inside the folder, so a menu row can
  never run a file outside it.
- The menu title is the file name without a `.sh` extension.
- A script runs detached with `sh`. It gets no input, its output is discarded, and the shell does not wait for it.

## Scripting service

- `list {}` returns the installed apps that declare a dictionary: `{apps: [{id, name, commands, classes}]}`.
- `dictionary {app}` returns one app's `{commands, classes}`. An app with no dictionary, or not installed, is an error.
- The dictionaries come from the `[[script.command]]` and `[[script.class]]` tables of `manifest.toml`.
- Sending a command is not a service call. `osascript` resolves the phrase against the dictionary and sends the event
  straight to the app's control socket (`bifrost_app::app_events`).
