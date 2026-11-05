# BifrOSt MCP server

`bifrost-mcp` lets an agent (Claude Code, or any MCP client) see and drive a BifrOSt PC from inside the machine.
It is one Rust binary (`springboard/mcp`) speaking MCP over stdio (JSON-RPC 2.0, one message per line), run as root
on the machine and reached over a single ssh channel. Every call is a local syscall on Midget; nothing spawns a
helper process except `exec`.

## System service (lab installs only)

`bifrost-mcp.service` runs `bifrost-mcp --serve`. It is remote root exec for agents, so it exists only on lab machines:

- The unit has `ConditionPathExists=/etc/bifrost/lab-install`. The lab installer (the ISO that carries `/etc/bifrost/lab.token`)
  writes that marker; the `springboard-shell` scriptlet (`bifrost_mcp_policy`) enables and restarts the server where it exists
  and stops and disables it everywhere else. A machine the lab installer set up before the marker existed (it has
  `/etc/ssh/sshd_config.d/05-lab.conf`, written by no other path) gets the marker from the scriptlet on its next update, so
  Midget keeps it. A public install never runs it and has no `/run/bifrost-mcp.sock`.
- MCP Streamable HTTP at `http://<machine>:7420/mcp` (POST JSON-RPC, JSON replies, keep-alive; no server stream).
  Remote clients send `Authorization: Bearer <token>`; the token is made on first start in
  `/var/lib/bifrost-mcp/token` (0600). Every TCP request needs the token, loopback too (a sandboxed app with network could otherwise reach it); agents on the machine use the unix socket. Round trip from Titan: 1.2 ms for `observe`.
- It listens on `127.0.0.1` unless `BIFROST_MCP_LISTEN` says otherwise (reach it with `ssh -L 7420:127.0.0.1:7420`). The lab
  file `/etc/bifrost/mcp.env` (from `packaging/arch/mcp-lab.env`) sets `0.0.0.0` so the lab tools reach the VLAN, lab-NIC and
  Wi-Fi addresses, together with `BIFROST_MCP_ALLOW` (default and lab value: the private IPv4 ranges): a peer outside loopback
  and those ranges is dropped before it can send a byte. The traffic is still plain HTTP on a trusted lab network; narrow
  `BIFROST_MCP_ALLOW` to the lab ranges or set `BIFROST_MCP_LISTEN=127.0.0.1` for forwarding only.
