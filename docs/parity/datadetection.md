# DataDetection parity

Official reference: https://developer.apple.com/documentation/datadetection (macOS 12.0 and later).
Matrix row: `docs/parity-matrix/data/datadetection.json`. Code: `springboard/ui/src/detect.rs` (`bifrost_ui::detect`,
re-exported by the SDK).

## What was built

Two more match kinds in the existing plain-text detector, both pure Rust with std only:

- `Kind::Email` (mirrors `DDMatchEmailAddress`): `local@domain.tld`. The local part is ASCII letters, digits and `._%+-`.
  The domain is dot-separated labels, and its last label is at least two letters, so `a@b` and `a@b.c` are not addresses.
  A trailing sentence dot is not part of the match. A link keeps its `@` (`https://user@host.is` is one link), and a
  `www.` host after an `@` is an address (`x@www.is`).
- `Kind::Money` (mirrors `DDMatchMoneyAmount`): an amount with a currency symbol or code before it (`$12.50`, `€ 3,99`,
  `ISK 2.000`) or `kr` or a code after it (`1.500 kr`, `12,99 EUR`). Digits group with `.` or `,` and a fraction follows
  a `.` or `,`. A bare number is not money, and a code glued to a word (`12kronur`) is not money either.

Overlap priority is now link, email, date, address, money, phone. Kinds listed by the caller in any order produce the
same result. The Notes text area still asks only for links and is unchanged.

Tests (`springboard/ui/src/detect.rs`, `mod tests`): emails need a local part and a top-level label; a link keeps its
`@` and a `www.` host after an `@` is an address; money takes symbols, codes and `kr`; bare numbers and glued words are
not money; email, money, link, date and phone share a text without overlap. The existing tests are unchanged.
The SDK test `data_detectors_are_reachable_from_the_sdk` still uses the four original kinds.

## Official API names mirrored

| Apple | BifrOSt | Status |
|---|---|---|
| DDMatch, DataDetector | `detect::Match`, `detect::find` | partial (range only) |
| DDMatchLink | `Kind::Link` | parity |
| DDMatchEmailAddress | `Kind::Email` | partial (range only) |
| DDMatchPhoneNumber | `Kind::Phone` | partial |
| DDMatchPostalAddress | `Kind::Address` | partial |
| DDMatchCalendarEvent | `Kind::Date` | partial |
| DDMatchMoneyAmount | `Kind::Money` | partial (range only) |
| DDMatchFlightNumber | none | gap |
| DDMatchShipmentTrackingNumber | none | gap |

## REA evidence

- Framework page `https://developer.apple.com/documentation/datadetection`: `ev_32d54383c45c4bc26b56927f872de0ae62b4c401125dc968da7060a21b5609b6`
  (recorded by the matrix author; the row's `rea_evidence`). A fresh `tools/parity/rea-page.sh` run in this session was
  refused by the sandbox guard for worktree-isolated agents, so no new evidence id was recorded here.

## Remaining gaps

- Flight numbers and shipment tracking numbers: no pattern separates them from ordinary words or ids without a carrier
  list or a tracking format. Both need data or a decision, not only a regex.
- Typed match values: `phone` has no parsed number, `address` no city or postcode, `date` no time or range, `email` no
  parts, and `money` no amount or currency value. Apple returns these per match; BifrOSt returns the byte range only.
- Pasteboard detection (`detectPatterns`, `detectedPatterns`, `detectValues`, `detectedValues`): the detector could run
  over pasteboard text, but `springboard/sdk/src/pasteboard.rs` has no hook.
- Transit information: needs transit data.

## Verification status

Not built or tested in this session, by rule. The next integration build must run `cargo test -p bifrost-ui detect`
and the SDK `kits` test, and `python3 tools/check-names.py` must pass (it passed on the changed files here).
