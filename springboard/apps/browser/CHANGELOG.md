## Unreleased

- `browser.content_rules` compiles, stores and applies web content blocker lists (block, make-https, css-display-none) to every tab; `browser.messages` reads what pages post to the `control` message handler; `browser.screenshot` takes `format: "pdf"` and prints the page as a paginated PDF.

## 0.14.3

- BifrOSt 0.10.0 round (waves 14 and 15): see the system CHANGELOG for this app's changes.

## 0.14.2

- Kept the keychain-backed password-group row type explicitly named at its import so the declared keychain permission is traceable to the code that uses it.

## 0.14.1

- The Web Inspector console keeps its last 500 input lines for Up and Down recall; the oldest are dropped.

## 0.14.0

- File > AutoFill Password fills the sign-in form of the open page from the shared passwords (the same logins as the Passwords app and Settings). The item is enabled only on a page with a password field. It picks the newest login whose site matches the page's host or a parent domain, fills the field before the password and the password, and skips notes and certificates. The app asks for the `keychain` permission and the `is.bifrost.passwords` group.
- View > Show All Tabs (Cmd+Shift+backslash) covers the content with a grid of cards, one per open tab, with its title and address. A card selects its tab; Escape or the menu item closes it.
- Not yet: page snapshots in the overview (the engine keeps none), the password save prompt, and a bookmark folder manager.

## 0.13.4 — 2026-10-12

- Rebuilt on the BifrOSt 0.6.4 SDK: the shared shell/app wire codec, frame and list caps, and the window materials API.

## 0.13.3

- Build fix: the Develop menu actions (user agent, responsive size and pixel ratio) and the bookmark star now sit in the Browser impl, not the App trait impl, so the shell builds again.

## 0.13.2

- The address field shows a star at its trailing edge: an outline for a page that is not a bookmark, filled for one. Clicking it adds or removes the bookmark.
- Develop menu: User Agent presets (checked when chosen; applied on the next engine start), Responsive Design Mode with its size presets and device pixel ratio. The custom WxH entry is removed.

## 0.13.1 — 2026-10-12

- Removed unused helpers and fields (control is_idle, page menu kind, preferences full height, perf first_frame_ms); the bench measures startup from outside the app.

## 0.13.0 — 2026-10-12

- View > Find in Page (Cmd+F) opens the find bar from the menu bar. Find Next (Cmd+G) and Find Previous (Cmd+Shift+G)
  move between matches; both are greyed out while the bar is closed.
- View > Zoom In (Cmd+=), Zoom Out (Cmd+-) and Actual Size (Cmd+0) are in the View menu, greyed out when no page is open.
  Zoom is remembered for the site, as before.
- File > Show Downloads opens the Downloads list under the toolbar button, and File > Clear Finished Downloads removes
  the finished and failed entries from the list (the files stay on disk).
- Not yet: a content blocker list, and Print for web pages. The engine cannot stop a request before it is sent, so a
  block list would not block anything, and the engine has no print path.

## 0.12.0 — 2026-10-12

- History menu: the menu has Back and Forward submenus with the pages before and after the current page in the selected
  tab, nearest first, up to ten on each side. Choosing one moves the tab there one page at a time, the same way the Back
  and Forward buttons work. A new page you open drops the forward pages, as before.
- Long addresses in the menu are shortened (scheme and trailing slash removed, cut to 60 characters).
- Not yet: website data. Cookies and site storage cannot be listed or removed per site, because the engine does not
  expose its cookie store. Clear Website Data in Settings still removes everything.

## 0.11.0 — 2026-10-12

- Reader from the page's structure: the reader page is now rebuilt from a snapshot of the article's element tree instead of the page's raw HTML. Only paragraphs, headings, lists, quotes, code, tables, images and links come through, with their text escaped, and scripts, forms and navigation stay out.
- View > Show Reader: the View menu has a Show Reader item (Cmd+Shift+R) with a check mark while the reader page is up. It is greyed out on pages without an article.
- File > Share: the File menu has Share, which sends the page address to the share menu (Copy and the apps that take links).
- Style: the content area behind the page uses the window content colour (white in Light, dark grey in Dark) instead of the pure black detail colour.

