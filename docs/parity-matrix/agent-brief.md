# Brief: Apple macOS 27 (Golden Gate) SDK vs BifrOSt parity matrix

You fill rows of a parity matrix: for each Apple framework that runs on macOS 27, what Apple offers (from the
**official Apple documentation only**) and what BifrOSt (`~/Projects/bifrost-os`, a macOS-style Linux desktop written
in Rust) offers in its place, with file-path evidence.

The repo is READ-ONLY for you. Write only the output file named in your task (in `$OUT`). Do not commit, build, or run apps.

## Sources and tools

- Official Apple docs (the DocC JSON that developer.apple.com renders):
  `python3 -I tools/parity/docc.py <path> [--depth 2]` where `<path>` is e.g. `appkit`, `appkit/nswindow`, `foundation/undomanager`.
  It prints the topic sections and each symbol's abstract and macOS availability. Use `--depth 2` sparingly (large).
  Do not use memory for Apple facts; cite the doc URL.
- REA (reverse-engineering tool) evidence of the rendered official page:
  `tools/parity/rea-page.sh https://developer.apple.com/documentation/<path>` prints a REA `evidence_id` and the rendered
  headings. Run it ONCE for the framework landing page and record the `evidence_id` in `rea_evidence`
  (if it fails, write the error text there instead; do not retry more than once).
- BifrOSt code: BEFORE any grep/glob in `~/Projects/bifrost-os`, load and call the index once
  (`ToolSearch` query `select:mcp__akurai_index__code_search,mcp__akurai_index__symbol_lookup`, then
  `mcp__akurai_index__code_search` with repo `bifrost-os`); a hook blocks repo searches until an index tool has been
  called. After that, Grep/Read freely. Code never uses Apple names (an owner rule), so search for the *concept*
  (e.g. "undo", "pasteboard", "notification") and read `docs/bifrost-ui.md`, which maps Apple names to our APIs.

Output directory `$OUT` is given in your task (agents write there; `bifrost pc parity import $OUT` copies the files into `docs/parity-matrix/data/`).

## Where BifrOSt features live (start here)

- SDK for apps: `springboard/sdk/src/` (`bifrost_app::*`): widgets, glyphs (symbol glyphs), defaults (UserDefaults),
  locale, keychain, testing (XCTest-like), usernotif, notification (NotificationCenter, Distributed), service_management,
  status_item, share, print, pasteboard, dnd, layout, control (control socket RPC), intents (App Intents), quicksearch
  (Spotlight index), peek (Quick Look), webview + webview_dev (Servo engine), view/ (retained views, animation,
  autolayout, forms, scroll), document (NSDocument), log (os.Logger), dispatch (GCD), fswatch (FSEvents),
  file_coordination, url (URLSession), coding (Codable/plist), restoration, attributed (rich text), spellcheck,
  workspace (NSWorkspace), observable (Combine/Observation), cache, process_info.
- Pure UI models: `springboard/ui/src/` (`bifrost_ui`, SDK `controls`): anim, ax (accessibility), cache, calendar,
  collection, constraint (Cassowary), detect (data detectors), diff (diffable snapshots), focus, format (formatters),
  fuzzy, geometry, gesture, keys (shortcuts), menu, nav, observe, panels (font/colour panels), pathbar, responder,
  scroll, search, sheet, sidebar, split, stack, table, textfit, text, undo, uttype, values (controls).
- Renderer: `springboard/render/src/`: ui.rs (colours, type scale), appearance (light/dark), glass (vibrancy), motion,
  font/cff/gvar (font engine), symbols, emoji, imgdec (PNG/JPEG), clipboard, purgeable, gfx/.
- Shell/OS: `springboard/src/shell/` (desktop, menus, dock, notifications `usernotif.rs`, sharing, dnd, extras,
  atspi/ accessibility bridge, camera/, magnifier, control_center, login), `springboard/src/media/` (v4l2, hardware
  video decode), `springboard/src/bare/` (bluetooth.rs, app_policy.rs bubblewrap sandbox, app_manager.rs cgroups,
  docbroker.rs file grants, scanout), `springboard/src/gpu/` (GPU presenter).
- Daemons: `springboard/audiod` (audio playback/capture service), `netd` (Wi-Fi, DHCP), `wm` (Wayland window server,
  Smithay), `updated` (software update), `bifrostd` (service supervisor), `mcp` (agent automation), `core/src/log.rs`,
  `core/src/host.rs` (host statistics).
- Apps: `springboard/apps/*-desktop` (Files, Notes, Calendar, Contacts, Reminders, Music, Podcasts, Books, Photos-like
  photocraft, PDF pdfcraft, Weather (Open-Meteo), Console, Shortcuts, Assistant, Browser ...). An app is evidence of a
  feature only if the feature is reusable or the app is the system's counterpart (e.g. Contacts app for Contacts UI).
- Docs: `docs/bifrost-ui.md` (Apple-parity sections with gaps), `docs/*.md` (app-isolation, file-portal-design,
  keyboard-system-app, window-server, gpu-first, milestone-unified-logging, agent-api, system-recovery ...),
  `HANDOFF.md`. Cloud services (accounts, contacts sync, backups) are in `~/Projects/bifrost-cloud`.

## Status values (use exactly these)

- `parity`: BifrOSt has an app-facing API covering the framework's main features.
- `partial`: some features have a counterpart; others are missing.
- `os`: the capability exists in the BifrOSt/Linux system layer (service, kernel, shell) but apps get no SDK API for it.
- `gap`: relevant to a macOS-style desktop and nothing exists yet.
- `n/a`: tied to Apple-only hardware, stores, services or other platforms, with no sensible BifrOSt counterpart.

Be strict: `parity` needs the main features, not one class. Every BifrOSt claim cites a path that exists
(`springboard/sdk/src/undo.rs`-style, optionally `:line`). If unsure, say `partial` or `gap` and explain.

## Output file (JSON, write with the Write tool)

```json
{
  "framework": "AppKit (User Interface)",
  "doc": "https://developer.apple.com/documentation/appkit",
  "macos_since": "10.0",
  "status": "partial",
  "bifrost": "bifrost_app::widgets, bifrost_ui::values, springboard/sdk/src/view/",
  "summary": "One or two plain sentences: what is covered and what is not.",
  "rea_evidence": "ev_...",
  "features": [
    {"feature": "Buttons and switches", "apple": "NSButton, NSSwitch", "bifrost": "springboard/ui/src/values.rs (Toggle)", "status": "partial", "note": "no NSButton bezel styles"}
  ],
  "gaps": ["Short, concrete missing capabilities, most important first"]
}
```

Features: one row per official topic section or major capability (5 to 25 rows; more only for huge frameworks).
Write plain, specific English: no marketing words, no "seamless", no "robust".
