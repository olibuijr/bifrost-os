# Parity sdk-os-05: app-facing models over system capabilities

These rows were `os`: the system already did the work, and apps had no SDK type for it. Each row now has a typed
model or a service-facing value, tested headless. Nothing here opens a socket, a display or a device.

## What is built

| Row | Module | Tests | Wired | Not built |
|---|---|---|---|---|
| Cross-process shared framebuffers | `springboard/proto/src/sys.rs` (`shm_create`, `shm_len`, `shm_resize`) | `springboard/proto/tests/parity_sdk-os-05.rs` | Shell and proto use the same calls | No app-facing type wraps a shared file |
| Multi-plane video buffers | `springboard/ui/src/surface_layout.rs` | `springboard/ui/tests/parity_sdk-os-05.rs` | No; the decode path keeps its own planes | Apps do not receive planes |
| Window capture (covered window) | `springboard/ui/src/capture_filter.rs` | `springboard/ui/tests/parity_sdk-os-05.rs` | Agent tooling takes the pixels | No app pixel source |
| Multicast groups | `springboard/ui/src/multicast.rs` | `springboard/ui/tests/parity_sdk-os-05.rs` | No; the shell joins on its own socket | No socket behind the model |
| TLS and security options | `springboard/ui/src/tls_options.rs` | `springboard/ui/tests/parity_sdk-os-05.rs` | No; the shell client is fixed to rustls | No handshake uses the options |
| Immediate and hang diagnostic reports | `springboard/sdk/src/diagnostic_report.rs` | `springboard/sdk/tests/parity_sdk-os-05.rs` | The shell's parser reads the log text | No delivery to apps when a record is written |
| Zip archive creation and extraction | `springboard/sdk/src/archive.rs` (existing) | `springboard/sdk/tests/parity_sdk-os-05.rs` | Yes, through the archive module | Stream input and the reference stream format |

## What needs the lab machine

- Multicast: joining a group from the shell and checking that a datagram loops back needs Midget.
- TLS: applying `TlsOptions` to a live handshake needs a server; the shell's client still uses its own setup.
- Diagnostics: the shell writes the log only on panics and stalls, so delivery to an app needs a shell hook first.
- Shared buffers: the GPU path (dma-buf import) and descriptor passing into apps are not built here.

## Not in this round

- Passing a buffer to another process (descriptor passing into the SDK) and GPU-backed buffer import: both need a
  channel and a GPU context that the SDK does not own yet.
- The audio service, distributed invocation, the developer test rig, and the diagnostic upload path: these are
  system or developer tooling, not app APIs.
