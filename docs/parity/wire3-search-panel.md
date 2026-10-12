# Wire 3: the Search panel and the `search-index` service

Status: wired in code on branch `wire3/search-panel`, not built or run on a device (no builds were allowed this round;
the lead builds once). Owned files: `springboard/src/shell/spotindex.rs`, `springboard/src/shell/quicksearch.rs` (the
Documents group and its rows), `springboard/src/shell/services/search_ranking.rs`, `springboard/sdk/src/search_ranking.rs`.
The wire protocol (`springboard/proto`) is unchanged.

## What is now wired

| Piece | Where | What a user sees |
|---|---|---|
| Content type of an indexed item | `spotindex.rs` `kind_of`: the `uttype` type of the content URL's extension (`bifrost_ui::uttype::type_for_extension`) | the row reads "Q1 report · PDF document" (`uttype::description`) |
| Thumbnails | `spotindex.rs` `load_thumbnail` / `decode_thumbnail` (PNG, aspect kept, centred in the row's icon square); `quicksearch.rs` `search()` caches them and `draw()` shows one in place of the app icon | a PNG thumbnail in place of the app icon |
| File type filters | `spotindex.rs` `parse_query`, `kind_word`: "pdf", "png", "jpg" and the other extension words in `EXTENSION_WORDS`; "photos" or "pictures" (`public.image`), "video" or "movies" (`public.movie`), "audio" or "music" (`public.audio`), "documents" (`public.content`), "text" (`public.text`). Matching uses `uttype::conforms`. | "pdf" lists the PDFs |
| Time filters | `spotindex.rs` `time_phrase`, `calendar`, `rolling`: "today", "yesterday", "this|last week|month|year" (calendar, weeks start Monday), "past week|month|year", "last|past N days|weeks". A lead word ("from", "in", "on", "during", "since") before a phrase is dropped with it. The window is checked with `bifrost_ui::metaquery::Predicate::ModifiedAtOrAfter` and `ModifiedBefore` on the item's indexing time. | "pdf from last week" lists last week's PDFs |
| Filter-only queries | `rank_hits`: a query with filters and no words lists the matches, newest first | "photos" alone lists every indexed picture |
| Service `search-index` `search` | reply items gain `"kind"` (the `uttype` identifier or null); filters work there too | apps get the same filtering as the panel |
| SDK `search_ranking::Hit` | new `kind: Option<String>` field, filled by `parse_hits` | apps read the type of each hit |

Dates are read in UTC. The time of an item is the time the app indexed it (or last replaced it), which is persisted as
`"indexed"` in `<data>/quicksearch-index/<app>.json`; items saved before this round read as indexed at time 0 and so
only match filters that reach back to 1970.

## How a user reaches it

Pull the home screen down to open Search and type the query. The Documents group lists indexed items (up to three
rows). Words must all match; file type and time words narrow the list, and an empty word list with only filters
lists the matches. Opening a result still starts the app and sends it the continue-activity message.

## How to verify on the test PC (Midget)

1. Host tests (run by the lead): `cargo test --offline --profile fast -p springboard spotindex` (filters, calendar
   maths, thumbnails, ranking), `cargo test --offline --profile fast -p springboard quicksearch services::search_ranking`,
   and `cargo test -p bifrost-app search_ranking` in `springboard/sdk`.
2. Build and push the shell: `tools/midget-push-shell.sh` (see `docs/bifrost-pc-workflow.md`).
3. Index three items from an app (or from a test that calls `CSSearchableIndex::index_searchable_items`): a PDF whose
   content URL ends in `.pdf`, a PNG with a thumbnail path inside the app's data dir, and a note with no extension.
   Index the PDF a week ago, if the clock allows, to test "last week"; otherwise test "pdf" and "today".
4. Open Search with `bifrost pc midget tap-window` (window-local pixels, see `docs/bifrost-pc-workflow.md`) and type:
   "pdf" (only the PDF), "photos" (only the PNG, with its thumbnail), "pdf from last week" (the PDF only when it was indexed
   last week), "documents" (the PDF and the note's text file, not the PNG). Confirm the row label reads
   "Title · PDF document" and the PNG row shows its thumbnail.
5. Screenshot with `bifrost pc midget screen PATH` and check the rows against the light and dark rules in `CLAUDE.md`.

## REA evidence

- Reused from the earlier core-spotlight rows (not captured again this round):
  - `ev_4438bf59535e4bd6ba0ca70598c6a17e34d2cbccb6b6a1ca563a4e22a50cb41b`: the Core Spotlight framework page.
  - `ev_72be97def9ad512462bd3e80e8021e0da7c2b7045b159448d224fb001dae7bc8`: the searchable item expiration date page.
  - `ev_fe9048d16059585825a4a0febd592ccef542b985cc0a07c3469e449382bb8e8c`: the language recognizer page.
- Official docs read this round with `tools/parity/docc.py -I`: `corespotlight/csuserquery` (a user query "initiates
  searches from your interface and offers suggested text completions"; `init(userQueryString:userQueryContext:)`,
  `responses`, `suggestions`). No new REA page capture was run this round, so no new evidence id exists for the filter
  behaviour. The filter grammar is this model's choice and is documented in `spotindex.rs`.

## Remaining gaps

- No suggested completions. `CSUserQuery.suggestions` (the completions while typing) is not built, and the panel has no
  completion row for indexed items.
- Dates are the indexing time, not the item's own date. The wire item has no content modification date, so
  "last week" means "indexed last week" for now.
- The file type comes only from the content URL extension. An item without an extension, or with a URL that does not
  name its file, has no type and is not matched by a file type word.
- Kind words are always filters: a query "text" or "documents" no longer matches items whose title contains that word.
- Thumbnails are decoded from PNG only; other formats show the app icon. Thumbnail files must lie inside the app's data
  dir and be at most 4 MiB.
- Time phrases use UTC, not the user's time zone (no time zone model is wired into the shell's search).
- `ui/src/searchquery.rs` (the app-side query model with `fetch_attributes` and `filterQueries`) is still not wired;
  the service `search` covers the same ranking and filters for apps.
- Not built or run on a device in this round.

## Matrix rows (feature as written in `docs/parity-matrix/data/core-spotlight.json`)

| Slug | Feature | Status after this wire | BifrOSt path |
|---|---|---|---|
| core-spotlight | User query interface and suggestions | partial (filters and the content type label now apply; suggestions still missing) | springboard/src/shell/quicksearch.rs (Documents group), springboard/src/shell/spotindex.rs (`parse_query`) |
| core-spotlight | Searchable items and attribute sets | partial (content type from the URL and the thumbnail are now shown) | springboard/src/shell/spotindex.rs (`kind_of`, `load_thumbnail`), springboard/src/shell/quicksearch.rs (`indexed_label`, thumbnails in rows) |
| core-spotlight | App-side search queries | partial (the service `search` takes the filters and returns `kind`) | springboard/src/shell/services/search_ranking.rs, springboard/sdk/src/search_ranking.rs |

The matrix data and its published database are not edited in this branch (lead's job).
