# Keyboard settings (round 75): Settings > Keyboard made live

Slug `keyboard-settings`. Owner rule: the Keyboard pane changes what the input path does, not only what the page shows.

## Reference evidence (clean room)

- `~/Projects/macos-ref/extract/ref27-index.tsv`: `System/Library/ExtensionKit/Extensions/KeyboardSettings.appex` and `KeyboardSettingsIntents.appex`, plus `System/Library/PreferencePanes/Keyboard.prefPane` in the reference index.
- `KeyboardSettings.appex/Contents/Info.plist` (no extension attributes of note; the pane is an extension with no declared URL schemes or document types).
- `KeyboardSettings.appex/Contents/Resources/en.lproj/Keyboard.searchTerms`, read with `plistlib` for its section keys: `Keyboard`, `ModifierKeys` (Caps Lock, Control, Option, Command, Globe key), `CustomizeModifierKeys`, `FunctionKeys` (Fn, F1 to F12), `Shortcuts`, `Spotlight`, `Services`, `TextReplacements`, `Spelling`, `UseSmartQuotes`, `InputSources`, `CapsLockInputSources`, `Dictation`, `Accessibility`, `TouchBarSettings`, `LanguageInputMethods`.
- No REA evidence ids: the extract did not contain the pane's nib or storyboard, so the choice labels and the key-repeat slider ends come from the BifrOSt Keyboard pane's existing wording and the repeat ranges in `springboard/wm/src/input.rs`.

## Rows

| Entry (matrix) | Status | BifrOSt path | What is done | Still needs Midget checking |
|---|---|---|---|---|
| `path:/System/Library/ExtensionKit/Extensions/KeyboardSettings.appex` (settings extension, Keyboard page) | partial, closes on Midget | `springboard/src/shell/pages.rs` (`PageId::Keyboard`, `ModifierKeys`, `Shortcuts`), `springboard/src/shell/settings.rs` (toggles, picks, sliders), `springboard/src/shell/kb_input.rs`, `springboard/src/shell/options.rs` | Key Repeat sliders (Delay Until Repeat, Key Repeat) drive the shell's repeat; Modifier Keys remaps Caps Lock, Control, Option and Command; Function Keys switch F1 to F12 to standard keys; Keyboard Navigation switches Tab focus; Keyboard Shortcuts switches off the ten shell shortcuts; Text Replacements expand as a hardware keyboard types a word. | Remapped modifiers while a key is held (a settings change mid-press could leave the modifier count stuck until the next release); F1 to F12 reaching the terminal and text fields; Tab-off behaviour in Settings and Files; text replacement expansion in Files, Settings and Browser fields. |
| `path:/System/Library/PreferencePanes/Keyboard.prefPane` (Keyboard page) | partial | `springboard/src/shell/pages.rs` (`PageId::Keyboard`) | The same page as above, with the existing Keyboards and Text Replacements links kept. | Same as above. |
| `path:/System/Library/ExtensionKit/Extensions/KeyboardSettingsIntents.appex` | gap, not touched in this slice | none | none | Intents for the Keyboard settings. |

## Design

- **Input path**: `springboard/src/shell/kb_input.rs` is the one place the input path reads. Settings publishes a `KeyPrefs` with `kb_input::set` (called from `Settings::apply_keys` and `Shell::apply_options`). `src/bin/shell.rs` reads it with `kb_input::get()` on every key event. This is the same pattern as `bare::pointer_prefs`.
- **Key repeat**: `KeyRepeat` takes the delay and rate through `set_timing`; the defaults stay 400 ms and 30 Hz. The keys match `key_repeat_delay_ms` and `key_repeat_rate` in `settings.ini`, which `wm/src/input.rs` already reads, so the compositor path follows the same values.
- **Modifier remap**: `Remap::apply` changes the evdev code before `HwKeys::key_in` sees it. Caps Lock, Control, Option and Command each choose one of Default, Caps Lock, Control, Option, Command, Escape or No Action.
- **Function keys**: `HwKeys::typed` returns `F1` to `F12` (as `Out::Named`) only when Standard is on. The terminal's key map (`apphost.rs`, `ext_type_out`) turns those names into evdev codes.
- **Tab navigation**: `hw_type` drops a Tab that no terminal or external app should see when Keyboard Navigation is off.
- **Shortcuts**: `kb_input::BINDINGS` lists the ten shell shortcuts that can be switched off (Search, Switch Application, Close Window, Minimize Window, Full Screen, Tile Window, Cycle Windows, Next Input Source, Settings, Home). `kb_input::enabled` filters the `Shortcut` that `key_in` returns, so a switched-off key falls through to the app in front. The Keyboard Shortcuts page shows these as switches; the reference lists keep the app shortcuts.
- **Text replacements**: `WordBuffer` follows the characters a hardware keyboard types. When a space or punctuation ends a word that matches a replacement, the shell erases the word and types the phrase before the key. `text_services::expand_word` does the lookup. Expansion is off in the terminal and in external apps, and a pointer press clears the buffer.

## Limits and decisions

- Shortcuts are switched on or off, not rebound. Recording a new key combination needs a key-capture sheet that does not exist yet.
- The Globe key is not remapped, and the Caps Lock change only affects the shell's own key path (the compositor's xkb setup is not touched).
- Text replacements can be removed in Settings but not added: adding needs a text entry in the settings page (not in this slice). Replacements expand only on the hardware-keyboard path; the on-screen keyboard does not expand them yet.
- Smart quotes, Spelling and Dictation sections in the reference are not in this slice.
- The matrix rows were not edited (the lead closes them).

## Tests

Host unit tests (not run here; the lead builds): `kb_input` (remap, function keys, shortcut filter, word buffer, the shared `KeyPrefs`), `options` (new keys round-trip, clamping, the slider mappings, `key_prefs`), and the existing `pages`/`hwkeys` tests keep their assertions.
