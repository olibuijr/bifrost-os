# Changelog

## 0.1.0 — 2026-10-13

- First version: a Stocks watchlist in a full-size window. Each symbol shows its last close and its change since the
  open, with Up, Down and remove buttons on its row.
- Add a symbol in the field at the top (AAPL, MSFT, or ^SPX for an index). The list keeps its order and stops at 30
  symbols.
- A price chart of the selected symbol: the last 90 daily closes, with the high and low on the axis and the first and
  last dates along the bottom.
- Quotes and daily history come from Stooq over HTTPS. Quotes refresh after 15 minutes and the chart after 6 hours; a
  failed request waits a minute before it is tried again.
- The watchlist, the last quotes and each chart are cached in the app's data directory. With no network the cached
  values stay on screen and the band says so.
