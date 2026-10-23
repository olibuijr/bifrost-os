# Stocks

Stocks is a watchlist of quotes with a price chart of the selected symbol. The app is `springboard/apps/stocks-desktop`
(crate `bifrost-stocks`, id `is.olibuijr.stocks.desktop`, version 0.1.0). It is a full-size window built on the
Golden Gate SDK kit (`bifrost_app::kit`), with the add field and the list drawn directly.

## Screen

- The title "Stocks" sits after the traffic lights in the unified band. At the right, the band says when the quotes
  were last updated, or that they are updating, unavailable, or offline with saved quotes.
- The add field takes a symbol: `AAPL` becomes `aapl.us`; a symbol with a suffix (`sap.de`) or an index caret (`^SPX`)
  keeps its form. Return adds it and selects it. A note under the field says what happened: the symbol was added, is
  already listed, is not a symbol, or the list is full (30 symbols).
- Each row shows the symbol, the last close, and the change since the day's open in per cent. Every row has three
  buttons on the right: Up, Down (both move the row, the selection follows it) and the remove mark. Tapping a row selects
  it. The list scrolls when it is longer than the card.
- The chart card shows the selected symbol's last price and change, the time of the quote, the change over the chart's
  sessions, and a line of its last 90 daily closes with the high and low on the axis and the first and last dates
  along the bottom. The line is green when the period ends above where it began and red when it ends below.

## Data

- Quotes and history: Yahoo Finance's public chart endpoint, `https://query1.finance.yahoo.com/v8/finance/chart/<symbol>?range=1mo&interval=1d`
  for a quote (the price, the name, the previous close and a month of closes for the sparkline) and `range=6mo` for
  the chart; the last 90 sessions are kept. One request per symbol. The service code (symbol rules, addresses, the
  parser and the fetcher) is `springboard/src/shell/market_quotes.rs`, shared with the Weather app's watchlist page.
  Stooq, the source before, answers 404 and a browser check (E2E W4-14).
- Privacy (owner rule, privacy-first): a request carries the symbol, the range and the interval and nothing else: no
  key, account, cookie or crumb. The fetcher (`market_quotes::QuoteHttp`) sends the generic `User-Agent: Mozilla/5.0`,
  keeps no cookie the service sets (`Client::without_cookies`) and refuses plain HTTP, redirects included; the test
  `a_quote_request_sends_the_symbol_and_nothing_that_identifies_the_person` checks the bytes that leave.
- The sidebar's foot credits the source: "Quotes from Yahoo Finance · delayed" (`market_quotes::ATTRIBUTION`); the
  Weather app's watchlist page shows the same line. It is a free public source, not a licensed market feed, and its
  quotes are delayed. The chart card shows when each quote was priced ("Priced ..."). Do not use the figures for
  trading decisions.
- Under the testing kit (`BIFROST_TEST_PIN=1`) the network is off, so snapshots never show live prices.

## Caching and offline use

Everything the app shows is cached in its data directory (`BIFROST_DATA`):

| File | Holds |
|---|---|
| `watchlist.txt` | the symbols, one per line, in order |
| `quotes.csv` | the last good quote answer, as received |
| `history-<symbol>.csv` | the last good history answer of each symbol, as received |

Files are written through a temporary file and renamed. The age of a cached answer is the file's age, so a restart shows
the right "Updated" time. Refresh rules (`feed.rs`):

- Quotes are fetched when they are more than 15 minutes old, at once after a symbol is added, and at start.
- History is fetched when it is more than 6 hours old, for the selected symbol.
- A failed fetch keeps the data already held and waits 60 seconds before it asks again.
- An answer with no priced rows is an error, so an outage page never empties the list.

With no network the cached quotes and charts stay on screen, and the band says "Offline. Showing saved quotes."

## Code map

| Module | Role | Tests |
|---|---|---|
| `model.rs` | watchlist rules (add, remove, move, select, limit), quote change, chart scaling | yes |
| `data.rs` | symbol rules, request addresses, CSV parsers, fetch functions | yes |
| `store.rs` | cache files; the watchlist codec and history file names | yes |
| `feed.rs` | when a fetch is due, applying results, cached data and failures | yes |
| `state.rs` | the screen's state, the add field, touch and key input, the background jobs | no (needs a UI) |
| `view.rs` | layout, hit testing, drawing; the status text and age wording | yes (pure parts) |
| `main.rs` | the `App` implementation: configure, input, tick, draw, keyboard | no |

Background jobs run on `DispatchQueue::global(Qos::Utility)`. Each posts its result to a mailbox and wakes the main
queue; `tick` applies the results on the app loop, so no UI call waits on the network or the disk.

## Not in 0.1.0

- No compliance snapshot test yet (`tests/compliance.rs` from the template); the golden images are not made.
- No sorting or column choice, no change in other currencies, no news, no chart periods other than the 90 sessions.
- No network path watch (the app retries on its own schedule instead of when the connection returns).
