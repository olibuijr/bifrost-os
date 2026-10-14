# Parity sdk-complete-38: media, intents, background activities, upscaling, fetch and pixel conversion

Wave w8 completed the partial rows listed below. Each row stays `partial`. The headless tests are in
`springboard/*/tests/parity_sdk-complete-38.rs`; nothing here has run on Midget yet.

## What was added

| Area | Module | What it mirrors | What works now | What still needs the lab machine or is out of scope |
|---|---|---|---|---|
| Background activities | `springboard/sdk/src/activity_schedule.rs` | Background activity criteria (interval, tolerance, repeats) | Decide wait, run or finished from a caller clock; run work when due | The system does not drive the schedule; no device wake-ups |
| Search entities | `springboard/sdk/src/searchable_entity.rs` | Entity indexing for search | Entity to searchable item, expiry, batch order, validation | Not tied to App Intents entity queries; the shell does not index entities |
| Shortcut phrases | `springboard/sdk/src/intents.rs` (`Intent::phrase`) | Shortcut provider phrases | Trimmed, unique phrases in manifest TOML and intent JSON | Tile colour; parameter-bearing shortcuts; shell reads of phrases |
| Fetch predicates | `springboard/ui/src/fetch.rs` | Predicate set membership, range and prefix tests | Set membership, inclusive ranges, prefix match | No predicate language or string parser |
| Skip and position requests | `springboard/ui/src/remote_command.rs` (`SkipIntervalCommand`, `seek_target`) | Skip interval and change-position commands | Clamped skips and seeks; disabled commands refuse | Apps do not receive command events yet |
| Denoised upscaling | `springboard/render/src/temporal_upscale.rs` (`box_denoise`) | Temporal denoised scaler | 3x3 input denoise before accumulation | Box filter, not a learned denoiser; no GPU history buffer |
| Pixel conversion | `springboard/render/src/plane_arith.rs` (`invert`, `convert_transfer`) | Image conversion and arithmetic | Inversion; linear and sRGB transfer for 8-bit planes | No GPU kernels; no colour matrices |

## Not changed

- Volume control and route picker: the shell owns volume and no route picker exists; this needs the audio hardware.
- Service events from device and hardware sources: needs hardware events.
- Text input client: needs an app text view to adopt the trait, which is outside this wave.
- Now Playing fields set by apps: needs the audio service protocol.
