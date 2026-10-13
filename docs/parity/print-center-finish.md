# Print Center and printing: finish (round 75)

Slice `print-center-finish`. One printing model across the Print Center app, Settings > Print & Scan and the system print
sheet, all on the CUPS client in `springboard/ui/src/cups.rs` (IPP over the local scheduler). The lpstat parser is gone.

## Reference evidence

- Shipped bundle: `~/Projects/macos-ref/extract/ref27/System/Applications/Utilities/Print Center.app/Contents/Info.plist`
  (bundle id `com.apple.printcenter`, version 1.0, document type "Docs" as Viewer, and a Print Center widget extension in
  `PlugIns/`). The index entry is `System/Applications/Utilities/Print Center.app` in `ref27-index.tsv`.
- Not yet measured: the Print Center nib (menus, toolbar buttons and their connections) and the `en.lproj` label tables.
  The brief asks for `decode_interface_builder` and `inspect_plist` on them. Not done in this round; the Add and Delete
  wording below is the BifrOSt wording from the existing app and Settings page, not a copy of the reference labels.
- REA evidence ids: none recorded this round.

## Rows

| Row (matrix entry) | BifrOSt path | Done | Still needs Midget check |
|---|---|---|---|
| Print Center (`apps`, `path:/System/Applications/Utilities/Print Center.app`) | `springboard/apps/printers-desktop` (version 0.2.0) | Lists printers and queues, pause and resume, set default, hold, release and cancel a job (0.1.0). New: **Add Printer** adds the driverless printer whose address is on the clipboard; **Delete Printer** removes the selected queue after a second press. Both go through `cups::Client` in the worker thread. | Add with a real IPP Everywhere device and a `socket://` address; delete with a queue that has jobs; footer messages at the window width. |
| PrintAndScan (`settings`, `path:/System/Library/PreferencePanes/PrintAndScan.prefPane`) | `springboard/src/shell/print_scan_page.rs`, `springboard/src/shell/settings.rs` | The page already listed printers, set the default and added by address. Its address path now calls `cups::add_device` (queue naming moved to `bifrost_ui::cups::queue_name`, its copy deleted). | Add by address from the prompt; name clash numbering (`-2`). |
| print sheet (no matrix row, `print_sheet.rs` gap note) | `springboard/src/shell/print_sheet.rs` | Printer list now from `cups::Client::printers`, stopped queues are not offered, Save as PDF stays last. New **Pages** row: All, or only the page in the preview. Printer, paper, orientation, scale, copies and preview were already there. | Pages row with a multi-page desktop capture; the sheet is taller (6 rows) on a small screen. |

## Migrations and deletions

- `springboard/ui/src/print.rs`: `parse_lpstat`, `print::Printer`, `print::printers()` and their test are deleted. The
  module no longer lists printers; the doc comment points at `crate::cups`.
- `springboard/src/shell/print_sheet.rs`: uses `bifrost_ui::cups::{Client, Printer, PrinterState}`. The `enabled` flag is
  replaced by `usable()`, which is "state is not Stopped" (a stopped queue is the CUPS disabled state).
- `springboard/src/shell/print_scan_page.rs`: `finish()` calls `cups::add_device`; its `queue_name` and `View::names`
  helpers and the queue-name test are deleted (the test moved to `ui/src/cups.rs`).
- `ui/src/cups.rs`: new `queue_name` (moved), `add_device` (trims, refuses bad addresses before any request, numbers the
  name past existing queues). Tests for both.
- `apps/printers-desktop`: `Action::Delete`, `Request::Add`, `Reply::Added`, `Model::confirm_delete`,
  `model::device_from_clipboard`, toolbar buttons 7 (Add Printer) and 8 (Delete Printer / Confirm Delete).

## What is still a model or a gap

- **Add Printer in Print Center reads the clipboard.** The SDK has no text prompt for an app. The Settings page has the
  prompt; the app points the person at it through the footer. A real prompt needs an SDK sheet API (not built here).
- **Submission still uses `lp`** (`ui::print::PrintOperation::run`) for the sheet and for other apps. The CUPS client has
  `print_document` (Print-Job over IPP), but it takes no options yet, so `media`, `landscape`, margins and copies are not
  expressible through it. Moving submission is a follow-up that needs IPP attribute mapping in `PrintInfo`.
- **Pages** only prints the previewed page by making the PDF of that page alone; the `pages` range in `PrintInfo` is not
  used by the sheet (it stays `None`).
- The sheet's Pages row is a toggle, not a range entry.
- Not built or run in this round (code and commit only). The lead builds and tests the three crates (`bifrost-ui`,
  the shell, `bifrost-printers`).

## Shared-file edits

None to `sdk/src/lib.rs`, `ui/src/lib.rs` or `view/mod.rs`. The shell `CHANGELOG.md` and `Cargo.toml` are not bumped; the
shell change is internal (no app-facing API change except that `bifrost_ui::print::printers` is removed, which the
lead should note for the next shell release).
