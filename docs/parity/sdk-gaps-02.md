# Parity sdk-gaps-02: route addresses, encrypted models, remote sessions

Pure models in `springboard/ui/src` (crate `bifrost-ui`), tested headless in `springboard/ui/tests/parity_sdk-gaps-02.rs`.
None of them touches a socket, a device or a compositor.

| Module | Mirrors | What is wired | What needs the lab machine or another repository |
|---|---|---|---|
| `route_address` | route addresses of a routing controller: a full or partial IP address | `RouteAddress::parse` and `matches` (IPv4 and IPv6, CIDR networks), `KnownRoutes` | a feed from netd that fills the list; an app-facing setter |
| `model_seal` | encrypted models | seal and open a model's text form under a password (`sealed_box`, PBKDF2 and AES-256-GCM) or a raw key | key generation, keychain storage of the key, a build step that encrypts models |
| `remote_session` | a remote media session and its device list | `RemoteMediaSession::new`, `set_devices`, `connect`, `disconnect`, `connected` | the transport that moves the media between devices; an extension that supplies the device list |

Existing modules that the matrix rows now cite, with no change in this round:

- `push_registration`: device token checks, per-app registration with expiry, push routing and the web push `Subscription` check.
- `map_snapshot`: Web Mercator tiles, pin placement and zoom fitting for a static map image.
- `ml_model` and `ml_tensor`: the CPU evaluator for dense, relu and softmax models.
- `springboard/sdk/src/intents.rs`: declared intents served on the control socket.

Blocked and left as gaps: the device discovery extension and the notification content extension both need an
extension point in the shell (`springboard/src/shell/extras.rs`, `springboard/src/shell/notify/`) before an app can
supply a receiver or a notification view.
