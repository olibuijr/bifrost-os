# BifrOSt Browser

Independent Linux desktop App Store app (`is.olibuijr.browser.desktop`), built with `tools/pc-app.py build|publish`.

Since 0.3.0 the Browser is an ordinary Golden Gate SDK app (`window = "full-size"`): the shell draws the traffic
lights, the app draws the unified toolbar with the kit (`toolbar_band`, `toolbar`, `search_field`) and a
Safari-style tab strip that appears only with two or more tabs. Pages render through the SDK web view
(`bifrost_app::webview`, see docs/bifrost-ui.md "Web view") in the engine process `bifrost-webview`.

## Engine

`engine/` is the engine crate: libservo 0.7.0 with a software (offscreen) GL context. It is built as one extra
workspace member of an unmodified Servo v0.7.0 checkout:

```sh
bifrost pc browser-engine 0.7.0-bifrost.8   # build (tools/build-webview-engine.sh), then pin engine.json and ENGINE
```

The command writes the archive's SHA-256 to `engine.json` and its first 12 hex digits to `ENGINE` (`src/main.rs`). `tools/pc-app.py` bundles the archive as `assets/servo.tar.gz`; the first launch unpacks it to
`~/.cache/bifrost-browser/<ENGINE>/`. The profile (cookies, storage) is `~/.local/share/bifrost-browser/profile`.
The app runs in the shell's SDK sandbox (`network`, `storage`); Servo's content processes run inside it.

Engine builds use the release profile with LTO off, 32 codegen units, incremental on, debug off, lld. A first
build took about 16 minutes on Titan; an engine-only rebuild about 10 seconds.

## Develop (devmenu.rs)

Cmd+Option+U View Source (curl fetch, line-numbered `data:` document in a new tab), Cmd+Shift+R Reload From Origin,
Cmd+Option+R Responsive Design Mode (iPhone 15 393x852 @2x, letterboxed on DETAIL_BG with a size readout; presets,
custom WxH and DPR 1/2/3 are in `DevMenu`, menu UI follows). User agent presets live in `DevMenu` and the engine
accepts `ua`. Gaps: Servo 0.7.0 has no cache-bypass reload flag (`reload-hard` runs a normal reload) and reads the
user agent at start-up only, so `ua` is not applied yet. http pages show "Not Secure".

## Web compatibility

Polyfills (`engine/src/compat.js`, defined only when absent): `IntersectionObserver`, `ResizeObserver`, `requestIdleCallback`.
They approximate the specs (intersection on scroll, resize and a 250 ms tick; size polling per frame). CJK and emoji
glyphs need the fonts in `packaging/arch/runtime-packages.txt` (Noto CJK, Noto Emoji, DejaVu).

