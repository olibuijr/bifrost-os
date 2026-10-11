# Theme 14: Writing Tools (script detection and writing assistance)

Two pure Rust models in `springboard/ui/src` (bifrost-ui). Std only, no unsafe, no system service. Neither module
touches the shell or the text view yet: the caller supplies the assistant's suggestions and the models check, apply
and track them.

## What was built

- `springboard/ui/src/script.rs` (`pub mod script`). The standard script set of the Natural Language script type
  (29 scripts, tags such as `latin`, `cyrillic`, `canadianAboriginalSyllabics`). Functions: `script_of(char)`,
  `runs(text)` (byte ranges per script; common characters stay in the run they sit in), `dominant(text)` (ties go to
  the first-seen script), `Script::tag()` and `Script::from_tag()`. Han is one script (`Han`): the simplified and
  traditional Chinese split needs a lexicon and is not guessed.
- `springboard/ui/src/writing_assist.rs` (`pub mod writing_assist`). The writing-assistance model:
  - `Behavior` (none, default, complete, limited) with `resolve(available)`. `Default` resolves to `Complete` when the
    assistant is available.
  - `ResultOptions` (plain text, rich text, list, table) with `negotiate(preferred, supported)`. Plain text is the floor.
  - `ContextScope` (user selection, visible area, full document) and `context_range()`, which clamps and snaps to
    char boundaries.
  - `Suggestion` (byte-range replacement) and `apply()`, which checks every range (bounds, char boundaries, overlaps)
    before applying any of them.
  - `map_range()`, which keeps the context and pending suggestions on the same words after an edit.
  - `Session`: `begin`, `offer`, `accept`, `reject`, `stop`. One task at a time. Accepting drops overlapping stale
    suggestions and shifts the rest.

Both files have a `#[cfg(test)] mod tests` (6 behaviour tests each). The tests were not run: this round does not build.

## Official API names mirrored

Apple documentation (DocC JSON via `tools/parity/docc.py`):

- Natural Language, `NLScript`: https://developer.apple.com/documentation/naturallanguage/nlscript (standard scripts
  listed in the page's "Getting standard scripts" section).
- AppKit, `NSWritingToolsCoordinator`: https://developer.apple.com/documentation/appkit/nswritingtoolscoordinator
  (`preferredBehavior`, `behavior`, `preferredResultOptions`, `resultOptions`, `updateRange`, `stopWritingTools`,
  `TextUpdateReason`, `ContextScope`, `TextReplacementReason`).
- AppKit, `NSWritingToolsBehavior`: https://developer.apple.com/documentation/appkit/nswritingtoolsbehavior
  (`none`, `default`, `complete`, `limited`).
- AppKit, `NSWritingToolsResultOptions`: https://developer.apple.com/documentation/appkit/nswritingtoolsresultoptions
  (`plainText`, `richText`, `list`, `table`).

The `State` enum of the coordinator was not read (the DocC path failed), so the session phases are BifrOSt's own
(`Task`: proofread, rewrite) and do not claim Apple's state names.

## REA evidence

- `ev_55992895081a472235edeee9283a5fca0823f22ebedc6a02ea7e028dfae847eb`: NLScript rendered page
  (title "NLScript | Apple Developer Documentation").
- `ev_8f0e7a50c633c5d1903f2a15dd14722bd0c09e8f5bb5678b439eaf2b2a00aae7`: NSWritingToolsCoordinator rendered page.

## Matrix rows closed

| Framework slug | Feature (as in data/<slug>.json) | New status | BifrOSt path |
|---|---|---|---|
| appkit-text | Writing Tools | partial | springboard/ui/src/writing_assist.rs |
| natural-language | Script detection (NLScript) | partial | springboard/ui/src/script.rs |

Still open: the text view does not yet call the session (no coordinator object, no system assistant, no animations or
effect views). The script model has no language identification and no per-app SDK API.

## Verification

- `python3 tools/check-names.py`: exit 0.
- No builds, per the task rules. Test expectations were checked by hand against the code.
