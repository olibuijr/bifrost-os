# AppKit (Text): parity round

Official reference: https://developer.apple.com/documentation/appkit (text topics). Matrix row:
`docs/parity-matrix/data/appkit-text.json`.

## What was built

Two pure models in `bifrost-ui` (`springboard/ui/src/`). Neither draws anything: the SDK draws the bar or the list
marker, and the text layout places it.

### `textfind.rs`: find bar model

Mirrors the find and replace actions of the official text finder reference (`NSTextFinder`, macOS 10.7): the search
string, the match options, the selected match, and the bar's counter.

- `Options { ignore_case, whole_word }`: ignore_case is on by default, as the find bar searches.
- `find_all(text, query, opts)`: every match in order, without overlaps, as byte ranges on character boundaries.
- `replace_all(text, query, opts, with)`: the replaced text and the number of replacements.
- `TextFinder`: holds the query and matches for one text. `set_query`, `set_options`, `refresh` (after the text changed,
  keeping the selected match number), `find_next(from)` and `find_previous(from)` with wrap-around, `position()`
  for the "n of m" counter, and `replace_selected`.

### `textlist.rs`: list marker model

Mirrors the text list reference (`NSTextList`, macOS 10.0): the marker format, the starting item number and the
prepend-enclosing-marker option for nested items.

- `Format` covers the official marker constant names that have a plain text form: disc, circle, square, box, diamond,
  check, hyphen, decimal, lowercase and uppercase alpha, lowercase and uppercase roman, lowercase and uppercase
  hexadecimal and octal. `is_ordered()` separates numbered from bullet formats.
- `List { format, starting_item_number, prepend_enclosing_marker }`, with `marker(index, enclosing)` giving the marker
  of an item. A nested item with the option on shows the enclosing marker before its own, for example `2.b.`.
- Decisions: alphabetical markers are bijective base 26 (`a`, `z`, `aa`), as spreadsheet columns count. Zero and
  numbers past the range of letters or roman numerals fall back to decimal. Alpha markers use Latin letters only;
  the official reference localizes them, and that is a remaining gap.

## Official API names mirrored

- `NSTextFinder` (`appkit/nstextfinder`): find bar, `performAction`, `findBarContainer`, the client's search string.
- `NSTextList` (`appkit/nstextlist`): `markerFormat`, `marker(forItemNumber:)`, `startingItemNumber`,
  `NSTextList.MarkerFormat` (constants `disc` to `uppercaseRoman`, `octal`), `NSTextList.Options`
  (`prependEnclosingMarker`).

## REA evidence

- `ev_72425a3bb00f14df7a67904ee7d16675a3590975da03c540c2c645351f34e544`: REA passive capture of
  https://developer.apple.com/documentation/appkit/nstextfinder as rendered by Chromium. It shows the find bar
  overview, the client protocol and the action and validation flow.
- `ev_7e5b3e279855ac0e30000128079ed17f7b6bd5dfc69bb0d5aed74e943849d0a0`: the framework row's evidence from the parity
  matrix (AppKit text landing page), unchanged.

## Remaining gaps

- No find bar widget: nothing draws the bar or selects matches in a text view. The model is ready for it.
- No list layout: attributed text does not store list membership, and the layout does not draw markers.
- Latin letters only for alphabetical markers, no localization.
- Multi-line text view widget, TextKit layout manager and text containers, paragraph styles, tables and attachments,
  Writing Tools: unchanged, still gaps.

## Tests

Unit tests in `springboard/ui/src/textfind.rs` (8) and `springboard/ui/src/textlist.rs` (6). Not run in this stage;
the build stage runs them. Owner-rule check: `python3 tools/check-names.py` on the new files passes.
