# Wire: the `dock` system service (Dock tile badge, progress, bounce)

Branch `wire/svc-dock`. The `dock` service lets an app set its own Dock tile: a badge, a progress bar and an
attention bounce. The shell draws them on the app's Dock icon. It uses the existing service channel
(`ToShell::ServiceCall`, `docs/bifrost-ui.md` "System services"); the wire protocol is unchanged.

## What is now wired

| Parity model | Where it was | Now wired |
|---|---|---|
| `NSDockTile` badge (`badgeLabel`, four characters shown) | `springboard/ui/src/dock_tile.rs` (`DockTile`) | `dock` service `badge` sets `AppHost::badge`, the same field `ToShell::SetBadge` already used, so the desk's badge drawing is unchanged. The text goes through `clean_badge`. A test checks that `clean_badge` and `DockTile::displayed_badge` agree. |
| `NativeProgressIndicator` (determinate and indeterminate) | `springboard/ui/src/values.rs` (`Progress`) | `dock` service `progress` stores a `Progress` on the host (`AppHost::tile`). `desktop_dock.rs` draws it as a thin bar on the lower part of the icon (`dock_tile_progress`, called from `draw_desk_dock`). |
| `NSApplication.requestUserAttention` (informational and critical) | No model; the launch bounce only (`desktop_dock.rs`) | `dock` service `bounce` opens an attention window on the host. The icon hops with the launch-bounce kinematics (`bounce_lift`, `dock_tile_bounce`). An informational bounce lasts three hops, a critical one ten seconds. Activating the app ends either one, and a call while the app is already in front does nothing. |

The service keeps no state of its own. Its state sits on the app's host, which is what the shell already
reads when it draws the dock.

## How a user reaches it

An app that is installed and running calls the service through the SDK client `springboard/sdk/src/dock_tile.rs`:

- `dock_tile::set_badge(ctx, "3")`. Needs the `notifications` permission. An empty text clears the badge.
- `dock_tile::set_progress(ctx, Some(0.5))` shows a bar, and `set_progress(ctx, None)` clears it.
  `dock_tile::set_indeterminate(ctx)` shows a sweeping bar.
- `dock_tile::bounce(ctx, critical)` bounces the Dock icon.
- `dock_tile::state(ctx)` reads back `badge`, `progress`, `indeterminate` and `bouncing`.

The calls return the request id. The reply arrives in `App::service_reply`, or through `service::take_reply`.

A call from an app that is not installed, or not running, is an error reply and does nothing.

The shell side is `springboard/src/shell/services/dock_tile.rs` (`DockService`, name `dock`). It is registered
in `services::all()`. `AppHost` gets one new field, `tile: TileState`, defined in `desktop_dock.rs`.

## How to verify on the test PC (Midget)

Use Midget only; the VM is not used for this.

1. Build the fast profile and push the shell (`cargo build --offline --profile fast --bin shell --features desktop-wayland`
   in `springboard/`, then `tools/midget-push-shell.sh --no-build shot.png`). Then `bifrost pc publish` and
   `bifrost pc midget ota` until `phase=accepted`.
2. Open an installed app that declares `notifications` in its manifest. Use `bifrost pc control call` (see the
   `bifrost-app-control` skill) or an app that calls the SDK client. Send `set_badge("3")`, `set_progress(Some(0.5))`
   and `bounce(false)`.
3. Take a Midget screenshot (`bifrost pc midget screen PATH`). Expect a badge on the icon, a bar across its lower
   part and a hop of the icon. Call `state` and check the reply.
4. Focus the app's window (`bifrost pc midget tap-window APP X Y`). The bounce should stop.

## REA evidence

Official pages were read through `tools/parity/docc.py` and rendered with `tools/parity/rea-page.sh`:

- `ev_9edad8ae0c9577f2271dce2edb1d93b8fcd80f734e1dce0e9ecaaadb07d0a23b`: NSDockTile, rendered official page.
- `ev_939d33b9d37f2c915cd5109e4d4d9cda1c189698a1ded52c92506a3737557bc4`: NSDockTile `badgeLabel`, rendered official page.
- `ev_15fbbe86b681eb3618d4a197b0421e5aacf1c7f89dfa7024028e87185638363d`: `requestUserAttention(_:)`, rendered official page.
  It says that activating the app cancels the request and that a call to an app that is already active has no effect.
  The shell follows both rules.

## Decisions and limits

- The badge is written to the existing `AppHost::badge` field, so the existing `SetBadge` path and the new
  service path share one rule.
- A progress bar is drawn across the icon, not inside a custom content view. An app has no drawing surface in the
  Dock; the reference's content view is not modelled.
- The bounce hops with the launch-bounce kinematics. Its duration comes from `motion::duration(Kind::DockBounce)`.
  Under Reduce Motion the bounce has no offset, but its window still runs.
- A Dock tile menu and the app's own Dock icon drawing are not modelled.
- `NSDockTile.size` and `owner` are not modelled.
- The matrix row for NSDockTile (`docs/parity-matrix/data/appkit-user-interface.json`) is not changed here. The
  lead updates it when the work lands.

## Remaining gaps

- No SDK call yet for the Dock tile's `size`, its `owner`, or a custom `contentView`.
- The bounce lasts a fixed window. It ends on activation, but there is no request handle to cancel it early
  (the reference returns an identifier for `cancelUserAttentionRequest`).
- `docs/bifrost-ui.md` "System services" does not yet list the `dock` service. It is left for the lead to add.
- Not built and not run. The lead builds once. The new unit tests are in `services/dock_tile.rs`,
  `sdk/src/dock_tile.rs` and `desktop_dock.rs` (`attention_bounce_hops_then_stops`).
