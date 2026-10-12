# Wiring wave 5: printing (File > Print…)

Branch `wire5/printing`. Builds on the printing models of waves 1 to 4 (`springboard/sdk/src/print.rs`,
`print_panel.rs`) and on `appkit-graphics-printing.md`. The matrix data under `docs/parity-matrix/data/` is not
edited in this round; the rows below are for the lead to update after the Midget check.

## What is now wired

- **File > Print…** (shell menu bar). The row is enabled while a shell window is front and no external app is in front
  (an external app prints through its own File menu). Its key equivalent is shown as Cmd+P.
- **The print sheet** (`springboard/src/shell/print_sheet.rs`). The shell captures the desktop as the document (the same
  frame drawing the screenshot uses), then shows a card over it with:
  - **Printer**: the enabled CUPS queues from `lpstat` (`bifrost_app::print::printers`), then **Save as PDF**. The
    default queue is preselected. Tap the row to cycle.
  - **Paper**: A4, Letter, A3, A5, Legal (`PrintInfo` / `PaperSize`). Tap to cycle.
  - **Orientation**: Portrait or Landscape. Tap to toggle.
  - **Scale**: 50, 75 or 100 percent. Tap to cycle.
  - **Copies**: 1 to 9. Tap to step.
  - **Preview**: the current page scaled into a page outline, with `<` and `>` to turn pages and "Page i of N".
  - **Cancel** and **Print**.
- **Pagination** (`layout`): a document wider than the printable width is reduced to fit it, and the document is split
  into pages that fit the imageable height at the chosen scale (`PrintInfo::imageable`).
- **Output** (`write_pdf`, `Job::run`): all pages go into one PDF (raw RGB image pages, std only, with a valid xref).
  - A printer receives the PDF through `PrintOperation` (`lp`), with the paper, orientation, margins and copies applied.
    The temporary file is removed after submission.
  - Save as PDF writes `Print <unix>.pdf` into the Documents folder of the shell data directory.
  - Print runs in a background thread, so the desktop does not wait for CUPS.
- The sheet takes every touch while it is open (`modal_event` in `mod.rs`), and it is drawn over the desktop in the
  modal layer.

Hooks outside the new file (kept small): `mod.rs` (module, field, init, modal touch, modal draw), `desktop_bar.rs`
(`Action::Print`, the File menu row, the dispatch arm, the `MENU_KEYS` entry).

## How to reach it

File menu > Print… (Cmd+P as shown). The sheet opens over the desktop; pick a printer or Save as PDF, change the
settings, then Print.

## How to verify on Midget (test PC)

1. Publish a build with this branch and boot Midget (`bifrost pc midget ota`, until `phase=accepted`).
2. Open a window (for example Notes with `bifrost pc midget open-app notes` or the Files app), then open the File menu
   on the bar. Find the bar position with `bifrost pc midget state`, then `bifrost pc midget tap X Y` on "File".
3. Tap "Print…". Take `bifrost pc midget screen /tmp/print-sheet.png` and check the card, the printer rows and the page
   preview.
4. Tap the Paper row, the Orientation row and the Scale row. The preview page changes shape, and the page count
   updates when the scale changes.
5. Tap Save as PDF in the Printer row, then Print. The file appears in the Documents folder of the shell data directory
   (`Print <unix>.pdf`). Copy it off the machine and open it: the page should match the screenshot.
6. If CUPS is present (`lpstat -p -d` lists a printer), select that printer and Print. Check the job with `lpstat -o`.
   On a Midget image without CUPS only Save as PDF is listed; that is expected.

Host checks (no build run in this round; the lead's single build runs them): `cargo test -p springboard print_sheet`
(the `springboard/src/shell/print_sheet.rs` tests: layout, targets, PDF structure, sheet hits, copies).

## REA evidence (official reference)

- NSPrintOperation (print operation, run, PDF and EPS operations, print info): `ev_afe290ce1d5430f91b2bab15f9e73f0d1a42817f34b008a005404dbebbbf8cfe`
- NSPrintPanel (the print panel, accessory controllers, job style): `ev_adb552613616f49cda936e888122d0d2a419372b8fe786c237af291a37e69475`
- NSPrintInfo (paper size, margins, imageable rectangle): `ev_0866478580df9f5fc16e5089e0ccf5ae099f5699a1a9020aef6bbe01965f13f9`

The official DocC structure came from `python3 -I tools/parity/docc.py appkit/nsprintoperation` (and `appkit/nsprintpanel`,
`appkit/nsprintinfo`). Behaviour followed from those pages: a print operation runs on a print info object (paper size,
margins), the print panel is the place to choose the printer and the options, and the PDF operation writes the view's
pages to a PDF.

## Matrix rows this round touches

| Slug | Feature (exactly as in `docs/parity-matrix/data/<slug>.json`) | BifrOSt path |
|---|---|---|
| appkit-graphics-printing | Printing: print info and operation | `springboard/src/shell/print_sheet.rs` (Job, Layout, paper and scale) over `springboard/sdk/src/print.rs` |
| appkit-graphics-printing | Printing: print panel, page layout, accessory views | `springboard/src/shell/print_sheet.rs` (sheet: printer, paper, orientation, scale, copies); accessory views not done |
| appkit-graphics-printing | Printing: printer discovery | `springboard/src/shell/print_sheet.rs` (`printers` and `targets`) over `springboard/sdk/src/print.rs` |
| application-services | Core Printing (PMPageFormat, PMPrintSettings, PMSession) | `springboard/src/shell/print_sheet.rs` (`PrintSheet`, `Setup`, `Job`) over `springboard/sdk/src/print.rs` |
| core-graphics | PDF creation and printing (CGPDFContext, print to PDF) | `springboard/src/shell/print_sheet.rs` (`write_pdf`, Save as PDF) |
| quartz | PDF writing, print workflow and Quartz filters (QuartzFilter, QuartzFilterManager) | `springboard/src/shell/print_sheet.rs` (`write_pdf`); no filters |

Not changed: `application-services` "Print dialog plug-ins (PDEPlugIn, PDEPanel)" (`print_panel.rs` is still not wired),
`pdfkit` "Printing PDF documents" (existing PDF files are still submitted by apps, not printed by the shell).

## Remaining gaps

- **Keyboard**: Escape does not cancel the sheet, and Tab does not move focus between the rows. Cmd+P appears in the
  menu, but the shell does not dispatch it as a chord yet (the key table is display-only for shell menus).
- **Page range**: the sheet prints every page. `PrintInfo::pages` is not set.
- **Document**: the captured document is the desktop frame, not an app's document. Apps still print their own
  documents through `PrintOperation` and their own File menus.
- **Output size**: pages are raw RGB, so a full-screen page is about 6 MB uncompressed. A deflate writer would shrink it.
- **Printer list**: `lpstat` runs on the UI thread when the sheet opens. A slow CUPS scheduler delays the sheet.
- **Plug-ins and accessory views**: no print dialog plug-in panels are drawn; `print_panel` options are not fed into
  `lp` from the sheet.
- **Midget**: CUPS on the Midget image is not checked. Only Save as PDF is verified until the check is done.
- **Matrix**: rows stay `partial` until the Midget check above passes; the row notes should record the version.
