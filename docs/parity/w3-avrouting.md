# Parity wave 3: AVRouting (custom media routes)

Official reference: https://developer.apple.com/documentation/avrouting (macOS 13.0, Mac Catalyst 16.0).

## What was built

Three pure Rust models in the `bifrost-ui` crate (`springboard/ui/src/`), std only, no hardware or network access.
They are the app-side bookkeeping that a custom routing controller needs. Nothing is wired into the shell, the SDK or
the audio service yet, and the models have not been run on the VM or Midget.

- `route_auth.rs` (`RouteAuth`): the set of routes the user authorized for this app, the active state of each
  route, revocation, and a one-shot change flag. Activation needs an authorization. Revoking a route deactivates it.
- `route_action.rs` (`ActionItems`, `ActionItem`): custom action items for a route picker. Each item names a
  configured type and may override its title. Items whose type is not configured are not shown.
- `route_picker.rs` (`destinations`, `Destination`): picker rows. Built-in outputs come first, in the order given.
  Discovered receivers follow, sorted by name ignoring case. Each row carries the active state from `RouteAuth`.

## Official API names mirrored

| BifrOSt | Apple |
|---|---|
| `RouteAuth::authorize`, `authorized_routes`, `is_authorized` | `AVCustomRoutingController.authorizedRoutes` |
| `RouteAuth::set_active`, `is_active` | `setActive(_:for:)`, `isRouteActive(_:)` |
| `RouteAuth::invalidate` | `invalidateAuthorization(for:)` |
| `RouteAuth::take_changed` | `authorizedRoutesDidChange` notification |
| `ActionItems::add`, `items`, `remove` | `customActionItems` |
| `ActionItem::kind`, `override_title` | `AVCustomRoutingActionItem.type`, `overrideTitle` |
| `destinations` | the route picker list fed by the controller's routes |

## REA evidence

- `ev_14110f8179b556b56f5de61c07bede4c7dfac89ff255facccf244207b2b7d697` (AVRouting landing page)
- `ev_ecb28080d8dc98e44a867a406f2b095dc41edab64b2a4afb42d8fe08cc315f97` (AVRouting landing page, second render)
- `ev_abe61406d007df4e757ecc746ade6e08e974af13eb084e1f00390fbb981d0c30` (AVCustomRoutingController page)

## Matrix rows closed (status `partial`)

| Framework | Feature | Status | BifrOSt path |
|---|---|---|---|
| AVRouting | Authorized routes and route activation | partial | springboard/ui/src/route_auth.rs |
| AVRouting | Custom action items in the picker | partial | springboard/ui/src/route_action.rs |
| AVRouting | System route picker with custom destinations | partial | springboard/ui/src/route_picker.rs |

## Not covered

- Route IP addresses (`knownRouteIPs`, `AVCustomRoutingPartialIP`) and route events (`AVCustomRoutingEvent`) need
  network state and the device discovery extension. They stay gaps.
- The picker UI, the discovery extension point and the controller delegate are not built. `route_picker.rs` is the
  row model only.