- `/run/bifrost-mcp.sock` (root and the desktop account's group): newline-delimited JSON-RPC, for agents on the machine.
- All sessions share one context behind a lock, so input from several agents is applied in order.

Claude Code over HTTP (no ssh):

```json
{"mcpServers":{"bifrost":{"type":"http","url":"http://10.99.0.200:7420/mcp","headers":{"Authorization":"Bearer ${BIFROST_MCP_TOKEN}"}}}}
```

The server is `springboard/mcp` (crate `bifrost-mcp`, 0.5.x). See also: [the `browser` tool](bifrost-mcp-browser.md) for web debugging (console with file:line, network, DOM, storage, styles, page screenshots with `scale`, `inspect` by point), the `files`, `term` and `lifecycle` tools below, and [agent-api.md](agent-api.md) for the element map behind `ui`.

CLI shortcuts from Titan: `bifrost pc mcp call TOOL [JSON]`, `bench`, `eval`, `push`, `deploy-app APP` and `token`.

## Use it from Claude Code (stdio over ssh)

The repo's `.mcp.json` registers the server as `bifrost`. Its command is `tools/bifrost-mcp-ssh`, which finds Midget
(VLAN 20, lab NIC, Wi-Fi; `MIDGET_HOST` overrides) and runs the packaged `/usr/bin/bifrost-mcp`, or a newer dev build
in `/usr/local/bin`. In a session in this repo the tools appear as `mcp__bifrost__*`.

- Dev loop: edit `springboard/mcp/src/`, `tools/mcp-push.sh` (fast build, strip, install to `/usr/local/bin`), then
  restart the MCP server (`/mcp` in Claude Code), or test one call: `tools/bifrost-mcp-ssh call observe` (prints the
  time; images go to `/tmp/bifrost-mcp.png` on Midget).
- Release: `bifrost pc publish` builds and packages it with the shell.

## How it reaches the system

| Layer | Path | Used by |
|---|---|---|
| Compositor (`bifrost-wm`) | one persistent Wayland connection: `wlr-screencopy` (screen, regions), `ext-image-copy-capture` + `ext-foreign-toplevel-image-capture-source` (one window, even covered), `wlr-foreign-toplevel` (window list, activate, close, minimize, maximize, fullscreen), `wlr-virtual-pointer` (exact compositor pixels), `zwp-virtual-keyboard` (text, with a keymap generated per call, so any Unicode is typed whatever the layout) | `observe`, `screenshot`, `click`, `pointer`, `scroll`, `type`, `window` |
| Kernel | one persistent uinput keyboard, created on first use (400 ms once) | `key`: the shell's evdev shortcuts and every client see it like a real keyboard |
| Shell | control socket `/run/bifrost/shell-control.sock` (Unix socket, `src/bare/control.rs`; served to root and the installed account only, see [bifrost-pc-workflow.md](bifrost-pc-workflow.md); `bifrost-mcp` runs as root; `SPRINGBOARD_CONTROL_SOCKET` overrides the path) | `observe` (state), `ui` (accessibility map, `ax list/press`), `open`, `wait`, `shell` |
| Apps | each app's JSON-RPC control socket (`appdata/<id>/control.sock`, `bifrost_core::app_control_socket`; `sys::app_sockets` lists them), connected as the account's uid (SO_PEERCRED) | `app` (methods, `intent.*`, app state) |
| System | `/proc`, files, `bash`, `journalctl` | `system`, `file`, `exec` |

The connection reconnects by itself after a shell or compositor restart.

## Tools

`observe`, `screenshot`, `click`, `pointer` (hover, drag), `scroll`, `key`, `type`, `window`, `open`, `ui`, `app`, `wait`,
`exec`, `file`, `system`, `shell`, `batch`, `lifecycle`, the dev loop tools (`logs`, `crash`, `dev`, `perf`) and the per-app tools `browser`,
`term` and `files` (their schemas are appended in `tools.rs` `list()`). Section markers `BROWSER HOOK` in `tools.rs` dispatch and `list()` are where the `browser` tool plugs in. Descriptions in `springboard/mcp/src/tools.rs` (`list()`) are written for the
model; the evals below decide changes to them.

### `files` (Files app)

Drives and inspects the Files app over its control socket (`files.*` methods, `springboard/apps/files/src/control.rs`; the
tool is `springboard/mcp/src/files.rs`). It launches Files when it is not running. One tool, `action` picks:

| Action | Parameters | Result |
|---|---|---|
| `state` | | window path, view (icon, list, columns), sort, hidden, sidebar, status bar, selection, rows, busy job, notice, history |
| `list` | `path`, `limit`, `hidden`, `sort` | entries with name, path, kind, size, mtime, items, tags (no `path`: the rows on screen) |
| `select` | `names`, `paths`, `indices`, `all`, `none`, `add` | the selection |
| `open` | `path` or `index` | opens a folder (navigates) or a file (default app) |
| `navigate` | `path`, `back`, `forward`, `up`, `timeout_ms` | waits for the listing, returns the state |
| `set_view` | `view`, `sort`, `hidden`, `sidebar`, `status_bar` | the state |
| `new_folder` | `name`, `path` | the created path |
| `rename` | `path` (default the selection), `name` | old and new path |
| `move`, `copy` | `paths` (default the selection), `dest`, `timeout_ms` | waits for the job: `done`, `skipped`, `failed`, `cancelled` |
| `trash` | `paths`, `timeout_ms` | same report; items go to Recently Deleted |
| `undo` | | reverts the last rename, move, copy or new folder done through these methods (also Edit > Undo) |
| `search` | `query`, `timeout_ms` | waits for the hits under the open folder (empty query clears) |
| `get_info` | `path` | the Get Info rows and the entry |
| `tags` | `path`/`paths`, `set`, `add`, `remove`, `toggle` (colour names) | tags per path; no change parameter only reads |

Navigate, search, copy, move and trash are async: the app replies with the SDK pending marker (`RpcError::pending`) and
completes the call from `tick` (`control::complete`), as the Browser does. Errors: -32602 invalid parameters, -32003 busy
(another job runs), -32004 not found or nothing to undo, -32000 timeout. Trash is not undoable through `undo` yet; use Put Back.

Coordinates are screen pixels. A downscaled screenshot (default longest side 1280) reports `image_scale`; click with
`image_scale` and image coordinates. A window screenshot reports where the window sits on screen; `click` with
`window=NAME` takes that window's image coordinates. Wayland does not tell clients where windows are, so the server finds
the window's own pixels in a screen capture (exact match of textured 16x16 blocks; cached offset and centred placement
tried first). A covered window can be captured but not located; activate it first.

### Dev loop tools

