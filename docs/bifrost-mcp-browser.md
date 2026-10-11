# BifrOSt MCP: the `browser` tool

One MCP tool (`springboard/mcp/src/browser.rs`) for web development and daily-driver checks in the BifrOSt
Browser. It maps each `action` to a `browser.*` method on the Browser's app control socket
(`/var/lib/springboard/appdata/is.olibuijr.browser.desktop/control.sock`, JSON-RPC 2.0, same uid) through
`sys::app_rpc`. It launches the Browser (`open-app`) and waits up to 15 s for the socket if it is not running.
The method contract lives with the devtools-browser track; this tool consumes it exactly.

| action | method | notes |
|---|---|---|
| state, tabs | browser.state, browser.tabs | tabs prints `*id [index] url "title"`, `*` = current |
| new_tab, select_tab, close_tab, reopen_tab | browser.tab.* | `id` = tab id |
| navigate | browser.navigate | `wait` load (default), commit, none; reply `loaded URL "title" (tab N)` |
| back, forward, reload | browser.back/forward/reload | `hard` on reload |
| eval | browser.eval | result text; a JS error is an `isError` result |
| console, network | browser.console / browser.network | cursor per session via `next_seq`; filters `level`, `text`; `follow_ms` waits for new rows; `reset`, `since_seq`, `clear` |
| dom | browser.dom | mode query (default, rects), outer_html, text, attrs; html capped at 200 KB |
| screenshot | browser.screenshot | image content block + saved `/tmp/bifrost-browser-shot-N.png`; scale info in the text. `scale` (0.1 to 4) resamples the PNG in the Browser app (box filter down, bilinear up); `width`/`height` in the reply are the scaled size |
| wait | browser.wait | for load, selector, js, console, idle |
| viewport, zoom, find, user_agent | same names | emulation presets or custom size/dpr |
| input | browser.input | click/move/key/text/scroll; `selector` resolves to the element centre via dom |
| devtools, source | same names | docked inspector; document HTML |
| inspect {x,y} | browser.inspect | element at a page point (CSS px): `{path, tag, id, class, selector, rect}`; it is selected in the Elements panel and highlighted in the page until deselected or the panel closes |

## Limits and errors

Console and network replies are capped at 200 rows per call (`limit`; the reply says when capped). Text over
60 KB is written to `/tmp/bifrost-browser-<kind>-N.txt` and the reply carries a 4 KB head and the path.
Errors carry a `hint:` line for: Browser not running, engine dead (-32002), tab gone (-32001), timeout
(-32000), missing method (-32601, an older Browser build), invalid params (-32602). The transport helper
(`sys::app_rpc`) returns only the error message, so hints match on the message text.

Console entries carry `source` and `line` for `console.*` calls, uncaught errors and unhandled promise rejections (derived in the page from the error stack by the collector user script, matched to the engine's line by level and text; a message the stack cannot place keeps `line` 0). Each `console` call drains the page's captured entries first.

If the Browser has no `browser.screenshot`, `screenshot` falls back to the window capture of the Browser
(chrome included, not cropped) and says so. Network data comes from injected fetch/XHR wrappers plus resource
timing, so requests made before instrumentation can be missing.

## Example: debug a failing fetch

```
browser {action:"navigate", url:"https://app.test/dashboard"}      -> loaded https://app.test/dashboard "Dashboard" (tab 2)
browser {action:"console", level:"error"}                           -> #14 [error] Uncaught (in promise) TypeError: Failed to fetch (app.js:88)
browser {action:"network", text:"/api/"}                            -> #31 GET 500 https://app.test/api/items fetch 212ms 0B
browser {action:"eval", js:"fetch('/api/items').then(r=>r.text())"} -> db down
browser {action:"screenshot", selector:"#items"}                    -> image + "640x300 px, dpr 1.5, saved /tmp/bifrost-browser-shot-1.png"
```

Send these in one round trip with the `batch` tool: steps of `{tool:"browser", args:{...}}`.

## Testing

`cargo test -p bifrost-mcp browser` runs the mock-socket tests (the transport is a closure). The host-side
acceptance script `tools/mcp-browser-smoke.py [host]` drives a running Midget through the tool;
`tools/mcp-bench.py` has a `browser_state` case.
