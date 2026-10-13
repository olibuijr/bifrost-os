# Accessibility parity (wave 2)

Two gaps from the Accessibility matrix (`docs/parity-matrix/data/accessibility.json`) are now modelled in pure Rust in
`springboard/ui`: Switch Control scanning and the Accessibility Inspector's audit checks. Both are pure models. They do
not read switch hardware, draw anything, or open an inspector window.

## What was built

- `springboard/ui/src/switch_scan.rs` (`pub mod switch_scan;` in `springboard/ui/src/lib.rs`). `Scanner` walks the
  elements of a screen in reading order and a switch press selects the highlighted one. `Style::Auto { interval_ms }`
  moves the highlight on `tick` once the interval passes. `Style::Manual` moves it only on `step`, forward or back.
  `start`, `step`, `tick`, `select`, `stop` and `set_items` are the surface. Reading order and disabled-element skipping
  come from the existing `focus::next`, so keyboard focus and switch scanning cannot disagree. A highlight that vanishes
  when the screen changes is dropped, and the next step starts again at the first element.
- `springboard/ui/src/ax_audit.rs` (`pub mod ax_audit;` in `springboard/ui/src/lib.rs`). `Audit::run` takes the elements
  an `ax::Map` produces and the window bounds, and returns `Issue { address, kind, detail }`. The kinds are `Unlabeled`
  (an interactive role with no name), `Empty` (zero size), `Clipped` (partly outside the window), `OffWindow` (entirely
  outside), `SmallTarget` (an interactive element below `min_target`, default `MIN_TARGET_PT` = 44 pt, a BifrOSt default),
  `DuplicateName` (a repeated role and name, reported on the later element) and `LowContrast`. The contrast helpers are
  `luminance`, `contrast_ratio`, `contrast_ok` and `contrast_issue`, using the WCAG 2.x relative-luminance formula,
  with 4.5:1 for body text and 3:1 for large text.

Both modules have unit tests (8 in `switch_scan`, 8 in `ax_audit`).

## Official API names mirrored

- `AccessibilityTechnology.switchControl`: https://developer.apple.com/documentation/accessibility/accessibilitytechnology/switchcontrol
  (macOS 15.0, Mac Catalyst 18.0). The Switch Control page is at https://developer.apple.com/documentation/accessibility/switch-control.
- Accessibility Inspector: https://developer.apple.com/documentation/accessibility/accessibility-inspector. Its page
  says the inspector "display[s], query[ies], and test[s] accessibility information", and audits for "clipped text and
  unlabeled elements, and uses appropriate text size and color contrast levels". `ax_audit` mirrors those checks.
- The audit article (`performing-accessibility-audits-for-your-app`) returned 404 from the DocC endpoint. The check list
  above relies on the Accessibility Inspector landing page only.

## REA evidence

- Accessibility landing page: `ev_7495858641a99e4a9756ef7c1c3a92bdd997894f78248fd6257f0748565e7095`
- Accessibility Inspector page: `ev_29aeb5d313161e77a265e1f5d1c56cf463d9da1f6ae7dc3c66dcf921415196cc`
- AccessibilityTechnology.switchControl page: `ev_317df59db5425495a2a43ac1c88607060cf6485a0d7db306a27b0f53e3019eb3`

## Matrix rows closed

| Framework | Feature (as in data/accessibility.json) | New status | BifrOSt path |
|---|---|---|---|
| accessibility | Switch Control | partial | `springboard/ui/src/switch_scan.rs` |
| accessibility | Accessibility Inspector | partial | `springboard/ui/src/ax_audit.rs` |

## Not done, and why

- Switch Control has no input source. Nothing in the shell turns a switch, game controller or sound into `step` or
  `select`, and no shell chrome draws the highlight. Those need hardware and renderer work outside this pure-Rust scope.
- The Accessibility Inspector has no window or app-facing panel. `ax_audit` returns the issues; nothing shows them yet.
- The contrast check is not wired to the renderer's colours or to a live text element. A caller must supply both colours.
- `Voice Control`, `Braille` and `audio graphs` stay gaps. This pass did not touch them.
- `tools/parity/parity.py search` crashes in this worktree (`semantic.py` fails to load the embedding cache with a numpy
  format error), so the pre-build "what already exists" search was done with the akurai index and grep instead.
