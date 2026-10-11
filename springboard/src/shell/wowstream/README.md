# WoW (`shell/wowstream`)

Low-latency game-streaming client for the WoW server. The protocol driver and the in-stream
controls live in the `wowstream-client` crate (`~/Projects/wow-private/wowstream/client`, pure
Rust); everything outside the stream is drawn with SpringBoard's own parts.

| File | Role |
| --- | --- |
| `mod.rs` | `WowStream`: session lifecycle, views, effect and event glue, `Shell` hooks |
| `launcher.rs` | game cards page (state: the core's `launcher::Nav`), connection chip, confirm sheet |
| `settings.rs` | login and stream options: grouped list, `TextField`s, segmented profile, switches, shell keyboard |
| `controls.rs` | the core's `Ui` at landscape size and touch dp; shell keyboard to game keys |
| `video.rs` | decoder thread: hardware H.264, NV12 -> RGBA at the letterboxed size |
| `render.rs` | text helpers and the painter for the core's `DrawList`s (strict list order) |
| `audio.rs` | Opus `Sink`; **no-op for now** (no pure-Rust Opus decoder offline) |
| `keys.rs`, `icon.rs`, `emblem.png` | evdev -> char, the app icon, the launcher artwork for `wow` |

## Views

Settings (when open) over the launcher (core `Nav` on the menu) over the stream (`Nav` on the
stream). First run, or a failed login, opens Settings. The launcher and Settings sit under the
shell's status bar; the stream hides it.

## Landscape

WoW is a landscape app and does no rotating of its own. The shell has a per-app flag
(`shell::rotate::prefers_landscape`): while WoW is in front the shell lays out landscape
(1920x1200), whichever way the device is held and whatever Rotation Lock says, and turns between
the two landscape sides with the accelerometer (upright or flat keeps the last side). The shell
turns the frame to the panel and maps touches back, so WoW draws and takes touches in a plain
landscape frame. Every other part of the shell follows Rotation Lock. Sensor details:
`bare/orient.rs` (the one detector, the one axis table `DEFAULT_AXES` / `accel_axes`, and the
orientation-to-panel table in `gfx/rotate.rs`).

## Lifecycle

The client runs only while WoW is on screen: it connects when the app is in front and is closed
(keys released, `Bye` sent) as soon as it is not. Settings live in `<data dir>/WoW/wowstream.conf`.
While a stream is live the device stays awake and the screen does not idle off.

## Keys and text

Physical keys: the Settings page types them; in the stream they go to `Ui::hardware_key`;
`KEY_BACK` steps back and leaves the app from the launcher. The core's Kbd button would open its
own keyboard; `Controls` closes that at once (by tapping its Hide key, whose position it probes
from a scratch keyboard) and shows the shell's keyboard instead, forwarding its characters as
game key taps with shift.
