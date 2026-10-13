# Tips (`springboard/apps/tips-desktop`)

Tips is a library of short how-to cards about BifrOSt. It ships with 32 cards in five collections (Getting Started,
Desktop, Windows, Files and Apps), each describing a feature the system or one of its apps has today.

## Using it

- The sidebar lists All Tips, New, and one row per collection, each with a count. The search field above the rows
  filters the list: every word must appear in a card's title, text, keywords or collection name, ignoring case.
- A card marked New has not been opened yet. Opening a card (a tap, or Left and Right while reading) clears the
  marker. Mark All Read in the toolbar clears every marker.
- While a card is open, Left and Right move to the previous and next card of the list it came from. Back in the
  toolbar, or Escape, returns to the list, which keeps the scope and the search. Escape in the list clears the search.
- Typing in the list filters it. Tapping the search field brings up the on-screen keyboard.

## Structure

- `src/model.rs`: the pure model (collections, scopes, search, New counts, reading order, text wrapping, the read
  state). Unit tests cover it.
- `src/catalogue.rs`: the card text, `TIPS`. Tests check the count, the ids, the content and that no card uses a
  platform name the project avoids.
- `src/store.rs`: the read state in `seen.txt` in the app's data directory, one card id per line. A missing file means
  nothing has been read yet.
- `src/app.rs`: the window. Sidebar and toolbar come from the SDK widgets (`widgets::source_list`, `search_field`,
  `toolbar`); cards and text are drawn directly. Touch hit tests use the same geometry as the drawing.
- `src/main.rs`: the SDK `App` glue (configure, touch and key input, tick, controls, draw).

## Adding a card

Add a `Tip` to `TIPS` in `src/catalogue.rs` with a new, unique, kebab-case id. Keep the body to a few plain sentences
about something the system does today, and check the names of apps and menus against the apps' own CHANGELOG entries.
The New marker appears for every user until they open the card.

## Known limits

- No compliance block is generated for this crate yet (`tests/compliance.rs`), so `tools/check-app-compliance.py`
  reports it until the generator runs.
- The window has no menu bar entries of its own.
