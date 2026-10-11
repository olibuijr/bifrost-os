# BifrOSt desktop improvement review — 2026-10-08

Read-only code/runtime review on Midget, shell 0.4.13. No release or runtime configuration change performed for this review.

## Current evidence

App Store foreground, Settings/Files/Remote open, Browser running, sharing service active. A 10-second /proc CPU sample measured shell 4.10%, labwc 0.30%, Servo processes 0.10%, Files and WayVNC 0%, expressed as percentages of one CPU core. These are a short idle sample, not input-latency measurements. Shell PSS snapshot 208 MiB, compositor 104 MiB, Browser process tree 175 MiB. PSS varies with pages, session, cache warmth, and shared-memory ownership; compare identical workloads before claiming savings.

Journal repeatedly reports roughly 25–33 ms CPU drawing with App Store idle and 100% root-frame damage. The prior selection benchmark p95 was 37.6 ms, above a 60 Hz frame budget. Screenshot: /home/olibuijr/Pictures/tab/improvement-review.png. The desktop top bar still displays Wi-Fi and a 100% battery on this wired desktop; File/Edit/Help and desktop network/power dropdowns are absent. No failed systemd units were reported. power-profiles-daemon is not installed.

## Ranked work

| Rank | Change | Evidence | Benefit and validation |
|---|---|---|---|
| 1 | Schedule painting from actual state changes | bin/shell.rs:396 wakes every second while in an app; shell/desk.rs:702 refreshes background built-ins every second; focused built-ins always render. | Largest immediate CPU opportunity. Give models revision numbers and timers only for genuinely live views; wake on input, worker results, app frames, animation deadlines. Compare idle CPU, wakeups, and p95 interaction latency with identical windows. Preserve clock/network/update progress correctness. |
| 2 | Refresh existing app packages after App Store updates | shell/apphost.rs:139 only appends previously unknown IDs. Existing manifests/paths stay stale. Registry snapshots are intentionally leaked. Earlier Browser/Remote installs needed a new shell session or explicit current-package launch. | Updates should take effect on next launch without respring. Stable IDs/slots with replaceable owned metadata; keep running processes on their existing version until closed. Verify update, close, relaunch resolves the current package; check removal/reinstall and no dangling references. |
| 3 | Reduce the SDK-to-Wayland pixel pipeline | shell/desk.rs:713 creates a fresh full-window Image, paints it, then Damage::between scans it; external body cache remains another image. bare/wayland.rs:392 converts/copies pixels into SHM. | Even partial app updates can incur full-window composition and comparison. Propagate body damage into retained window composition; repaint chrome only when changed. Measure copied/scanned pixels and allocations per selection, resize and animation. Later assess direct client Wayland surfaces or GPU textures with fences/lifetime rules. Hardware compositing alone does not make shell rasterization GPU accelerated. |
| 4 | Replace network polling with notifications | shell/desktop_network.rs:61 polls every three seconds; collect invokes nmcli and powerprofilesctl subprocesses repeatedly, through timeout. | Subscribe to NetworkManager/power D-Bus changes, cache state, and request Wi-Fi scans only when appropriate. Measure process launches/wakeups; exercise unplug/replug, connect failures, and missing adapters. |
| 5 | Finish desktop integration | Live screenshot and HANDOFF: tablet-style top status bar, wrong battery representation on wired desktop, missing app menus/network/power selectors. | Establish one desktop command/menu model shared with apps and keyboard shortcuts. Source network/power state from actual services; represent unavailable controls honestly. Verify wired-only hardware, optional Wi-Fi, laptop battery and missing power daemon separately. |
| 6 | Make package downloads recoverable and bounded | store/install.rs:478 deletes .part files after errors; download_to creates/truncates them. store/net.rs get_chunked buffers each range; a range-ignoring fallback fetches the entire package into memory. Browser package is 66.6 MB. HTTP client already retries a failed reused connection once. | Preserve verified partial progress with ETag/If-Range, bounded backoff, cancellation and streaming fallback. Retain checksum verification and atomic installation. Validate interrupted downloads and servers that ignore ranges. Earlier connection-closed failure was observed; its precise cause is unconfirmed. |
| 7 | Reduce shell coupling to built-in applications | shell/mod.rs 4,589 lines, settings.rs 2,456, desk.rs 1,053. Cargo directly depends on Boa, iOSTerm and media codecs. Recent shell-only build/publish took 44 s; first Servo engine build 11m16s, warmed chrome rebuild 4.8 s. | Split by ownership and release boundaries, starting with App Store/Settings and shared UI crates. Gate optional engines/codecs only after dependency/link analysis. Measure shell edit build time and binary/PSS changes. Line count alone is not a performance metric. |
| 8 | Tighten browser assets and product behavior | Browser start.html is 1.1 MiB, predominantly an embedded base64 WOFF font. Servo chrome Stop action only warns that stopping is unsupported (desktop/gui.rs:518). | Ship the font as a reusable file; implement or remove unavailable Stop behavior. Compare browser navigation/tab correctness and matched-page PSS rather than assuming Rust is smaller. Document the maintained Servo patch and reproducible engine build. |

## Memory follow-up

Glyph masks already have a 4 MiB bound (font.rs:562); shrinking this blindly could hurt text performance. Rendered strings use a 512-entry count limit (shell/cache.rs:18), not a byte budget; consider byte-aware eviction after measuring string/image size distribution. Current window caches discard minimized/closed entries. Demand-allocated Wayland buffers reduced mapped SHM 59.85 → 47.77 MiB in the prior four-window check, but whole-shell PSS did not fall across that restart. Track both allocation bytes and PSS/RSS rather than presenting one as the other.

Start with rank 1, then rank 2. Both address observable problems without requiring a wholesale renderer rewrite.

## Implementation follow-up — 2026-10-08

Shell 0.4.15 is installed on Midget. Ranks 1–6 have initial implementations:
HUD-only band redraw, unchanged foreign-window snapshots no longer invalidate
windows, owned refreshable app registry, retained SDK damage composition,
D-Bus-triggered network snapshots with a 60-second fallback, dedicated Wayland
menu layers, and bounded streaming downloads with retained partial files and
three attempts. Native Edit commands currently cover native windows and Terminal;
SDK apps need a command protocol. Download resume validates byte offsets and final
SHA-256; ETag/If-Range and cancellation UI remain future work.

All 12 focused review tests passed after the peripheral-battery correction.
The first CPU samples remained around 4% of one core. A later 4.5% sample may
include Browser downloading; no CPU improvement or total-memory reduction is
established. Recheck equivalent idle workloads after recovery.

Browser 0.2.1 was published and its App Store download started. It separates the
font from HTML and replaces unsupported Stop with Reload. Installation and
next-launch verification are pending because Midget stopped answering ping/SSH.
Published shell 0.4.16.1791434261-1 corrects a live finding: hidpp mouse battery scope=Device was mistaken
for system power. Its installation must be confirmed after Midget reconnects.
Shell/application decomposition (rank 7) has not been implemented.
