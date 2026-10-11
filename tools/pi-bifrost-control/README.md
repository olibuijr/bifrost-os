# pi extension: BifrOSt app control

Lets a pi agent see and drive running BifrOSt PC apps over their JSON-RPC control sockets
(`tasks/app-control.md`, `springboard/sdk/src/control.rs`).

| Tool / command | Does |
|---|---|
| `bifrost_apps {host?}` | Lists apps with a live control socket (id, pid, data dir); flags stale sockets |
| `bifrost_app_methods {app, host?}` | Lists the methods an app serves |
| `bifrost_app_call {app, method, params?, host?}` | Calls one method and returns its JSON result |
| `/bifrost-apps [host]` | Same list as `bifrost_apps`, shown to the user |

`host` is an ssh alias such as `midget`. Omit it for this machine.

Local sockets live in `$XDG_RUNTIME_DIR/bifrost/apps/<app-id>.sock` and accept only the owning user.
`app` must be a reverse-DNS id such as `is.olibuijr.notes.desktop`; paths are refused.

## Remote hosts (Midget)

Remote apps are reached over ssh. The apps' sockets accept only the installed account's UID,
and root is rejected. So the extension sends one request to a small stdlib-only relay on the
remote host, which runs as the account read from `/var/lib/springboard/account`. Sandboxed
SDK apps have no `XDG_RUNTIME_DIR`, so the relay looks for `control.sock` in each app's data
directory (`/var/lib/springboard/appdata/<id>/`) as well as in the runtime directory.

Environment:

| Variable | Default | Use |
|---|---|---|
| `BIFROST_SSH` | `ssh` | ssh command (tests replace it with a local shim) |
| `BIFROST_SSH_OPTIONS` | `-o BatchMode=yes -o ConnectTimeout=5` | Extra ssh options, space-separated |

Midget's lab address changes its host key on every reinstall. If plain `ssh midget` reports a
changed host key, remove the stale 10.99.0.200 entries from `~/.ssh/known_hosts` after checking
that the reinstall is expected. Setting `BIFROST_SSH_OPTIONS` to the lab options used by
`tools/midget.py` (`-o UserKnownHostsFile=/dev/null -o StrictHostKeyChecking=no`) also works,
but it turns host-key checking off.

Midget status: the installed apps (Browser 0.2.1, Remote 0.1.6) predate the control socket, so
`host: midget` reports no apps. Notes desktop 1.0.3 is the first store build with the socket.
It is published, but installing it on Midget still has to be done from the App Store.

Methods whose names contain a write verb (`create`, `add`, `update`, `set`, `delete`, `remove`,
`complete`, `import`, ...) ask the user to confirm first. Without a UI they are refused. The app
still enforces its own rules.

## Use

```bash
pi -e ./tools/pi-bifrost-control/index.ts          # one session
# or make it permanent:
ln -s "$PWD/tools/pi-bifrost-control" ~/.pi/agent/extensions/bifrost-control
```

## Test

```bash
node --test tools/pi-bifrost-control/client.test.ts   # local sockets, and the remote relay through a fake ssh
# index.ts is checked by tsc against pi's published types (@earendil-works/pi-coding-agent 0.80.x).
```

The client has no pi dependency, so it runs under Node 22+ without a build step.
`index.ts` imports pi's packages and needs pi to load it.
