# Text input everywhere (round 75, slug `text-input-everywhere`)

Wave 3 put the accent hold (an apostrophe, then a letter, opens the candidate list; digits pick) only in the shell's
Search field. This slice moves it into the shared text-input path that every SDK text field and text view types through.

## What is done

- **Shared composition model.** `bifrost_ui::compose_method::Composer` holds the armed apostrophe and the open
  candidate list (an `InputSession`). `type_text` and `end_for_control` run the flow over a `ComposeTarget` (marked
  text, commit, plain insert). The flow was previously written inside `springboard/src/shell/compose_window.rs`; it now
  lives once, in the ui crate.
- **SDK text fields.** `bifrost_ui::text::TextEditor` owns a `Composer`. `type_text` is the typed-key entry point,
  `candidates` the open list, `end_composition` ends it before any command (`perform`, pointer press, chords, other
  keys). Paste, IME commits (`set_marked_text`, `unmark_text`, `delete_surrounding`) and undo or redo also end or drop it.
  `sdk/src/view/forms.rs` `TextField` routes typed text through `type_text` and draws the list with the new
  `sdk/src/widgets.rs` `candidate_list` (a popover of numbered rows under the field, the same look as the shell's list).
- **Text views.** `bifrost_ui::textview::TextView::insert` types through `type_text`, and `TextView::candidates` exposes
  the list for the app to draw with `widgets::candidate_list`. Apps that use `bifrost_app::controls::textview::TextView`
  (Voice Memos) get the accent hold in their typed text.
- **Shell Search field.** `compose_window.rs` keeps only its painting and a `SearchField` adapter onto the shell's
  `TextField`, so the Search field runs the same flow as the SDK fields.

## Migrations and deletions

- Deleted from `compose_window.rs`: the private compose flow (`compose_finish`, `compose_open`), the `Compose`/`Step`
  calls, the session field, and the session-level test (`an_open_session_lists_its_candidates_in_order`). The
  composer's own tests now live in `compose_method.rs` (`a_digit_picks...`, `a_held_apostrophe...` and four more).
- `TextView::insert` now calls `type_text` instead of `insert_text` (typed text, not paste).
- `TextField` (SDK) `KeyInput::Text` calls `type_text` instead of `insert_text`.

## Evidence used

- The existing shell tests (`springboard/src/shell/compose_window.rs`, before the change) define the behaviour kept:
  digit picks, control key keeps the letter, held apostrophe given back.
- Reference names only: none used. Code and comments contain no platform or product names.

## Not done in this slice (for the lead)

- **Kana composition.** No Search-field kana path exists in this base. The only kana code is the `ja-romaji` mode
  registration in `ui/src/input_method.rs` (a provider controller), not wired to any field. Nothing was moved.
- **RichTextView** (`sdk/src/attributed.rs`) has no marked-text model, so the accent hold is not wired there yet.
  It needs a marked range in `RichText` before `ComposeTarget` can be implemented for it.
- **TextEdit** (`apps/textedit-desktop`) edits its own cell vector, not `TextEditor`. Not migrated.
- **Mail compose.** The compose sheet in `springboard/src/shell/compose_sheet.rs` is the social compose model, not a
  text view; no mail-compose text path was changed.
- **Notes** uses the SDK `TextField` and text views, so it gets the hold through `TextEditor`. Nothing app-specific.
- **Text replacements and the spelling underline / correction menu** in SDK fields are not wired. The shell already
  has `sdk/src/text_services.rs` (check, correction, replacements over the `text` service); the field would need a
  spelling pass on draw and a menu on the marked word. Not started.
- **Shell list drawing** still has its own copy of the candidate geometry (`ROW_PT`, `PAD_PT`, `WIDTH_PT`) because the
  shell draws through `Surface` and cannot import the SDK. The model is shared; the painting is duplicated by necessity.

## Needs Midget checking

- The accent list appears under an SDK text field (`TextField` draws it below its frame; a field at the bottom of a
  window may clip it). The shell's Search list is unchanged in position and look.
- Voice Memos: an apostrophe typed in the transcript opens the list and digits pick, with no lost characters.
