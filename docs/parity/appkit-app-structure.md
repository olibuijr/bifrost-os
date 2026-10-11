# AppKit (App Structure) parity: round 1

Framework page: https://developer.apple.com/documentation/appkit (App Structure topics: App and Environment,
Documents Data and Pasteboard, Cocoa Bindings, Resource Management, App Extensions).

## What was built

1. **Bindings controllers** (`springboard/ui/src/controller.rs`, `bifrost_ui::controller`). Pure std model.
   - `ObjectController<T>` mirrors the single-object controller: `content`, `set_content`, `can_add`/`add`,
     `can_remove`/`remove`, and an `editable` flag.
   - `ArrayController<T>` mirrors the array controller: `arranged_objects` from sort descriptors (`add_sort`,
     `clear_sort`, chained in order) and a filter (`set_filter`, `clear_filter`), `rearrange`, a selection held as
     content indexes (`set_selection_indexes`, `add_selection_indexes`, `remove_selection_indexes`,
     `selection_index`, `selected_objects`), `select_next`/`select_previous` with their `can_` checks, `add`,
     `insert_at_arranged`, `remove_at_arranged`, `remove_selected`, and `set_content`.
   - Flags with the same defaults as the reference: `avoids_empty_selection`, `preserves_selection`,
     `selects_inserted`, `clears_filter_on_insert`. Inserts and removals shift the selection so it keeps pointing at
     the same objects.
2. **App run states** (`springboard/core/src/occlusion.rs`, `lib.rs` `pub mod occlusion`). Pure model with no clock
   of its own: `Occlusion::report(Event, now_ms)` and `tick(now_ms)` return `Transition::{None, Suspend, Resume}`.
   Focus, on-screen visibility and occlusion (covered by other windows) are separate facts. An app that has focus or
   that the user can see keeps running, including visible unfocused windows. An unseen app is suspended after
   `GRACE_MS` (2000 ms), once per change. Focus or visibility resumes it at once.

## Official API names mirrored

- `NSObjectController`: `content`, `add`, `remove`, `canAdd`, `canRemove`, `isEditable`.
- `NSArrayController`: `arrangedObjects`, `sortDescriptors`, `filterPredicate`, `rearrangeObjects`,
  `selectionIndexes`, `selectedObjects`, `setSelectionIndexes`, `selectNext`, `selectPrevious`, `canSelectNext`,
  `canSelectPrevious`, `insert(_:atArrangedObjectIndex:)`, `remove(atArrangedObjectIndex:)`, `remove(_:)`,
  `avoidsEmptySelection`, `preservesSelection`, `selectsInsertedObjects`, `clearsFilterPredicateOnInsertion`.
- Cocoa Bindings page: the controller classes it lists under Core Controllers and Array-Based Data.

The Rust names are our own (for example `ArrayController`, `select_next`). Code comments do not use framework names.

## REA evidence

- `ev_d0f40ad6d3e0981142fe4208241602b78d2733c4e8e1ba807934f96e7d4f6f01`: rendered AppKit landing page, with the
  App Structure section (App and Environment, Documents Data and Pasteboard, Cocoa Bindings, Resource Management,
  App Extensions).
- `ev_13fb77cf2a8baeb76d5ba523cb7154f39cc259d6e99eaf466bc087940ef72843`: the evidence recorded in the matrix before
  this round.
- Official DocC was read for `appkit`, `appkit/cocoa-bindings`, `appkit/nsarraycontroller` and
  `appkit/nsobjectcontroller`. Those pages are the source of the names above.

## Tests

`ui::controller` has 7 behaviour tests (object controller editing and read-only state; filter and sort order;
selection shifting on insert; selects-inserted; removal and the empty-selection rule; select next and previous;
filter clearing on insert). `core::occlusion` has 6 (visible app keeps running; suspension only after the grace
period and only once; a briefly covered app never suspends; focus resumes at once and restarts the grace period;
minimized app suspends and resumes; focus alone keeps a covered app running). Nothing was built or run in this
round, as the task requires.

## Remaining gaps

- No key-path binding from controllers to views, and no key-value observation. `rearrange` must be called after an
  object changes in place.
- No tree controller or dictionary controller.
- The shell does not send window focus, visibility or occlusion events to apps, so `Occlusion` is not yet driven.
  Memory-warning callbacks to apps are still missing (task/008-lifecycle-recovery.md, step 2).
- Nib and storyboard loading, a general app extension host, iCloud-style sync, and multi-type pasteboard
  publishing to the Wayland selection are unchanged.
