# InputMethodKit parity, wave 3: candidates window model

Owner: parity wave 3 (branch `parity/w3-inputmethodkit`, from `parity/integration`).
Matrix source: `docs/parity-matrix/data/inputmethodkit.json` (the lead updates the matrix; this file lists the rows closed).

## What was built

`springboard/ui/src/candidates.rs` (`pub mod candidates;` in `springboard/ui/src/lib.rs`), a pure model of the
candidates window that an input method shows next to the caret. It draws nothing; the shell paints it.

- `PanelType` (`SingleColumnScrolling`, `ScrollingGrid`, `SingleRowStepping`): the three panel styles.
- `LocationHint` (`Above`, `Below`, `Left`, `Right`): the side of the caret the window prefers.
- `place(hint, caret, w, h, screen_w, screen_h, gap)`: top-left of the window. A side that would leave the screen
  flips to the opposite side. The result is clamped to the screen on both axes.
- `clamp_opacity(v)`: opacity clamped to 0.0 to 1.0; NaN means opaque.
- `Candidates`: the item list, selection (`select`, `selected`, `selected_string`, `clear_selection`), stepping that
  stops at both ends (`move_by`), row moves in a grid (`move_row`, one row is `columns` items), paging
  (`page_range`, `page_of`), scrolling that keeps the selection visible (`ensure_visible`), show, hide and the
  location hint, `position`, the auto-dismiss flag and opacity.

Behaviour rules the tests check: a new list clears the selection; a selection outside the list is refused; stepping
never wraps; with nothing selected a forward step selects the first item and a backward step the last; a grid with
zero columns counts as one column.

Not built here: the shell does not yet draw this window, no input method sends it candidates, and nothing reads
the selection keys.

## Official API names mirrored

Official pages (developer.apple.com, DocC via `tools/parity/docc.py`):

- `IMKCandidates`: `init(server:panelType:)`, `setPanelType(_:)` / `panelType()`, `show(_:)`, `hide()`,
  `isVisible()`, `setDismissesAutomatically(_:)` / `dismissesAutomatically()`, `setAttributes(_:)`, `update()`,
  `selectCandidate(_:)`, `selectCandidate(withIdentifier:)`, `selectedCandidate()`, `selectedCandidateString()`,
  `clearSelection()`, `setCandidateData(_:)`.
- `IMKCandidatePanelType` constants: `kIMKSingleColumnScrollingCandidatePanel`,
  `kIMKScrollingGridCandidatePanel`, `kIMKSingleRowSteppingCandidatePanel`.
- `IMKCandidatesLocationHint` constants: `kIMKLocateCandidatesAboveHint`, `kIMKLocateCandidatesBelowHint`,
  `kIMKLocateCandidatesLeftHint`, `kIMKLocateCandidatesRightHint`.
- Opacity: `IMKCandidatesOpacityAttributeName` (in the attributes dictionary; mirrored as `clamp_opacity` and `set_opacity`).

Pages with REA evidence:

- `https://developer.apple.com/documentation/inputmethodkit/imkcandidates`
  REA evidence id: `ev_1c3cb519f9ac32f72b363cae98a64324299b2ec0560bc9840eeffb73addb5f41`
- `https://developer.apple.com/documentation/inputmethodkit` (framework landing page, from the matrix row):
  REA evidence id: `ev_d889a7762311f1422e0f0dd0cf934d9bdcf457bd459b3adb2cd0f2726112a81b`

Not mirrored: `IMKCandidatesSendServerKeyEventFirst`, `IMKStyleType`, `showAnnotation(_:)`, sublists
(`showSublist`, `attachChild`), selection keys and their key layout. They stay gaps.

## Matrix rows closed

New status is `partial`: the model exists and is tested, the shell does not draw it yet.

| Framework slug | Feature (as written in the matrix) | Status | BifrOSt path |
|---|---|---|---|
| inputmethodkit | Candidates window | partial | springboard/ui/src/candidates.rs (`Candidates`, `place`) |
| inputmethodkit | Candidate panel types, location hints and opacity | partial | springboard/ui/src/candidates.rs (`PanelType`, `LocationHint`, `clamp_opacity`) |
| inputmethodkit | Enumerations, constants and data types | partial | springboard/ui/src/candidates.rs (`PanelType`, `LocationHint`) |

The "Enumerations" row stays partial: `IMKStyleType` is still missing.

## Checks

- `python3 tools/check-names.py`: exit 0 (no platform names in code or comments).
- No build or test run in this wave, as the task required. The tests in `candidates.rs` were worked out by hand from
  the rules above (screen 800 x 600, window 150 x 80, gap 4 in the placement tests).