## 0.10.0 — 2026-10-12

- Import Bookmarks: Settings (Cmd+,) has an "Import Bookmarks..." row. It reads the newest bookmark export in Downloads (a `bookmark*.html` file in the Netscape Bookmarks form that browsers and Safari write) and adds the links you do not have yet. Bookmarks you already have keep their names. The import reports how many it added. Folder names are not kept, only the links and their titles.
- Scripts can import too: `browser.bookmarks` takes `op=import` and `path`, and reports `imported`.
- Clear history from a time: `browser.history` takes `clear_since` (seconds since the epoch) and removes the entries last visited at or after that time, reporting `cleared`. The Settings sheet still clears all history.
- Under the hood: page-source fetches, site icons, download probes and the download progress redraw run on the shared dispatch queues instead of raw threads and channels. Nothing changes on screen.
- Style: the page dimming behind sheets, the switch knob and the element-inspector overlay now come from named colour tokens; the overlay uses the app tint. Corner radii and spacing use the shared layout and spacing tokens.
- Copy a link: Cmd+Shift+C copies the current page: the URL as plain text, and the link with its title as HTML, through the shared pasteboard, so other apps can paste it.
- Copy an image: Cmd+Shift+K copies the newest finished PNG or JPEG download to the shared pasteboard. Pages' own images are not copied yet (the engine does not report them).
- Offline: while the network is down, an address you choose shows an "You are offline" page with a Try again link. When the network comes back, the waiting tab loads its address by itself. A tab that you move on from no longer waits.
- Pointer: the pointer becomes an I-beam over the address field.
- Dock: the Browser's dock icon shows the download progress while downloads run (an indeterminate bar when a total is unknown) and clears when they finish.
- Older shells without these services keep working: a refused copy falls back to the plain clipboard, and the network state stays "online".

## 0.9.0 — 2026-10-11

