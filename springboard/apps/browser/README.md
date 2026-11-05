# BifrOSt Browser

Independent Linux desktop App Store app (`is.olibuijr.browser.desktop`), built with `tools/pc-app.py build|publish`.

Since 0.3.0 the Browser is an ordinary Golden Gate SDK app (`window = "full-size"`): the shell draws the traffic
lights, the app draws the unified toolbar with the kit (`toolbar_band`, `toolbar`, `search_field`) and a
Safari-style tab strip that appears only with two or more tabs. Pages render through the SDK web view
(`bifrost_app::webview`, see docs/bifrost-ui.md "Web view") in the engine process `bifrost-webview`.

## Engine

`engine/` is the engine crate: libservo 0.7.0 with a hardware GL context and an explicit CPU fallback. It is built as one extra
workspace member of an unmodified Servo v0.7.0 checkout:

```sh
bifrost pc browser-engine 0.7.0-bifrost.16   # build (tools/build-webview-engine.sh), then pin engine.json and ENGINE
```

The command writes the archive's SHA-256 to `engine.json` and its first 12 hex digits to `ENGINE` (`src/main.rs`). `tools/pc-app.py` bundles the archive as `assets/servo.tar.gz`; the first launch unpacks it to
`~/.cache/bifrost-browser/<ENGINE>/`. The profile (cookies, storage) is `~/.local/share/bifrost-browser/profile`.
The app runs in the shell's SDK sandbox (`network`, `storage`); Servo's content processes run inside it.

Engine builds use the release profile with LTO off, 32 codegen units, incremental on, debug off, lld. A first
build took about 16 minutes on Titan; an engine-only rebuild about 10 seconds.

## Run on a local Linux desktop

`tools/run-local` hosts the built browser with GTK4 outside the BifrOSt shell. It requires GTK4 4.10 or newer,
Python GObject bindings, and a systemd user session. It runs `springboard/target/fast/bifrost-browser`, with the
pinned engine archive available as `springboard/target/fast/assets/servo.tar.gz`. The launcher limits the process
tree to 2 GiB of RAM and disables swap; this is a resource cap, not proof of lower memory usage.

State and logs default to `~/Work/bifrost-browser`; `BIFROST_BROWSER_WORK`, `BIFROST_BROWSER_REPO` and
`BIFROST_LOCAL_BINARY` override the work directory, repository and executable. The host presents complete frames
on GTK's display clock. Wheel input uses 40 pixels per notch; surface input uses 2× sensitivity, with no synthetic
momentum. F10 and Alt+F open the main menu. The native file chooser uses the SDK document portal protocol.

## Start Page (startpage.rs, starttab.rs)

A blank tab (`about:blank`) shows the Start Page: Favorites (the bookmarks), Frequently Visited and Recently Viewed (from history), drawn
natively over the page area, so the tab keeps its blank-page address, history and session handling. Tiles show the site icon already
cached by `favicon.rs`, else a monogram on a colour taken from the host; nothing is fetched. The data and layout are pure functions with unit
tests in `startpage.rs`. There is no Privacy Report: the Browser does not count blocked trackers.

## Develop (devmenu.rs)

Cmd+Option+U View Source (curl fetch, line-numbered `data:` document in a new tab), Cmd+Shift+R Reload From Origin,
Cmd+Option+R Responsive Design Mode (iPhone 15 393x852 @2x, letterboxed on DETAIL_BG with a size readout; presets,
custom WxH and DPR 1/2/3 are in `DevMenu`, menu UI follows). User agent presets live in `DevMenu` and the engine
accepts `ua` through `Servo::set_preference`, so subsequent requests use the new value without restarting.
Reload From Origin clears Servo's HTTP cache before reloading; the API cannot target only one origin.
http pages show "Not Secure".

## Web compatibility

The engine uses native `IntersectionObserver` (explicitly enabled) and `ResizeObserver`. Polling observer
approximations and injected platform shims have been removed. `requestIdleCallback` is not exposed by this Servo
release; sites must feature-test it and use their own fallback. CJK and emoji need the runtime fonts listed in
`packaging/arch/runtime-packages.txt`.

