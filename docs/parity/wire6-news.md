# Wire 6: News (toolbar search, link cards, language facts)

News (`springboard/apps/news-desktop`, version 1.6.0) gets article search in its toolbar. Everything is in the app's
own `src/` folder, plus this note. No SDK, shell, kit or matrix file changed. Wire 5 had listed search as the next
step; this round wires it.

## What is now wired

| Matrix slug | Feature (as in data/<slug>.json) | BifrOSt path | How it is used in News |
|---|---|---|---|
| swiftui-app-data | Search | `springboard/apps/news-desktop/src/news.rs` (`BAR_SEARCH`, `key`, `search_focused`), `src/wire.rs` (`search_rows`), `springboard/ui/src/textfind.rs` (case-insensitive match), `springboard/sdk/src/widgets.rs` (`search_field`) | A search field at the trailing end of the toolbar filters the list by headline in Today, the Reading List and each feed. Stays Partial in the matrix: no search scopes, tokens or suggestions. |
| natural-language | Language identification (NLLanguageRecognizer, NLLanguage) | `springboard/apps/news-desktop/src/wire.rs` (`facts`) | Already wired by an earlier wave; unchanged here. Shown per article as the language, word count and reading time. |
| link-presentation | Rich link view (LPLinkView) | `springboard/apps/news-desktop/src/wire.rs` (`link_card`) over `springboard/ui/src/linkmeta.rs` | Already wired by an earlier wave; unchanged here. The card under an article shows the title and site. |

Rows not changed: `link-presentation` "Representative title, icon and image of a URL" (the picture is shown above the
article, from the feed, not from a page fetch), and `link-presentation` "Sharing a URL" (wired by an earlier wave).

## How a user reaches it

1. Open News. The search field is at the right of the toolbar, before the window's trailing edge.
2. Click the field, then type. The list keeps the headlines whose title contains the text, ignoring case. It applies
   to the tab you are on (Today, Reading List or one feed).
3. Escape or the clear button (the x at the field's trailing end, shown while there is text) empties the field and
   leaves it. Return leaves the field and keeps the filter.
4. When nothing matches, the list says "No matching articles".
5. Pressing any row, a source, or the Back button leaves the field, and the filter stays until it is cleared.

## How to verify on the test PC (Midget)

1. Build the app and deploy it as in `docs/bifrost-pc-workflow.md`, or `bifrost pc midget ota` for a system build that
   carries it. Then `bifrost pc midget open-app is.olibuijr.news.desktop`.
2. `bifrost pc midget screen /tmp/news.png`: the search field is at the right of the toolbar.
3. Click the field (`bifrost pc midget tap-window APP X Y` with window-local pixels) and type "bridge" or any word in
   the headlines. The on-screen keyboard appears (the shell asks for it). Only headlines containing the word stay.
4. Press Escape: the field empties and the full list returns. Type a word no headline has: "No matching articles".
5. Click a source in the sidebar: the filter applies to that feed. Tapping an article leaves the field.
6. Host tests: `cargo test -p bifrost-news wire::` (not run here; the lead builds once).

## Tests

- `src/wire.rs`: `search_keeps_headlines_that_contain_the_query_in_order` (query "BRIDGE" keeps the first and the third
  of three titles: "Council votes on bridge", "Bridge repairs begin"; "Snow in the north" goes),
  `a_blank_query_keeps_every_row_and_a_missing_word_keeps_none` (blank keeps 2; " snow " keeps "Snow" only; "volcano"
  keeps none).
- The compliance gate (`tests/compliance.rs`) now expects a published search field (`EXPECT_SEARCH = true`). The
  toolbar gained one item, so the snapshot needs `BIFROST_BLESS=1` after a visual check on the host preview.
- The key and click paths (`News::key`, `Target::Search`, `Target::ClearSearch`) need the app host and are verified
  on Midget only.

## REA evidence

None gathered in this round. No Apple page was read for this round; the search behaviour follows the existing kit
search field and the notes find model, both already in the repo.

## Remaining gaps

- **Link previews from the page:** the link card uses only the feed's title, site and description. The matrix
  "Representative title, icon and image of a URL" row needs a fetch of the page's metadata (the `linkmeta` model's
  `from_html` path), and the card does not draw the metadata picture or the icon. Not wired.
- **Shared links into News:** a link shared to News arrives as `Input::Open` and is not turned into an article or a
  preview. Not wired.
- **Background refresh while the app is not running:** News refreshes its feeds every 10 minutes while it runs (and
  the widget timeline is kept current). The system does not refresh News in the background; the SDK has no
  background task API for apps. Not wired.
- **Cookies and session storage** (`http_cookies.rs`, `CFNetwork` row): the feeds need no cookies, and the SDK's
  `URLSession` keeps none. Not wired; no app use yet.
- **Search scope:** the field matches headlines only, not the article text or the source, and has no scopes or
  suggestions. The notes find model (`textfind`) is used for the match, not the whole text-search model.
- **Refresh does not force a network check** (carried over from the wire 5 note).

## Matrix rows touched

| slug | feature (as in data/<slug>.json) | BifrOSt path |
|---|---|---|
| swiftui-app-data | Search | `springboard/apps/news-desktop/src/news.rs`, `springboard/apps/news-desktop/src/wire.rs` (`search_rows`) |