- Settings: the sheet shrinks to fit short windows and scrolls by dragging inside it (content is clipped to the card); the label column and width follow the window.
- Settings: footers are left-aligned with the rows (HIG form footers), drawn locally; the shared `footer()` is unchanged. Long footers end in an ellipsis.
- Tab hover is cleared on touch cancel, keys, commands and menu activation. The SDK has no pointer-leave or window-deactivate input, so hover can still stay after the pointer leaves through the window edge.
- HttpOnly cookies for downloads: Servo 0.7.0 does expose the jar (`SiteDataManager::cookies_for_url`, `CookieSource::HTTP`). The engine answers the downloads cookie probe from the jar (all cookies matching the view's URL, HttpOnly included) instead of `document.cookie`; the app is unchanged.

- Tests: unit tests for `skip_probe`, `url_host`, the same-site cookie rule (now a function), `host_of`, network formatting, Elements runs and folding, and the Inspector hook-first console merge. `tools/browser-smoke.py` drives the Browser over MCP on a Midget (open, eval, console, network, dom, screenshot scale, inspect, new_tab/select_tab, downloads).

- Web Inspector: Sources tab (document source with line numbers and HTML/CSS/JS syntax colours, "Find in file" keeps matching lines, Refresh) and Storage tab (localStorage, sessionStorage, document.cookie; tap selects and copies, double tap edits in a bar under the list, Delete removes). Elements: double tap an attribute row or a text-only element to edit it (written through eval), plus a "Computed Style" list (display, position, box model, colour, font) under the selected node. New `devtools.rs`; HttpOnly cookies stay invisible.
- MCP browser tool: `storage` {kind=local|session|cookie|all, key+value sets, key+delete removes} and `styles` {selector}; `devtools` panel accepts sources and storage.

- Reader (Safari parity): a document button at the leading edge of the address field appears on pages with an article; Cmd+Shift+R or the button shows the title, byline and body as a clean page (system font stack, 680 px measure, Light or Dark to match the window) in the same tab, with the original URL kept in the address field; Esc, the button or Cmd+Shift+R returns to the original page. The article is found by a small script (paragraph text length, link density, `<article>` bonus; a page needs about 600 characters). Reload Ignoring Cache moves to Cmd+Alt+Shift+R (Cmd+Shift+R is Reader, as in Safari).
- Print... (Cmd+P) is not offered: Servo 0.7.0 has no printing API, so it cannot be implemented.

- Bookmarks (round 57, `src/library.rs`): Cmd+D adds the page and opens a small popover to rename it (Remove, Done; Return or Escape also close). Favorites bar under the toolbar (View > Show Favorites Bar, Cmd+Shift+B, remembered in prefs) with one button per bookmark and an overflow chevron menu. Cmd+Option+B opens the Bookmarks panel and Cmd+Y the History panel (history grouped by day, newest first) with a search field, Up/Down to select, Return or double click to open, Delete to remove. Control methods and MCP actions `browser.bookmarks {op=list|add|remove|rename, url, title, query, limit}` and `browser.history {query, limit, remove, clear}`. Cmd+Y no longer fills the address suggestions. Day groups use UTC days; the panel does not scroll with the wheel yet (arrow keys scroll).

- Favicons (Safari, `src/favicon.rs`): tabs, the idle address field (left of the host) and the suggestion list (a globe or magnifier until an icon exists) show 16 pt site icons. After each load the app reads the page's `link rel=icon / apple-touch-icon` hrefs (best size first, SVG skipped: the SDK has no rasterizer), curl fetches them off the UI thread (then `/favicon.ico`), PNG and ICO (PNG or 24/32-bit BMP frames) are decoded and reduced to 32x32, and cached per host under `<profile>/favicons/` for 7 days (memory only in a private window). Control method and MCP action `browser.favicon {id?}` returns `{id, host, url, cached}`. Gaps: SVG, JPEG and palette-BMP icons fall back to `/favicon.ico` or the placeholder.

## 0.8.1 — 2026-10-11

- Elements: syntax-coloured runs advance by their text width, so tags read `<html lang="en">` without gaps.

## 0.8.0 — 2026-10-11

- Settings (Cmd+,) is now a Safari-style sheet hanging under the toolbar: the window chrome stays visible and only the page is dimmed. Inline "Settings" title with Done, a grouped card (Homepage field, Search engine segments, Restore tabs switch) with a footnote footer, and a separate destructive group with "Clear History..." and "Clear Website Data..." that each ask for confirmation in an alert. Prefs keys are unchanged.
- Private Window review: the second process gets its own `BIFROST_CONTROL` socket and `--private`; the dark toolbar scrim is stronger (alpha 140) so it reads as dark in Light mode too.

- Toolbar: the reload button becomes Stop (xmark) while a page loads and stops the load (HIG CTL, Safari parity).

- Downloads: the probe and the curl fetch now send the page's cookies (`document.cookie` read after each load; HttpOnly cookies remain a gap), the previous page as Referer and the browser's User-Agent, so login-gated downloads are detected. A failed or 405/501 HEAD is retried as `GET Range: bytes=0-0` (size from `Content-Range`). URLs with no query and an HTML-like or missing extension skip the probe. The progress wake thread runs once and exits when no download is active.

- Chrome (round 56): host-only centred address field when idle (full URL, selected, on focus); Downloads popover with shadow, arrow, file icon, magnifier reveal, closing on outside click, Escape, tab switch, navigation; tab close X only on hover or selected tab, wider centred titles.

- Tabs (round 56): a revived (discarded) tab keeps Back: the app tracks each tab's history (50 entries, `tabdiscard.rs`) and Back loads the previous entry when the engine has none (Servo 0.7.0 cannot inject history; forward entries of a discarded tab are lost). A control eval on a revived tab now waits for LoadFinished. Crash banner (`crashbanner.rs`): replaces the meta-refresh page with a Safari-style banner over the page ("This webpage was reloaded because a problem occurred." with a close button); only that tab reloads; a second crash within 60 s shows "A problem repeatedly occurred" with a Reload button and no auto reload.

- Web Inspector: Network panel has a column header (Name, Method, Status, Type, Size, Time) with right-aligned numbers; fetch and XHR sizes fall back to the body length, duration falls back to responseEnd - startTime, unknown size or time shows "-" (Servo 0.7.0 hard-codes Resource Timing sizes to 0 and has no status for images, scripts or stylesheets).
- Web Inspector: Elements tree draws disclosure triangles and indent guides and colours tag, attribute name and value.
- Web Inspector: console entries from the page hook replace the engine copy (text, file, line); engine-only messages are kept.

## 0.7.0 — 2026-10-11 (engine 0.7.0-bifrost.12)

- Compat audit (Servo 0.7.0 webidls): `structuredClone`, `AbortSignal.timeout/any`, `scrollIntoView` options and `CSS.supports` are native; `ResizeObserver` is native but pref-gated, `navigator.clipboard` is pref-gated off (`dom_async_clipboard_enabled`). Added guarded polyfills: `MediaQueryList.addEventListener` fallback, `AbortSignal.throwIfAborted`, `Object.groupBy`.
- Default user agent is now Safari-compatible (engine `Preferences.user_agent`); `BIFROST_BROWSER_UA` overrides, empty keeps Servo's.
- CJK tofu: Servo's Linux fallback already tries Noto Sans CJK HK/SC/TC for Han and fullwidth forms; the fix is the font package, no engine change.

- Stability (round 55): console and network logs are also capped by bytes per tab (2 MiB each, oldest first); history
  entries keep at most 2 KiB of title and skip URLs over 2 KiB; the SDK drops an engine line that grows past 4 MiB
  without a newline; the frame size arithmetic cannot overflow on a garbled `frame` line; fuzz-style tests cover the
  menu, console and escape parsers and the line handler. A crashed content process already reloads only its tab
  behind the "This webpage was reloaded because a problem occurred" page; session saves are already atomic (temp file,
  fsync, rename).

- Tab discarding (Safari-style, `src/tabdiscard.rs`): a background tab idle for 15 minutes, or the least recently used ones while engine PSS is over 1.5 GiB (the 2 most recent are kept), has its engine view closed; its id, URL, title and scroll stay, and selecting it (or a control/inspector call on it) re-creates the view and restores the scroll after load. `BIFROST_TAB_DISCARD_MIN` and `BIFROST_TAB_BUDGET_MIB` override (0 = off). SDK: `WebProcessPool::discard`, `is_discarded`, `revive`, `pid`. Unit tests cover the policy.
- Idle wakeups: the browser now declares `next_wake` (the 2 s watchdog ping only), so a static page has no other timers. Fix: the ping previously ran only when something else woke the app, and the first tick after a long idle or hidden spell could kill a healthy engine as wedged; the watchdog is also reset on resume.
- Background throttling: Servo 0.7.0 already throttles hidden views (`hide` -> `SetWebViewHidden` -> `SetThrottled`); documented, no app code needed.

- Elements: tapping an element selects it. The page gets a highlight overlay (a fixed, pointer-transparent outline div that follows scroll and resize; removed on deselect, panel switch, close or navigation), and the panel shows the node's box size, position and attributes above the tree. Tree nodes now carry an element-index path.
- Inspect Element: the page context menu entry opens the Elements panel at the element under the click (point lookup with `document.elementFromPoint`), unfolds the tree down to it and scrolls it into view. New control method and MCP action `browser.inspect {x, y}` returns `{path, tag, id, class, selector, rect}` and selects it too.
- Console file:line: the collector user script wraps `console.*` and listens for `error` and `unhandledrejection`; the source and line come from the error stack. The app merges them into the engine's console lines (same level and text) and adds unhandled rejections, which the engine does not report. `browser.console` drains the page's captures first.
- `browser.screenshot` `scale` (0.1 to 4) is applied: the app resamples the engine's PNG (`resample.rs`, box filter down, bilinear up); `width` and `height` in the reply are the scaled size.
- Adds the `png` dependency to the Browser.

Daily-driver track (round 55).

- Settings (Cmd+,): a sheet with homepage, default search engine (DuckDuckGo, Google, Bing, Startpage), restore tabs on launch, Clear History and Clear Website Data. Stored in the app's UserDefaults (`browser.homepage`, `browser.searchEngine`, `browser.restoreSession`); code in `src/prefs.rs`. New windows open the homepage; Clear Website Data restarts the engine on a deleted profile and reopens the same pages.
- Private Window (Cmd+Shift+N): starts a second browser process with `--private`: no history, no session file, a throwaway engine profile (`~/.cache/bifrost-browser/private-PID`, deleted on exit, stale ones at the next private launch), dark toolbar and a "Private Browsing" address field.
- Tabs reorder by dragging in the strip (the dragged tab is selected); Cmd+Shift+[ and ] switch tabs (already present since 0.5).
- Long-press on the page (0.5 s without moving) opens the context menu (link, image, text menus as for right-click).

- Downloads (round 55, `downloads.rs`): a main-frame navigation whose response is an attachment (Content-Disposition) or a type the browser cannot show (not text, image, audio, video, PDF, JSON, XML) is saved to `~/Downloads` with a unique name (`name 2.ext`, partial file `name.ext.download`) instead of being shown. Servo 0.7.0 has no download hook (`load_web_resource` only injects responses, `request_navigation` has no headers), so it is app-side: a `curl -I` probe off the UI thread when a view starts loading an http(s) URL, then the view is stopped and curl downloads. A Downloads toolbar button (after the first download) opens a popover with progress, cancel, open (click row) and reveal (opens the folder), and Clear. Control: `browser.downloads {cancel, clear}`; MCP browser action `downloads`. Limits: the probe sends no cookies, so login-protected downloads are not detected; each http(s) navigation costs one extra HEAD request.

## 0.6.2 — 2026-10-11 (engine 0.7.0-bifrost.11)

- Fix: a new or restored tab could show another tab's page. Servo builds web views visible and all views share
  one scene, so restored tabs that were never shown drew over the current one; new views now start hidden.

## 0.6.1 — 2026-10-11 (engine 0.7.0-bifrost.10)

- Fix: switching tabs showed the previous tab's page. Showing a view hands WebRender a new root display list
  asynchronously, and the frame that carries it is not attributed to the shown view; the engine now repaints on
  any new frame while a view is visible.

## 0.6.0 — 2026-10-11 (engine 0.7.0-bifrost.9)

- Hard reload (`browser.reload` with `hard`, Reload From Origin): Servo 0.7.0 cannot bypass the cache per request, so the engine now clears the whole in-memory HTTP cache (`NetworkManager::clear_cache`, no per-origin API) before reloading.
- Documented limits (read in servo-0.7.0): `show_console_message` carries only level and text, so console source file:line stays empty except for eval errors; `browser.screenshot` `scale` is still accepted but not applied (use `browser.viewport` with `dpr`). Elements highlight overlay and inspect-from-point are not started.

- Web compatibility: `engine/src/compat.js` polyfills `IntersectionObserver`, `ResizeObserver` and `requestIdleCallback` at document start (only when absent; Servo 0.7.0 keeps the first two behind prefs), for bifrost.olibuijr.com hero.js. Installed through the same user content manager as the network collector, which is now created even before `dev collector` arrives (the collector script is added only if set by then; views built earlier keep their manager).
- CJK text: Servo falls back to `Noto Sans CJK *`, `WenQuanYi Micro Hei` and `TakaoPGothic` via fontconfig and none shipped, so `noto-fonts-cjk`, `noto-fonts-emoji` and `ttf-dejavu` are added to `packaging/arch/runtime-packages.txt` (effective on install or system package update).

- Performance: `perf::tick` runs each loop turn (debounced history writes) and `perf::on_exit` flushes on suspend;
  View Source no longer forces a redraw every tick while the fetch runs (the fetch thread wakes the loop on
  completion); window suspend/resume now also tells the engine to stop/restart painting (`set_suspended`).
  `tools/bench.py --browser` runs `bench_browser_*` and checks the budgets.

- Stability (round 54): the engine watchdog is wired in (`ping` every 2 s, a wedged engine is killed and restarted), the restart-storm limit and engine crash lines (unified log) use `recover.rs`; Reload on the error page forgives the storm. `browser.screenshot` full page always restores the rendering size, including on timeout (`dev shotabort`). Control replies no longer block the UI thread (nonblocking write, writer thread when the client is slow) and the completion queue is bounded (256). Tab lookups use `.get(i)`.

Daily-driver track (round 54).

- Script dialogs: `alert()`, `confirm()` and `prompt()` appear as a sheet titled with the page's host, with OK / Cancel and a text field for prompt (Return confirms, Escape cancels). Extra dialogs from a flooding page are refused. Engine `dialog` / `dialog-reply`, SDK `WebViewEvent::Dialog` and `WebProcessPool::dialog_reply`.
- Context menu: right-click opens the page's menu (Back, Forward, Reload, Open Link in New Tab, Copy Link, Copy, Paste, Select All, image links) plus Inspect Element, which opens the Web Inspector. Servo performs the chosen action. Engine `rclick`, `menu` / `menu-reply`; SDK `WebViewEvent::Menu`, `secondary_click`, `menu_reply`.
- `<select>` popups use the same menu, placed under the element, with the current option checked and `<optgroup>` headings.
- Not done: downloads (Servo 0.7.0 has no download or non-HTML response hook, only `load_web_resource`), long-press for touch.

## 0.5.0 — 2026-10-11 (engine 0.7.0-bifrost.8)

- Stability: the engine contains a panic in any one command (reported as `crashed`), caps command lines, bounds its note queue and in-flight evals, answers `ping N` with `pong N` for a hang watchdog, and takes `suspend` / `resume`. The session file is versioned (`v 1`), fsynced, kept as `.bak`; an unreadable one moves to `session.txt.corrupt`. `src/recover.rs` holds the watchdog, restart-storm limit (3 per minute) and crash-log helpers for the app and SDK to wire in. Unit tests for the line protocol and session parser; `tests/stability.rs`.

Developer tools and complete control (devtools-browser track; versions are bumped at release).

- Web Inspector: Console, Network and Elements tabs, a resizable panel with a remembered height, Cmd+Option+I/J/N/C.
  Console entries carry a sequence number and a time, Copy takes every visible line, the prompt has history and
  multi-line input. Network log (fetch, XHR, failed loads, Resource Timing, 2000 entries per tab) and a read-only
  DOM tree with a selector filter.
- Control socket: `browser.state`, `tabs`, `tab.new/select/close/reopen`, `navigate` (wait none/commit/load), `back`,
  `forward`, `reload`, `eval` (typed results, error capture), `console` (seq, since_seq, clear, per tab; `console`
  stays as an alias), `dom`, `screenshot` (viewport, rect, selector, full page, PNG up to 16 MB), `network`, `wait`,
  `viewport`, `zoom`, `find`, `user_agent`, `input`, `devtools`, `source`. Documented in README "Web Inspector".
- SDK: async replies on app control sockets (`RpcError::pending`, `control::complete`, error data reason) and
  `webview_dev` (token-correlated eval, screenshots, page input).
- Engine: `engine/src/devtools.rs`; needs `mod devtools;`, `"dev" => self.dev_command(rest)` in `Engine::command` and
  `devtools::collector_builder` in `Engine::build` (stability track).

- Performance: history writes are debounced (at most every 5 s, atomic, flushed on exit); history lookups use a
  url index; address suggestions match without allocating; the engine pipe parser no longer allocates per line.
  New `perf.rs` hooks and `bench_browser_startup|idle|memory` in tools/bench.py.

- MCP: new `browser` tool (state, tabs, navigate, eval, console, network, dom, screenshot, wait, viewport, input, devtools, source) over the control socket; see docs/bifrost-mcp-browser.md.

## 0.4.3 — 2026-10-11

- Resize goes through `WebView::resize` only, which resizes the shared rendering context and every view on it. The
  engine used to resize the context first, so Servo's resize returned early and pages stayed at 800 x 600. The
  post-resize timer from 0.4.1 is removed: Servo reports the repaint itself. Engine 0.7.0-bifrost.7.

## 0.4.2 — 2026-10-11

- New tabs are built at the window's size: 0.4.0 and 0.4.1 could lay pages out at 800 x 600 (engine 0.7.0-bifrost.6).

## 0.4.1 — 2026-10-11

- Pages fill the whole content area again after a resize (0.4.0 could stay at the engine's startup size).

## 0.4.0 — 2026-10-11

Stability, performance and the first daily-driver and web-developer features. Engine 0.7.0-bifrost.4.

- Stability: when the engine exits, the browser restarts it (at most 3 times a minute) and reopens your tabs. A tab whose
  content process crashes reloads with "This webpage was reloaded because a problem occurred". The engine reports setup
  failures instead of panicking, rejects malformed commands and oversized frames, and unpacks under a lock, so two
  launches no longer race.
- Performance: frames are drawn straight from shared memory (one copy fewer), resize bursts are coalesced, and an idle
  page no longer wakes the engine on a timer.
- History and bookmarks with Safari-style Top Hits under the address field. Cmd+D bookmarks the page, Cmd+Y lists
  bookmarks.
- Find in page (Cmd+F, Cmd+G, Shift+Cmd+G), page zoom (Cmd+Plus, Cmd+Minus, Cmd+0), remembered per site, session
  restore and reopen closed tab.
- Web Inspector Console (Option+Cmd+I): console messages and uncaught errors per tab, with a JavaScript prompt.
- Develop: View Source (Option+Cmd+U), Reload From Origin (Shift+Cmd+R), Responsive Design Mode (Option+Cmd+R) and a
  "Not Secure" label for http pages.

## 0.3.2

- Compliance: the tab strip uses the card surface token instead of the sidebar colour, its selection plate uses the
  shared selection radius (9 pt), and the load bar uses the spacing token.
- Call `enable_sudden_termination` at startup: the browser keeps no unsaved state, so the shell may stop it at once.
- Application icon is now `icon.png` (rasterised from `icon.svg`, 180 x 180).

## 0.3.1

- Follows a Light/Dark switch immediately, including while the window is in the background.

## 0.3.0 — 2026-10-10

- Rebuild the Browser as a Golden Gate SDK app: full-size window with the shell's traffic lights, the unified
  toolbar (back/forward capsule, centred address field, reload, new tab) and a Safari-style tab strip shown only
  with two or more tabs. Light and Dark follow the system.
- Pages render through the new SDK web view (`bifrost_app::webview`) in the bundled engine process
  `bifrost-webview` (Servo 0.7.0 via libservo, engine 0.7.0-bifrost.3); servoshell and its chrome patch are gone.
- Shortcuts: Cmd/Ctrl+L address, T new tab, W close tab, R reload, [ and ] back and forward.

## 0.2.1 — 2026-10-08

- Load the SF UI web font from a reusable asset instead of a large base64 HTML string.
- Open the native start-page URI without exposing installed package paths in the address bar.
- Keep Reload functional while loading; remove the unsupported Stop action.

## 0.2.0 — 2026-10-08

- Adopt BifrOSt desktop typography and dark system colors in the actual native GPU-rendered browser chrome.
- Add consistent traffic-light window controls, dragging, address bar, navigation and tabs; remove engine-development switches.
- Use the BifrOSt start page for new tabs and subset SF UI fonts to keep packaged font data small.

# BifrOSt Browser

## 0.1.3 — 2026-10-07

- Remove the remaining /app bind argument; render the BifrOSt start page using the verified native sandbox launch.

## 0.1.2 — 2026-10-07

- Fix bubblewrap package mounting: use the existing read-only package path instead of creating /app on a read-only root.

## 0.1.1 — 2026-10-07

- Fix Arch startup: replace Servo’s incompatible Gaol prlimit64 seccomp policy with a bubblewrap filesystem/user namespace sandbox, retaining multiprocess rendering. No unsandboxed fallback.
- Keep profile/cache writable, hide other home files, and mount package/system files read-only.

## 0.1.0 — 2026-10-07

- First desktop App Store release, using the checksum-pinned Servo 0.7.0 Rust browser engine and its upstream native toolbar.
- Native GPU-rendered Wayland window, BifrOSt start page, address/search bar, navigation and tabs supplied by Servo.
- Separate user-owned profile and engine cache; launch as the installed local account.
- Browser updates ship independently of system updates. Servo is experimental; site compatibility and memory savings are not guaranteed.
