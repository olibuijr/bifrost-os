# XPC services (parity sdk-rest-14)

Mirrors: XPC services managed by launchd (on-demand launch, idle shutdown, restart on crash) and the privileged helper.

## What is wired

- `springboard/sdk/src/service_client.rs` (`bifrost_app::service_client`): an app-facing client for a background
  service. `ServiceId` validates the identifier, `socket_path` gives `<root>/run/<id>.sock`, and `ServiceClient`
  connects, pings and makes raw calls with the `bifrost_proto::svc` framing. Failures map to `ServiceError`:
  not installed, not listening, timeout, disconnected, wrong service, too large, protocol, I/O.
- `springboard/bifrostd/src/lib.rs`: the supervisor binds each service socket and starts the service on the first
  waiting connection (socket activation), restarts it with backoff after a crash and swaps a new version in after a ping.
- Tests: `springboard/sdk/tests/parity_sdk-rest-14.rs`, all headless, each serving one connection on a temporary socket.

## What needs the lab machine or is missing

- Idle shutdown: no service is stopped when nothing uses it.
- Declaration: an app cannot list a service in its manifest, and the sandbox binds only the sockets it is granted.
- Verification: a sandboxed app reaching a live service has not been checked on Midget.
- Privileged helper (`bifrost-privd`): not reachable from apps by design. The daemon admits only the shell, the MCP
  server and root tools by peer executable path, and the app sandbox does not bind its socket.
