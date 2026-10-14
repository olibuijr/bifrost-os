# App compliance gate

Owner goal: every app in `springboard/apps` stays fully compliant with our SDKs, the UI kit and the design guidelines
(CLAUDE.md "Design", skill `bifrost-hig`), and tests catch it whenever an SDK, API or feature changes.

## Run it

| What | Command |
|---|---|
| Static gate, ratchet (what acceptance runs) | `python3 tools/check-app-compliance.py` |
| One app with findings, file:line and fix hints | `python3 tools/check-app-compliance.py --app springboard/apps/notes-desktop` (a dir name works too) |
| Fail on any finding | `python3 tools/check-app-compliance.py --strict` |
| Machine output | `python3 tools/check-app-compliance.py --json` |
| Lock in fixes (after fixing, never to hide a regression) | `python3 tools/check-app-compliance.py --update-baseline --write-doc` |
| Runtime gate, one app | `cd springboard && cargo test --offline --profile fast -p bifrost-notes --test compliance` |
| Runtime gate, all apps (also part of `bifrost pc acceptance`) | `cargo test --offline --profile fast --workspace --no-fail-fast compliance` |
| Triage without failing | `COMPLIANCE_SOFT=1 cargo test ... --test compliance -- --nocapture` |
| Accept new goldens | `BIFROST_BLESS=1 cargo test ... --test compliance`, look at `tests/snapshots/*.png`, commit |
| Apps affected by an SDK/render/ui/proto change | `python3 tools/check-app-compliance.py --changed-sdk [--base main]` |
| Recreate the test files from the template | `python3 tools/check-app-compliance.py --gen-tests --write` |

Accept a finding in place with `// compliance-allow: RULE reason` on its line or the line above (use sparingly;
the reviewer reads the reason). Fixing is preferred: the score and the baseline only improve.

## Static rules

Rule IDs are the gate's own and stable; the third column names the bifrost-hig rule each one enforces.

