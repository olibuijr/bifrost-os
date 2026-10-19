# Desktop application isolation

Since shell 0.4.19 the shell centrally launches installed PC SDK and native packages through
`bare/app_policy.rs`. App entry points run as the installed non-root account,
inside bubblewrap filesystem/PID/IPC/UTS/user namespaces. Launch fails if identity,
permission validation or sandbox setup fails; there is no unrestricted fallback.
The trusted desktop shell still runs as root for existing system operations.
Non-Wayland launch paths are separate and unchanged.

Only the selected read-only package, its writable private data, system runtime
files and declared resources are mounted. The SDK reads fonts from read-only
packaged assets rather than the shell state directory. Private tmp/proc/dev are
created. Other app data, system/session buses and unrelated inherited descriptors
are hidden. NoNewPrivileges and dropped capabilities prevent privilege escalation.
The child closes inherited descriptors on exec, then preserves only its assigned
SDK channel. SDK apps cannot create further user namespaces; native Browser can
create its existing nested sandbox.

Supported manifest capabilities:

| Capability | Resource |
|---|---|
| none | Private writable home/data; isolated network namespace |
| network | Host network namespace |
| storage | Local account's full home directory (Files compatibility) |
| audio | Sound devices and the existing audio service socket, when present |
| camera | Photo Booth only: the specific `/dev/videoN` character devices that probe as supported streaming-capture nodes; no `/dev` subtree or unrelated hardware is mounted |
| screen | Color Meter only: declares intent to use screen sampling; each read still requires the person's existing screen-recording authorization through the shell service |
| notifications | May post notifications through the shell |
| documents | May ask for files through the shell's open and save panels; binds nothing, each chosen file arrives as a descriptor ([file-portal-design.md](file-portal-design.md)) |
| keychain | May store secrets with the shell (`ToShell::Keychain`); binds nothing |
| accessibility | May post input events and install event taps through the shell's `events` service; the service checks the manifest on every call |
| display-config | May ask the shell's `display` service for another display mode |
| usb | May ask the shell's `usb` service for raw access to one USB device; binds nothing (no `/dev/bus/usb` reaches the sandbox). The person picks each device on the permission sheet, and the shell runs the transfers (see "USB and HID device access" in [bifrost-ui.md](bifrost-ui.md)) |
| hid | The same for a raw HID device (the shell reads and writes its hidraw node for the app); keyboards, keypads and security keys are never offered |
| gpu | For an SDK-drawn app whose engine renders offscreen on the GPU and reads frames back (the Browser's Servo engine). Binds `/dev/dri`, read-only `/sys/dev/char`, `/sys/devices` and `/run/udev`, and the Wayland socket (`WAYLAND_DISPLAY`, `WINIT_UNIX_BACKEND=wayland`) for the hidden window that yields the GL context. The window is still presented from SDK frames; a `native` app gets the same bindings (with all of `/sys`) without declaring it |
| remote-desktop | Fixed Remote sharing broker request, restricted to the Remote package and requiring network/storage |

Entitlements are separate from permissions and first-party only (`is.olibuijr.*`). `entitlements = ["log-store"]` on a
`system-app` mounts the system journal read-only so `LogStore::local()` works (the Console app); it is set by
`Policy::with_entitlements`, never from `permissions`.

`entitlements = ["system-monitor"]` on a first-party `system-app` (Activity Monitor) lets the app see the whole machine: the
sandbox keeps the host PID and network namespaces and binds the host `/proc` read-only, so the app lists every process and
`ss -tinp` reports the machine's sockets and interfaces. In bubblewrap terms (`app_policy::namespace_args`, `proc_args`):
`--unshare-all` is replaced by `--unshare-user --unshare-ipc --unshare-uts --unshare-cgroup-try` (no `--unshare-pid`, no
`--unshare-net`; bubblewrap has no `--share-pid`), and `--proc /proc` becomes `--ro-bind /proc /proc` (a user namespace cannot
mount procfs for a PID namespace it does not own). Everything else is unchanged: the user namespace, `--cap-drop ALL`, the
separate app account and the read-only mounts. Writes to other processes are no wider than before: Quit, Force Quit and Send
Signal call `kill(2)` as the app account, so they reach only processes that account owns; processes of other users and root
need the privileged-helper path, which this entitlement does not give. The shared network namespace means the app is no longer
cut off from the network, which is why only first-party system apps may hold it: `Policy::with_entitlements` honours it only
for an `is.olibuijr.*` id of kind `system-app`, and it is never read from `permissions`.

`entitlements = ["endpoint-security"]` on a first-party `system-app` lets the app subscribe to file open, exec, modify and
close events of the whole computer, and to allow or deny an open or exec before it happens (the `endpoint-security`
shell service, `bifrost_app::endpoint_security`). The sandbox gets nothing extra: the root half (`bifrost-privd`, fanotify,
`privd/endpoint_security.rs`) serves only the shell, and the shell service checks the manifest (`services::app_has_entitlement`)
on every call, so an app without the entitlement cannot subscribe and an app that stops running loses its subscription.
Only the path and process id of an event are passed on; an authorization nobody answers within two seconds is allowed.

The `camera` and `screen` capabilities are restricted to the Photo Booth and Color Meter package identities respectively; another package declaring either is rejected at launch. A `camera` declaration only exposes the host's enumerated V4L2 streaming-capture nodes matching supported formats, one node per bind. The `screen` declaration does not itself grant capture: `screen_sample` and window-image services require both this manifest capability and the person's authorized screen-recording status. A missing declaration or denied/not-yet-authorized consent fails closed.

Unsupported permissions fail launch. Storage currently grants the full home,
not individual user-selected files. Native applications additionally receive
their compositor socket, read-only session/hardware discovery metadata and GPU
devices; no D-Bus socket. The document picker exists for SDK apps (the `documents` permission, so an app need not hold `storage`);
Wayland privileged protocol filtering and the picker for native apps are future work. Unpackaged Arch
applications launched through the Linux application launcher are outside this
installed-package policy. Do not claim universal application isolation.

Remote 0.1.5 sends Start/Stop Sharing over its authenticated SDK socket. The shell
uses the host's assigned app ID and installed manifest, not a caller-supplied
identity, executable or unit arguments. One broker operation runs at a time on
an off-thread worker. A fixed systemd unit runs the packaged sharing server as
the local user with NoNewPrivileges, a read-only system and private temporary
storage; only its home is writable. The service can survive closing the UI.
The app never receives root, the system bus, or a generic command execution API.

Verification: focused permission/account and protocol tests; actual Midget
sandbox probes in `out/render-performance/isolation-live-probe.log` checked
UID 1000, zero capabilities, NoNewPrivileges, hidden system bus/other app data/home,
read-only system, writable app data, network denial without permission and
connectivity with permission. No audio was played.
