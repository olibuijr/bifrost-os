# Parity wave 4, theme 01: extension-based UI in a host app (remote views)

Pure Rust models for two gaps of this theme: the life cycle of a Live Activity, and the host side of a remote view.
Both are std-only, in `springboard/ui/src` (bifrost-ui), with no I/O, no clock and no processes. Nothing is wired into
apps or the shell yet, and nothing has been run on the VM or Midget.

## What was built

- `springboard/ui/src/live_activity.rs`: `Registry<T>` of live activities per app and the person's on/off switch.
  - `request` starts an activity, optionally scheduled for later (pending until its start time).
  - `update` replaces the content of an active or stale activity.
  - `end` ends an active or stale activity with an optional final content and a `Dismissal` policy
    (`Default`, `Immediate`, `After(t)`). `After` is cut back to the four-hour window (`DISMISSAL_WINDOW_S`).
  - `state` derives pending, active, stale, ended or dismissed from the times. There is no timer.
  - `content`, `visible`, `enabled` and `set_enabled` read the registry.
  - BifrOSt choice, not Apple's: `Dismissal::Default` keeps an ended activity for the full four-hour window.
    The official page only says "the system's default".
- `springboard/ui/src/remote_view.rs`: `Session` for one remote view, with `Bounds` in points.
  - `connect`, `connected`, `lost` and `stop` drive `Phase` (Idle, Connecting, Active, Reconnecting{attempt}, Failed).
  - Losses in a row above `max_losses` give `Failed`. A good connection resets the count.
  - `fit` lays the view out at a clamped size. `report` lets only an active view resize itself, and returns the new
    size only when it changed. `shows_placeholder` is true in every phase except active.

Both modules have `#[cfg(test)] mod tests` (8 and 6 behaviour tests, expected values worked out from the rules above).
`pub mod live_activity;` and `pub mod remote_view;` were added to `springboard/ui/src/lib.rs`.

## Official API mirrored

- ActivityKit: `Activity.request(attributes:content:pushType:style:alertConfiguration:startDate:)`, `update(_:)`,
  `end(_:dismissalPolicy:)`, `ActivityState` (`pending`, `active`, `stale`, `ended`, `dismissed`),
  `ActivityUIDismissalPolicy` (`default`, `immediate`, `after(_:)`, four-hour window), `ActivityContent` (state and
  stale date). Official page: https://developer.apple.com/documentation/activitykit/activity
  (and `/activitystate`, `/activityuidismissalpolicy`).
- ExtensionKit: `EXHostViewController` (`placeholderView`, `makeXPCConnection()`, delegate notifications of activation
  and deactivation, the host view for an extension's scene). Official page:
  https://developer.apple.com/documentation/extensionkit/exhostviewcontroller

Not mirrored: ActivityKit push tokens and push-to-start, alert configuration, the Dynamic Island presentation, the
XPC transport and the delegate callbacks. The remote view has no transport; the shell supplies it later.

## REA evidence

- ActivityKit `Activity` page: `ev_6a35a54e34806e5d6b47de238073086eaac9ec78f87a7405aec9376e04605e67`
- `EXHostViewController` page: `ev_ed2c3b7ffb81eb989e0379e9ee2d0a226404eb6ba0cff5e08f38348b87b5ba9a`

## Matrix rows closed (new status: partial)

| framework slug | feature (as in data json) | BifrOSt path |
|---|---|---|
| widgetkit | Live Activities (ActivityConfiguration, DynamicIsland, ActivityKit) | springboard/ui/src/live_activity.rs |
| extensionkit | Extension-based UI in a host app (remote views) | springboard/ui/src/remote_view.rs |
| extensionkit | Host view controller for remote extension views | springboard/ui/src/remote_view.rs |

Remaining gaps for these rows: no app-facing API for either model, no Dynamic Island or lock-screen presentation, no
XPC or other process transport for remote views, and no extension process hosted by the shell.
