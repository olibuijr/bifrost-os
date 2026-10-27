# Changelog

## 0.2.6

- Rebuilt for 0.10.44: every touch says whether a mouse or a finger made it, so a mouse drag no longer scrolls lists on the desktop (the wheel, trackpad and scroll bar still do).

## 0.2.5

- The price chart colour follows the change since the previous close, matching the header and list.

## 0.2.4

- Rebuilt with the 0.10.40 SDK: a background app redraws at its new size after a resize.

## 0.2.3

- Rebuilt with the current SDK: the app redraws after an action from the shell (Look Up, Services, Spotlight) and accepts the shell's account-credential connections.

## 0.2.2

- Rebuilt with the current SDK and renderer: shell actions such as Look Up and Services reach the app, and vibrant text draws inside clipped areas.

## 0.2.1

- Rebuilt with the current renderer and SDK: Chinese, Japanese and Korean text draws whole, and a file opened with this app from Files or the Dock opens as a document.

## 0.2.0 — 2026-10-23

- Quotes from Yahoo Finance's public chart endpoint (Stooq is gone), sent without cookies or identifiers, with the source named.
- A sidebar with sparklines and a detail pane with the chart; unknown symbols say Not Found.

## 0.1.4

- BifrOSt 0.10.0 round (waves 14 and 15): see the system CHANGELOG for this app's changes.

## 0.1.3 — 2026-10-14

- Use the shared grouped-card radius and appearance-aware pressed-row color.

## 0.1.2 — 2026-10-14

- Add the generated app compliance harness.

## 0.1.1 — 2026-10-14

- Use shared spacing tokens for the empty-watchlist message and chart date labels.

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
