# Wave 5: Screen Time configuration for apps

## What was built

- `springboard/sdk/src/screen_time.rs` (`bifrost_app::screen_time`): an app-facing
  `Configuration` with `enforces_child_restrictions`, and a `ConfigurationObserver`
  with `start_observing`, `stop_observing`, `configuration` and `poll`.
- The configuration is read from the shell's `screentime.ini` (a `key=value` file).
  Child restrictions count as enforced while the Downtime window is on, or while any
  app has a daily limit above zero. This is the same rule the shell uses to decide
  what it blocks (`springboard/src/shell/screentime/config.rs`, `Config::block`).
- The module is registered with one `pub mod screen_time;` line in `springboard/sdk/src/lib.rs`.
- No new crates, no unsafe, std only.

## Apple API mirrored

Official pages (DocC via `tools/parity/docc.py`):

- `https://developer.apple.com/documentation/screentime/stscreentimeconfigurationobserver`
  (`init(updateQueue:)`, `configuration`, `startObserving()`, `stopObserving()`)
- `https://developer.apple.com/documentation/screentime/stscreentimeconfiguration`
  (`enforcesChildRestrictions`)

Deviations, deliberate:

- There is no update queue. `ConfigurationObserver::new` takes the settings file path,
  and the app calls `poll()` from its own run loop. Nothing is delivered in the background.
- `enforcesChildRestrictions` has no parent account behind it here. It is derived from
  the shell's own limits and Downtime settings.

## REA evidence

- `ev_05d72d0f8a0d2fd9effa2c7c24e84c5ba2ad175fe0e59af4e1153172c876f548`: the Screen Time
  landing page, rendered. It lists the configuration queries (observer and configuration),
  and it says the APIs do nothing on visionOS.

## Tests

`cargo test` was not run (no builds in this wave). The module has six `#[cfg(test)]`
tests with expected values worked out by hand: empty and unrelated settings, Downtime
on and off, positive and zero limits, last-line-wins, comments and malformed lines, and
an observer test over a real file (missing file, change detected, stopped observer silent).

## Matrix rows closed

| Framework | Feature (as written in data/screen-time.json) | New status | BifrOSt path |
|---|---|---|---|
| Screen Time | Configuration observer and configuration (STScreenTimeConfigurationObserver, STScreenTimeConfiguration) | partial | springboard/sdk/src/screen_time.rs |

## Not done in this wave

- The other Screen Time rows are not Apple APIs (per-app usage, daily limits, Downtime),
  so no SDK API was added for them. Web usage (`STWebpageController`) and web-history
  deletion (`STWebHistory`) are already partial and were not touched.
- The shell does not yet write `screentime.ini` through the SDK, and the SDK does not
  yet reach the file from a sandboxed app. Access to the file is a system-layer decision.
- Not tested on the VM or Midget.
