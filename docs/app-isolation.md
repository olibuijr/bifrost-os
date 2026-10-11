# Desktop application isolation

Shell 0.4.19 centrally launches installed PC SDK and native packages through
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
| remote-desktop | Fixed Remote sharing broker request, restricted to the Remote package and requiring network/storage |

Unsupported permissions fail launch. Storage currently grants the full home,
not individual user-selected files. Native applications additionally receive
their compositor socket, read-only session/hardware discovery metadata and GPU
devices; no D-Bus socket. Wayland privileged
protocol filtering and a document-picker portal are future work. Unpackaged Arch
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
