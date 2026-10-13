# Input source HUD, per-app input source, fcitx5 bridge (slug: input-source-hud)

Round 75. Code and commits only: nothing here is built or run. The lead builds, tests and checks the Midget behaviour.

## Reference evidence

No REA or plist measurement was made this round. The shell's input-source behaviour (Ctrl+Space, the input menu,
the source labels) already exists in the code from earlier waves, and the reference index
(`~/Projects/macos-ref/extract/ref27-index.tsv`) lists the input method bundles only as heuristic `gap` rows. The
HUD's look follows the existing volume HUD (`springboard/src/shell/volume_hud.rs`), not a measured reference. A
follow-up should run `inspect_plist` on the input menu's owning bundle before the HUD's timing and size are trusted.

## Matrix rows

| Entry | Status here | BifrOSt path |
|---|---|---|
| JapaneseIM-RomajiTyping | candidate, not closed: the Japanese (Romaji) source exists and now shows in a HUD. Needs Midget check. | `springboard/src/shell/kb_layouts.rs` (`Layout::Japanese`), `springboard/src/shell/input_source_hud.rs` |
| JapaneseIM-KanaTyping | partial, not closed: kana composition is in process (`bifrost_ui::kana_method`); no kana keyboard layout; kanji and candidates come only from fcitx5 when it is installed | `springboard/src/shell/fcitx_bridge.rs`, `springboard/src/shell/compose_window.rs` |

## Per item

### 1. HUD on a source switch (done, needs Midget check)

- API: none (shell internal). `SourceHud` (`show`, `alpha`, `visible`, `next_wake`, `tick`, `rect`, `draw`).
- Path: `springboard/src/shell/input_source_hud.rs`. Shell wiring in `springboard/src/shell/mod.rs` (fields,
  `next_wake`, `tick_inner`, `hud_damage`, `draw`, the two frame gates).
- Triggers: Ctrl+Space (`hwkeys.rs`, `Shortcut::InputSource`) and the input menu rows (`desktop_bar.rs`,
  `Action::InputSource`). Both call `Shell::show_input_source`.
- Behaviour: a fixed-width panel under the status bar names the source ("Icelandic", "English (US)",
  "Japanese (Romaji)"), holds for 1.2 s, fades over 250 ms.
- Midget check: panel position under the status bar, legibility over the wallpaper, and that it does not clip on the
  smallest screen.

### 2. Menu bar shows the current source (already present, unchanged)

- The menu bar extra shows `Layout::short_name()` (IS, ABC, あ) and the input menu lists the enabled sources with a
  check on the active one (`desktop_bar.rs`, `Menu::Input`). No change this round.
- Not an icon: the label is text. A glyph would need a reference asset, which was not measured.

### 3. Per-app input source (done, named per app)

- The shell only knows apps, not documents, so the memory is per app, not per document. This is a gap against the
  brief's "per document" wording: a document-level option needs the app to report its document to the shell, which
  the SDK does not do yet.
- Setting: Settings > General > Keyboard > Keyboards > "Remember Input Source for Each App" (`pages.rs`, `settings.rs`,
  `Toggle::RememberInputSource`). Stored as `remember=0|1` in `kb_layouts.ini`. Off by default.
- Model: `springboard/src/shell/input_memory.rs` (`InputMemory::focus`). The shell calls `input_memory_tick` from
  `tick_inner` on every tick; a change of the app in front keeps the source of the app left and restores the source
  of the app coming to the front. The desktop, the Switcher and the lock screen are not apps and keep the source in use.
- Midget check: switch source in app A, switch to app B, back to A.

### 4. fcitx5 bridge for CJK (partial)

- `springboard/src/shell/fcitx_bridge.rs`. When `fcitx5-remote` is on the PATH, a switch to the Japanese (Romaji)
  source runs `fcitx5-remote -s mozc`, and a switch to a Latin source runs `fcitx5-remote -c`. Both run on their own
  thread. The call site is the shell's hardware-source change in `springboard/src/bin/shell.rs`.
- `fcitx5-remote` options checked from its `--help` on this machine: `-s <imname>`, `-c`, `--check`.
- Not done: the `mozc` input method name is the usual fcitx5 unique name but was not checked against an installed
  fcitx5-mozc; no pinyin source is wired (fcitx5 pinyin would be a fourth source); routing the Search field's keys
  through fcitx5 (the Wayland text-input protocol) is not built, so the in-process kana composition still handles
  the Search field. No dictionary is shipped with BifrOSt, so candidates come from fcitx5 only.

## Files touched

- New: `springboard/src/shell/input_source_hud.rs`, `springboard/src/shell/input_memory.rs`,
  `springboard/src/shell/fcitx_bridge.rs`, `docs/parity/input-source-hud.md`.
- Edited: `springboard/src/shell/kb_layouts.rs` (remember flag, load/save), `springboard/src/shell/mod.rs`
  (module lines, fields, wiring; shared file), `springboard/src/shell/hwkeys.rs`, `springboard/src/shell/desktop_bar.rs`,
  `springboard/src/shell/pages.rs` (Toggle variant, Keyboards page group), `springboard/src/shell/settings.rs` (toggle arm),
  `springboard/src/bin/shell.rs` (fcitx call).

## Shared-file edits for the lead

- `springboard/src/shell/mod.rs`: three `pub mod` / `use` lines, three struct fields and their initialisers, one
  `next_wake` block, one `tick_inner` line and one `tick` block, one `hud_damage` block, one `draw` block, and the
  frame gate lines (two, `settings_key` and the terminal quiet check, each gained `input_hud` terms). Merge with care
  against any other wave touching the same functions.
- `springboard/src/shell/pages.rs`: the `Toggle` enum gained `RememberInputSource`; the Keyboards page gained a second group.

## Migrations and deletions

- None. No superseded code existed for the HUD or the per-app memory. The source-switch call in `hwkeys.rs` no longer
  ignores the new source (it passes it to the HUD).

## Unit tests added

- `input_source_hud.rs`: show/hold/fade, a second switch restarts the hold, `tick` clears after the fade, hide, a shell
  switch names the new source.
- `input_memory.rs`: restore on return, desktop and Switcher remember nothing, option off keeps nothing, same app in front.
- `fcitx_bridge.rs`: args for Japanese and Latin sources, PATH with no fcitx5 means not installed.
- `kb_layouts.rs`: no new tests; the remember flag is read through `remembers()` and set through `set_remember()`.
  `load` and `save` skip the file in test builds, so the flag cannot be checked by a unit test without a refactor.