CJK punctuation: the engine's font fallback list is fixed in Servo (`components/fonts/platform`). For Han text it tries
Noto Sans CJK HK, SC and TC; the marks of CJK Symbols and Punctuation (U+3000 to U+303F, for example the ideographic full
stop U+3002), CJK Strokes, Enclosed CJK Letters and Months, CJK Compatibility and CJK Compatibility Forms are script
Common, so for them it tries only the generic families, Noto Sans, the Noto symbol fonts, the DejaVu and Free families and,
last, `Droid Sans Fallback`, and a page showed a missing-glyph box (example.com in Chinese; U+FF0C is in a block the list
handles, so it rendered). `packaging/arch/fontconfig/61-bifrost-cjk-fallback.conf` is a fontconfig scan rule that gives
Noto Sans CJK SC (package `noto-fonts-cjk`) the second family name `Droid Sans Fallback`, so the installed CJK font answers
that last fallback. The Arch package `ttf-droid` is no use for this: its own rules rename that font to `Droid Sans`.
Fontconfig applies a scan rule only while it scans a font directory, so `springboard.install` rebuilds the system cache
(`fc-cache -sf`) until `fc-list "Droid Sans Fallback"` lists the faces; the Browser reads the family list at engine start,
so restart it after a manual rebuild. SC is the face that draws the full stop and comma lower left, as simplified Chinese
does; Japanese and traditional Chinese pages see that placement for these marks until Servo selects fallback fonts by
language.

CSS `backdrop-filter` is not supported (Servo 0.7.0): see "Remaining gaps" in `docs/servo-review.md`.

The existing default browser user-agent profile is retained; `BIFROST_BROWSER_UA` can override it at startup.
The Develop menu and control API update the native preference for subsequent requests. Native APIs include CSS Grid,
Selection, structuredClone, AbortSignal.timeout/any, CSS.supports and scrollIntoView. API presence does not imply
complete web-platform conformance. `navigator.clipboard` remains disabled; keyboard copy/paste uses the native
clipboard delegate bridged to the app's pasteboard.
Compat page list (check after engine changes; not yet verified on Midget): bifrost.olibuijr.com (hero.js), example.com,
ja.wikipedia.org (CJK), news.ycombinator.com, github.com, developer.mozilla.org, docs.rs, lobste.rs.

## Performance budgets

The engine keeps the documents of the three history entries on each side of the current one alive in the back-forward
cache (`engine/src/memory.rs`, `session_history_max_length`) and retains every navigation URL; a farther entry reloads
when it is reached. Back and Forward to a live entry restore the page as it was, scroll offset included. Servo 0.7.0 has
no scroll restoration of its own (`history.scrollRestoration` is not implemented), so with no live documents (the setting
used to be 0) Back always reloaded the page at the top. The matched ten-navigation workload with no live documents
still retained canvas and image allocations: approximately 640 MiB PSS before and 641 MiB after. These settings do not
establish a whole-process memory improvement. The JavaScript GC heap limit is 255 MiB per runtime; other GC settings retain upstream defaults. The
HTTP response cache retains at most 256 weighted entries (this is an entry limit, not a byte limit).
The heap limit does not include images, rendering surfaces or other native allocations.

`python3 tools/bench.py --browser` (desktop session, browser open): startup <= 1500 ms, idle CPU <= 1 %, idle wakeups
<= 60/s, PSS <= 600 MiB. A static page must draw nothing: `tick` returns true only for engine frames or events.
Window suspend stops engine painting. The only periodic wake is the watchdog ping (`next_wake`, 2 s, none while the
window is hidden). Native `hide` / `show` control painting; Servo 0.7.0 has no public page-visibility or
background-throttling setter. Hidden page JavaScript may continue running.
Background tabs are discarded (`src/tabdiscard.rs`): idle 15 min, or least recently used while engine PSS exceeds
1.5 GiB (2 most recent kept); the tab keeps its id, URL, title and scroll and reloads on select. Env:
`BIFROST_TAB_DISCARD_MIN`, `BIFROST_TAB_BUDGET_MIB` (0 = off). Form input and page state are lost on discard.

## Stability wiring
`tick` pings the engine every 2 s (`WebProcessPool::send_ping` / `take_pong`); 10 s without a `pong` kills it and the normal `EngineExited` path restarts it, limited to 3 per minute (`recover::StormGuard`; Reload clears it). Crashes are written to the unified log via `recover::crash_line`. A timed-out screenshot sends `dev shotabort` so the view returns to the window size.

## Known gaps

