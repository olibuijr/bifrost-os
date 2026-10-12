# Wire: shell service `widgets_timeline` (branch wire/svc-widgets)

Wires the WidgetKit timeline model (`bifrost_ui::timeline`) and the live activity registry
(`bifrost_ui::live_activity`) into the shell and the SDK. Apps give the shell widget timelines and live
activities over the service channel; the shell keeps them and shows the current entry in Notification
Centre's widget area. The wire protocol (`springboard/proto`) is unchanged.

## What is now wired

| Matrix slug | Feature (as in `docs/parity-matrix/data/widgetkit.json`) | BifrOSt path | Status |
|---|---|---|---|
| `widgetkit` | Timeline updates (TimelineProvider, TimelineEntry, Timeline, WidgetCenter reload) | `springboard/src/shell/services/widgets_timeline.rs`, `springboard/sdk/src/widgets_timeline.rs` | Wired (service, shell store, reload event). Not built or run yet. |
| `widgetkit` | Today View and widget list ordering | `springboard/src/shell/notify/center.rs` (`center_widget_rects`, the widget rows in the draw) | Wired: Notification Centre shows a widget row for each entry on screen. |
| `widgetkit` | Live Activities (ActivityConfiguration, DynamicIsland, ActivityKit) | `springboard/src/shell/services/widgets_timeline.rs` (activity methods), `springboard/sdk/src/widgets_timeline.rs` | Partly wired: the registry and life cycle work through the service; the activity shows as a row in Notification Centre only. No lock-screen strip and no compact tile. |

Models used, unchanged: `bifrost_ui::timeline::{Entry, ReloadPolicy, Timeline}`,
`bifrost_ui::live_activity::{Registry, Content, Dismissal, State, Error}`.

## How an app reaches it

The service is `widgets_timeline`, registered in `springboard/src/shell/services/mod.rs`. The SDK client is
`bifrost_app::widgets_timeline` (`springboard/sdk/src/widgets_timeline.rs`).

- `reload(ctx, kind, &[Entry], ReloadPolicy)` replaces the app's timeline for `kind` (max 64 entries, kind 1-64 bytes).
- `remove(ctx, kind)`, `current(ctx, kind)`, `activities(ctx)`.
- `start_activity(ctx, title, text, stale_at, start_at)`, `update_activity(...)`, `end_activity(ctx, id, final, dismissal)`.
- The shell sends the `reload` event (`parse_event` decodes it) when a timeline's reload date passes. Each
  timeline is requested once until the app reloads it.
- Replies arrive as `App::service_reply` (or `service::take_reply`); events as `App::service_event`.

Timelines are per app: another app cannot read or change them. Live activities can be updated or ended only
by the app that started them. Dismissal defaults to the model's 4-hour window.

## How to verify on the test PC (Midget)

Use Midget only; the VM is not used. The lead builds once.

1. Build and push the shell (`fast` profile, then `tools/midget-push-shell.sh --no-build shot.png`).
2. Run the host tests for the store: `cargo test -p springboard widgets_timeline` (in `springboard/`).
   These test `Store` directly; they do not touch the global store.
3. Run the SDK encoding tests: `cargo test -p bifrost-app widgets_timeline`.
4. On Midget, open Notification Centre (swipe down from the left two thirds of the status bar, or tap the clock).
   With no app calling the service, the widget area is empty and the title reads "No Notifications".
5. To see a row, an app must call `widgets_timeline.reload` or `start_activity` (no app in this change does yet).
   Then the row appears above the notification cards with the entry on screen.

## REA evidence

- `ev_25581cd455549184dcccbbac55a9b96f656c29cb64c8276d88386335cf5936f2`: TimelineProvider page (timeline,
  entries, refresh policy `at End` and `after(_:)`).
- `ev_700ad66fe55c1ed19e4c3015f3aabd394966c5bb9dad88e907a6bbe89c2e6ae8`: ActivityKit `Activity` page
  (request, update, end).
- Official text: `python3 -I tools/parity/docc.py widgetkit/timelineprovider` and `activitykit/activity`.

## Remaining gaps

- No app calls the service yet. Nothing in the repo reloads a timeline on a user's behalf.
- Reload is one event per timeline; the shell does not retry a reload the app never answers.
- Live activities show in Notification Centre only. No lock-screen strip, no compact tile, no stale-date styling
  beyond the model's state.
- Timelines and activities are in memory (a process-wide store). They are lost on a shell restart; apps must
  re-send them. No persistence.
- Entries carry a title and text only. No relevance scores (Smart Stacks), no widget families, no interactivity.
- The code is not compiled yet (the lead builds once). The Notification Centre widget area has no draw test.
