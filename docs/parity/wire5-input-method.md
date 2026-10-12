# Input method parity, wiring wave 5

Branch `wire5/input-method`. Scope: the input source switcher, the menu bar input menu, and composing text in the shell's
own text fields. The provider side (`InputMethodKit` server, controllers, candidates window) is still not wired. See gaps.

## What is now wired

| Piece | Where | What a user sees |
|---|---|---|
| Input source switch | `springboard/src/shell/kb_layouts.rs` (`next_in`, `cycle`, `Layout::keymap`, `Layout::short_name`); `hwkeys.rs` (`Shortcut::InputSource`) | Ctrl+Space moves to the next enabled keyboard (Icelandic, English (US)). Cmd+Space still opens Search. |
| Hardware layout follows the source | `springboard/src/bin/shell.rs` (`seen_source` before `hw_keys.typed`) | Typed keys use the newly chosen keymap (`is` or `us`) from the next key press, whether the change came from Ctrl+Space or the menu. |
| Menu bar input menu | `springboard/src/shell/desktop_bar.rs` (`Menu::Input`, `Action::InputSource`, extras in `menu_buttons`, rows in `own_menu_rows`) | A menu extra left of Control Center labelled `IS` or `ABC` (the active source). It shows only when two or more keyboards are enabled. Its rows list the enabled keyboards, the active one checked; choosing one makes it active. |
| Composing text in shell fields | `springboard/src/shell/textfield.rs` (`set_marked`, `unmark`, `marked_range`, underline in `draw`) | A `TextField` can hold a composition: text marked and underlined, committed (kept) or cancelled (removed, caret falls back). A key typed by the user ends the composition and keeps the text. |

Ctrl+Space was bound to Search before this wave. It is now the input source switch, and Search moved to Cmd+Space only
(`hwkeys.rs`, `chord` tests updated: Ctrl+Space is `InputSource`, Cmd+Ctrl+Space stays `QuickSearch`).

Only the layouts in `kb_layouts` are switched (Icelandic and English (US)). The enabled set is changed in Settings >
General > Keyboard > Keyboards, as before.

## How a user reaches it

1. Settings > General > Keyboard > Keyboards: switch on English (US). The input menu appears in the menu bar.
2. Ctrl+Space (with a hardware keyboard) moves to the next source. Or click the input menu and choose a keyboard.
3. Type in any text field: the keys follow the chosen keymap.

## How to verify on Midget (test PC)

Build and push the fast shell, then:

```
cd springboard && cargo build --offline --profile fast --bin shell --features desktop-wayland
tools/midget-push-shell.sh --no-build shot.png
bifrost pc midget state
```

1. Enable English (US) in Settings > General > Keyboard > Keyboards. Screenshot the menu bar: `IS` or `ABC` appears
   next to Control Center.
2. Press Ctrl+Space on the hardware keyboard. The label changes. Type `y` and `z` in the Search field: on the US source
   `y` and `z` come out as on a US board, on the Icelandic source the same keys differ.
3. Press Cmd+Space: Search opens (unchanged).
4. Click the input menu: two rows, the active one with a check. Choose the other one: the label changes.
5. Compose in a shell text field: not reachable from the UI yet (see gaps). Covered by host tests only.

Host tests added (not run in this round; the lead builds once):
- `kb_layouts::tests`: `next_in` wraps, a source not enabled moves to the first enabled one, keymap and short labels.
- `hwkeys::tests::chords_map_to_shortcuts` / `plain_keys_and_other_chords_do_nothing`: Ctrl+Space is `InputSource`,
  Cmd+Ctrl+Space stays `QuickSearch`.
- `textfield::tests`: `composing_text_is_marked_then_kept_or_removed`, `a_cancelled_composition_is_removed_and_the_caret_falls_back`,
  `a_typed_key_ends_the_composition_and_composing_is_cut_at_the_limit`. Expected values worked out from the code: for
  `"にほ"` with `sel_start` 3 the caret is one char into the marked text (3 bytes is `"に"`); a cancelled `"xy"` at
  `sel_start` 1 leaves `"ab"` with the caret back at 2.

## REA and official evidence

Official pages (checked this round with `python3 -I tools/parity/docc.py inputmethodkit`): the InputMethodKit landing page
lists `IMKCandidates`, `IMKInputController`, `IMKServer`, `IMKMouseHandling`, `IMKServerInput`, `IMKStateSetting`.

REA evidence recorded for this area (from earlier waves, not recaptured in this round):

| Page | evidence_id |
|---|---|
| InputMethodKit | ev_4f76b381a49e71bf77bec3bd4cf89ff199682ce9f119dac1893dd155febd1ba1 |
| IMKServerInput | ev_7aa64715087049591aa99482930a721ab4c0b6a5a2bd62e0d169dc1fc722ec5b |
| IMKInputController | ev_174418e7339be3454e258a6cc496de04ac01010826df86b75531ae6406a54b03 |
| IMKStateSetting | ev_b589d4abcb28d6a9be365d9f45c83794e3709fcfc5aef9a2b45d824095483ed4 |
| InputMethodKit (matrix) | ev_d889a7762311f1422e0f0dd0cf934d9bdcf457bd459b3adb2cd0f2726112a81b |

No REA page was captured for the Ctrl+Space source switch: the InputMethodKit pages do not describe the system hotkey. The
behaviour (Ctrl+Space selects the previous input source, Cmd+Space Spotlight) is the reference platform's default keyboard
shortcut; a REA capture of the keyboard settings page is still owed.

## Matrix rows

| Slug | Feature (as in the matrix json) | BifrOSt path |
|---|---|---|
| inputmethodkit | Marked text (composition) set and clear | springboard/src/shell/textfield.rs (`set_marked`, `unmark`, underline in `draw`) |
| inputmethodkit | Commit composition | springboard/src/shell/textfield.rs (`unmark`, commit) |

Rows stay `partial`. No row is `parity`: the composition in shell fields is not reachable by a user, and the input
method provider side is missing.

## Remaining gaps

- Candidate window: not drawn. `input_method::InputController` and `candidates::Candidates` exist, but nothing in the shell
  creates a controller, and the keyboard app never emits candidates. Its drawing needs a candidate source first.
- Composition reaches external apps only (`apphost.rs`, `ext_forward_composition`). A keyboard composition does not
  route to the shell's own `TextField` yet, so the underline in shell fields is reached by no user path.
- The on-screen keyboard keeps the layout it was shown with until it is shown again (its config is sent on claim).
  Changing the source while it is up does not reconfigure it.
- No input source HUD (the brief label flash) is drawn on a switch. The menu bar label is the only feedback.
- No provider process can connect: `InputMethodKit` server and connections stay partial.
- The Help menu does not list Ctrl+Space yet.