| Rule | Level | bifrost-hig | What | Fix |
|---|---|---|---|---|
| GG-01 | error | GG-01 | sidebar/kit app is not a full-size window | set window = "full-size" in manifest.toml (and min_shell = "0.4.84"); keep the leading 87 pt of the top band free for the traffic lights |
| GG-02 | warning | GG-02 | hand-rolled toolbar | draw it with widgets::toolbar_band + widgets::toolbar (ToolbarItem::icon, in_group) and publish each button as a control |
| GG-03 | warning | GG-03 | hand-rolled search field | use widgets::search_field + controls::search::{layout, hits_clear} |
| GG-04 | warning | LAY-04, GG-04 | hand-rolled sidebar | use widgets::source_list with SidebarItem::row(..).with_symbol(..) and SourceList::hit |
| GG-05 | warning | GG-02, LAY-04 | hand-rolled path bar | use widgets::path_bar + controls::pathbar::{layout, hit} |
| COL-01 | warning | COL-01 | literal colour | use a named constant from render/src/ui.rs (add one if needed); never pack()/rgba()/0xRRGGBBAA in app code |
| COL-02 | warning | GG-05, COL-05 | fixed dark token (does not follow Light/Dark) | use the Pair token (LABEL, LABEL_2, CARD_BG, DETAIL_BG, SIDEBAR_BG, SEPARATOR_C, FIELD_BG, ...) with .now() |
| COL-03 | warning | COL-05 | hard-coded white/black background | fill with a scheme Pair (DETAIL_BG, CARD_BG, FIELD_BG ...); WHITE is right only for text on the accent |
| TYP-01 | warning | TYP-01 | literal font size | use a MAC_*_PT token from render/src/ui.rs |
| TYP-02 | warning | TYP-01 | literal size in fit() | measure with the same MAC_*_PT token the text is drawn with |
| TYP-03 | warning | TYP-04 | mobile text-size constant on the PC | use the MAC_*_PT constant (Golden Gate HIG table) |
| LAY-01 | warning | LAY-04, COL-01 | literal corner radius | use prefs_layout m::* (window 17, card 13, plate 9, tile 5) or a named token; capsules pass a huge radius |
| LAY-02 | warning | LAY-04 | literal radius constant | point the constant at prefs_layout m::* instead of a number |
| LAY-03 | warning | LAY-03 | assumes a fixed resolution | derive layout from the window size (ctx.cfg, Layout) and ui.pt(); no screen-size literals |
| LAY-04 | warning | LAY-03 | large literal ui.pt() position/size | derive the position from the window size or use a layout token (SIDEBAR_W, UNIFIED_BAR_PT, ...) |
| LAY-05 | warning | LAY-03 | literal spacing in ui.pt() | use the SPACE_* token |
| LAY-06 | warning | LAY-01 | legacy status-bar inset | take the top inset from bifrost_app::layout::Layout |
| LAY-07 | warning | TYP-03 | hand-drawn large title | use bifrost_app::widgets::large_title |
| APP-01 | error | APP-01 | manifest incomplete | add the missing key to manifest.toml |
| APP-02 | error | APP-01 | architecture is not x86_64-linux | set architecture = "x86_64-linux" |
| APP-03 | error | APP-01 | icon missing | add the icon file named by the manifest |
| APP-04 | error | APP-02 | CHANGELOG has no entry for the version | add a CHANGELOG.md entry for the manifest version |
| APP-05 | error | APP-02 | Cargo.toml version differs from manifest | bump manifest.toml and Cargo.toml together |
| APP-06 | warning | APP-01 | permissions not minimal | drop a permission the code does not use; an unknown one is rejected at install |
| APP-07 | error | APP-01 | invalid kind or window | kind is app|system-app|system|service; window is "full-size" or "native" (or absent) |
| APP-08 | warning | GG-01 | full-size window with old min_shell | full-size windows need min_shell >= 0.4.84 |
| APP-09 | error | APP-01 | upstream package incomplete | upstream.toml needs repo, url, sha256, binary, licenses; manifest needs publisher and license |
| APP-10 | error | APP-01 | icon is not a 180x180 PNG | export the manifest icon as a square 180x180 PNG (PNG signature and IHDR size are read; no re-encoding needed) |
| APP-11 | error | APP-01 | manifest declares an app without an executable target | add a binary target before publishing this app |
| RES-01 | warning | CTL-04 | raw thread::spawn | use bifrost_app::dispatch (DispatchQueue::global(Qos).async_, serial queue, DispatchGroup); see docs/bifrost-ui.md "Migrating thread::spawn" |
| RES-02 | warning | CTL-04 | polling next_wake with a worker thread | finish work on the main queue (DispatchQueue::main().async_) or wake_fd; return None from next_wake when idle |
| RES-03 | warning | CTL-04 | no ctx.enable_sudden_termination() but no unsaved state | call ctx.enable_sudden_termination() once in configure (docs/bifrost-ui.md "Sudden termination") |
| SDK-01 | error | CTL-01 | deprecated SDK API | use the replacement named at the item (#[deprecated] note) or in docs/bifrost-ui.md |
| CHR-01 | error | CHR-07 | app menu repeats the shell Edit menu | an app menu titled File, View, Window or Help merges into the shell menu of that name (its items lead); the shell Edit already holds the standard items, so route them through the shell instead of a second Edit (`edit_menu()` next to the shell Edit is flagged too) |
| CHR-02 | warning | CHR-02 | hand-built standard Edit items | standard Cut/Copy/Paste/Undo come from the shell Edit menu and ctx commands; do not rebuild them |
| TST-01 | error | APP-02 | tests/compliance.rs missing | python3 tools/check-app-compliance.py --gen-tests --write |
| TST-02 | warning | APP-02 | tests/compliance.rs differs from the template | python3 tools/check-app-compliance.py --gen-tests --write (keep KNOWN_GAPS) |

Score = 100 minus a penalty per rule (error 4, warning 1.5, times 1 + log2(count), at most 18 per rule). The baseline
`docs/app-compliance-baseline.json` holds the finding count per app and rule; default mode fails when any count rises or
a rule appears that the app did not have.

APP-10 checks the icon named by `manifest.toml` (`icon = "icon.png"`, relative to the app directory). The gate reads the
PNG signature and the IHDR chunk with the Python stdlib (no Pillow): the file must be a PNG and exactly 180 x 180 pixels.
It runs only when the icon file exists (APP-03 covers a missing one). Other formats and sizes, including a non-square
180 x 120, are reported with the measured size. The `*craft` apps are prebuilt upstream packages (`upstream.toml`): only
the manifest, CHANGELOG and upstream rules (APP-*) apply to them.

Permissions the gate knows (KNOWN_PERMS in the script): network, storage, audio, camera, screen, notifications,
documents, keychain, and remote-desktop. Camera is restricted to Photo Booth and screen sampling to Color Meter, as
described in [app-isolation.md](app-isolation.md). The shell also accepts accessibility and display-config; the gate
reports those as unknown until the list is extended.

Status (2026-10-14): the scan now separates runnable app packages from model-only workspace crates. Locate is an
unpackaged model crate; Screenshot has a draft manifest but no executable. Files retains one baseline RES-02 watcher
deadline finding. The current ratchet and table below reflect the repaired app set; the incomplete packages remain
visible as release blockers rather than being hidden by a baseline update.

## Runtime checks (`tests/compliance.rs` in each app crate)

Generated from `tools/templates/app-compliance.rs`. The test spawns the real app binary through
`springboard::shell::apphost::AppHost` (no compositor, no GPU), configures 1200x780 pt at scale 1.0 and 1.5625, Light and
Dark, and asserts:

- render: hello arrives, a frame of the configured size with more than one colour, the process stays alive;
- chrome: sidebar rows (Row/List), toolbar buttons and a search field are published as controls when the source uses
  `source_list`, `toolbar` or `search_field`;
- labels: every visible Button, TextField, Toggle, Checkbox, Slider, SegmentedControl and MenuItem has a label;
- menus: the app's menus are unique, every item has a title, and a menu titled File, Edit, View, Window or Help merges into the shell's menu of that name;
- snapshot: a quarter-size golden per appearance at scale 1.0 (`tests/snapshots/<app>-<light|dark>.png`, about 170 KB;
  `MAX_DIFF_FRAC` per app tolerates live data such as dates).

`KNOWN_GAPS` in each file lists check ids that fail today (seeded with `menus` where CHR-01 fires). They print as
`known gap` and do not fail; delete the id when the app is fixed. The system keyboard has `FRAME_REQUIRED = false`
(it draws only after `keyboard_config`). Apps without a `tests/compliance.rs` yet show TST-01; generate it with `--gen-tests --write`,
triage with `COMPLIANCE_SOFT=1` and bless the first goldens.

The generic harness checks the app's initial launch state. Preview's source-list sidebar is document-contextual, and
Journal's timeline is empty until an entry is created, so their empty-launch cases do not require sidebar rows. Preview's
variable-height page thumbnails remain a custom gallery.

## SDK change trigger

`--changed-sdk` diffs `springboard/{sdk,render,ui,proto}` against the base ref, collects the public items touched
(declarations in changed lines and hunk headers; all public items of a changed file that has none), and lists each app
whose source names one, with the compliance test command. Re-run those, then bless or fix. The lead does this before
merging any SDK, kit or render change.

## Briefing a per-app agent

Give it: its row below, `python3 tools/check-app-compliance.py --app <dir>` output, `docs/golden-gate-sdk-adoption.md`
and `docs/bifrost-ui.md` "Golden Gate kit". Done means: findings reduced, `--update-baseline --write-doc` committed,
its `tests/compliance.rs` passing with a shorter `KNOWN_GAPS`, goldens blessed and reviewed in Light and Dark.

## Per-app summary

Worst first. Regenerate with `--update-baseline --write-doc`.



<!-- app-compliance-table:begin -->

| App | Score | Findings | Top 3 rules |
|---|---:|---:|---|
| screenshot-desktop | 96 | 1 | APP-11 x1 |
| files | 98 | 1 | RES-02 x1 |
| activity-desktop | 100 | 0 | none |
| archive-desktop | 100 | 0 | none |
| assistant-desktop | 100 | 0 | none |
| audio-setup-desktop | 100 | 0 | none |
| automator-desktop | 100 | 0 | none |
| bluetooth-exchange-desktop | 100 | 0 | none |
| books-desktop | 100 | 0 | none |
| browser | 100 | 0 | none |
| cadcraft-desktop | 100 | 0 | none |
| calculator-desktop | 100 | 0 | none |
| calendar-desktop | 100 | 0 | none |
| chess-desktop | 100 | 0 | none |
| clock-desktop | 100 | 0 | none |
| colormeter-desktop | 100 | 0 | none |
| colorsync-utility-desktop | 100 | 0 | none |
| console-desktop | 100 | 0 | none |
| contacts-desktop | 100 | 0 | none |
| daw | 100 | 0 | none |
| deckcraft-desktop | 100 | 0 | none |
| designcraft-desktop | 100 | 0 | none |
| dictionary-desktop | 100 | 0 | none |
| diskutil-desktop | 100 | 0 | none |
| effectcraft-desktop | 100 | 0 | none |
| filmcraft-desktop | 100 | 0 | none |
| fontbook-desktop | 100 | 0 | none |
| freeform-desktop | 100 | 0 | none |
| games-desktop | 100 | 0 | none |
| grapher-desktop | 100 | 0 | none |
| gridcraft-desktop | 100 | 0 | none |
| image-capture-desktop | 100 | 0 | none |
| journal-desktop | 100 | 0 | none |
| keyboard | 100 | 0 | none |
| lightcraft-desktop | 100 | 0 | none |
| locate-desktop | 100 | 0 | none |
| mail-desktop | 100 | 0 | none |
| maps-desktop | 100 | 0 | none |
| messages-desktop | 100 | 0 | none |
| music-desktop | 100 | 0 | none |
| news-desktop | 100 | 0 | none |
| notes-desktop | 100 | 0 | none |
| passwords-desktop | 100 | 0 | none |
| pdfcraft-desktop | 100 | 0 | none |
| photobooth-desktop | 100 | 0 | none |
| photocraft-desktop | 100 | 0 | none |
| podcasts-desktop | 100 | 0 | none |
| preview-desktop | 100 | 0 | none |
| printers-desktop | 100 | 0 | none |
| reminders-desktop | 100 | 0 | none |
| remote | 100 | 0 | none |
| screen-sharing | 100 | 0 | none |
| script-editor-desktop | 100 | 0 | none |
| setup-assistant | 100 | 0 | none |
| shortcuts-desktop | 100 | 0 | none |
| soundcraft-desktop | 100 | 0 | none |
| stickies-desktop | 100 | 0 | none |
| stocks-desktop | 100 | 0 | none |
| sysinfo-desktop | 100 | 0 | none |
| textedit-desktop | 100 | 0 | none |
| tips-desktop | 100 | 0 | none |
| vectorcraft-desktop | 100 | 0 | none |
| videoplayer-desktop | 100 | 0 | none |
| voicememos-desktop | 100 | 0 | none |
| weather-desktop | 100 | 0 | none |
| wireless-diagnostics-desktop | 100 | 0 | none |
| wordcraft-desktop | 100 | 0 | none |

<!-- app-compliance-table:end -->
