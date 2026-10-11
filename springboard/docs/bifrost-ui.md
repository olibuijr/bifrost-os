
### WebView: Web Inspector console (gap note)

`WebViewEvent::Console` / `EvalResult`, `WebProcessPool::eval` and `set_devtools` map onto WKWebView's
`evaluateJavaScript:completionHandler:` and Safari's Web Inspector Console. Gaps: WKUIDelegate has no console
callback (Safari reads it through the inspector protocol); source URL and line are empty because Servo's
`show_console_message` carries only level and text; the Network tab, failed resource loads and uncaught
exceptions outside the console stream are not captured yet. `evalresult` text is Rust `Debug` of the JS value.
