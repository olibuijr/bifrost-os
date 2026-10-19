# Servo integration review

Reviewed against the pinned Servo **0.7.0** source. The public Rust documentation can track a newer revision;
check signatures against the source before adopting an API.

## Component boundaries

The browser owns window chrome and browser policy. The SDK owns transport and web-view state. The separate
engine process owns Servo objects. Moving a function into another component does not introduce another
thread, message queue, copy of browser state, or rendering loop.

| Component | Responsibility |
| --- | --- |
| `src/main.rs` | Shared browser model, identifiers and entry point |
| `src/app.rs` | SDK `App` implementation and construction |
| `src/lifecycle.rs` | Engine startup, window configuration, timers, suspension and shutdown |
| `src/navigation.rs` | Tab selection, navigation and address state |
| `src/input.rs` | Dispatch input to the active chrome control or page |
| `src/keyboard.rs` | Keyboard shortcuts and standard edit commands |
| `src/pointer_input.rs` | Pointer, touch, hover and wheel handling |
| `src/layout.rs` | Geometry and accessibility controls |
| `src/drawing.rs` | Drawing browser chrome and page content |
| `src/actions.rs` | Toolbar and preferences actions |
| `src/menus.rs` | Application menu declarations and dispatch |
| `src/events.rs` | Engine and system-service callbacks |
| `src/engine_loader.rs` | Locate and extract the pinned engine archive |
| `src/profile_data.rs` | Offline removal of storage outside the public Servo API |
| `engine/src/main.rs` | Engine model and wire-protocol reference |
| `engine/src/runtime.rs` | Servo construction, bounded event loop and process lifetime |
| `engine/src/commands.rs` | App protocol dispatch to Servo APIs |
| `engine/src/input.rs` | Translate input into native Servo events |
| `engine/src/delegate.rs` | Servo delegate callbacks and clipboard delegate |
| `engine/src/events.rs` | Translate delegate notifications into protocol events |
| `engine/src/rendering.rs` | Resize and shared-memory frame presentation |
| `engine/src/graphics.rs` | Hardware GL context ownership and explicit software renderer selection |
| `engine/src/favicon.rs` | Encode Servo's decoded favicon pixels |
| `engine/src/find.rs`, `find.js` | Bounded DOM search where Servo has no public find API |

Methods shared between these modules are visible only within the browser or engine crate. Existing feature
modules (downloads, session, inspector, reader, library and others) retain their own state. `Browser` and
`Engine` each have one owner; the split does not add a service layer or shared mutable globals.

## Native APIs adopted

- `WebViewDelegate::notify_favicon_changed` and `WebView::favicon` replace custom favicon discovery,
  fetching and image-format parsing. The browser retains only display/cache duties.
- `ClipboardDelegate` and native input events implement page clipboard and mouse/hover/wheel behavior.
- `WebView::clear_session_history` clears native history as well as browser history.
- `SiteDataManager` clears supported site storage and cookies; `NetworkManager` clears the HTTP cache.
  Unsupported persistent storage requires stopping the engine before resetting its profile.
- `Servo::set_preference` updates the user agent at runtime.
- Native `IntersectionObserver` and `ResizeObserver` replace injected polling implementations. Intersection
  Observer requires an explicit preference in this release and remains an experimental upstream feature.
- Privileged `cookies_for_url` supplies download cookie snapshots. Evaluating JavaScript no longer bypasses
  the web platform's HttpOnly cookie restrictions.
- The `url` crate parses origins and hosts; `serde_json` serializes developer-tool values.

## Memory and scheduling

Servo normally retains 20 live documents in each direction of session history. `session_history_max_length` is 3
(`engine/src/memory.rs`): navigation entries are all retained, the documents of the three nearest entries on each side
stay alive and a farther entry reloads when it is reached. The distance matters for Back and Forward: a live
document returns as it was, scroll offset included, while Servo 0.7.0 has no scroll restoration
(`history.scrollRestoration` is commented out of its `History.webidl`), so a reloaded entry starts at the top. At 0, which
the engine used before, every entry was evicted and Back always landed at the top of the page (the end-to-end test of
2026-10-19 on bifrost.olibuijr.com, finding B5); the ten-navigation workload showed no whole-process memory gain
from it. An entry beyond the distance (or a discarded tab, `src/tabdiscard.rs`, which keeps only the scroll of its
current page) comes back at the top. The JavaScript heap preference is 255 MiB: the pinned implementation treats the
upper bound, 256, as out of range and falls back to an unlimited heap. This is not a limit on the whole engine process
or all tabs combined.