User agent: Safari 17.5 on macOS by default (engine `DEFAULT_UA`); env `BIFROST_BROWSER_UA` overrides at engine start (empty = Servo's Firefox-style UA).
Native in Servo 0.7.0 (no polyfill): structuredClone, AbortSignal.timeout/any, CSS.supports, scrollIntoView options. Pref-gated off: navigator.clipboard.
Known breakages: fullwidth CJK punctuation is tofu when no Noto Sans CJK font is installed (fallback list is fixed in Servo); sites needing Clipboard API or WebAuthn.

Compat page list (check after engine changes; not yet verified on Midget): bifrost.olibuijr.com (hero.js), example.com,
ja.wikipedia.org (CJK), news.ycombinator.com, github.com, developer.mozilla.org, docs.rs, lobste.rs.

## Performance budgets

`python3 tools/bench.py --browser` (desktop session, browser open): startup <= 1500 ms, idle CPU <= 1 %, idle wakeups
<= 60/s, PSS <= 600 MiB. A static page must draw nothing: `tick` returns true only for engine frames or events.
Window suspend stops engine painting. The only periodic wake is the watchdog ping (`next_wake`, 2 s, none while the
window is hidden). Hidden views are throttled by Servo itself (`hide` -> `SetWebViewHidden` -> `SetThrottled`).
Background tabs are discarded (`src/tabdiscard.rs`): idle 15 min, or least recently used while engine PSS exceeds
1.5 GiB (2 most recent kept); the tab keeps its id, URL, title and scroll and reloads on select. Env:
`BIFROST_TAB_DISCARD_MIN`, `BIFROST_TAB_BUDGET_MIB` (0 = off). Form input and page state are lost on discard.

## Stability wiring
`tick` pings the engine every 2 s (`WebProcessPool::send_ping` / `take_pong`); 10 s without a `pong` kills it and the normal `EngineExited` path restarts it, limited to 3 per minute (`recover::StormGuard`; Reload clears it). Crashes are written to the unified log via `recover::crash_line`. A timed-out screenshot sends `dev shotabort` so the view returns to the window size.

## Known gaps

Hard reload clears the whole HTTP cache (Servo has no per-origin or bypass-reload API). Servo 0.7.0 `show_console_message` passes level and text only, so console source file:line comes from the page-side collector (see Console below) and can be missing for messages the stack cannot place.

No Stop (Servo 0.7.0 cannot cancel a load), no pointer hover or text selection by drag (the desktop pointer
reaches SDK apps as a finger, so a drag scrolls), no page context menus, `<select>` popups, alerts or file pickers
yet, and no bookmarks or downloads.

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
{cancel, clear}`, MCP browser action `downloads`. Known gaps: probe and download send no cookies; one extra HEAD per
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
| `browser.tabs` | | ordered `[{id, index, url, title, loading, can_back, can_forward, current, crashed, zoom, secure}]` |
| `browser.tab.new` | `url?, background?` | `{id}` |
| `browser.tab.select` / `browser.tab.close` | `id` | `{ok}` |
| `browser.tab.reopen` | | `{id}` (error -32602 when nothing was closed) |
| `browser.navigate` | `url, id?, wait? none/commit/load (default load), timeout_ms?` | `{id, url, title, status: started/committed/loaded/timeout}` (a timeout is a result, not an error) |
| `browser.back` / `browser.forward` | `id?` | `{ok}` (false when there is no history) |
| `browser.reload` | `id?, hard?` | `{ok}` (`hard` runs a normal reload: Servo 0.7.0 has no cache bypass) |
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
| `browser.user_agent` | `ua?` | `{ua}`; setting returns `restart_required:true` (Servo reads it at start-up only) |
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

## Page dialogs, context menu and select popups

Servo reports these as embedder controls (`show_embedder_control`). The engine keeps each one under a token and
forwards it (`dialog`, `menu`); `src/pageui.rs` draws the sheet or popover menu and answers with `dialog-reply` /
`menu-reply`. Unanswered controls are answered with their default when dropped (Servo's `Drop`), a page that withdraws
one sends `control-hide`. Right-click comes from the shell as `Input::Context`. Gaps: downloads (no Servo hook in 0.7.0),
long-press, colour and file pickers (cancelled), Inspect Element only opens the inspector (no node picking).

## Memory bounds (round 55)

Per tab, console and network logs keep at most 5000/2000 entries and 2 MiB of text each. History keeps 5000 entries,
each with at most 2 KiB of title; longer URLs are not recorded.
long-press, colour and file pickers (cancelled).

## Settings, private window, tabs (round 55)

Cmd+, opens the Settings sheet (`src/prefs.rs`): homepage, search engine, restore-on-launch, Clear History, Clear Website Data.
Cmd+Shift+N launches `bifrost-browser --private` (history, session and profile are not kept; marked by a dark toolbar).
Tabs drag to reorder; a still finger held 0.5 s on the page opens the context menu. Unverified on device: that the shell gives the
spawned private process its own window, and that ticks run while a finger is held (long-press also fires on lift).
