# Wire: the `search-index` system service

Status: wired in code, not built or run on a device yet (the lead builds once). Branch `wire/svc-search`.

## What is now wired

| Model in the tree | Where it is wired | Matrix row |
|---|---|---|
| `bifrost_ui::language::LanguageRecognizer` / `dominant_language` | `springboard/src/shell/spotindex.rs` (`Entry::new` sets each indexed item's language, `rank_hits` reads the query's language), `search-index` service method `language` | `natural-language`, feature "Language identification (NLLanguageRecognizer, NLLanguage)" |
| `bifrost_ui::lexical_tags::words` (word tokens) | `spotindex.rs` `rank_hits`: the query is split into words and every word must match an item | `natural-language`, feature "Tokenizing text (NLTokenizer)" |
| `shell::fuzzy::Matcher` (fold and prefix, word start, subsequence ranks) | `spotindex.rs` `score`, one matcher per word | `core-spotlight`, feature "Searchable items and attribute sets" (search side) |
| Item expiry (no model existed; Apple's expiration date of a searchable item) | `spotindex.rs` `Index::set_expiry` / `prune`, persisted as `"expires"` in `<data>/quicksearch-index/<app>.json`, swept on the shell tick and before every search | `core-spotlight`, feature "Searchable items and attribute sets" |

The wire protocol (`springboard/proto`) is unchanged. Apps reach everything through the service channel.

### Ranking (spotindex)

1. Every word of the query must match an item (title, keywords or description; case and accent folded, so "reykjavik" finds "Reykjavík").
2. The weakest field among the words decides first: title, then keywords, then description.
3. Then the match kinds (prefix, word start, subsequence), then an item whose recognised language differs from the query's ranks lower (language-aware tie-break; items with no recognisable language are not penalised).
4. Then shorter matches, then the newest item.

The Search panel's "Documents" section (`quicksearch.rs`, `spotindex::search`) uses this ranking unchanged in code, and now also hides expired items.

### Service `search-index` (springboard/src/shell/services/search_ranking.rs)

| Method | Params | Reply |
|---|---|---|
| `search` | `{query, limit}` (limit 1 to 50, default 10) | `[{id, domain, title}]`, the calling app's own items only |
| `expire` | `{id, at}`; `at` is Unix seconds, `null` clears | `true` when the app has an item with that id |
| `language` | `{text}` | BCP 47 code (for example `is`, `en`), or `null` |

Unknown methods and bad params are error replies, never panics. The service is registered by one line in `services/mod.rs`.

### SDK client (springboard/sdk/src/search_ranking.rs)

`bifrost_app::search_ranking::{search, expire, language, parse_hits, parse_language, Hit, SERVICE}`. Each call returns the request id; the reply arrives as `App::service_reply` or through `bifrost_app::service::take_reply`.

## How a user reaches it

- Items: an app indexes its content with Core Search (existing `CSSearchableIndex` SDK). Search (the desktop search panel) then finds items by several words, in any order, and prefers the query's language.
- Expiry: an app that indexed a time-limited item (a ticket, an offer, a draft) calls `search_ranking::expire(ctx, id, Some(unix_secs))`. After that time the item disappears from Search, even if the app is not running.
- Apps can search their own items through `search_ranking::search` (for example an in-app search field that uses the same ranking).

## How to verify on the test PC (Midget)

Build with the fast profile, then push to Midget (no VM):

1. `cargo test --offline --profile fast -p bifrost-ui language lexical_tags` (pure models, unchanged) and the shell tests `cargo test --offline -p springboard spotindex services::search_ranking` from `springboard/`, then the SDK tests `cargo test -p bifrost-app search_ranking`.
2. Index two notes with an app, one of them with a short expiry in the past: `expire` it with a Unix time 60 s ahead, wait, and open Search. The expired note must be gone; the other must appear for a query of two words in any order (for example "trip reyk" finds "Trip to Reykjavík").
3. Check the language tie-break with an Icelandic and an English note that share a title word.

Commands for Midget use `bifrost pc midget` (see `docs/bifrost-pc-workflow.md`). Use `tap-window` for input.

## REA evidence

- `ev_72be97def9ad512462bd3e80e8021e0da7c2b7045b159448d224fb001dae7bc8`: official page for the searchable item's expiration date ("the date after which the searchable item should no longer exist"; the system also expires items after a period when the property is not set). Source: developer.apple.com/documentation/corespotlight/cssearchableitem/expirationdate.
- `ev_fe9048d16059585825a4a0febd592ccef542b985cc0a07c3469e449382bb8e8c`: official page for the language recognizer (dominant language, hypotheses, hints and constraints; short text is less reliable). Source: developer.apple.com/documentation/naturallanguage/nllanguagerecognizer.
- Docc lookups (`tools/parity/docc.py`) agree: `corespotlight/cssearchableitem/expirationdate`, `naturallanguage/nllanguagerecognizer`.

## Remaining gaps

- Language identification is the ui model's: no n-gram model, so short queries and Scandinavian text are weak evidence (the page says the same). The tie-break only orders results; it never drops a match.
- No language hints from the user's locale are passed to the recogniser yet.
- No word-embedding or sentence similarity (NLEmbedding) in ranking; the embedding model is not wired.
- Expiry is not reported back to apps (no event when an item expires), and the item expiry is not in the Core Search wire item (apps set it through the service).
- Sentence and paragraph tokenisation are not used (word units only).
- The Search panel draws no thumbnails for the Documents section (existing gap).
- Not built or run on a device in this round.
