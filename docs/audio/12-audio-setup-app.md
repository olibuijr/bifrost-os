# Audio Setup app (task 12)

Status: app written and building on the host with `cargo check`. Not yet run on Midget. The HAL client bodies
(`bifrost-audio-hal`, task 06) and the server's aggregate handling (task 11) are still `todo!()` in the skeleton, so
until the lead merges those tasks the window panics on its first server call.

Task file: `tasks/audio/12-audio-setup-app.md`. Contract: `docs/audio-architecture.md`.

## What is implemented

App `springboard/apps/audio-setup-desktop/` (package `bifrost-audio-setup`, app id `is.olibuijr.audiosetup.desktop`,
`window = "full-size"`, Golden Gate kit: `widgets::toolbar`, `widgets::source_list`, `Toolbar`/`SourceList` from the
SDK's controls).

| File | Role |
|---|---|
| `src/main.rs` | The `App` impl: events (touch, keys, published controls), the 2 s refresh tick, drawing |
| `src/window.rs` | State (`AudioSetup`), layout in points, toolbar and sidebar builders, hit tests, actions |
| `src/draw.rs` | Paints the window and publishes its accessibility nodes |
| `src/model.rs` | Pure device model: `DeviceInfo`, `Kind`, number formatting (rate, buffer, latency, volume, channels) |
| `src/draft.rs` | The aggregate or multi-output form: members, clock, validation, composition text |
| `src/device.rs` | Server calls through `bifrost-audio-hal`: list, read properties, create, edit, remove |

Behaviour:
- **Sidebar**: "Hardware" devices, then "Aggregates" (aggregate and multi-output devices the app made). Rows show a
  speaker or a stacked-devices symbol. Unplugged devices are dimmed.
- **Detail view** of the selected device: name, manufacturer and transport, then a card with the output format,
  sample rate, channels (in and out), buffer size with its range, output latency, safety offset, volume and the
  identifier (UID). Latency and buffer times are frames divided by the rate.
- **Toolbar**: New Aggregate, New Multi-Output, Edit and Remove (the last two only for a made device with no editor
  open), Refresh.
- **Editor**: a name field, a kind switch (Aggregate or Multi-Output), the member card (click a row to add or remove a
  device; "Set as Clock" on a member row sets the clock), and Create or Save and Cancel. Return saves, Escape cancels,
  Delete removes a character. A refused request shows its sentence under the form. Multi-output checks that every
  member has an output and that all run at one rate; aggregates may mix rates because drift compensation handles it.
- The list is read at start and then every 2 seconds (`REFRESH_S`), so a new device or aggregate appears on its own.
- Light and Dark: colours are the kit's `Pair` tokens resolved with `now()`. Type sizes are the kit's MAC_* constants,
  card radius 13 pt, field radius 6 pt, sidebar width 220 pt. No literal font sizes.

Logging: the app does not log yet. Server errors reach the window as sentences (`device::describe`). When logging is
added, use subsystem `is.olibuijr.audio`, category `client`. The code uses neutral names only.

## Contract addition (for task 11 to implement, and for the lead to review)

The skeleton has no way to create, edit or remove an aggregate. This task adds two property selectors. They are
defined in `src/draft.rs`, not in `audio/types/src/props.rs`, because that file is not owned by this task. The lead
should move them to `props.rs` when merging (both are additive):

| Selector | Address | Get | Set | Meaning |
|---|---|---|---|---|
| `agcm` (`COMPOSITION`) | aggregate or multi-output device, global | `PropertyValue::String(text)` | on the system object, `PropertyValue::String(text)` | read the members and clock; set to create (first line empty) or replace (first line is the UID) |
| `agrm` (`REMOVE`) | made device, global | | `PropertyValue::Bool(true)` | destroy the device |

The composition text (`Draft::encode`, decoded by `Draft::decode`), one field per line:

```
<UID, empty for a new device>
<transport: "aggregate" or "multi-output">
<clock UID, empty when none>
<name>
<member UID>        (one line per member, in order)
```

Transport strings `aggregate` and `multi-output` are the ones `model::Kind` reads. Any other transport is hardware.

## Tests (host)

`cargo test -p bifrost-audio-setup` runs these (not run here: the lead runs the tests):

- `model::tests`: classification by transport; `frames / rate` in milliseconds (64 frames at 48 kHz = 1.333 ms,
  256 = 5.333 ms, 1024 = 21.333 ms, 480 at 44.1 kHz = 10.884 ms); rate, format, buffer, latency and volume text;
  channel summary and subtitle. The format test calls `StreamFormat::float32` and `integer`, which are todo in task 01.
- `draft::tests`: toggle order and clock rules; aggregate validation (mixed rates allowed, unknown member refused);
  multi-output validation (output needed, one rate); composition round trip, with a worked example line by line;
  decode refuses hardware and short input.
- `device::tests`: error sentences; the two selectors' bytes (`agcm`, `agrm`).
- `window::tests`: default names; toolbar enabling rules; member and clock activation and the save refusal with the
  "Give the device a name." sentence; accessibility id mapping; editor geometry fits 1200x780 and a 600x300 window.

Drawing and the compliance gate are not unit-tested. See the Midget steps below.

## What is left (todo_left)

1. Tasks 06 and 11 must implement the HAL calls this app uses (`System::connect`, `devices`, `Device::get`/`set`) and the
   composition and remove handlers above. Until then the window cannot list devices.
2. The compliance gate: `tests/compliance.rs` is generated by `tools/gen-app-compliance-tests.py`. It is not generated
   here, because the lead runs the tests. Run the generator for this app, then `bifrost pc smoke audio-setup` in Dark
   and Light, and review with the `bifrost-hig` checklist.
3. The sidebar and the member card do not scroll. A machine with more than about 11 editable devices shows only the
   first ones in the editor, and the sidebar grows past the window. A scroller is the next step.
4. The app icon is a copy of the Calculator icon (`icon.png`). Replace it with an Audio Setup icon.
5. The volume is read but not written from this window. Volume belongs to the Settings page and the Control Center
   (task 09).
6. Process taps, per-device stream selection (which channels of an interface a client uses) and the MIDI Studio tab
   (after task 15) are not in this window.
7. `tests/compliance.rs` snapshot goldens do not exist yet (they come with item 2).

## Midget verification (for the lead)

These need the server (tasks 05, 06 and 11) on Midget. Run from Titan.

1. Build and install the app on Midget with the usual flow (`tools/pc-app.py build`, then `publish`, then
   `ssh midget` and `/usr/bin/springboard --install-store-app is.olibuijr.audiosetup.desktop`).
2. Check the server sees the devices: `bifrost pc audio ls`. The ALC233 analog output (card 0 device 0) and HDMI/DP
   pcm=3 should appear. Note the UIDs.
3. Open the app: `bifrost pc midget open-app is.olibuijr.audiosetup.desktop`, then `bifrost pc midget state`.
   Confirm the sidebar lists the hardware devices and that selecting one shows its format and rate.
4. Create an aggregate: tap New Aggregate (`bifrost pc midget tap-window "Audio Setup" X Y`, using the toolbar
   position from the screenshot). Type a name with `bifrost pc midget key` (one key at a time). Tap the two member
   rows (analog and HDMI/DP pcm=3), set the clock on one, and tap Create.
5. Check the aggregate: `bifrost pc audio ls` lists a new device with transport `aggregate` and the two members, and
   the sidebar shows it under "Aggregates".
6. Remove it: select it, tap Remove. `bifrost pc audio ls` no longer lists it.
7. Screenshot in both appearances: `bifrost pc midget screen /tmp/audiosetup-dark.png`, then
   `bifrost pc midget appearance light` and a second screenshot. Review both with the `bifrost-hig` checklist.