Hard reload clears the whole HTTP cache (Servo has no per-origin or bypass-reload API). Servo 0.7.0 `show_console_message` passes level and text only, so console source file:line comes from the page-side collector (see Console below) and can be missing for messages the stack cannot place.

Servo 0.7.0 has no public stop-loading API. The address field shows Stop in place of reload while a page loads (the reference does), but the engine's stop command is a no-op, so pressing it does not end the load yet; reload is the field's button otherwise.
Desktop input uses native mouse events for selection and hover, and native wheel events for wheel/trackpad input,
including page wheel handlers and `preventDefault`. Developer pointer commands use the same native mouse path.
Tablet input remains touch input. Script dialogs, context menus and select popups use native embedder controls.
File inputs use the system document chooser, including multiple selection and accepted file types. Color inputs
use browser controls with RGB sliders and hex entry. Servo's native unload request opens a Stay / Leave sheet
before navigation away from an unsaved document. Hovered link destinations come from native status callbacks.
Find uses standard DOM TreeWalker, Range and Selection APIs because
neither an embedder find method nor `window.find` is available. It searches the visible main document, capped at
50,000 text nodes, 2 million UTF-16 units and 10,000 matches; iframe/shadow-root search and highlight-all remain gaps.

Favicons come from `notify_favicon_changed` / `WebView::favicon`; discovery, fetching and image decoding are owned
by Servo. The app only caches and scales the supplied pixels. `browser.favicon` returns the decoded icon as a PNG
data URL because Servo does not expose the chosen resource URL.

## Web Inspector

Cmd+Option+I toggles a docked bottom panel; Cmd+Option+J shows the Console, +N the Network, +C the Elements. Drag the
panel's top edge to resize it; the height and the last tab are remembered (`browser.inspector.height`, `.panel` in the
app defaults). Code: `inspector.rs` (panel), `network.rs`, `elements.rs`, `control.rs` (control methods, pure),
`browser_ctl.rs` (runs them), engine side `engine/src/devtools.rs`, SDK side `sdk/src/webview_dev.rs`.

- **Console**: `console.*` and uncaught errors from the engine (`console ID LEVEL SOURCE LINE TEXT`, 4 KB per line,
  5000 entries per tab, oldest dropped). Every entry has a per-tab `seq` and a `ts_ms`. Filter All/Errors/Warnings/Logs,
  text search, Clear, Copy (all visible lines with seq and time), tap a line to copy it. The prompt evaluates JavaScript
  in the page: Up/Down walk the history, and a trailing backslash or an open bracket or backtick continues the
  statement on the next line. Warnings and errors also go to the unified log (`bifrost pc log`).
- **Network**: method, name, status, type, size and time per request, coloured by outcome (failed and 4xx/5xx red,
  redirects and unknown status grey, cached orange). Filter All/Errors/Fetch-XHR/Other, URL search, Clear. Servo 0.7.0
  gives an embedder no request hooks, so the log is built in the page: a collector wraps `fetch` and `XMLHttpRequest`,
  listens for failed `img`/`script`/`link` loads, and the drain adds Resource Timing. 2000 entries per tab. Limits:
  status of images, scripts and styles is unknown (shown "-"), `from_cache` is a guess from `transferSize == 0`,
  iframe and worker requests are not seen, and requests made before the collector ran are only seen through Resource
  Timing (the engine installs the collector at document start once `devtools::collector_builder` is wired).
- **Console source and line** (`pagetools.rs`): the same collector script wraps `console.log/info/warn/error/debug` and listens for `error` and `unhandledrejection`; the file and line come from `new Error().stack` (first frame outside the wrapper). The Inspector drains the queue every 500 ms while open (and `browser.console` drains before answering), then gives the engine's line with the same level and text its `source` and `line`; unhandled rejections are not reported by the engine and are added as errors. The engine formats arguments its own way, so a match is by equal text or a shared prefix. Unverified on Midget: the stack format of Servo's SpiderMonkey (`fn@url:line:col` is assumed), and whether the engine also reports unhandled rejections (they would show twice).
- **Elements**: a read-only DOM tree (tag, id, class, attributes, text of leaves, 1500 elements), tap a branch to fold
  it, the filter field takes a CSS selector (matches and their ancestors), Refresh re-reads the page. Tap an element
  to select it: a blue outline overlay (`div#__bifrost_hl`, fixed, `pointer-events:none`, follows scroll and resize) is
  drawn in the page and the top rows show `tag#id.class  W x H at X, Y` plus up to 12 attributes; tap it again,
  switch panel, close the panel or navigate to remove it. Inspect Element in the page context menu (and
  `browser.inspect`) selects the element at a point, unfolds the tree to it and scrolls to it. Nodes are addressed by an
  element-index path from `<html>` (`0.1.3`). Limits: the overlay is page content (it appears in screenshots, the tree
  skips it), page zoom is assumed to scale CSS px uniformly, and the overlay colour is a literal systemBlue because it
  lives in the page.

