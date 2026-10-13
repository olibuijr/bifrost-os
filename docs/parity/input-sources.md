# Input sources, parity round 75

Branch `worktree-wf_7b944f38-f89-1`, base `wave-base` (1df21171). Slice: the input-methods rows in
`docs/parity-matrix/os/input-methods/`. Nothing was built or run in this round (the lead builds once). Matrix JSON is not
edited here; closures are reported by the lead.

## Evidence used

- Matrix rows: `docs/parity-matrix/os/input-methods/{a-c,d-f,j-l,p-r,s-s,t-v,0-9}.json`. The rows are heuristic
  (`method: heuristic`) and mostly `gap`; the one `partial` is `AppleKeyboardLayouts` (`springboard/src/shell/kb_layouts.rs`).
- Reference bundles: `~/Projects/macos-ref/extract/ref27-index.tsv` lists the input-method bundles, the CharacterPicker and
  InputMethodKit accessibility bundles. The extract under `ref27/` holds only the Applications, Library and usr trees
  so far; the input-method bundles' Info.plist and resources are not extracted yet, so no REA evidence was captured here.
  Behaviour is taken from the existing parity notes (`wire5-input-method.md`, `wire6-input-candidates.md`) and from the code.
- Engines on the system: `fcitx5` 5.1.22 is installed (`/usr/lib/fcitx5`), with the frontends (`libdbusfrontend.so`,
  `libibusfrontend.so`, `libwaylandim.so`) and `libemoji.so`, but no Japanese or Chinese engine addon (no kana, pinyin,
  kkc or anthy). So no CJK engine exists to call, and the built-in kana method is the fallback the brief asks for.

## Rows

| Row (matrix name) | API or surface | BifrOSt path | Done | Still open |
|---|---|---|---|---|
| `AppleKeyboardLayouts` | keyboard layouts applied in the input path | `springboard/src/shell/kb_layouts.rs` (three sources), `springboard/src/installer/keymaps.rs` (`key_levels`, the tables), `springboard/src/bin/shell.rs` (the hardware layout follows the source) | Japanese (Romaji) is a third source on the US keymap; the Settings toggles list all three; the keyboard viewer draws the active source's keys from the same tables | Settings toggles and the menu label need a Midget check; the on-screen keyboard keeps the source it was shown with |
| `JapaneseIM-RomajiTyping` | romaji to kana composition | `springboard/ui/src/kana_method.rs` (`KanaMethod`, `katakana_of`); `springboard/src/shell/compose_window.rs` (`kana_search_key`, `kana_show`, `kana_open_list`, `kana_end`) | romaji becomes marked hiragana in the Search field; Space opens the hiragana and katakana list; a digit picks; a sokuon (`kk`) and `nn` work; `-` is the long-vowel mark | Search field only (no other shell field or app takes kana yet); no kanji; no candidate from a dictionary. Partial |
| `JapaneseIM-KanaTyping` | direct kana keyboard layout | none | not started | the kana keyboard map is not built (gap) |
| `CharacterPalette` | Emoji & Symbols viewer | `springboard/src/shell/character_viewer.rs` (`CharacterViewer`, `Hit`, `Shell::char_viewer_*`); catalogue `springboard/render/src/emoji.rs` (`Category`, `CATEGORIES`, moved from the keyboard app); menu `desktop_bar.rs` (Edit > Emoji & Symbols) | a floating panel with ten emoji tabs, paged 50 to a page, a pick types into the open Search field or copies to the clipboard; Done or Escape closes it | no search field over the catalogue, no recents, no symbol tabs (the catalogue holds emoji only); needs a Midget check of the panel placement |
| `PressAndHold` | accent popup on key hold | `springboard/src/shell/hwkeys.rs` (`KeyRepeat::accent_hold`); `springboard/src/bin/shell.rs` (the hold check runs before the repeat); `compose_window.rs` (`accent_popup_ready`, `accent_hold`) | holding a letter with accents in the Search field takes the letter back and opens its accent list, digits pick | Search field only; other fields still repeat the key. Partial |
| `EmojiFunctionRowIM` | emoji on the function row | none | not started | gap; no function-row emoji on a PC keyboard |
| Keyboard Viewer (system UI, no matrix row of its own) | Input menu > Show Keyboard Viewer | `springboard/src/shell/keyboard_viewer.rs` (`BOARD`, `legend`, `KeyboardViewer`, `Shell::keyboard_viewer_*`); menu `desktop_bar.rs` (Input) | a floating ANSI board, plain and Shift legends from the active source, the held key lit; Done closes it; the rest of the screen stays usable | AltGr levels not drawn; the ANSI shape for every source; needs a Midget check |
| Input source menu and switch (already built) | Ctrl+Space, menu bar input menu | `springboard/src/shell/hwkeys.rs` (`Shortcut::InputSource`), `desktop_bar.rs` (`Menu::Input`), Help row added | Help now lists Input Source with Ctrl+Space | no source HUD on a switch (the menu bar label is the only feedback) |

## Not built, with the reason

- CJK engine (pinyin, kana kkc): no engine addon is installed under fcitx5, and no dictionary ships. The built-in kana
  method above is the only Japanese path. A pinyin path needs a dictionary first.
- Source HUD (a label flashed on a switch): not drawn. It needs the damage and frame-cache wiring that the volume HUD
  has, and no visible gap is worse without it. The menu bar label stays the feedback.
- Ctrl+Cmd+Space as a shortcut for the character viewer: the chord is already Search (`QuickSearch` takes Cmd or Ctrl with
  Space before the input source switch), so the viewer is reached from the Edit menu only.

## Midget checks owed

1. Settings > General > Keyboard > Keyboards lists Icelandic, English (US) and Japanese (Romaji); the menu bar label reads
   `あ` for the Japanese source.
2. On Japanese, type `nihon` in Search: marked text `にほn`; Space shows the hiragana and katakana list; `2` picks katakana.
3. Hold `e` in Search on English: the accent list opens with é, è, ê, ë; `2` picks è.
4. Edit > Emoji & Symbols: tabs switch, the pages turn, a pick lands in the Search field, Done closes it.
5. Input menu > Show Keyboard Viewer (with two sources): Icelandic shows ð on the key after P; holding a key lights it.

## Shared files touched (merge with care)

- `springboard/src/shell/mod.rs`: module lines, the `char_viewer` and `keyboard_viewer` fields and their `None` initialisers,
  two touch hooks in `modal_event`, the modal draw block, and the two quiet-frame checks.
- `springboard/src/shell/desktop_bar.rs`: two `Action` variants, the Edit row, the Input rows, the Help row, two dispatch arms.
- `springboard/src/shell/hwkeys.rs`: `KeyRepeat::accent_hold` and its test; the Back shortcut checks the viewer first.
- `springboard/src/bin/shell.rs`: one call to `keyboard_viewer_key` after `tap_key`; the accent hold check before `due`.
- `springboard/src/shell/settings.rs`: the layout toggle index is bounded by the source count.
- `springboard/render/src/emoji.rs`: the catalogue appended; `springboard/apps/keyboard/src/emoji.rs` re-exports it. The
  keyboard app (`bifrost-keyboard`) is bumped to 1.1.1 in `apps/keyboard/Cargo.toml` and in `Cargo.lock`, with a
  CHANGELOG line; it has no `manifest.toml` of its own.

## Migrations and deletions

- The emoji category catalogue moved from `springboard/apps/keyboard/src/emoji.rs` to `springboard/render/src/emoji.rs`;
  the keyboard keeps a `pub use` of `Category` and `CATEGORIES`, so its callers are unchanged. No copy is left in the app.
