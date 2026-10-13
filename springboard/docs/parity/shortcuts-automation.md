# Parity: Shortcuts app and automation (slug `shortcuts-automation`)

Round 75. Reference: the macOS 27 Shortcuts app bundle, listed in `~/Projects/macos-ref/extract/ref27-index.tsv`
(`System/Applications/Shortcuts.app`). Its Info.plist, nib and man-page files are not extracted under
`~/Projects/macos-ref/extract/ref27/` yet, so the behaviour here comes from the existing code, the matrix rows and the
index entry. The rows below are not closed; the lead decides closures. Nothing was built or run in this round.

## Action library

| Reference area | BifrOSt path | Done | Still needs Midget check |
|---|---|---|---|
| Actions: settings toggles (Wi-Fi, Low Power, Do Not Disturb), brightness, open app, screenshot, timer | `springboard/src/shortcuts.rs` (`Action`) | Existing, now in the shell crate | Each runs on the shell only when the app can send it; still unrun in the app |
| Actions: show notification | `springboard/src/shortcuts.rs` `Action::Notify`, `NOTE_CHOICES` | Model, editor row, plan line, file format | Run path: needs the shell's notification service from the app |
| Actions: copy text (clipboard) | `Action::CopyText`, `COPY_CHOICES`; `cmd/src/shortcuts.rs` `perform` uses `cmd/src/pasteboard.rs` `copy` | Model, editor row, and a real copy from the command line | Copy from the app is not wired |
| Actions: files (open a folder) | `Action::OpenFolder`, `FOLDER_CHOICES`; `cmd/src/shortcuts.rs` `perform` uses `cmd/src/open.rs` | Model, editor row, and a real folder show from the command line through the shell's `open-folder` request | Folder show from the app is not wired |
| Actions: shell script | `Action::RunScript`, `SCRIPT_CHOICES`; `cmd/src/shortcuts.rs` `perform` runs `sh -c` | Model, editor row, and a real run from the command line | Not run by the app (a shell script from the app needs a privileged path, not built) |

Values come from fixed choice lists, so a stored value always has a known meaning and cannot hold `;` or a tab. A
value outside its list is dropped when the file is read, the same way unknown kinds are.

## Shortcuts command (`shortcuts`, `springboard/cmd`)

| Reference area | BifrOSt path | Done | Still needs Midget check |
|---|---|---|---|
| `shortcuts list` | `cmd/src/shortcuts.rs` `list_text`; `cmd/src/bin/shortcuts.rs` | Id, name, step count, one line each | Reads the app's file from `springboard::shell::apphost::root()/appdata/<app id>` |
| `shortcuts view NAME-OR-ID` | `cmd/src/shortcuts.rs` `view_text`, `resolve` | Reminder and numbered steps; exact name, then partial; ambiguous names ask for the id | |
| `shortcuts run NAME-OR-ID` | `cmd/src/shortcuts.rs` `run_steps`, `perform` | Runs copy, folder and script steps; stops at the first step that does not run, like the app; exits 1 | Settings-service steps are reported as not run, by design |

The app's `run` still reports that the device cannot run shortcut actions. The command runs the steps it can reach
from a user shell, so the two differ on purpose. Notification steps are not run by the command either.

## Shared model

- `springboard/src/shortcuts.rs` (new, `pub mod shortcuts;` in `springboard/src/lib.rs`) owns the shortcut list: the
  file format, defaults, the action library, plan lines (`Action::line`), `stored` and `app_data_dir`.
- `springboard/apps/shortcuts-desktop/src/model.rs` is deleted. Its callers (`automation.rs`, `widget.rs`,
  `services.rs`, `main.rs`) import `springboard::shortcuts`.
- Removed from the app's `automation.rs`: the app-local `FILE` constant (now `shortcuts::FILE`), the app-local
  `stored` (now `shortcuts::stored`) and the app-local line match in `plan_text` (now `Action::line`).

## Intents and Quick search

- The app's two declared intents (Run Shortcut, Plan Shortcut) and the entity query over shortcuts are unchanged. The
  new action kinds appear in plan results (one row per step) and in the Add Action list.
- Not done: other apps' SDK intents are not listed as actions. That needs a shell-wide intent registry, which is not
  part of this slice. The "Intents" section still lists only the Shortcuts app's own two intents.
- Not done: the menu bar "run shortcut" entry and the Quick search result for shortcuts (the shell side). Both need
  changes in `springboard/src/shell`, which this slice does not own.

## Tests

Unit tests were added for the model (every kind round-trips, choice lists are validated, separators are excluded),
the command (parsing, id and name resolution, list and view text, stop-on-failure, unreachable steps refused without
side effects) and the app's shared-model paths. None were run: the lead builds.

## Shared-file and snapshot notes for the lead

- `springboard/src/lib.rs`: one line, `pub mod shortcuts;`.
- `springboard/cmd/src/lib.rs`: one line, `pub mod shortcuts;`.
- `springboard/apps/shortcuts-desktop/tests/snapshots/*.png`: the Add Action picker has four more rows, so any golden
  that shows that picker changes. Regenerate with `BIFROST_BLESS=1` after the build.
- Version: Shortcuts app 1.7.0 in `manifest.toml`, `Cargo.toml` and `CHANGELOG.md`.
