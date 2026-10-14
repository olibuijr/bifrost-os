# CoreAudioKit parity (team J)

Branch `parity/team-j-audio-unit`, based on `arm`. Built on this branch and not yet verified on Midget.

## What was built

- `springboard/ui/src/network_audio_browser.rs` (new module, pure std): the model behind a network audio device
  browser.
  - `Receiver`: a stable identifier, a display name, host, port and the time of the latest announcement.
  - `NetworkAudioBrowser`: a browser with a time-to-live (default 30 seconds). `announce` reports a new receiver as
    `Found` once and refreshes a known one silently. `expire(now_ms)` removes receivers that have been silent for
    longer than the time-to-live and reports them as `Lost`, in identifier order.
  - `destinations`: route picker rows for the listed receivers, built with `route_picker::destinations`, so the app's
    `RouteAuth` decides which rows are active.
  - Stopped browsers ignore announcements and expiry, as the device browser does for its snapshots.

The discovery source is not part of this module. The system layer reads announcements and calls `announce` and
`expire`. No socket is opened and no receiver is contacted.

## Official API mirrored

`CANetworkBrowserWindowController` (CoreAudioKit) lists network audio devices for the user to pick. The model mirrors
the list and its changes, not the window. The official page is
https://developer.apple.com/documentation/coreaudiokit.

## Tests

`#[cfg(test)] mod tests` in `network_audio_browser.rs` covers the time-to-live setting, a single `Found` event,
silent refresh and renaming, field validation with trimming, the stopped state, expiry boundaries (exactly one
time-to-live stays listed), re-finding a lost receiver, and the sorted route picker rows with their active state.
Tests were not run in this round.

## Matrix rows

| Feature (`data/coreaudiokit.json`) | Status | BifrOSt path |
|---|---|---|
| Network audio device browser | partial | springboard/ui/src/network_audio_browser.rs |

## Not done

- A discovery source that feeds `announce`. Nothing scans the network yet.
- A window or SDK view on top of the list.
- The route indicator row stays partial as before; `route_picker.rs` has the rows but no SDK API exposes them.