## Downloads (downloads.rs)

Non-displayable responses are saved to `~/Downloads` and listed in a popover (toolbar button appears after the first
download). Servo 0.7.0 cannot hand over a response body (`load_web_resource` only intercepts with an embedder-built
response; `request_navigation` carries no headers), so a `curl -I` probe runs per http(s) navigation, and on
"download" the view is stopped and curl writes `NAME.download`, renamed when done. Control `browser.downloads
{cancel, clear}`, MCP browser action `downloads`. Known gaps: cookies use a native snapshot restricted to the exact origin; one extra HEAD per
navigation; the same URL is not re-probed in a view until it navigates elsewhere.

### Control socket methods

The Browser's app control socket (`/var/lib/springboard/appdata/is.olibuijr.browser.desktop/control.sock`, JSON-RPC
2.0, same uid only; from the MCP server through the `app` tool) serves these methods; `app.methods` lists them.
Every method takes an optional `id` (the tab's `WebViewId`, default the current tab). Calls that need the page or the
engine answer later (SDK `control::RpcError::pending` + `control::complete`) and take an optional `timeout_ms`
(default 10000, 1 to 60000). Errors: -32601 unknown method, -32602 invalid params, -32000 timeout, -32001 no such tab,
-32002 engine not running (`error.data.reason` says why).

| Method | Params | Result |
|---|---|---|
| `browser.state` | | `version, engine_version, engine_alive, engine_pid, restarts, tabs, current, devtools_open, viewport {w,h,dpr}, zoom, user_agent` (`restarts` = engine restarts in the last minute) |
| `browser.memory` | `timeout_ms?` | Servo's native memory report (`results`: processes with `pid`, `isMainProcess`, allocation `reports`). Engine-wide; does not revive discarded tabs. Report categories can overlap; do not sum them as PSS. |
| `browser.engine_preferences` | `timeout_ms?` | Effective native Servo preferences as JSON; does not revive discarded tabs. |
| `browser.tabs` | | ordered `[{id, index, url, title, loading, can_back, can_forward, current, crashed, zoom, secure}]` |
| `browser.tab.new` | `url?, background?` | `{id}` |
| `browser.tab.select` / `browser.tab.close` | `id` | `{ok}` |
| `browser.tab.reopen` | | `{id}` (error -32602 when nothing was closed) |
| `browser.navigate` | `url, id?, wait? none/commit/load (default load), timeout_ms?` | `{id, url, title, status: started/committed/loaded/timeout}` (a timeout is a result, not an error) |
| `browser.back` / `browser.forward` | `id?` | `{ok}` (false when there is no history) |
| `browser.reload` | `id?, hard?` | `{ok}` (`hard` clears the HTTP cache, then reloads) |
| `browser.eval` | `js, id?, timeout_ms?` | `{ok:true, result, type}` or `{ok:false, error}`; `type` is undefined/null/boolean/number/string/array/object/element; results are JSON (depth 32); a promise is not awaited. The input and the result are also recorded in the console as levels `input` and `result` |
| `browser.console` (alias `console`) | `id?, level? error/warn/info/log/debug/input/result, limit?=200, since_seq?, clear?` | `{entries:[{seq, ts_ms, level, text, source, line}], next_seq}`; pass `next_seq` back as `since_seq` |
| `browser.dom` | `selector?="html", mode? outer_html/text/attrs/query, limit?=200, max_bytes?=200000` | `{matches, html or text, truncated}` or `{matches, items:[{tag, id, class, text, attrs, rect {x,y,w,h}}]}` (`attrs` = the first match only) |
| `browser.screenshot` | `full_page?, selector?, rect? {x,y,w,h}, max_pixels?=16000000, format? png/pdf, paper? letter/a4, margin?, header_footer?` | `{width, height, format:"png", encoding:"base64", data, dpr, scroll {x,y}}` up to 16 MB |
| `browser.screenshot` with `format:"pdf"` | as above; prints the whole page unless `selector` or `rect` is given | `{format:"pdf", encoding:"base64", data, pages, width, height, dpr}`; the page is printed as pictures, scaled to the paper width and cut into pages (no selectable text) |
| `browser.content_rules` | `op? list/compile/remove/clear/test, identifier?, rules? (rule list JSON text), url?, resource?, top_url?` | `{lists:[{identifier, rules}], rules, available}`; `test` gives `{block, block_cookies, make_https, css_selectors, notifications}`. Lists are kept in the profile and applied to every tab: block (loads and navigations), make-https (navigations), css-display-none; block-cookies and notify are not applied by the engine |
| `browser.messages` | `since_seq?, clear?` | `{messages:[{seq, tab, name, body, main_frame}]}`: what pages posted with `window.bifrost.messageHandlers.control.postMessage(value)` (a page loaded after the engine started) |
| `browser.network` | `since_seq?, limit?=200, clear?` | `{entries:[{seq, ts_ms, method, url, status, type, from_cache, duration_ms, size, error?}], next_seq}` |
| `browser.wait` | `for: load/selector/js/text/console/idle, selector?, js?, text?, timeout_ms?` | `{ok, waited_ms}` (`ok:false, timeout:true` when it ran out) |
| `browser.viewport` | `width?, height?, dpr?, preset? iphone/ipad/desktop, reset?` | `{width, height, dpr}` (CSS px; the override is Responsive Design Mode, `dpr` rounds to a whole number) |
| `browser.zoom` | `factor? 0.3 to 5` | `{factor}` (remembered per site like Cmd+Plus) |
| `browser.find` | `query, dir? n (new)/f/b/c (clear)` | `{count}` |
| `browser.user_agent` | `ua?` | `{ua}`; setting updates the native preference and returns `restart_required:false` |
| `browser.input` | `kind click/move/key/text/scroll, x?, y?, key?, mods? [ctrl,shift,alt,meta], text?, dx?, dy?` | `{ok}`; page-local CSS px of the current tab, left button only (it goes through the touch path) |
| `browser.devtools` | `open?, panel? console/network/elements` | `{open, panel}` |
| `browser.source` | `id?` | `{url, html}` (the live DOM as HTML, with the doctype) |
| `browser.inspect` | `x, y, id?, timeout_ms?` | `{path, tag, id, class, selector, rect {x,y,w,h}}` of the element at the page point (CSS px of the viewport, `document.elementFromPoint`); also selected and highlighted (see Elements). -32602 when nothing is there |

Screenshots use `WebView::take_screenshot`: the viewport, a rectangle (`rect` is in CSS pixels relative to the
viewport; `selector` scrolls the element into view and clips it to the viewport), or `full_page` (the context is
resized to the document height, at most `max_pixels` and 8192 px high, and restored; the page's viewport height
changes for that moment, so `vh` units and sticky headers may differ). The engine writes a PNG to a temporary file and
the app encodes it. `scale` (0.1 to 4) resamples the PNG in the app (`src/resample.rs`: box filter down, bilinear up, 64 Mpx cap), so `width` and `height` are the scaled size; the capture itself is still at the view's device pixel ratio (use `browser.viewport` with `dpr` for a sharper source).

Everything needs the engine's `devtools` module to be called from `engine/src/main.rs` (see its docs): until then the
engine ignores `dev ...` lines and the calls that need the page time out with -32000.

## Servo rules (read the Servo source, do not guess)

The Servo 0.7.0 source is in `out/servo-source/servo-0.7.0` after an engine build; answer engine questions from it.

- Resize only through `WebView::resize`. It resizes the shared `RenderingContext` and relayouts every view on it
  (`components/paint/painter.rs` `resize_rendering_context`), and returns early when the context already has the
  size. Calling `rc.resize` first therefore skips the viewport update: pages stay at the startup 800 x 600 (0.4.0-0.4.2).
  Resize the context directly only while no view exists.
- A new view takes its viewport from the context size when it is built (`add_webview`), so apply a pending size before
  building one.
- Repaints: `spin_event_loop` calls `notify_new_frame_ready` for views that need painting, and Servo wakes the loop
  through the `EventLoopWaker`. No idle timer is needed (`components/servo/webview.rs`, "Rendering Model").

## Native page controls

Servo reports these as embedder controls (`show_embedder_control`). The engine keeps each one under a token and
forwards it (`dialog`, `menu`, `color-picker`, `file-picker`). `src/pageui.rs` owns sheets and popovers;
`src/uploads.rs` requests file descriptors through the SDK document portal. Files the user selects are streamed
off the UI thread to private staging files, because Servo 0.7 accepts paths. Their names are preserved, and the
copies are removed on document navigation, tab close or normal browser exit. Limits: 64 files and 1 GiB per
selection, 2 GiB retained per window. The document portal uses up to 16 extension filters; All Files remains
available, as HTML `accept` is a chooser hint. The local GTK host uses `Gtk.FileDialog` for the same portal protocol.

The native `request_unload` callback opens a browser-authored Stay / Leave sheet, with Stay selected initially.
Pending requests are bounded; navigation, close, crash and shutdown retire their tokens. Retired unload requests
explicitly deny, since Servo's responder otherwise defaults to allowing. This covers navigation requests emitted
by Servo; closing a tab or quitting still destroys the view directly and does not offer an unsaved-work prompt.
Right-click comes from the shell as `Input::Context`. Remaining gaps include native download callbacks (not public
in Servo 0.7), IME composition, and inspector node picking.

## Memory bounds (round 55)

Per tab, console and network logs keep at most 5000/2000 entries and 2 MiB of text each. History keeps 5000 entries,
each with at most 2 KiB of title; longer URLs are not recorded.

## Settings, private window, tabs (round 55)

Cmd+, opens the Settings sheet (`src/prefs.rs`): homepage, search engine, restore-on-launch, Clear History, Clear Website Data.
Cmd+Shift+N launches `bifrost-browser --private` (history, session and profile are not kept; marked by a dark toolbar).
Tabs drag to reorder; a still finger held 0.5 s on the page opens the context menu. Unverified on device: that the shell gives the
spawned private process its own window, and that ticks run while a finger is held (long-press also fires on lift).


## Components and review

The process entry/model is `src/main.rs`; `app.rs` is the SDK boundary. `lifecycle.rs` owns startup/timers,
`navigation.rs` tabs and address state, `input.rs` routing, `keyboard.rs` edit commands/shortcuts,
`pointer_input.rs` mouse/touch/scroll, `actions.rs` actions, `menus.rs` menu dispatch, `layout.rs` geometry,
`drawing.rs` rendering, `events.rs` callbacks and `engine_loader.rs` archive extraction.

The engine has separate runtime, delegate, command, input, event, rendering, favicon and find components.
`configuration.rs` owns startup/feature preferences, `memory.rs` owns subsystem budgets, and `event_loop.rs`
owns bounded input delivery, wake coalescing, scheduling and shutdown. Native diagnostics cross the existing
typed SDK/control boundary; see [embedding patterns](docs/embedding-patterns.md).
`engine/src/rule_match.rs` links to the SDK rule matcher so staging the engine copies the shared implementation.
See [the Servo review](docs/servo-review.md) for source contracts, remaining gaps and validation results.

## Rendering selection

Browser Settings > Rendering defaults to **GPU**. Select **CPU fallback** for software rendering; changing the selection reloads open pages. The saved choice applies on subsequent launches. GPU mode reports an initialization error if hardware rendering is unavailable. `browser.engine_preferences` includes `_bifrost_renderer` with the active mode and GL driver. Frames currently use CPU readback and shared memory for presentation.

## Toolbar and page menu

The toolbar follows the reference: Back and Forward in one capsule, the address field centred (the page menu icon at its
leading edge, reload, or Stop while a page loads, at its trailing edge), and Share, New Tab and Show All Tabs in one glass
capsule at the trailing edge. The toolbar takes the page's colour (`themecolor.rs`: `<meta name="theme-color">`, else the
background at the top of the page) under the glass groups. There is no sidebar yet, so no sidebar button.

The page menu (the field's icon) opens common tab actions, bookmarks, history, downloads, Find in Page, page actions,
developer tools, and Settings. Page Actions includes zoom and Reader. Use Alt+F or F10 to open the menu,
arrow keys/type selection to choose a row, Enter to activate, and Escape to dismiss. Settings includes
homepage, search engine, session restore, GPU/CPU renderer selection and website-data controls.
