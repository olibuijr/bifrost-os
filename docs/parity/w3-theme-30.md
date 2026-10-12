# Wave 3, theme 30: Route events

Theme "Route events" (size 8, frameworks AVRouting, Force Feedback, Social). Pure Rust, std only, host unit tests only.
Not built, tested or wired into apps or the shell; the rows below are partial until they work on the VM and Midget.

## What was built

- `springboard/sdk/src/route_events.rs` (new): a route event log for an app that offers its own media destinations.
  - `RouteReason` (activate, deactivate, reactivate), `DeviceRoute` (id, optional Bluetooth identifier and network
    endpoint), `RouteEvent` (route and reason).
  - `RouteEventLog` keeps each route's state (active, and authorized once activated). Rules: activate makes a route
    active and authorized; deactivate needs a known route; reactivate needs an authorized route. A rejected event
    changes nothing and is not queued. The queue is bounded: the oldest event is dropped and counted.
  - `take_events` drains the queue; route state survives it.
- `springboard/sdk/src/social.rs` (extended): `ServiceTypes`, a registry of service types with whether each can take
  a request now. An unregistered type is not available, and no type is registered by default. Covers the availability
  question only; no service type is wired to a network or a shell surface.
- `springboard/sdk/src/lib.rs`: one line, `pub mod route_events;`.

Force Feedback was not built. Its cooperative-level row is marked `gap` with the note that Apple's own API leaves that
call unimplemented, so there is nothing to match, and the wider force-feedback rows need hardware access, which this
theme excludes.

## Official API names mirrored

- AVRouting: `AVCustomRoutingEvent` (route, reason), `AVCustomDeviceRoute` (bluetoothIdentifier, networkEndpoint),
  `AVCustomRoutingEventReason` (activate, deactivate, reactivate). Official DocC: `avrouting/avcustomroutingevent`,
  `avrouting/avcustomdeviceroute`, `avrouting/avcustomroutingeventreason`.
- Social: `SLComposeViewController.isAvailable(forServiceType:)`. Official DocC: `social/slcomposeviewcontroller`.

## REA evidence (rendered official pages, 2026-10-12)

- `ev_d570d56f662aa2f90a6243519d4fe5f68eedf60e4136ea5acbf2484a5299babf`: AVCustomRoutingEvent
- `ev_38bff99c293d51a91e7ca70ab095362bc504fbde796090677275241ee924bf83`: AVCustomRoutingEventReason
- `ev_aec8ec74ddcc568d6263badcbe3841ce3372379ddbe9c3aa81c2325c97a39a30`: SLComposeViewController (isAvailable(forServiceType:))

## Matrix rows closed (status partial)

| Framework slug | Feature (as in data file) | Status | BifrOSt path |
|---|---|---|---|
| avrouting | Route events | partial | springboard/sdk/src/route_events.rs |
| social | Service availability check | partial | springboard/sdk/src/social.rs |

## Not closed

- AVRouting "Authorized routes and route activation", "Route addresses on the local network", "Custom action items in
  the picker", "System route picker with custom destinations": outside the route-event model (picker UI, receiver
  address registry, per-app authorization).
- Force Feedback rows (device properties, effect creation, cooperative level, app-facing haptic request): need
  hardware or an input stack that this theme does not reach.

## Known limits

- The event log is not connected to any picker, transport or app event loop.
- Route state is in memory only and is never persisted.
- Tests were written by hand from the rules above and have not been compiled or run (no builds in this wave).
