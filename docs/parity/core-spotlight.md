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

## Team N wave: assistant search tool

Branch `parity/team-n-core-spotlight`, built on `arm`. Not yet verified on Midget.

6. Foundation models search tool, first slice (`springboard/ui/src/spotlight_tool.rs`). `tool(items)` returns a
   model `Tool` named `search_indexed_content` (`TOOL_NAME`). It takes `query` (text, required) and `limit`
   (1 to `MAX_RESULTS`, default `DEFAULT_RESULTS`). Each call runs `usersearch::run_user_query` over the supplied
   `IndexedItem`s (app, identifier, `Record`) and returns numbered lines with the app, title, a description snippet
   (keywords when there is no description, cut to `SNIPPET_CHARS` characters on a character boundary) and the
   identifier, so the model can name the source. An empty query is reported to the model as a tool error through
   `ToolSet::output_for`. The caller chooses which items the assistant may see.

Tests (host only, not run here): `returns_matches_with_app_and_identifier`, `no_match_is_a_plain_answer_not_an_error`,
`limit_caps_the_number_of_items`, `schema_rejects_a_missing_query_and_an_out_of_range_limit`,
`an_empty_query_is_reported_to_the_model`, `keywords_stand_in_for_a_missing_description`,
`long_descriptions_are_cut_on_a_character_boundary`, `the_tool_is_described_with_its_schema`.

Remaining gaps for this row: `springboard/apps/assistant-desktop` does not register the tool yet, and nothing feeds
it the shell index. The Foundation pipeline types (`SearchSource`, `CoreSpotlightSource`, `FileSource`,
`CustomStage`, `SearchPipelineData`, `ContactResolver`, `SearchCount`, `SearchResultsTable`, `SearchStatistic`,
`SearchTextResult`) are not modelled.