| Tool | What it does |
|---|---|
| `logs` | Journal lines new since the previous call with the same filters (cursor per session and filter set, `--after-cursor`). Filters: `unit`, `priority` (`err`, `0..3`), `filter` text, `app=<id>`, `subsystem` and `category` (exact Unified Logging fields, any unit; 0.4.1); `follow_ms` waits for new lines; `reset` restarts at the tail. SDK apps are spawned by the shell (through bwrap, `apphost.rs`) and inherit its stderr, so their output is in the `springboard` unit with no id tag: `app` is that unit filtered on the id appearing in the line. |
| `crash` | `list`: `coredumpctl` since boot (or `since`) plus the tail of the shell's `crash.log`; `info` (pid or exe): identity header and a trimmed backtrace; `panic`: `<data dir>/crash.log` (panic hook in `src/bare/crashlog.rs`, rotated at 256 KB) and `last_exit`. |
| `dev` | `upload` (base64, `append` for chunks, `mode`, returns size and sha256 of the whole file), `install_package` (`springboard --install-package`), `install_store_app`, `restart` `shell` / `compositor` (`systemctl restart springboard`, as `tools/midget-push-shell.sh`; the window server `bifrost-wm` comes back with the session; waits for the control socket and reports the version) / `app:<id>` (SIGTERM, `open-app`, waits for the control socket) / `mcp` (delayed `systemctl restart bifrost-mcp`). A restarted shell is locked: `key enter`. |
| `perf` | Shell `perf` line, machine CPU, and CPU% per process over `ms` from `/proc/<pid>/stat` deltas. |
| `wait` `screen_changed` / `screen_stable` | Hashes cheap captures (screen, `region`, or `window`) every 40 ms; stable means unchanged for `stable_ms`. |
| `screenshot` `diff=true` | Adds the bounding box that changed since the previous capture of the same target. |

App accessibility needs no new tool: the shell caches each app's `ControlsTree` (`apphost.rs`) and serves it through
`ax list app/` and `ax press`, so `ui prefix=app/<id>` reads an SDK app's controls (as mapped by `docs/agent-api.md`,
"How apps appear") and `ui press=NAME` presses one. Apps that draw custom views and declare nothing show no elements.

Host side: `bifrost pc mcp deploy-app APP` (`tools/mcp-deploy-app.py`, wrapper `tools/mcp-push-app.sh`) packs the app
(`tools/pc-app.py pack`), uploads it over the HTTP endpoint in 3 MB chunks with a sha256 check, installs it, then restarts
(if running) or opens it.

### App control and debugging

| Tool | What it does |
|---|---|
| `app` | `describe=true` (or `method` omitted) lists methods with signatures via `app.methods {verbose:true}` (falls back to the plain list). A result shaped `{encoding:"base64",format:"png",data,width,height}` (also under an `image` key) is returned as a real image content block with the other fields as text. `file=<path>` saves any result (decoded bytes for an image, JSON text otherwise) and is the way to read results larger than `max_output` (default 100000 bytes). |
| `screenshot app=<id>` | Resolves the window by app id, unminimizes and activates it, retries up to 8 times (200 ms) until a frame is present. |
| `logs app=<id>` | The `springboard` unit lines naming the id (stderr), plus a second section with the app's Unified Logging subsystem (`BIFROST_SUBSYSTEM=<id>`). Cursors are per session and filter set and are dropped when more than 256 exist. |
| `crash app=<id>` | Process family and control socket state, coredumps naming the app (or its short name, so engine and content children that carry it are included), `crash.log` lines mentioning it, the shell's `last_exit` and recent warnings from the journal. |
| `perf app=<id>` | Only the app's process family (every process whose command line mentions the id, plus all descendants by parent pid) with summed RSS and PSS (`smaps_rollup`). |
| `observe` | Adds one line per app control socket: pids, RSS, PSS, and `cgroup_mem` when the app has a private cgroup (`cgroup=shared` when it sits in a `.service` cgroup). The focused window is the `*` row. |
| `window` | `action=list` (optional `window` or `app` filter), `app` as an alias for `window`. `resize` and `move` answer `E_UNSUPPORTED`: wlr-foreign-toplevel has neither request and `bifrost-wm` only answers activate, close and the maximize, minimize and fullscreen states. |

#### `wait` conditions on apps and the system

| Parameter | Waits until |
|---|---|
| `app_ready=<id>` | the app's control socket answers `app.info` |
| `app_method=<id>`, `method`, `params`, `path`, `equals` / `contains` / `matches` | the value at `path` (`a.b[0].c`) of the method result equals (JSON or text), contains the substring or matches the `*` glob; with none of the three, it is truthy |
| `log_line=<text>` (`unit`, `priority`, `subsystem`) | a journal line that arrives after the call contains the text |
| `process_exit=<pid or substring>` | no matching process is left |
| `app_exit=<id>` | the socket is gone or stops answering |

