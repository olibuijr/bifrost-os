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
| TYP-03 | warning | TYP-04 | iOS text-size constant on the PC | use the MAC_*_PT constant (macOS HIG table) |
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
| RES-01 | warning | CTL-04 | raw thread::spawn | use bifrost_app::dispatch (DispatchQueue::global(Qos).async_, serial queue, DispatchGroup); see docs/bifrost-ui.md "Migrating thread::spawn" |
| RES-02 | warning | CTL-04 | polling next_wake with a worker thread | finish work on the main queue (DispatchQueue::main().async_) or wake_fd; return None from next_wake when idle |
| RES-03 | warning | CTL-04 | no ctx.enable_sudden_termination() but no unsaved state | call ctx.enable_sudden_termination() once in configure (docs/bifrost-ui.md "Sudden termination") |
| SDK-01 | error | CTL-01 | deprecated SDK API | use the replacement named at the item (#[deprecated] note) or in docs/bifrost-ui.md |
| CHR-01 | error | CHR-07 | app menu repeats a shell menu | the shell bar already provides File, Edit, View, Window and Help: give app menus their own titles and route standard commands through the shell |
| CHR-02 | warning | CHR-02 | hand-built standard Edit items | standard Cut/Copy/Paste/Undo come from the shell Edit menu and ctx commands; do not rebuild them |
| TST-01 | error | APP-02 | tests/compliance.rs missing | python3 tools/check-app-compliance.py --gen-tests --write |
| TST-02 | warning | APP-02 | tests/compliance.rs differs from the template | python3 tools/check-app-compliance.py --gen-tests --write (keep KNOWN_GAPS) |

Score = 100 minus a penalty per rule (error 4, warning 1.5, times 1 + log2(count), at most 18 per rule). The baseline
`docs/app-compliance-baseline.json` holds the finding count per app and rule; default mode fails when any count rises or
a rule appears that the app did not have.

APP-10 checks the icon named by `manifest.toml` (`icon = "icon.png"`, relative to the app directory). The gate reads the
PNG signature and the IHDR chunk with the Python stdlib (no Pillow): the file must be a PNG and exactly 180 x 180 pixels.
It runs only when the icon file exists (APP-03 covers a missing one). Other formats and sizes, including a non-square
180 x 120, are reported with the measured size. The twelve `*craft` apps are prebuilt upstream packages (`upstream.toml`): only
the manifest, CHANGELOG and upstream rules (APP-*) apply to them.

## Runtime checks (`tests/compliance.rs` in each app crate)

Generated from `tools/templates/app-compliance.rs`. The test spawns the real app binary through
`springboard::shell::apphost::AppHost` (no compositor, no GPU), configures 1200x780 pt at scale 1.0 and 1.5625, Light and
Dark, and asserts:

- render: hello arrives, a frame of the configured size with more than one colour, the process stays alive;
- chrome: sidebar rows (Row/List), toolbar buttons and a search field are published as controls when the source uses
  `source_list`, `toolbar` or `search_field`;
- labels: every visible Button, TextField, Toggle, Checkbox, Slider, SegmentedControl and MenuItem has a label;
- menus: the app's menus are unique and none is File, Edit, View, Window or Help (the shell bar adds those);
- snapshot: a quarter-size golden per appearance at scale 1.0 (`tests/snapshots/<app>-<light|dark>.png`, about 170 KB;
  `MAX_DIFF_FRAC` per app tolerates live data such as dates).

`KNOWN_GAPS` in each file lists check ids that fail today (seeded with `menus` where CHR-01 fires). They print as
`known gap` and do not fail; delete the id when the app is fixed. The system keyboard has `FRAME_REQUIRED = false`
(TODO(lead): it draws only after `keyboard_config`). The first run of the new tests was not compiled or run when they
were written: expect to triage with `COMPLIANCE_SOFT=1` and bless the first goldens.

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
| keyboard | 86 | 16 | COL-01 x9, TYP-01 x5, COL-02 x1 |
| calculator-desktop | 88 | 15 | LAY-04 x11, COL-01 x3, GG-04 x1 |
| weather-desktop | 92 | 7 | LAY-04 x4, COL-03 x3 |
| assistant-desktop | 100 | 0 | none |
| books-desktop | 100 | 0 | none |
| browser | 100 | 0 | none |
| cadcraft-desktop | 100 | 0 | none |
| calendar-desktop | 100 | 0 | none |
| clock-desktop | 100 | 0 | none |
| console-desktop | 100 | 0 | none |
| contacts-desktop | 100 | 0 | none |
| daw | 100 | 0 | none |
| deckcraft-desktop | 100 | 0 | none |
| designcraft-desktop | 100 | 0 | none |
| effectcraft-desktop | 100 | 0 | none |
| files | 100 | 0 | none |
| filmcraft-desktop | 100 | 0 | none |
| freeform-desktop | 100 | 0 | none |
| games-desktop | 100 | 0 | none |
| gridcraft-desktop | 100 | 0 | none |
| lightcraft-desktop | 100 | 0 | none |
| music-desktop | 100 | 0 | none |
| news-desktop | 100 | 0 | none |
| notes-desktop | 100 | 0 | none |
| pdfcraft-desktop | 100 | 0 | none |
| photocraft-desktop | 100 | 0 | none |
| podcasts-desktop | 100 | 0 | none |
| reminders-desktop | 100 | 0 | none |
| remote | 100 | 0 | none |
| shortcuts-desktop | 100 | 0 | none |
| soundcraft-desktop | 100 | 0 | none |
| vectorcraft-desktop | 100 | 0 | none |
| voicememos-desktop | 100 | 0 | none |
| wordcraft-desktop | 100 | 0 | none |

<!-- app-compliance-table:end -->
