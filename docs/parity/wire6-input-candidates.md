# Input candidates, wiring wave 6

Branch `wire6/input-candidates`. Scope: a candidate window for composing text, fed by a simple built-in compose method,
in the shell's Search field. The input method server, the per-session controller and key routing through a provider
process are still not wired (see gaps). Nothing was built or run in this round (the lead builds once).

## What is now wired

| Piece | Where | What a user sees |
|---|---|---|
| Built-in compose method (model) | `springboard/ui/src/compose_method.rs` (`Compose`, `Step`, `candidates_for`, `pick`, `PREFIX`, `MAX_SHOWN`); exported from `bifrost_ui` in `springboard/ui/src/lib.rs` | A typed apostrophe arms the method. The next letter with alternates opens its list: `a` gives a, á, à, â, ä, ã, å, æ (upper case when the letter is upper case). Icelandic letters: á, é, í, ó, ú, ý, ö, æ, ð, þ. |
| Candidate window over the Search field | `springboard/src/shell/compose_window.rs` (new): `ComposeWindow`, `compose_search_key`, `compose_open`, `compose_finish`, `draw_compose` | A floating list at the top centre of the screen, one numbered row per candidate (`1   á`, `2   à`, ...). It is painted last in `Shell::draw`, above the windows. |
| Composition in the field | same file; `springboard/src/shell/quicksearch.rs` (`field_mut`) | The letter shows as marked (underlined) text in the Search field while the list is open. Digits 1 to 9 pick a candidate and commit it as plain text. |
| Session model | `springboard/ui/src/imsession.rs` (`InputSession`, the existing provider-side model) | The shell holds one `InputSession` per open list (client 0, `SingleColumnScrolling`). It takes the candidates, the selection and the commit. |
| Key routing | `springboard/src/shell/hwkeys.rs` (`hw_type`: the Search arm calls `compose_search_key`); `springboard/src/shell/mod.rs` (field `compose_ime`, the draw call, the constructor) | Keys typed while Search is open pass through the compose method first. Any other key (Enter, Backspace, Escape, arrows) ends an open list and keeps the letter. |

Other keys: a typed key that is not a digit ends the list and keeps the letter, then the key is typed normally. An
apostrophe typed before a letter with no alternates (for example `'z`) is inserted as typed, so the text is never lost.

## How a user reaches it

1. Open Search (drag the home screen down, or Cmd+Space on a hardware keyboard).
2. Type an apostrophe, then `e`. The `e` appears underlined in the field and a list opens: `1 é 2 è 3 ê 4 ë`.
3. Press `2`. The field now reads `è`, with no underline.
4. Any other key (or Escape) ends the list and keeps the letter.

## How to verify on Midget (test PC)

```
cd springboard && cargo build --offline --profile fast --bin shell --features desktop-wayland
tools/midget-push-shell.sh --no-build shot.png
bifrost pc midget state
```

1. Pull the home screen down to open Search. Type `'` then `e` (keyboard or the on-screen keyboard): the list
   `1 é 2 è 3 ê 4 ë` appears over the top of the screen and `e` is underlined in the field. Screenshot it.
2. Press `2`: the field reads `è`, the list closes.
3. Type `'`, `o`, then Escape: the list closes and the field keeps `o`.
4. Type `'`, `z`: the field reads `'z`.
5. Light and Dark: the panel uses `MENU_SOLID` and `LABEL`, so it follows the appearance switch
   (`bifrost pc midget appearance light|dark`).

Host tests (not run in this round):
- `bifrost_ui::compose_method::tests`: alternates and their upper-case forms (`a` has 7, `A` starts with `Á`, `d` is `ð`),
  `pick` ("1" is index 0, "4" is index 3 of `e`, "5" and "0" pick nothing), the apostrophe arming and the open list for `o`
  (6 candidates `ó ò ô ö õ ø`), plain keys and an armed apostrophe kept for other keys (`''`, `' `), and `take_prefix`.
- `shell::compose_window::tests`: an apostrophe and `e` open `é è ê ë` with `e` marked at `(0, 1)`; `2` leaves `è` with no
  marked range. Escape ends the list and keeps `o`. An apostrophe followed by Backspace opens nothing and clears the armed
  state.

## REA and official evidence

Official page checked this round: `python3 -I tools/parity/docc.py inputmethodkit/imkcandidates` (IMKCandidates, macOS
10.5): `show(_:)`, `hide()`, `update()`, `setPanelType(_:)`, `setDismissesAutomatically(_:)`, `setSelectionKeys(_:)`.

No new REA capture was made in this round. The rendered pages recorded by earlier waves (`docs/parity/wire5-input-method.md`) are
reused:

| Page | evidence_id |
|---|---|
| InputMethodKit | ev_4f76b381a49e71bf77bec3bd4cf89ff199682ce9f119dac1893dd155febd1ba1 |
| IMKServerInput | ev_7aa64715087049591aa99482930a721ab4c0b6a5a2bd62e0d169dc1fc722ec5b |
| IMKInputController | ev_174418e7339be3454e258a6cc496de04ac01010826df86b75531ae6406a54b03 |
| IMKStateSetting | ev_b589d4abcb28d6a9be365d9f45c83794e3709fcfc5aef9a2b45d824095483ed4 |
| InputMethodKit (matrix) | ev_d889a7762311f1422e0f0dd0cf934d9bdcf457bd459b3adb2cd0f2726112a81b |

No REA capture exists for the candidate window page itself (IMKCandidates); owed before the row can move to `parity`.

## Matrix rows

| Slug | Feature (as in the matrix json) | BifrOSt path |
|---|---|---|
| inputmethodkit | Candidates window | springboard/src/shell/compose_window.rs (`draw_compose`, `compose_open`); springboard/ui/src/imsession.rs |
| inputmethodkit | Composed, original string and candidate queries | springboard/src/shell/compose_window.rs (`compose_open`, `compose_finish`); springboard/ui/src/imsession.rs (`InputSession::update`, `set_candidates`, `select_candidate`, `commit`) |
| inputmethodkit | Key binding and text events to the input method | springboard/src/shell/compose_window.rs (`compose_search_key`); springboard/src/shell/hwkeys.rs (`hw_type`); springboard/ui/src/compose_method.rs (`Compose::key`) |

Rows stay `partial`. The window is reached only from the Search field, and it has not been run on Midget.

## Remaining gaps

- Not anchored to the caret. The list sits at a fixed place at the top centre (`TOP_SHARE`). The Search field's caret
  rectangle is not exported to the shell, and `textinput::popup_origin` is not used yet.
- Only the Search field. Other apps' text fields do not get the compose method. The Settings, Browser and Files
  routes in `hw_type_app` do not call `compose_search_key`.
- No selection highlight and no arrow-key or page navigation in the list. Only digits 1 to 9 pick.
- No press-and-hold letter popup. The apostrophe prefix is the only trigger.
- No provider process: no `IMKServer`, no per-session `IMKInputController` for a client, no `wl_seat` text-input or
  input_method server. `InputSession` is driven by the shell itself.
- No `IMKCandidates` show, hide and dismiss as an external provider would call them. The window opens and closes with
  the composition only.
- No annotation window, no `IMKCandidatePanelType` grid or row styles beyond the single column, and no opacity from the
  model is applied to the drawing.
- Not tested on Midget or on a VM yet.
