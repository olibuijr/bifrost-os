# Wire 5: News (HTTP caching, Share Article, Larger Text)

News (`springboard/apps/news-desktop`, version 1.5.0) gets the feed and reading parity work. Everything is in the
app's own `src/` folder, plus this note. No SDK, shell or kit file changed. This round wires three things from the
reading and networking rows: conditional feed and picture loads, Share Article, and reader text size. Article search
is not wired (see the gaps).

## What is now wired

| Matrix slug | Feature (as in data/<slug>.json) | BifrOSt path | How it is used in News |
|---|---|---|---|
| foundation-networking-lowlevel | URL Loading: caching | `springboard/apps/news-desktop/src/feed_cache.rs` (`plan`, `stored_from`, `max_age_of`, `validators_of`, `Conditional`) | Every feed and picture load goes through `Conditional`. A stored copy is used while its max-age lasts. An expired copy is revalidated with `If-None-Match` and `If-Modified-Since`, and a 304 reuses the stored body. `no-store` responses are not kept. |
| network | HTTP and URL-based loading | `springboard/apps/news-desktop/src/feed_cache.rs` (`Conditional::get`, over `bifrost_app::url::URLSession::shared().bytes`) | The loads run on the SDK's `URLSession` (TLS, redirects), replacing the shell's plain `radio::Http` client that News used before. |
| link-presentation | Sharing a URL with metadata to the share sheet | `springboard/apps/news-desktop/src/main.rs` (`MENU_SHARE`, `share::show_picker`), `springboard/apps/news-desktop/src/news.rs` (`share_link`) | Edit > Share Article sends the open article's web address as `Payload::Url` to the share picker. Greyed out for pages without a web address (the same rule as the Reading List). |

Reader text size (View > Larger Text and Smaller Text) is a News feature built on `src/reader_size.rs`. It has no
matrix row of its own; it supports the reader rows of the Foundation and Reader matrix pages.

## How a user reaches it

1. **Faster refreshes:** open News and click Refresh (or let the app fetch on its own). Feeds that the server has not
   changed are answered "not modified" and show at once. Nothing to switch on.
2. **Share Article:** open an article, then the Edit menu > Share Article. The share menu lists Copy and the apps that
   take links. Greyed out until an article with an `http` or `https` address is open.
3. **Larger Text / Smaller Text:** open an article, then the View menu > Larger Text (or Smaller Text). The body text
   moves through five sizes (0.85, 1.0, 1.15, 1.3 and 1.5 times the body size). The title keeps its size. The size is
   kept when you open another article, and it resets when News restarts. Each item is greyed out at its end.

## How to verify on the test PC (Midget)

1. Lead build: `cd springboard && cargo build --offline --profile fast --bin bifrost-news`, then deploy as in
   `docs/bifrost-pc-workflow.md`, or `bifrost pc midget ota` for a system build that carries the app.
2. `bifrost pc midget open-app is.olibuijr.news.desktop`, then `bifrost pc midget screen /tmp/news.png`.
3. Caching: click Refresh twice in a row. The second refresh should not download the feeds again (check the network
   trace with `bifrost pc midget trace on`, then `trace off`). The headlines must not change.
4. Share: open an Icelandic article (ruv.is link). Open the Edit menu. "Share Article" is enabled; choose it and check
   that the share picker opens with the article's link. With no article open it is greyed out.
5. Text size: open an article, open the View menu. Choose Larger Text four times and check the body grows each time,
   the title does not, and "Larger Text" is greyed out at the end. Smaller Text returns it step by step.
6. Go Back to the list, open another article: the size is unchanged.
7. Host tests: `cargo test -p bifrost-news feed_cache` and `cargo test -p bifrost-news reader_size` (not run here).

## Tests

- `src/feed_cache.rs` (pure rules): `max_age_of` (max-age, no-cache, no-store, a bad number, no header),
  `validators_of` (case-insensitive names, blank values dropped), `stored_from` (kept or not: a 200 with a lifetime
  or a validator; not kept for 404, no-store, no lifetime and no validator, or over 2 MB), `plan` (fresh while the age
  is below max-age, so at exactly 60 s of a 60 s lifetime the copy is stale; stale with an ETag revalidates with
  `If-None-Match`; stale with no validators fetches again), `conditional_headers` (both validators, in that order).
- `src/reader_size.rs`: the default step is 1.0, steps stop at both ends (`larger(4) == 4`, `smaller(0) == 0`),
  `factor(99)` is the largest step (1.5).
- Expected values were worked out by hand from the code. The network path (`Conditional::get`) is not unit tested; it
  runs only on Midget.

The compliance gate (`tests/compliance.rs`) compares the menus and a snapshot. The View menu is new and the Edit menu
has one more item, so the snapshot needs `BIFROST_BLESS=1` after a visual check on the host preview.

## REA evidence

None gathered in this round. The brief's REA steps (`tools/parity/rea-page.sh`, `docc.py`) were not run, because
this worktree is no-build and the work was done from the code and the matrix rows. The rows named above carry no
evidence id in the copies read for this note. The lead should record rendered evidence for the URLCache
(`CachePolicy`), URLSession and LPLinkMetadata pages before these rows move to `parity`.

## Remaining gaps

- **Article search: not wired.** The find model `ui/src/textfind.rs` exists, but News has no text field in the toolbar
  and the app's events carry no text input. A search field (a kit `search_field` in the toolbar, filtering the list by
  title) is the next step.
- **Refresh does not force a network check.** The app's `Fetch` trait has no "revalidate now" flag, so a fresh copy
  (within its max-age) is used even after the Refresh button. Feeds with no max-age are always revalidated.
- **No disk cache for HTTP responses.** Stored copies are kept in memory (up to 64 URLs, 2 MB each); the feed files
  already on disk remain the offline copy, as before.
- **Cache policies per request** (`CachePolicy` values such as reload-ignoring-cache) and background refresh are not
  in the SDK's `URLSession`; the app cannot choose them.
- **The SDK's own `URLCache` is not used.** Its revalidation is private to `URLSession`'s task path; News does the same
  rules itself (see `feed_cache.rs`), because the blocking `bytes` call does not consult the SDK cache.
- **Share sheet placement:** the picker is given a zero anchor, as the browser's Share does; the shell places it.
- **Reader:** no Dynamic Type or system text size is read (the shell's `text_scale` is not applied to the reader), and
  the reader's translated text is re-wrapped from its wrapped lines when the size changes.
- The matrix rows in `docs/parity-matrix/data/` were not changed (per the brief).

## Matrix rows touched

| slug | feature (as in data/<slug>.json) | BifrOSt path |
|---|---|---|
| foundation-networking-lowlevel | URL Loading: caching | `springboard/apps/news-desktop/src/feed_cache.rs` |
| network | HTTP and URL-based loading | `springboard/apps/news-desktop/src/feed_cache.rs` (`Conditional::get`) |
| link-presentation | Sharing a URL with metadata to the share sheet | `springboard/apps/news-desktop/src/main.rs` (`MENU_SHARE`), `springboard/apps/news-desktop/src/news.rs` (`share_link`) |
