# Parity sdk-complete-34

Wave w8, task sdk-complete-34. Finishes the missing behaviour of the game-controller, symbol, scene-kit animation and
network rows. The transferable, prediction, web-view and token rows of this task are not changed here (see the task report).
Tests live in `springboard/sdk/tests/parity_sdk-complete-34.rs`.

## Modules added

| Module | Mirrors | What is wired | Needs the lab machine |
|---|---|---|---|
| `bifrost_app::gamepad_input` | the extended controller profile and its input events | re-exports `bifrost_ui::gamepad`; `describe` gives one line per change | reading a real controller on Midget |
| `bifrost_app::symbol_draw` | `SymbolRenderingMode`, symbol weight and scale | `SymbolStyle` gives stroke, size and layer colour; `paint_layer` paints one layer | visual check of the named icons on Midget |
| `bifrost_app::view::action` | view actions (move, rotate, scale, fade, wait, sequence, group, repeat) | pure poses over time | the window does not run action poses yet |
| `bifrost_app::http_log` | one-line request summaries for the log | `Exchange`, `redact_url`, `body_size`, outcome-based levels | the transport does not call it yet |
| `bifrost_app::url_configuration` | the session configuration and its cache policy | `prepare`, `CachePolicy::action`, `resource_expired`, `connection_limit` | the transport does not read it yet |

## Rules the modules hold

- Symbol hierarchy uses the opacity steps of `bifrost_ui::symbol_layers` (1.0, 0.5, 0.3 by rank).
- Action composition: offsets and turns add, scales multiply, and the later fade sets the opacity. A fade starts from fully opaque.
- Logged URLs never carry a credential: query values of sensitive names and passwords in the authority are replaced, and fragments are dropped.
- A session caps a request's timeout and fills in headers the request does not set; it never raises a timeout.
