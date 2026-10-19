# Embedding patterns adopted

Reviewed 2026-10-19 against the browser's pinned Servo 0.7.0 source. These are architecture references;
the implementation uses the existing SDK and native Servo APIs.

| Reference | Pattern | Bifrost implementation |
| --- | --- | --- |
| [Official minimal embedder, v0.7.0](https://github.com/servo/servo/blob/v0.7.0/components/servo/examples/winit_minimal.rs) | Owning event loop, delegate-driven painting, native input and logging | `engine/src/event_loop.rs`, `delegate.rs`, `rendering.rs`, `input.rs`; startup calls `Servo::setup_logging` |
| [Servo GTK runner](https://github.com/nacho/servo-gtk/blob/main/servo_gtk/servo_runner.rs) | Separate UI and engine lifetime, correlated messages | Existing `WebProcessPool` remains the process owner; `webview_dev` provides typed memory/preferences requests and replies |
| [Hermes browser core](https://github.com/KaykCaputo/hermes-browser/tree/master/crates/talaria-core) | Commands/events and browser state independent of engine types | Existing `control::Req`, `WebViewEvent` and `DevEvent` retain that boundary; diagnostics follow the same path without importing Servo into browser UI code |
| [retsurf memory policy](https://github.com/mxmgorin/retsurf/blob/main/src/browser/memory.rs) | Isolate memory policy from startup, features and rendering | `engine/src/memory.rs` contains subsystem budgets; `configuration.rs` contains startup options, feature flags and UA |

The references use different engine revisions: Hermes documents Servo 0.4.0, GTK pins a Git revision, and
retsurf pins a Servo fork. No numeric GC profile or missing API was adopted merely because another browser uses it.
In particular, pinned Servo 0.7.0 warns that incremental GC can corrupt memory; its default remains unchanged.

## Lifetime and input changes

- The event loop owns command delivery, wake coalescing and frame scheduling. Static pages block on work;
  animations use the existing 16 ms deadline. Each turn processes at most 128 commands before spinning Servo.
- The reader retains at most 256 KiB plus a boundary byte. It drains oversized lines and rejects the entire
  command, so an oversized script cannot execute as a truncated prefix.
- Closing a view releases its pending controls and cancels its held navigation decisions. Other tabs keep their
  decisions. Shutdown explicitly denies pending navigation: Servo's default navigation responder drop allows it.
- Normal desktop input and developer pointer input both use native mouse events. Wheel input uses native
  `WheelEvent`, preserving page handlers and `preventDefault`, rather than bypassing those handlers through
  a direct scroll request.

## Native diagnostics

`browser.memory` requests `Servo::create_memory_report`. `browser.engine_preferences` reads the effective native
preferences. Both use correlated typed SDK events, support `timeout_ms`, and inspect the engine without reviving
discarded tabs. Memory report categories overlap; they are allocation diagnostics, not a sum representing PSS.

The matched navigation experiment still measured approximately 640 MiB PSS before and 641 MiB after the
history/heap/cache configuration changes. Earlier collection thresholds and explicit diagnostic collection did
not improve that workload. No whole-process memory reduction is claimed. Background page throttling remains
unavailable through the pinned public API; hiding a view suppresses presentation only.

## Build review

The lead reviewed all three agent changes, including native responder defaults, JSON escape handling,
engine-wide request lifetime, command boundaries and the wake rearm order. The optimized engine and browser/SDK builds pass. The local launch reports hardware rendering on Intel UHD Graphics 630.
Details are recorded in the accompanying [Servo review](servo-review.md).

## Hardware rendering

GPU rendering is the default, with an explicit CPU fallback in Browser Settings. The public pinned [rendering context source](https://github.com/servo/servo/blob/v0.7.0/components/shared/paint/rendering_context.rs) distinguishes the hardware window context from the software adapter. The browser keeps a hidden native window alive and renders into Servo’s public offscreen context. `_bifrost_renderer` in the engine preferences diagnostic reports mode and actual driver; known software GL drivers are rejected in GPU mode. Shared-memory readback remains a performance limit.
