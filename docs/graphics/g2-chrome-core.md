# G2: chrome core (`chrome-core`)

Part of the graphics core rebuild (docs/graphics-architecture.md, docs/graphics-waves.md row 18). Owner files:
`springboard/chrome/src/surface.rs`, `springboard/chrome/src/link.rs`, the codec functions and tests in
`springboard/chrome/src/model.rs`, and `springboard/src/shell/chrome_host/server.rs`. The shell loop hook is three
lines in `springboard/src/bin/shell.rs`.

## What is implemented

### model.rs: the codec

- `encode_to_chrome` / `decode_to_chrome` and `encode_from_chrome` / `decode_from_chrome` for every variant of
  `ToChrome` (MenuBar, Dock, Panels, Appearance, ShowPanel) and `FromChrome` (MenuAction, DockActivate,
  DockCommand, MinimizeTarget, ControlChanged, NotificationAction, Hello).
- Layout: a version byte (`VERSION` = 1), a variant tag byte, then the fields in declaration order. Strings are a
  u32 byte length and UTF-8 bytes; lists are a u32 count; floats are their little-endian bits; bools are 0 or 1.
- Hostile input: every read is bounds-checked and returns `DecodeError`. A list count may not exceed the bytes left
  (each element takes at least one byte), so a short message cannot make a large allocation. A string longer than
  `MAX_LEN` is `TooLong`, a message longer than `MAX_LEN` is refused before parsing, NaN floats, bools other than
  0 or 1, bad UTF-8, unknown tags and trailing bytes are `BadValue`/`UnknownTag`. Submenus nest at most `MAX_DEPTH`
  (8) levels deep.
- Tests (8): round trip for every variant of both enums, stable bytes for an empty model and a Hello, every
  truncation of every sample rejected, bad version, unknown tag, trailing bytes, hostile values (bool, NaN, UTF-8,
  dock position), huge length claims, and the submenu depth limit at and past the limit.

### link.rs: the socket

- `Link::connect(client)` binds its own socket path `bifrost-chrome-<client>-<pid>.sock` next to the host's, connects it
  to `socket_path()` and sends `Hello`. `Link::send`, `Link::recv` (every waiting message, non-blocking, undecodable
  packets dropped), `Link::fd`. Dropping a `Link` removes its path.
- `HostLink::bind()` binds `socket_path()` (replacing a stale file). `poll()` accepts new clients (any sender with a
  path becomes a `ClientId`, numbered in arrival order) and returns every waiting message with its sender.
  `send(to, m)`, `broadcast(m)`, `fds()` and `client_count()`. A client whose send fails (its path is gone) is
  forgotten; a full queue (`WouldBlock`) drops that one message and keeps the client. Dropping the host removes its
  path.
- Tests (7): the client path is one safe component, the socket path name, Hello then models round trip, broadcast
  reaches every client and a gone client is forgotten, garbage packets are dropped and the host keeps serving, a
  missing host is an error for the client, and bind replaces a stale file and removes its path on drop.

### surface.rs: the connection and the surfaces

- `Display::connect()` connects to `WAYLAND_DISPLAY` and binds `wl_compositor` (4..=6), `zwlr_layer_shell_v1` (4..=5),
  `bifrost_layer_manager_v1` (1), `zwp_linux_dmabuf_v1` (4..=5) and `wl_seat` (5..=9, optional). It opens the GPU
  (`bifrost_gpu::Device::open(None)`), makes a `bifrost_2d::Renderer` and a command queue, and roundtrips once. A
  missing global or GPU is an `Err` that names it, so the caller can fall back.
- `create` makes the wl_surface, the layer surface (size, anchor, exclusive zone, margins and keyboard mode from the
  spec), and the `bifrost_layer_v1` context for the surface; the first commit asks the compositor for a configure.
  `update` re-applies the spec and commits. `destroy` releases the surface, its context and its buffers.
- Configure is acked in `dispatch`, which then reports `Configured` and a `Frame` so the client draws at once.
  `Closed` removes the surface.
- `draw(id, list, damage)`: each surface has three slots. A slot is an exportable `bifrost_gpu` texture (BGRA8,
  linear modifier) wrapped as a linux-dmabuf `wl_buffer` through `create_immed`, tagged with (surface, slot). `draw`
  picks the first free slot from the last one used (`pick_slot`), renders the display list with `Renderer::draw`,
  commits the GPU work and waits for it, binds the slot to the root contents (`bind_contents`), commits the context
  with the transaction (first draw: `Create`, `SetRoot` and `SetContents` of the root layer, then no structural
  changes), and requests a frame callback. A slot is busy until its `wl_buffer.release`.
- `commit_layers(id, tx)` commits a transaction for the surface's tree (extra layers, animations).
- `dispatch(timeout_ms, extra_fd)`: dispatches what is queued, reads the Wayland socket without blocking, applies
  configures, closes and releases, and returns `Event`s. It returns early when there are events and sleeps in 1 ms
  steps otherwise.
- Pointer enter, motion, leave, button and scroll, and keyboard enter, leave and key are delivered per surface
  (`Event::PointerEnter` and the others) from the surface the pointer or keyboard is on.
- Tests (7, pure logic): `pixel_size` (scale and rounding, never zero), `pick_slot` (first free, wrap-around, all
  busy, empty), `slot_tag` round trip, anchor bits, layer mapping, and the first-draw transaction shape.

### server.rs: the shell side

- `ChromeServer::start()` binds the `HostLink`. `start_if_requested()` returns `None` unless
  `chrome_host::requested()` (`BIFROST_CHROME=1`), and logs and returns `None` when the socket cannot be bound, so the
  shell keeps its own chrome.
