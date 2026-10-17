# Screen Time

Code: `springboard/src/shell/screentime/` (`usage.rs`, `config.rs`, `shield.rs`, `page`, `mod.rs`). The shell feeds the
tracker from its tick. Settings reads the records back for its charts.

## Usage records (`usage.rs`)

- One text file per local day, `screentime/<day>.txt`, with foreground seconds per app and per hour, pickups and
  notifications received per app.
- Days kept on disk are capped. One tick may credit at most a fixed gap to an app, so a stalled loop invents no usage.
- Unsaved usage is written out at a fixed interval.

## Configuration (`config.rs`)

- `screentime.ini` holds per-app daily limits, the Downtime schedule with its always-allowed apps, and temporary ignore
  grants.
- `Config::block` is the one policy decision. Settings itself is never blocked.
- Daily limit choices cycle by tapping an app in App Limits, and 0 means no limit.
- A warning notification is posted shortly before a daily limit runs out.
- "Ignore Limit" lasts 15 minutes or 1 more minute, as the shield offers it.

## The shield (`shield.rs`)

- A full-screen cover over an app whose limit is reached or that Downtime does not allow.
- It offers OK (back to the Home Screen) and the ignore choices. Each one asks for the Screen Time passcode when one is set.
- It takes touches only. Hardware keys (power, volume, the screenshot chord) never reach it. Settings is never shielded, and
  the shell drops the shield when the app leaves the foreground.
- The passcode is a keychain item under the shell data directory. The older `screentime_passcode.ini` is migrated once and
  removed.
