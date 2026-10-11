# Theme 23: App-side search queries

Status: partial. Built in pure Rust, std only, with no build run in this round.

## What was built

- `springboard/ui/src/searchquery.rs` (new, `bifrost_ui::searchquery`): a pure query model. A query string is run
  over the app's own records (title, description, keywords). Words must all match (AND), a word matches by prefix, and
  quoted text is a phrase whose last word may be a prefix. Filter queries are required of every hit and do not add to
  the score. Hits are ranked by field (title above keywords above description), and ties keep record order. Errors
  carry the domain `bifrost.quicksearch.query` and stable codes (1 empty query string, 2 no fields).
- `springboard/sdk/src/quicksearch.rs` (extended): `search_items(&SearchQuery, &[CSSearchableItem])` runs a query over
  the items the app indexed. It uses the same `wire_text` folding as indexing, so the title for the current locale
  and the keywords that include custom values and persons match exactly what the shell indexes. The query types are
  re-exported as `SearchQuery`, `SearchQueryContext`, `SearchQueryField`, `SearchQueryHit` and `SearchQueryError`.
- `springboard/ui/src/lib.rs`: one `pub mod searchquery;` line.

## Apple APIs mirrored

- `CSSearchQuery`: `init(queryString:queryContext:)` (the current form; the `attributes:` initializer is deprecated),
  `results`, `start()`, `foundItemsHandler`, `completionHandler`, `CSQueryContinuationActionType`. See
  https://developer.apple.com/documentation/corespotlight/cssearchquery
- `CSSearchQueryContext`: `fetchAttributes`, `keyboardLanguage`, `filterQueries` (`sourceOptions` is out of scope, mail
  only). See https://developer.apple.com/documentation/corespotlight/cssearchquerycontext
- Error domain: `CSSearchQueryErrorDomain` (mirrored by the `SearchQueryError` codes).

REA evidence (rendered official pages, `tools/parity/rea-page.sh`):
- `ev_7e6607a1093d0161903cd96804e4cd0a7bf791219f4185dfc60329f6862318a1` (CSSearchQuery)
- `ev_8c7c7e4c845237388a489b66ef1f1dfc4ac54ec6d45e84e383a045509c3a8d0b` (CSSearchQueryContext)

## Gaps (kept honest)

- The query runs only over items the app passes in. It does not query the shell's on-device index, so the status stays
  `partial`. A shell round-trip would need a new protocol message.
- No streamed batches (`foundItemsHandler`), no `cancel()`, no `protectionClasses`, no mail `sourceOptions`.
- `fetchAttributes` and `keyboardLanguage` are recorded but do not change the result.
- Matching has no diacritic folding, so "mjolk" does not find "Mjólk". Predicate strings in Apple's grammar
  (`attribute == value`) are not parsed; the query is free text with quoted phrases.

## Matrix rows closed

| Framework slug | Feature (as in data/<slug>.json) | New status | BifrOSt path |
|---|---|---|---|
| core-spotlight | App-side search queries | partial | springboard/ui/src/searchquery.rs; springboard/sdk/src/quicksearch.rs (search_items) |
| core-spotlight | Errors and error domains | partial | springboard/ui/src/searchquery.rs (Error, DOMAIN, code); springboard/sdk/src/quicksearch.rs (IndexError) |
| core-spotlight | Custom attributes, localized strings and persons | partial | springboard/sdk/src/quicksearch.rs (CustomAttribute, Person, LocalizedString), already present; the matrix entry was stale |
| foundation-files-persistence | Spotlight: metadata queries | partial | springboard/ui/src/metaquery.rs (pure predicate and sort model over file records, no SDK wiring) |

Not closed: `Index client delegate`, `Index descriptions`, `Index extensions and on-demand regeneration`, `Foundation
models search tool` (core-spotlight), the open-directory record query, and the core-services metadata query row, which
still has no SDK surface.

## Tests

- `springboard/ui/src/searchquery.rs` (`mod tests`): 7 tests covering prefix AND matching, field ranking and ties,
  quoted phrases, filter queries, field restriction with the no-fields error, and the empty-query error plus Unicode.
- `springboard/sdk/src/quicksearch.rs` (`mod tests`): 2 added tests over `search_items` (fields, custom keywords,
  filter queries, error mapping).
- Build and test were not run (no build in this round), so the tests are unexecuted.