- `fds()` for the shell's poll set. `service()` answers every `Hello` with the current models (the latest of each
  kind, in first-arrival order) and returns every other message for the shell to act on. `publish(m)` broadcasts a
  model and keeps it for later clients (`ShowPanel` is broadcast only, because it is a command).
- `Snapshot` keeps one model per kind and replaces it in place; tests (7) cover the replacement rules, the command
  rule and the replay order.
- The shell loop (`springboard/src/bin/shell.rs`): one `start_if_requested()` before the loop, `server.fds()` appended
  to the poll set, and `server.service()` after each wait. Nothing else in the shell changes; the producers in
  `chrome_host/{menubar,dock,panels}.rs` are the other clients' work.

## How it maps to the contract

- One window server (`bifrost-wm`) composites everything; apps and chrome are clients that draw into dma-buf backing
  stores. `Display` is the chrome client's half of that: the Wayland connection, the layer-shell surfaces, the
  dma-buf backing stores drawn with `bifrost-2d`, and the `bifrost_layer_v1` tree the window server animates.
- The chrome clients and the shell speak the codec over the socket. The shell's chrome server is the host end; the
  shell keeps the models and the session.
- Old paths stay as they were: the shell draws its own chrome unless `BIFROST_CHROME=1`, and `Display::connect`
  fails cleanly when the compositor lacks layer shell or `bifrost_layer_manager_v1` (labwc has no layer-tree manager).

## Contract changes

1. The socket is `SOCK_DGRAM`, not `SOCK_SEQPACKET`. std has no seqpacket socket and this crate has no libc, so the
   safe std API is the only option. The framing is the same (one message per packet). Each client binds its own
   path and the host replies by the sender's path. The crate root (`chrome/src/lib.rs`) still says `SOCK_SEQPACKET`;
   the lead should correct that line.
2. `Link::send` may return `WouldBlock` when the host's queue is full; the caller may retry.
3. `dispatch` does not wait on `extra_fd`: see the limits below.

## What is left (todo_left)

- **Waiting on two fds.** `dispatch` sleeps in 1 ms steps, and `extra_fd` is not polled. Use `poll(2)` once
  the crate may depend on rustix or libc (the lead's call; no Cargo changes were made). Until then the chrome clients
  wake about every millisecond while idle.
- **Keys.** `Event::Key` carries the evdev code, not an xkb keysym. Wire `xkbcommon` (or the compositor's keymap)
  when the chrome clients need text input.
- **Scale.** `Event::Configured` reports scale 1. Read the output scale from `wl_output` (or
  `wp_fractional_scale`) before Retina-like displays are supported.
- **Modifiers.** Backing stores use the linear modifier. Use the compositor's dma-buf feedback (`get_default_feedback`,
  as the SDK does) to pick the modifiers the device and compositor share.
- **Explicit sync.** `draw` waits on the GPU before it commits (`Submission::wait`). Use `wp_linux_drm_syncobj_v1`
  with the sync file once the window server supports it (wave G2 row 4).
- **Root frame.** The first transaction sets the root layer's contents but not its bounds; the window server sizes
  the root to the surface today. Set the bounds explicitly once the layer model needs them.
- **Commit.** `draw` relies on the layer context commit to commit the wl_surface on the server. Confirm this on
  Midget; if the window server needs a separate `wl_surface.commit`, add it to `draw` and `commit_layers`.
- **Output and seat.** Layer surfaces use the default output. Multi-output placement is not done.
- **Shell handling.** `ChromeServer::service` returns the messages (`MenuAction`, `DockActivate`, `DockCommand`,
  `ControlChanged`, `NotificationAction`, `MinimizeTarget`) and the shell ignores them for now (`let _ =` in
  `shell.rs`). The shell wiring is the menu bar, Dock and panels owners' work.
- **Producers.** Nothing calls `ChromeServer::publish` yet; `chrome_host/{menubar,dock,panels}.rs` are the other
  clients' work.

## Verify

Host (no GPU, no compositor):

```
cd springboard
CARGO_TARGET_DIR=/home/olibuijr/Projects/bifrost-os/springboard/target-agents cargo test --offline -p bifrost-chrome model link surface
CARGO_TARGET_DIR=/home/olibuijr/Projects/bifrost-os/springboard/target-agents cargo test --offline -p springboard --features desktop-wayland chrome_host
```

The codec, link, pixel and slot tests need no compositor. `Display::connect` needs a compositor and the GPU, so it is
checked on Midget.

Midget (Intel HD 530, Mesa ANV, Vulkan 1.4; the session runs labwc unless `/etc/bifrost/compositor` says
`bifrost-wm`):

1. Check the chrome socket. With the shell started under `BIFROST_CHROME=1`, the file
   `$XDG_RUNTIME_DIR/bifrost-chrome.sock` exists while the shell runs and is removed when it exits.
2. Check the window server's layer manager. Under `bifrost-wm` with the Vulkan renderer
   (`BIFROST_WM_RENDERER=vulkan`), `wayland-info` lists `bifrost_layer_manager_v1` and `zwlr_layer_shell_v1`. Under
   labwc it does not; `Display::connect` then fails with `bifrost_layer_manager_v1: ...`, which is the expected
   fallback.
3. Client round trip: once a chrome client (`bifrost-menubar`, `bifrost-dock`, `bifrost-panels`) exists, run it with
   `BIFROST_CHROME=1` and check with `ls /run/user/$(id -u)/` that its `bifrost-chrome-<client>-<pid>.sock` appears
   and disappears with it, and that the shell's `service()` gets its `Hello` (add a log line first if needed).
4. Frames: `bifrost pc midget screen /tmp/...` (or the compositor's screenshot) should show the client's surface,
   and `bifrost-wm`'s frame stats should show the surface committed. The G2 bench (`tools/bench.py --midget`) is
   the owner of the frame budgets (row 19).
