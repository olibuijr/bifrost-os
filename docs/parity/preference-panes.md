# Preference Panes: parity wave 2

Four Preference Panes gaps that can be built in pure Rust are now modelled in `bifrost-ui`. The models are pure and
tested. They are not yet wired into the shell's Settings app or the desktop menu bar, so no third-party pane can be
loaded yet.

## What was built

- `springboard/ui/src/pane.rs` (new): `PaneHost` runs a set of panes, with at most one selected at a time.
  - Lifecycle hooks (`PaneHooks`): `should_unselect` (a veto), `will_select`, `did_select`, `will_unselect`,
    `did_unselect`, with the same trait-object style as `state_machine.rs`.
  - `select` runs the veto first. A refusal leaves the selection unchanged and returns `PaneError::Refused`.
  - `PaneNotice` queue for observers: `IsAvailable`, `DoUnselect`, `CancelUnselect`, `SwitchToPane`,
    `UpdateHelpMenu`. Taken with `drain_notices`.
  - Text field autosave: `edit_field` records edits in the selected pane. Leaving the pane commits them when
    `auto_save_text_fields` is on and drops them when it is off. `saved_field` reads the committed value.
  - Help: `set_help` replaces a pane's `HelpItem`s (title, optional help book anchor). `help_menu` returns the
    items of the selected pane.
  - `PaneSpec` carries `initial_key_view` and `last_key_view` for the focus entry and exit.
- `springboard/ui/src/focus.rs` (extended): `entry` (initial key view, falling back to the first view in reading
  order) and `exit` (last key view, falling back to the last). Tab order itself still follows reading order.
- `springboard/ui/src/lib.rs`: one line, `pub mod pane;`.

## Official API names mirrored

Source: developer.apple.com Preference Panes pages, read through `tools/parity/docc.py` and rendered with REA.

- Pane class and lifecycle: `NSPreferencePane`, `willSelect()`, `didSelect()`, `willUnselect()`, `didUnselect()`,
  `shouldUnselect`, `reply(toShouldUnselect:)`, `NSPreferencePaneUnselectReply`.
- Notifications: `NSPreferencePrefPaneIsAvailable`, `NSPreferencePaneDoUnselect`, `NSPreferencePaneCancelUnselect`,
  `NSPreferencePaneSwitchToPane`, `NSPreferencePaneUpdateHelpMenu`.
- Keyboard focus: `initialKeyView`, `lastKeyView`, `firstKeyView`, `autoSaveTextFields`.
- Help menu: `updateHelpMenu(with:)`, `NSPrefPaneHelpMenuTitleKey`, `NSPrefPaneHelpMenuAnchorKey`.

Not mirrored: the bundle format and plist keys, `init(bundle:)`, nib loading (`loadMainView`), and
`NSPrefPaneHelpMenuInfoPListKey`. These need a bundle host, so they stay open.

Deliberate choices where the official page is not specific:
- `should_unselect` is asked only when another pane is about to be selected. Quitting the host is not modelled.
- `help_menu` returns the selected pane's items only. Global items from a bundle plist are not modelled.
- Uncommitted edits are dropped when auto save is off, which is the reading of `autoSaveTextFields` that the
  official page gives.

## REA evidence

- `ev_49498b2b0803b166aab9d3c504f04996610a085690b5255487d9e27551727afb`: Preference Panes landing page
  (`https://developer.apple.com/documentation/preferencepanes`).
- `ev_7655eb9786083ba8a71718c8587e0a0ff230b7db8cfa9a0cda620fda82d56441`: pane class page
  (`https://developer.apple.com/documentation/preferencepanes/nspreferencepane`).

## Matrix rows closed

| Framework | Feature (as in `docs/parity-matrix/data/preference-panes.json`) | Previous status | New status | BifrOSt path |
|---|---|---|---|---|
| Preference Panes | Selecting and deselecting panes | gap | partial | springboard/ui/src/pane.rs (PaneHost, PaneHooks) |
| Preference Panes | Pane notifications | gap | partial | springboard/ui/src/pane.rs (PaneNotice) |
| Preference Panes | Keyboard focus chain in a pane | partial | partial | springboard/ui/src/focus.rs (entry, exit); springboard/ui/src/pane.rs (PaneSpec, autosave) |
| Preference Panes | Help menu entries from a pane | partial | partial | springboard/ui/src/pane.rs (HelpItem, set_help, help_menu) |

All four stay `partial` because the models are not connected to the Settings app or to the shell's Help menu in
`springboard/src/shell/desktop_bar.rs`. Promoting them to `parity` needs that wiring and a pane host.

## Tests

- `pane.rs`: 7 tests (lifecycle order, veto with cancel, notice order and drain, unknown and repeated selection,
  autosave on and off, edits need a selected pane, help menu follows the selection).
- `focus.rs`: 2 new tests (entry, exit), plus the 3 existing ones.

No build or test run was done in this round, as the task required.
