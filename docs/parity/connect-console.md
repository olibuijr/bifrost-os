# Connect: Console app to the power and pasteboard services

Branch `connect/console`. App `springboard/apps/console-desktop` (Console 1.1.0). Only that folder changed, plus this note.

## What is connected

| Service | Method or event | Where in the app | What the user sees |
|---|---|---|---|
| `power` | `snapshot` (one call when the window opens) | `main.rs` `configure` | Footer: `Thermal <state>`, the battery and Low Power Mode |
| `power` | `thermalStateDidChange`, `powerStateDidChange`, `batteryDidChange` (pushed) | `main.rs` `service_event`, decoded by `bifrost_app::power_service::decode_change` | The footer updates when the machine reports a change |
| `pasteboard` | `write` to the `general` board, plain text | `main.rs` `copy_selected`, `services.rs` `copy_to_general` | Log menu > Copy Message copies the selected line |

Pure glue lives in `src/services.rs`: `PowerNote` (what the footer last heard, one part at a time), `thermal_label`, `battery_text`, `copy_line` (the line text), `copy_to_general`.

The footer line is added after the count in `console.rs` (`footer_text`); it is empty until the power service answers, so an older shell shows the footer as before.

## How a user sees it

- Open Console. The footer reads for example `12 of 4,310 messages · Thermal Nominal · Battery 42%, discharging, 1 h 15 min left`. On a machine with no battery the battery part is missing.
- Select a message, then open the Log menu. The last item is **Copy Message** (disabled when nothing is selected). Paste it anywhere to get `date process[pid] Level subsystem:category message`. Empty fields are left out.

## How to verify on the test PC (Midget)

1. Publish and update as usual (`bifrost pc publish`, then `bifrost pc midget ota` until `phase=accepted`).
2. `bifrost pc midget open-app is.olibuijr.console.desktop`, then `bifrost pc midget screen /tmp/console.png`. The footer ends with the thermal state (and the battery if Midget has one).
3. Unplug or plug the power, or change Low Power Mode in Settings: the footer changes without reopening Console.
4. Select a log line, Log > Copy Message, then paste into a text field of another app (for example Notes). The pasted text is the line.
5. On an older shell (no `power` or `pasteboard` service) the footer shows no power part, and Copy Message still copies through the plain clipboard.

## Host tests

Run from `springboard/` when the lead builds once: `cargo test -p bifrost-console`.

- `src/services.rs`: the power snapshot is one `ServiceCall` to `power`/`snapshot`; a snapshot and its changes update only their part; a pushed battery event decodes to the footer text; charging, full and unknown batteries read naturally; a copied line keeps its fields; a copy sends one `pasteboard`/`write` to the general board.
- `src/main.rs` (`glue_tests`): a snapshot reply fills the note; an error reply (older shell) leaves it empty; a pushed change for another service is ignored; Copy Message sends the selected line; an error reply to the copy removes it from the queue; Copy Message with no selection sends nothing.

The test font comes from `springboard/assets/SFUI.ttf` (the same path the other apps' tests read). That folder is git-ignored, so it must exist in the checkout that runs the tests.

## Gaps

- **Energy impact next to processes is not shown.** The Console lists log lines, not processes, and the power service has no per-process energy figure (it reports the thermal state, Low Power Mode and the battery). Nothing to show; the process list belongs to the Activity app.
- **Copy Message copies one line.** Multi-line selection and copying several lines are not wired.
- **Power events come only from the shell's power service.** No thermal history is kept; the footer shows the latest value.
- A second Copy Message while the first is still waiting for its reply keeps both in the queue; each reply is matched by its request id.
