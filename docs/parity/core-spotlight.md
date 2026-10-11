# Core Spotlight parity: what was built

Framework: Core Spotlight (https://developer.apple.com/documentation/corespotlight). Matrix row:
`docs/parity-matrix/data/core-spotlight.json`.

## REA evidence

- Landing page `https://developer.apple.com/documentation/corespotlight`:
  `ev_a841fd254a7cbcb143a65e1a18bbecff074c66482c25357cb9113a44f1478201` (rendered headings include the Searchable
  items, Indexes, Queries, Errors and Foundation models sections, and the note that file import extensions do not work
  on macOS).
- Official DocC facts (`corespotlight/csperson`, `corespotlight/csindexerror`, `corespotlight/cslocalizedstring`)
  were read with the DocC reader; they gave the initializer shapes and the error code list.

## What was built

All changes are in `springboard/sdk/src/quicksearch.rs` (`bifrost_app::quicksearch`). No wire change, no shell change.

1. Index errors (`IndexError`, mirrors `CSIndexError` codes). `CSSearchableIndex::index_checked` returns each refused
   item's position with its reason and sends the rest. Refused cases:
   - `EmptyIdentifier` (code 1): the item could never be replaced or deleted.
   - `IdentifierTooLong` (code 2): longer than `MAX_ID` bytes. The codec would cut it, and the cut identifier would no
     longer match the one the app deletes or continues by.
   - `DomainTooLong` (code 3): longer than `MAX_DOMAIN` bytes, so deleting by domain would miss the item.
   `IndexError::DOMAIN` is `bifrost.quicksearch.index`. `index_searchable_items` keeps its signature and now calls
   `index_checked`, so its old silent skip of empty identifiers is unchanged.
2. Custom attributes, persons and localized titles (`CustomAttribute`, `Person`, `LocalizedString`, mirroring
   `CSCustomAttributeKey`, `CSPerson` and `CSLocalizedString`). Builders `custom_attribute`, `person` and
   `localized_title` on `CSSearchableItemAttributeSet`. At index time `wire_text` picks the title for the current locale
   (exact identifier, then same language, then the first entry; an empty text falls back to `title`), and appends custom
   values, person display names and handles to the keywords, skipping empty and repeated values.

## Tests

`cargo` was not run (build and test stage is separate). Tests added in the existing `#[cfg(test)] mod tests`:
- `refused_items_are_reported_by_position_and_the_rest_are_sent`
- `identifier_and_domain_lengths_are_checked_against_the_wire_cap`
- `error_codes_and_domain_are_stable`
- `localized_title_follows_the_locale_and_falls_back`
- `custom_values_persons_and_handles_are_folded_into_keywords`

The existing tests (`index_sends_the_flattened_attributes_in_chunks`, `items_without_an_identifier_are_skipped`,
deletes, continue-activity) are unchanged in behaviour.

## Remaining gaps

- No app-facing query API (`CSSearchQuery`, `CSUserQuery`) and no suggestions.
- No index delegate and no index extensions for on-demand regeneration.
- The shell reports no indexing completion or failure, and there are no search query error codes.
- Custom keys, person records and localized strings are not typed on the wire; the Search panel cannot show them as
  attributes.
- No Foundation models search tool.
- No index descriptions or protection classes.
- Apple's contact identifier is kept on `Person` but not used.
- Keywords beyond the wire cap of 8 are dropped by the codec, so a long custom or person list loses its tail.