`timeout_s` defaults to 10 for these (max 300); a timeout error is `[E_TIMEOUT] ... last observed: <value>`.

#### Errors, caps, restarts

- Errors start with a stable code: `E_NO_APP`, `E_NO_WINDOW`, `E_NO_METHOD` (-32601), `E_BAD_PARAMS` (-32602), `E_TIMEOUT` (-32000
  or a deadline), `E_NO_TAB` (-32001), `E_ENGINE_DOWN` (-32002, with the app's `data.reason`), `E_APP_DOWN` (socket missing or
  silent), `E_BAD_ARGS`, `E_UNSUPPORTED`, `E_RPC` (other JSON-RPC error), `E_FAILED`.
- Any text block over 200000 bytes is cut with a note naming the remedies (`limit`, `max_output`, `file`).
- Per-tool deadlines: `app` 10 s (max 120), `wait` 5 s (app conditions 10 s, max 300), `exec` 30 s (max 600), `logs` `follow_ms` max 30 s.
- A shell or compositor restart is survived: the Wayland connection is rebuilt on the next call that needs it
  (`Ctx::wl`), and each app RPC opens a fresh socket connection. Verify with `dev restart target=shell`, then `observe`.

#### App conventions the MCP relies on

Apps answer JSON-RPC 2.0 on `appdata/<id>/control.sock` (SDK `sdk/src/control.rs`). The conventions:

- `app.info`, `app.methods` exist everywhere; `app.methods` may take `{verbose:true}` and return objects with a `name` and signature fields.
- **Image results**: `{encoding:"base64", format:"png"|"jpeg", data, width, height, ...}`. The `app` tool turns it into an image block;
  the browser's `screenshot` method (devtools-browser track) uses it, and the `browser` tool saves the file under `/tmp`.
- Errors use the JSON-RPC codes above; engine-backed methods put the cause in `error.data.reason`.
- Long-running methods should be answered asynchronously by the app; the MCP waits up to `timeout_s`.

### Term tool (`term`)

Drives and debugs Term (the terminal built into the shell, `src/shell/terminal/`). The MCP sends one line on the
shell control socket, `term <verb> key=value ...` (values percent-encoded; `bare/control.rs` `Request::Term`), and the
shell answers in `shell/terminal/control.rs`. Tab indexes are the ones `state` prints; `tab` defaults to the current tab.

| Action | Parameters | Result |
|---|---|---|
| `state` | none | JSON: `tabs`, `current`, `profile`, `profiles`, `zoom`, and `tab_list` with `title`, `cwd`, `job` (foreground process, empty at the prompt), `cols`, `rows`, `alive`, `unread`, `current` |
| `new_tab` | `cwd`, `command` | opens the Term window when it has no tab, starts a shell (`cd`s to `cwd`), types `command` and Enter; replies `ok tab=N` |
| `select_tab`, `close_tab` | `tab` | closing the last tab starts a fresh shell, as in the UI |
| `send_text` | `text`, `tab` | types the text; a newline is Enter. No trailing newline means the line is not run |
| `send_keys` | `keys` (string or list), `tab` | `Enter Tab Escape Backspace Up Down Left Right Home End PageUp PageDown Delete F1..F12`, `ctrl+<letter>`, `alt+<key>`, or one character; application cursor mode is honoured |
| `read_screen` | `tab`, `scrollback_lines` | `cursor: col C row R (0-based), size WxH`, then the screen text (trailing blanks trimmed per line) with that many history lines above it |
| `wait_for` | `text` or `regex`, `timeout_ms` (10000), `tab`, `scrollback_lines` | polls `read_screen` every 100 ms; returns the match and the screen, or an error with the last screen on timeout or shell exit |
| `resize` | `cols`, `rows`, `tab` | sets the grid and the PTY size; the next window resize re-fits it |
| `set_profile` | `name` (from `state`) or index | colour profile for all tabs; saved like the Settings choice |
| `clear` | `screen`, `tab` | clears scrollback; `screen=true` also sends Ctrl+L |

`wait_for` regex is a small built-in dialect (no regex crate in the MCP binary): literals, `.`, `[a-z]`, `[^x]`, `\d \s \w`,
`* + ?`, `^ $` per line, and `|`; groups and `{n}` are rejected. Matching is per line, case-sensitive. Typical loop:
`new_tab command="cargo test"`, `wait_for regex="test result|error"`, `read_screen scrollback_lines=200`.
Limits: the control line is 64 KiB, so very long `send_text` must be split; `read_screen` shows the live screen
(not a scrolled view); a shell that exited removes its tab.