The response cache is reduced to 256 weighted entries (not bytes). Engine commands use a bounded queue,
Servo wakeups are coalesced, and command work per event-loop turn is bounded. Hidden windows hide the native
web view as well as stopping frame presentation. Existing frame acknowledgments continue to permit only one
shared-memory frame in flight.

The wake flag is rearmed after draining commands and before spinning Servo, so a consumed wake cannot
suppress the next notification. An offline website-data clear restores browser tab state even if filesystem
removal fails, and reports that failure; a stopped engine cannot make the clear silently succeed without work.

## Remaining gaps

- Servo 0.7.0 has no public Stop-load API. The toolbar presents Reload; the compatibility protocol Stop command
  remains a documented no-op.
- Public storage APIs do not cover IndexedDB, CacheStorage or the authentication cache. Full removal uses
  an offline profile reset, preserving browser content rules.
- Find uses DOM TreeWalker/Range/Selection because there is no public native find API or `window.find`.
  Search is bounded and does not cross iframe or shadow-root boundaries.
- Downloads still require an external HTTP client because the public resource interception API does not
  expose the actual response body. Cookie snapshots are restricted to the exact origin and curl strips the
  explicit Cookie header on cross-origin redirects. This does not provide complete Servo network-policy parity.
- Native observer availability does not establish full web-platform conformance. No full WPT result is claimed.
- `WebView::hide` / `show` control visibility within the rendering context. Servo 0.7.0 has no public
  page-visibility or background-throttling setter: a hidden page can still report `document.visibilityState`
  as `visible`, and its JavaScript can continue. Suspension suppresses frame presentation; it does not claim
  browser-standard background-page throttling.
- CSS `backdrop-filter` is not implemented (end-to-end test finding B10: the sticky header of bifrost.olibuijr.com,
  `backdrop-filter: saturate(180%) blur(20px)`, shows the page behind it unblurred). Read in Servo 0.7.0 and its
  `stylo` 0.22: the property is declared with `servo_pref = "layout.unimplemented"`, so it is parsed only while the
  `layout_unimplemented` preference is on (off by default) and `@supports (backdrop-filter: blur(1px))` is false;
  and neither `components/layout` nor `components/paint` ever emits a backdrop filter, although the bundled
  WebRender (0.70) has `DisplayListBuilder::push_backdrop_filter`. The embedder cannot switch it on:
  `Preferences::layout_unimplemented` would only make the declaration parse, and would also enable every other
  unimplemented property, with nothing drawn for this one. The engine is the unmodified release, so nothing is patched
  here. What it would take: Servo layout builds the stacking context's backdrop filter list into the display list
  (`push_backdrop_filter` with the border-box clip, after the content behind it is painted), stylo enables the
  property for Servo (`layout.css.backdrop-filter.enabled`), and the engine pin (`engine.json`) moves to a release
  that has both; then re-check the compat page list in the README. `filter` itself is supported. Until then a site
  can keep its header legible with `@supports not (backdrop-filter: blur(1px))` and an opaque or near-opaque
  background (that query is false here); bifrost.olibuijr.com does not do this yet.

## Official references

