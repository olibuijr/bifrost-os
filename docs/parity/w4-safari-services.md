# Wave 4: Safari Services gap rows

Two gap rows of the Safari Services framework now have a pure Rust model. Both are in `springboard/ui/src/` and
are not yet wired into the SDK, the Browser app or the shell. Nothing was built or run in this wave, so the new
modules have host unit tests that were worked out by hand but not executed.

## What was built

### `springboard/ui/src/browser_sheet.rs` (module `browser_sheet`)

A model of a standard in-app browser sheet. It holds the address the sheet opened, the current address after
redirects, the configuration (reader on load, bar collapsing, dismiss button style), optional bar and control tint
token names, and the events the owner receives: initial load completed, redirect during that load, dismissed, and
open in the full browser. Each event is reported once. Only http and https addresses with a host are accepted.

Mirrored official API (developer.apple.com, `SFSafariViewController`):

- `init(url:)`, `init(url:entersReaderIfAvailable:)`, `init(url:configuration:)` mapped to `BrowserSheet::new`
  with a `Configuration`.
- `SFSafariViewController.Configuration`: `entersReaderIfAvailable`, `barCollapsingEnabled`.
- `SFSafariViewController.DismissButtonStyle`: `done`, `close`, `cancel` mapped to `DismissStyle`.
- `preferredBarTintColor`, `preferredControlTintColor`: mapped to `set_tints` with colour token names.
- `SFSafariViewControllerDelegate`: `safariViewController(_:didCompleteInitialLoad:)`,
  `safariViewController(_:initialLoadDidRedirectTo:)`, `safariViewControllerDidFinish(_:)`,
  `safariViewControllerWillOpenInBrowser(_:)` mapped to the `Event` variants.

Not mirrored: the web content itself (no web view here), PCM event attribution, prewarming, the data store, and the
activity button. Apple's page says the sheet cannot be inspected by the app (no access to history or AutoFill), and
the model keeps that rule by exposing only the URLs it was given.

### `springboard/ui/src/bookmark_import.rs` (module `bookmark_import`)

A parser for the HTML bookmark export that browsers write: folder headings followed by list blocks, and anchors
with an `HREF`. `parse` returns a tree of `Node::Folder` and `Node::Link`. `bookmarks` flattens the tree to links
with their folder paths. The parser handles comments, quoted attribute values that contain `>`, named and numeric
entities, unclosed folders and stray list closes. Links with an empty address are skipped.

Apple's official article `Importing data exported from Safari` describes the transfer only and gives no file
format. The HTML form is the common export written by browsers and is not taken from Apple documentation. The
module makes no claim about the exact bytes a particular browser writes; a caller that imports from a real export
must check it against one.

Not covered: saved passwords, reading the file from disk (the caller does that), browser detection, and writing
the imported bookmarks into the Browser app's store.

## Official sources

- Framework landing page: https://developer.apple.com/documentation/safariservices
- SFSafariViewController: https://developer.apple.com/documentation/safariservices/sfsafariviewcontroller
- Importing data exported from Safari: https://developer.apple.com/documentation/safariservices/importing-data-exported-from-safari

## REA evidence

- `ev_5e0af1602fa10cbf00b2e2976f68f55329df22fecda1d86dd8bf46ec784aec10`: Safari Services landing page (rendered
  topics, including the "Importing data exported from Safari" and `SFSafariViewController` entries).
- `ev_67994966c104f783c68535175a67b85c9989848de9644c551bc1759608b3a5d9`: `SFSafariViewController` page (rendered
  overview: the sheet, its Done and Action controls, and the statement that app code cannot read the browsing data).

## Matrix rows closed

These rows are for the lead to apply to `docs/parity-matrix/`. This wave did not edit that directory.

| Framework slug | Feature (as in the data file) | New status | BifrOSt path |
|---|---|---|---|
| safari-services | Standard in-app Safari browser sheet | partial | springboard/ui/src/browser_sheet.rs |
| safari-services | Importing data exported from Safari | partial | springboard/ui/src/bookmark_import.rs |

Rows left as they were: Safari settings access (gap), Authentication sessions (gap, deprecated), Safari web
extensions (gap).

## Checks run

- `python3 tools/check-names.py`: exit 0 after the new files were written.
- No cargo, rustc or test runs (wave rule).
