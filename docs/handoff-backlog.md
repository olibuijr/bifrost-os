# HANDOFF backlog (triage of 2026-10-11)

Every unfinished item in `HANDOFF.md` (rounds 1-60, 1638 lines), checked against `main` by five read-only triage passes.
Items found done or stale (tablet-only, superseded, already fixed) are dropped here. `OPEN` = confirmed in code;
`CHECK` = needs a Midget run or a test run before it can be closed. Owner rule: work every item to done, then clear
HANDOFF.md. Tick items here as they land (commit hash).

## Login and session
- [x] Login window on boot and lock, privd PAM auth (stage 1): b00869f1 (0.5.29)
- [ ] OPEN Adversarial security review of privd/login (never ran on `login-hardened`) — running 2026-10-11
- [ ] OPEN Stage 2: shell runs as the user (session script, migrate-user, privd socket path) — docs/privd-protocol.md
- [ ] OPEN Account picture on the login window (`account::avatar_path` not drawn); Caps Lock indicator has no key-state source
- [ ] OPEN Lock-screen app launch restarts the session (docs/snow-leopard.md round 8 item 1)
- [ ] OPEN Unlock after display-off needs several key presses (round 8 item 2)

## Installer and lab
- [x] Unattended install shows the progress page (black screen): 5b1ebb9e
- [x] Lab reinstall speed (mirror skip, store over lab link, 1 s menu, installer-only push): 35502706, 1ec2ce70
- [x] Look step (appearance, accent, wallpaper): 41b24132
- [ ] OPEN Partition editing beyond Erase/Free space (`installer/disks.rs` Mode) — L
- [ ] OPEN Release-ISO hardening review (`tools/check-iso-hardening.sh`, no record)
- [ ] CHECK Kernel/root corruption fallback; hard-limit OOM recovery; direct pacman bypassing the update guard

## Shell and desktop
- [ ] OPEN `window_fx` not wired to window open/close/minimize (hook `desktop_bar.rs` desk close/minimize)
- [ ] OPEN Settings readers missing: minimize effect, hot corners, Overview prefs
- [ ] OPEN Control Center Now Playing: plate click acts as transport; height not clamped on short screens
- [ ] OPEN Sheet default button / focus ring (and Overview selection) use TINT instead of CONTROL_ACCENT (`sdk/src/widgets.rs:554,709`)
- [ ] OPEN Quit has no confirmation for apps with unsaved work (`desktop_bar.rs` Quit = Close)
- [ ] OPEN Menu text re-measured on hover (`desktop_bar.rs` `menu_buttons`): cache per label
- [ ] OPEN Idle `draw()` still costs a dirty frame (`shell/mod.rs` needs_frame)
- [ ] OPEN Dark-only surfaces: Browser chrome, full-screen Clock, power alert (allowlisted in the appearance guard)
- [ ] OPEN Third-party native apps fuzzy (compositor scale 1 vs shell 2x): GDK_SCALE/QT_SCALE_FACTOR in app_policy
- [ ] OPEN `/data/local` leftovers: `render/src/emoji.rs:15`, `render/src/clipboard.rs:2`, `shell/storage_page.rs:114`
- [ ] OPEN Dead code after desk-always: tile/snap/dock_rects/desk_zoom_window etc. (clippy pass)
- [ ] OPEN Shell grows to ~102 MiB after app launches (heap profile)
- [ ] OPEN AT-SPI off in release builds; Orca untried
- [ ] OPEN AX value setting (`ax set`) for sliders and text fields; apps other than Files declare no ax controls
- [ ] OPEN Shell pages use no popup_button/gesture/focus kits (kit adoption in the shell)
- [ ] OPEN Large title not cached (`sdk/src/widgets.rs` large_title)
- [x] DONE labwc removal: `bifrost-wm` is the only window server (docs updated)
- [ ] CHECK Overview F3 over MCP key; Console duplicate Edit/View menus; new windows cascade into the dock; Open panel centring

## SDK and kits
- [ ] OPEN ScrollView wheel and hover (`sdk/src/view/scroll.rs`)
- [ ] OPEN TestWindow waits/expectations (XCTestExpectation parity) in `sdk/src/testing.rs`
- [ ] OPEN Log subsystem ownership enforced on read only (write side)
- [ ] OPEN `isEmphasized`, `maskImage`, vibrant fill levels not exposed
- [ ] OPEN Memory-warning callbacks; occlusion-based suspension; app window restore after a crash
- [ ] OPEN Typed drag and drop with uttype filters in the SDK
- [ ] OPEN Contacts share (SDK share to packages)
- [ ] CHECK print sheet, status-item symbols, font/colour panels drawn, format.rs callers on Locale, dispatch lane cap

## Apps
- [ ] OPEN Term: Settings window, Cmd+N, Cmd+I, prompt jumping, Cmd+click URLs, visual bell (profile_table not wired)
- [ ] OPEN Files: undo does not restore tag colours; disk-full copy leaves a partial tree
- [ ] OPEN Browser: orphan `src/recover.rs`; Servo Resource Timing sizes; console line 0 for inline scripts; Private Window untested
- [ ] OPEN Notes has no ax nodes (export unverified)
- [ ] OPEN Apps still on fixed dark colour constants (per-app Pair port); SDK adoption batch 2
- [ ] OPEN Console not preinstalled on existing systems
- [ ] CHECK DAW New Song clips; Books probe test; Craft apps window buttons; PhotoCraft Light/Dark

## Tools and docs
- [ ] OPEN `tools/_util.py` `changelog_newest` unused; `springboard/tools/aap.py` targets aarch64-linux-android
- [ ] OPEN `tools/boot.sh` referenced by old tests, not in the tree
- [ ] OPEN MCP: `logs app=` filters MESSAGE not SYSLOG_IDENTIFIER; `parse_predicate` quotes; `logs`/`wait` hold the lock
- [ ] OPEN README/CHANGELOG still say iOSTerm (code is clean)
- [ ] CHECK flaky `media::player` tests under parallel load (pass alone)