- [Embedding overview](https://book.servo.org/embedding/overview.html)
- [Servo public API](https://doc.servo.org/servo/)
- [WebView](https://doc.servo.org/servo/struct.WebView.html)
- [WebViewDelegate](https://doc.servo.org/servo/trait.WebViewDelegate.html)
- [ClipboardDelegate](https://doc.servo.org/servo/trait.ClipboardDelegate.html)
- [SiteDataManager](https://doc.servo.org/servo/struct.SiteDataManager.html)
- [Pinned release](https://github.com/servo/servo/releases/tag/v0.7.0)

The local `servo-skill` supplies an index of official Book chapters and public API entries. The pinned source
under `out/servo-source/servo-0.7.0` is the authority for this build.

## Validation status

Source decomposition is complete. All 57 browser/engine Rust modules parse successfully; the find script
passes `node --check`, and `git diff --check` passes.

The refactored browser passes `cargo +stable build --offline --profile fast -p bifrost-browser -j 2`
(optimized build, 1m 57s). The subsequent native integration engine contract run passed 10 checks on the CPU path.
Those earlier checks do not validate the new hardware renderer.

`cargo +stable test --offline --profile fast -p bifrost-browser --bin bifrost-browser -j 2` passes:
**110 passed, 0 failed**.

`tools/pc-app-smoke.py browser --sizes 1200x780 --scale 1.5625` passes in Dark and with `--light`.
Both screenshots were inspected: the standard toolbar renders in each appearance. These are isolated host
window checks using the previously packaged engine; they do not validate the rebuilt Servo implementation.

The hardware rendering engine build passes (`tools/build-webview-engine.sh 0.7.0-bifrost.14`, optimized, 12m 10s).
The browser optimized `fast` build passes (1m 27s). The local Wayland launch restored the saved mbl.is tab,
loaded successfully with zero engine restarts, and `browser.engine_preferences` reports
`_bifrost_renderer = {"mode":"gpu","driver":"Mesa Intel(R) UHD Graphics 630 (CFL GT2)"}`.
Engine pin: `servo-0.7.0-bifrost.14-e3ede1066413`. No new test suite was run in this GPU round.
The CPU fallback selection and restart path have compiled and been reviewed; switching that setting has not
been exercised at runtime in this round.

## Rendering policy

Hardware GL is the default. The CPU fallback is an explicit browser preference (`BIFROST_BROWSER_RENDERER=cpu` at the engine boundary). GPU mode rejects known software GL drivers instead of reporting them as hardware acceleration. Servo uses its public `WindowRenderingContext::offscreen_context` API; the hidden native window and display outlive every rendering-context reference.

The SDK frame contract is premultiplied RGBA8, numeric `0xAABBGGRR` on this little-endian target. Servo RGBA readback keeps red and blue in their original order. Frames still pass through CPU readback and shared memory; hardware rasterization does not make this transport zero copy.

## Scrolling frame ownership (2026-10-19)

The SDK previously acknowledged a shared frame in `draw` or at the next `poll`, then continued drawing
from the same mapping during later chrome/input redraws. The engine could overwrite those pixels at the
same time, exposing partial frames. The SDK now snapshots a complete frame while the engine is waiting
for acknowledgment, releases it only after that copy, and draws from the owned snapshot. A single reusable
viewport-sized allocation holds the snapshot; no frame history is retained.

This follows [Servo's rendering model](https://doc.servo.org/servo/struct.WebView.html#rendering-model):
new-frame notifications lead to painting and presentation of a complete frame. GPU rendering remains enabled.
The local GTK host presents the latest complete texture on its display clock and uses the reported scroll
unit: surface pixel deltas pass through, while wheel notches are converted to pixels. See
[GTK scroll units](https://docs.gtk.org/gtk4/method.EventControllerScroll.get_unit.html).

The scrolling fix builds successfully with the optimized `fast` profile (2m 01s). The restarted browser
loads the restored mbl.is tab with zero engine restarts; native diagnostics still report GPU mode on Intel
UHD Graphics 630. No automated tests or frame-time benchmark were run in this round; the specific visual
blinking symptom still needs confirmation during interactive scrolling. The local host changes are in
`~/Work/bifrost-browser/run-browser.py`, outside the repository.

## Local scrolling sensitivity and main menu

The local host now applies a separate 3× sensitivity gain after converting the GTK scroll unit, instead of
using raw surface pixel deltas with no gain. Touchpad momentum uses GTK's `KINETIC`/`decelerate` signal
(velocity in pixels/ms), integrated with exponential decay on the existing display clock. New gestures,
clicks and keys cancel momentum; idle pages acquire no additional timer. This is local host behavior in
`~/Work/bifrost-browser/run-browser.py`, not an engine preference or a native window-server change.

The top-right main menu reuses SDK menu rows, hit testing, keyboard navigation and accessibility controls.
It provides common browsing commands and Settings; page and developer actions dispatch through existing
browser handlers. The platform menu bar remains available in the OS host.

Version 0.15.1 builds with the optimized `fast` profile (17.50s). The live local host menu was inspected in
`~/Work/bifrost-browser/main-menu.png`; keyboard navigation opened Settings, captured in
`menu-settings.png`. Alt+F also opens the menu with the local host's symbolic shortcut mapping, captured in
`main-menu-alt-f.png`. F10 works as well. The menu uses shared SDK rows and named type/color tokens,
publishes accessibility items, and keeps Settings visible at the current window size. Only the current dark
appearance was inspected; no automated tests or scrolling frame-time benchmark were run in this round.

### Scroll overshoot correction

The user reported scrolling to the bottom after the local host's momentum change. Removed KINETIC,
the deceleration callback, stored velocity and display-clock integration entirely. The host now forwards
only incoming scroll events. Touchpad sensitivity is 2×; wheel conversion remains 40 pixels per notch,
without applying the touchpad multiplier to it. The prior 3× gain and generated momentum are superseded.
No claim of interactive smoothness is made until confirmed on the user's trackpad.

## Native controls and page feedback (0.16.0)

Reviewed the official [WebViewDelegate API](https://doc.servo.org/servo/trait.WebViewDelegate.html),
[FilePicker](https://doc.servo.org/servo/struct.FilePicker.html),
[ColorPicker](https://doc.servo.org/servo/struct.ColorPicker.html), and
[AllowOrDenyRequest](https://doc.servo.org/servo/struct.AllowOrDenyRequest.html). Hosted pages returned mixed
versions (FilePicker was labeled 0.6.0); the implementation follows the resolved
[v0.7.0 delegate source](https://github.com/servo/servo/blob/v0.7.0/components/servo/webview_delegate.rs).

- `FilePicker` uses native filter patterns, multiple selection, select/submit and dismiss. The SDK document
  portal grants only selected file descriptors. Browser `uploads.rs` streams private disk copies on the utility
  queue because the engine API requires paths; it preserves names without loading entire files into RAM.
  Limits are 64 files and 1 GiB per selection, 2 GiB retained per window. Copies live until navigation commits,
  tab close or normal exit; selected input files can remain referenced by page JavaScript throughout the document.
  The local GTK host adds the same portal protocol using asynchronous
  [Gtk.FileDialog](https://docs.gtk.org/gtk4/class.FileDialog.html). That adapter remains outside the repo in
  `~/Work/bifrost-browser/file_portal.py` and `run-browser.py`. Copies are now tracked in `tools/file_portal.py`,
  `tools/local-host.py` and `tools/run-local`, with configurable repository/work paths.
- `ColorPicker` uses Servo's actual selected RGB value and native submit path. Browser controls reuse SDK
  sliders, buttons and the shared ColorPanel model, with hex entry and a swatch.
- `request_unload` uses an origin-attributed browser message with Stay initially selected. Pending requests are
  bounded and explicit denial on retirement overrides Servo's permissive Drop default. This integrates requests
  emitted during navigation; directly closing tabs or quitting does not initiate unload confirmation.
- `notify_status_text_changed` updates typed SDK state for the hovered destination; drawing uses that native
  value without injecting scripts or making layout queries on pointer motion.

Responses are checked against view/token/control kind before consuming native requests. Withdrawn controls
close their UI. Path lists use JSON rather than delimiter splitting, preserving Unicode and escaped newlines.
Rendering, wheel scaling and the current GPU policy are unchanged in this round.

The optimized engine build passed in 11.44s; the optimized browser/SDK build passed in 1m 16s. The local launch
reports version 0.16.0, engine `servo-0.7.0-bifrost.15-5e041ae3e32a`, hardware rendering on Intel UHD Graphics 630,
and zero engine restarts. Manual inspection confirmed the color picker opened and applying `#cc8844` changed
the native input value. The GTK file chooser opened with the requested multiple-selection mode. File reading,
unload choices and hovered-link presentation have not yet been exercised end to end; no automated tests ran
in this round. Tab close/quit confirmation, full IME composition and device validation remain gaps.