- Nothing was built or run, per the wave rules. The modules are unverified by the compiler and tests.

## Round 75: screen reader speech, Accessibility Shortcut

Scope: screen reader speech, Zoom, Increase Contrast, Reduce Transparency, Reduce Motion, cursor size and the
Accessibility Shortcut. Matrix source: `docs/parity-matrix/data/accessibility.json` (rows are stale; not edited here).

Reference evidence: `~/Projects/macos-ref/extract/ref27-index.tsv` lists `System/Library/CoreServices/UniversalAccessControl.app`
(the Accessibility Shortcut panel) but its Info.plist and `.strings` files are not in `extract/ref27/` yet, so no panel
labels or option lists were taken from the reference. The panel option set used here is the BifrOSt one (below). No REA
evidence ids were recorded this round; re-check when the UniversalAccessControl resources land.

### Rows

| Feature (matrix row) | API / behaviour | BifrOSt path | State |
|---|---|---|---|
| Screen reader bridge, announcements | `announce`, reader moves | `springboard/src/shell/accessibility/banner.rs` (`announce_tick`), `speech.rs` | Done: announcements are spoken while Screen Reader is on, through `espeak-ng` or `espeak` when on PATH. Silent without an engine. Needs Midget check (engine installed, audible output, a move interrupts the previous utterance). |
| Accessibility Shortcut (Option+Cmd+F5) | one key toggles chosen features | `springboard/src/shell/accessibility/shortcut.rs`, `hwkeys.rs` (`Shortcut::AccessibilityShortcut`, `KEY_F5`), `settings.rs` (`set_accessibility`) | Done for the key and the toggle; the choice is the `a11y_shortcut` pref (comma list of `screen_reader`, `zoom`, `reduce_motion`, `increase_contrast`, `reduce_transparency`, `button_shapes`, `dim_flashing_lights`). Not done: the panel and the Settings choice list. With nothing chosen the key does nothing. |
| Increase Contrast, Reduce Transparency, Reduce Motion (live) | toggles apply without restart | `accessibility/mod.rs` `publish()` runs on every shell tick (`shell/mod.rs` ~1274) | Already live through the appearance globals and the display options file; no change this round. Needs Midget check. |
| Zoom (full screen and picture-in-picture) | screen-wide magnification | not built | Not done. The only zoom is the Display Zoom geometry factor (`ZOOMS`) and the camera Magnifier. A screen magnifier needs a final-frame transform in the GPU presenter (`springboard/src/gpu/wl.rs`) and a lens rectangle for picture-in-picture. |
| Cursor size | pointer scale setting | not built | Not done. The hardware cursor is set from `bare/kms.rs` `set_cursor(argb)`; a size setting needs a scaled cursor image there and a value in the display options file. |

### Migrations and deletions

None. The speech path is new; the reader's announcements already went through `Banner::post`, which is unchanged.
`Banner::tick` now returns whether a new text came on screen (its only caller is `announce_tick`).

### Files touched

- `springboard/src/shell/accessibility/speech.rs` (new)
- `springboard/src/shell/accessibility/shortcut.rs` (new)
- `springboard/src/shell/accessibility/banner.rs` (speak on new announcement, screen reader gate)
- `springboard/src/shell/accessibility/mod.rs` (`pub mod shortcut;`, `pub mod speech;`)
- `springboard/src/shell/mod.rs` (fields `speech`, initialiser; shared file, a few lines)
- `springboard/src/shell/hwkeys.rs` (variant, key constant, chord, dispatch, one test line)
- `springboard/src/shell/settings.rs` (`set_accessibility` next to `accessibility()`)

### Midget checks still needed

- Install `espeak-ng` on Midget and confirm the reader's moves are spoken and a new move cuts the old one off.
- Option+Cmd+F5 with a chosen feature set (edit `a11y_shortcut` in settings.ini) toggles the Settings switches while Settings is open.
- Live toggles of Increase Contrast, Reduce Transparency and Reduce Motion repaint without a restart.
