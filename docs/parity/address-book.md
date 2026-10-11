# Address Book (parity wave 2)

Official reference: https://developer.apple.com/documentation/addressbook (deprecated in macOS 10.11; the page says to use
the Contacts framework instead). REA evidence of the rendered page: `ev_e29251c74715132aeb40fd8bfff31861e067f843e0b76aba47e9f1b95593bc12`.

This round builds two pure models in `bifrost-ui` (`springboard/ui/src`). Neither is wired into the contact store or
an SDK call yet, so the matrix status is `partial` at most.

## What was built

### `springboard/ui/src/postal.rs` (`PostalAddress`)

Structured postal address (street, city, state or region, postal code, country) with a formatter. It mirrors the
behaviour of the Address Book formatter for a dictionary of address parts (the `ABCreateFormattedAddressFromDictionary`
function in the Address Book functions group, REA evidence above).

- `lines(postal_first)` returns the address in reading order: street, locality, country. The postal code goes after the
  state by default, or before the city when `postal_first` is set (common in Iceland and much of Europe).
- Blank and whitespace-only parts are dropped, so partly filled addresses leave no stray separators.
- `one_line(postal_first)` joins the lines with ", ".
- `is_empty()`.

Tests: 6 (`cargo test -p bifrost-ui postal` is for the lead to run; not run here).

### `springboard/ui/src/recordsearch.rs` (`Element`, `Comparison`, `matches`, `filter`)

Compound search over records, mirroring the Address Book search elements:

- `Element::Match` compares one property, or any property (`any_property`), with `Comparison::Contains`, `Equals`,
  `BeginsWith` or `EndsWith`. Comparisons ignore case.
- `Element::All` is a conjunction (every child matches); `Element::Any` is a disjunction (one child matches). Empty
  `All` matches everything and empty `Any` matches nothing, as the reference defines the empty cases.
- `filter(element, records, properties, values)` returns the indices of the matching records in order. The caller owns
  the records and supplies a function that returns a record's values for a property.

REA evidence for the search element API: the same landing page (`ev_e29251c7...`), and the `ABSearchElement` symbol
page (`init(forConjunction:children:)`, `matchesRecord(_:)`, `ABSearchConjunction`, `ABSearchComparison`), read with
`docc.py addressbook/absearchelement`.

Tests: 6 (single property, string comparisons, conjunction and disjunction, any-property matching, empty conjunction
and disjunction, a property with no values).

## Official API names mirrored

- `ABCreateFormattedAddressFromDictionary` (postal address formatting)
- `ABSearchElement`, `init(forConjunction:children:)`, `ABSearchConjunction`, `ABSearchComparison`, `matchesRecord(_:)`
  (compound search)

## Matrix rows

| Framework | Feature (as written in data/address-book.json) | New status | BifrOSt path |
|---|---|---|---|
| Address Book | Formatted postal addresses | partial | springboard/ui/src/postal.rs (PostalAddress::lines, one_line) |
| Address Book | Search | partial | springboard/ui/src/recordsearch.rs (Element, filter); the contact store still runs a single text query |

Rows checked and found already partly covered by wave 1 (the lead may want to update them; not edited here):

- Address Book | Groups: `springboard/ui/src/contact_groups.rs` holds a group tree with members and recursive membership
  (`Groups::members`), which the matrix still lists as `gap`. The store and SDK do not use it yet.

## Not done in this round

- Contact store and SDK: no call lets an app search or read contacts with these models, and the store still keeps one
  address as a free-text string. Wiring them in is the next step.
- Contacts photos, access authorization, people picker and vCard import are unchanged.
- Locale-specific address layouts beyond the `postal_first` switch are not modelled.