## Benchmarks: `tools/mcp-bench.py`

One persistent session from Titan, like Claude Code: round-trip p50 per tool. `--baseline` also times the old paths
(`midget.py` over a new ssh per call); `--record LABEL` appends to `docs/perf-history.jsonl` (`mcp_*_ms`).

2026-10-10, server 0.2.0, Midget 2560x1440, Titan over the lab link:

| Case | p50 |
|---|---|
| session start (ssh + initialize) | 1.3 s, once |
| observe (windows + shell state) | 0.4 ms |
| shell version / system / app list | 0.5 / 0.6 / 0.9 ms |
| exec `true` | 3.2 ms |
| screenshot region 800x60 | 17 ms |
| screenshot screen at 1280 / full 2560 | 80 / 116 ms |
| screenshot window + locate | 80 ms (259 ms before hints) |
| old `midget.py state` / `screen` | 1215 / 2744 ms |

## Evals: `tools/mcp-eval.py`

Tasks in `tools/mcp-evals.json` run through `claude -p` with only the bifrost tools allowed (built-in tools blocked,
no settings, no session), then are checked against Midget's real state over a separate MCP session (window open or
minimized, app data, file contents, answer against `ps`/`date`/`shell version`). Each run records pass, turns, tool
calls by name, tool errors, wall time and cost; `--record LABEL` appends to `docs/mcp-evals.jsonl` and keeps the
transcripts in `out/mcp-evals/LABEL/`. Read failing transcripts and fix the tool (description, defaults, errors that
say what to do next), then run again.

```
tools/mcp-eval.py --record rN-haiku               # all tasks, Haiku 5.5 (cheap, finds unclear tools fastest)
tools/mcp-eval.py --model claude-sonnet-5-5 --only calculator,settings-displays
```

Round history (see `docs/mcp-evals.jsonl`):

| Round | Pass | Mean calls | Mean wall | Tool errors | What changed after it |
|---|---|---|---|---|---|
| r0 (0.1.0) | 7/10 | 6.6 | 14.2 s | 10 | `open` takes app names; `key "+"` types; longer click; `system` says memory; `open` waits for any new window; window locate hints; checker ignores digit grouping |
| r1 (0.2.0) | 9/10 | 6.8 | 12.9 s | 6 | only failure is the shell's Spotlight; calculator still 29 calls of clicking (next: a key-entry hint or app control methods) |

Known OS failure the evals exposed (not an MCP bug): in r0 and r1 (2026-10-10) Cmd+Space did not open the search panel. The compositor has since taken the global chords (Super+Space sends `quicksearch` to the shell, `springboard/wm/src/keybindings.rs`); only r0 and r1 are recorded, so re-run the `spotlight-clock` task before relying on it.

## Safety

The server runs as root and allows everything (owner decision for Midget, the disposable lab machine). Do not register
it against a machine with data that matters.

### Lifecycle tool (restart, netboot, unattended reinstall)

Shipped in the package from MCP 0.5.0 (0.5.10 release).

`lifecycle` (`springboard/mcp/src/lifecycle.rs`) controls the machine's own boot. Every action returns a job id at once
and the machine goes down 3 s later (a `systemd-run` timer), so the caller reconnects instead of waiting on the call.

| action | does | guard |
|---|---|---|
| `status` | lab flag, uptime, version, first `efibootmgr` lines, pending preset, job ids | none |
| `restart` | `systemctl reboot` | none |
| `netboot` | `efibootmgr -n <PXE entry>`, reboot into the lab ISO | lab hardware + `confirm="NETBOOT <hostname>"` |
| `reinstall` | writes the unattended preset to `/boot/EFI/BIFROST/unattended.toml`, BootNext = PXE, reboot | lab hardware + `confirm="REINSTALL <hostname>"` |

Safety guard: `netboot` and `reinstall` refuse unless the machine is the lab machine, identified by hardware, not by a
file the last install could have dropped: DMI `product_name` = `10FLS2WK00` (ThinkCentre M900) and the onboard NIC
`00:23:24:dc:c8:40` (the same MAC `tools/midget-wipe.py` checks). The confirm phrase is the action in capitals plus the
machine's own hostname. The installer repeats the check against the disk id (see `docs/installer.md`). Job records live in
`/var/lib/bifrost-mcp/jobs/`; they vanish with a reinstall, so after one the evidence is the new boot (`lifecycle status`,
`system overview` uptime), which `tools/midget.py reinstall` follows. Without a session: `bifrost-mcp call lifecycle '{"action":"status"}'`.
