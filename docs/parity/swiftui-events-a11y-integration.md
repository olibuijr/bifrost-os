# SwiftUI event handling, accessibility and framework integration: parity slice

Two gaps from the matrix row set for this framework were built as pure models in `springboard/ui/src` (bifrost-ui). Both are
model-only: nothing calls them yet from the SDK or the AT-SPI bridge.

## What was built

### 1. Accessibility rotors (`springboard/ui/src/ax.rs`)

- `Rotor { name, roles }` and `STANDARD_ROTORS`: six rotors (Controls, Fields, Lists, Menu items, Text, Progress), each a
  list of the role words `role_name` writes.
- `Map::rotor_step(&rotor, from, forward)`: the next or previous element of the rotor's roles in map order, from the
  element `from` (which need not be in the rotor), wrapping at both ends. With no start it returns the first or last match.
  It returns `None` when no element has a role of the rotor.
- Mirrors: `accessibilityRotor(_:entries:)` (the user-visible labelled rotor that steps through matching elements).
- Official doc: https://developer.apple.com/documentation/swiftui/view/accessibilityrotor(_:entries:)
- REA evidence: `ev_28724b970d902561b302b9058f990758d7f8929991b324b4c02b196491aae571`
- Not built: exposing rotors through the AT-SPI bridge (`springboard/src/shell/atspi/`), and text-range rotors.

### 2. Focused values (`springboard/ui/src/focus.rs`)

- `FocusedValues`: a view publishes a value under a key (`publish`, `withdraw`), views are linked to a parent
  (`set_parent`, `remove_view`), and `value(focused, key)` returns the value of the focused view or of its nearest
  ancestor that publishes the key. The walk stops after one step per parent link, so a loop in the links ends it.
- Mirrors: the focused value property wrapper, which reads a value published by the focused view or one of its ancestors,
  the nearest one winning.
- Official doc: https://developer.apple.com/documentation/swiftui/focusedvalue
- REA evidence: `ev_903897d1fce65bf23ba87f2b3196f8aad2686b29be85e1649ad745eb277464c5`
- Not built: propagation of focused values to the window menu bar or to the SDK. The shell does not yet ask the focus
  chain for values.

## Tests

- `cargo` was not run (the task forbids builds). The tests are written and reviewed by reading only:
  - `springboard/ui/src/ax.rs`: `a_rotor_steps_over_its_roles_and_wraps_both_ways`,
    `a_step_from_outside_the_rotor_starts_at_its_place_in_the_map`,
    `a_rotor_with_no_matching_element_finds_nothing`,
    `the_standard_rotors_cover_every_role_except_the_window`,
    `a_disabled_control_is_still_reached_so_the_user_hears_it_is_unavailable`.
  - `springboard/ui/src/focus.rs`: `the_focused_view_answers_before_its_ancestors`,
    `a_focused_view_without_a_value_falls_back_to_the_nearest_ancestor`,
    `nothing_focused_or_an_unknown_key_gives_nothing_and_keys_do_not_leak`,
    `publishing_again_replaces_and_withdrawing_falls_back`, `a_loop_in_the_parent_links_ends_the_walk`,
    `a_new_parent_replaces_the_old_one_and_a_removed_view_takes_its_values`.
- `python3 tools/check-names.py` exits 0.

## Matrix rows closed

| Framework | Feature (as in `docs/parity-matrix/data/swiftui-events-a11y-integration.json`) | New status | BifrOSt path |
|---|---|---|---|
| swiftui-events-a11y-integration | Accessible navigation (rotors) | partial | springboard/ui/src/ax.rs (`Rotor`, `STANDARD_ROTORS`, `Map::rotor_step`) |
| swiftui-events-a11y-integration | Focus | partial | springboard/ui/src/focus.rs (`FocusedValues`) |

Both rows stay `partial`: the models exist, but neither is wired into the shell or the SDK yet.

## Notes for the matrix owner

- The matrix row "Accessible descriptions" and the gap "Accessibility rotors" in the JSON are out of date for the
  title/help split and rotors respectively. `ax.rs` already has `AxMeta` (title and help), announcement priority, and
  custom actions (`add_custom_action`). The matrix was not edited, as the task requires.
