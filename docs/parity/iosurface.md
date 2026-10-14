# IOSurface parity (sdk-rest-10)

## What it mirrors

The reference framework shares a buffer between processes by a port or an XPC object, and an app holds the buffer
as a typed surface. BifrOSt shares a buffer as one memfd descriptor passed with SCM_RIGHTS over a SOCK_SEQPACKET
channel.

## What is wired

- `springboard/sdk/src/buffer_handoff.rs`: `SharedBuffer` (create, byte_len, map), `send_buffer`, `receive_buffer`,
  and the typed `BufferError` (empty, too large, missing or extra descriptors, closed channel, system error).
- Tests: `springboard/sdk/tests/parity_sdk-rest-10.rs`, headless, on a socket pair made in the test process.

## What needs the lab machine

- A buffer passed from one app to another app. The shell owns the channel to each app, so the relay has to be
  added in the shell and checked on Midget with two running apps.
- GPU-backed buffers (dma-buf import into the compositor) stay in the shell. See the row note in
  `docs/parity-matrix/data/iosurface.json`.
