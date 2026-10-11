# WebKit parity: what was built

Parity row work for the official WebKit framework (https://developer.apple.com/documentation/webkit). Two gaps from
`docs/parity-matrix/data/webkit.json` were built in pure Rust with std only, as pure models in the bifrost-ui crate.
Neither is connected to the Servo engine process yet, so the framework status stays `partial`.

## What was built

1. Downloads: `springboard/ui/src/download.rs`
   - `Download` with `DownloadState` (`Requested`, `Running { received, expected }`, `Finished { bytes }`, `Failed`,
     `Cancelled`) and the transitions `response`, `receive`, `finish`, `fail`, `cancel`, `fraction`.
   - Terminal states never change. A server file name from Content-Disposition is applied only before bytes arrive.
   - `file_name_from_disposition` (`filename*` RFC 5987 preferred over `filename`) and `sanitize_file_name`
     (last path component only, unsafe characters replaced, 255-byte cap, `download` when empty).
   - Mirrors: `WKDownload` (class, `originalRequest`, `cancel(_:)`) and the `WKDownloadDelegate` progress and failure
     events. Source: https://developer.apple.com/documentation/webkit/wkdownload

2. Navigation policy: `springboard/ui/src/nav_policy.rs`
   - `Policy` (`Allow`, `Cancel`, `Download`), `NavigationPolicy` (scheme list, blocked hosts with subdomains),
     `NavigationAction` (per request, with the `download` link attribute) and `NavigationResponse` (per response).
   - `decide_action`: scheme must be listed, host must not be blocked, a download link asks for a file.
   - `decide_response`: scheme and host rules, status 204 and 205 cancel, `attachment` or a content type the page cannot
     show (anything outside text, image, audio, video, PDF, XML, JSON, and an empty type that the engine sniffs) downloads.
   - Mirrors: `WKNavigationActionPolicy` with `.allow`, `.cancel` and `.download`. Source:
     https://developer.apple.com/documentation/webkit/wknavigationactionpolicy

Both modules are registered with one `pub mod` line each in `springboard/ui/src/lib.rs`. Each has unit tests
(download: 6 tests; navigation policy: 6 tests). Nothing was built or run in this round, as the workflow requires.

## Official API names mirrored

- WKDownload, WKDownloadDelegate, WKNavigationActionPolicy (cases allow, cancel, download).
- Documentation read through `tools/parity/docc.py`:
  - `webkit/wkdownload`: "An object that represents the download of a web resource."
  - `webkit/wknavigationactionpolicy`: "Constants that indicate whether to allow or cancel navigation to a webpage from an action."

## REA evidence

- Framework landing page, rendered by Chromium and inspected with `tools/parity/rea-page.sh`:
  `ev_7671012c2206f964a2d30b1ef58f94b23f92301585521eb480e2293e5c74fbaa`
  (title "WebKit | Apple Developer Documentation"; the page lists the WKDOMNodeSnapshot class, the WebDriver and Safari
  topics, and the platform support "macOS 10.2+").
- Pre-existing framework evidence in the matrix row: `ev_c00e3910dd4fefc8089232cd0220229f53808dba7e19a37fe72997b31b283fe8`.

## Remaining gaps

- Downloads: `bifrost_app::webview` has no download delegate event, and the engine does not report downloads. The
  `Download` model is not called from the SDK. Byte transfer already exists in `springboard/sdk/src/url.rs`
  (`download_blocking`), but it is not linked to web content.
- Navigation policy: the engine does not ask for decisions, so `decide_action` and `decide_response` are not called.
  `stop_loading` remains a no-op because Servo 0.7.0 cannot cancel a load.
- Not addressed here: WKDOMNodeSnapshot, cookie and website-data API (WKHTTPCookieStore, WKWebsiteDataStore),
  page-to-app message handler, UI delegate (alert, confirm, file chooser), PDF output, context menus, custom user
  agent, web extensions, WebDriver for web content.
- Reach for the next round: wire `NavigationPolicy` into the engine's request path in `springboard/apps/browser/engine/`
  and give `springboard/sdk/src/webview.rs` a `WebViewEvent` for downloads. Each is a Servo-side change.
