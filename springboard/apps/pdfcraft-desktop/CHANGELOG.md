## 0.5.1

- Opens as a native Wayland window (from the Dock, Spotlight and the App Store).

# Changelog

## 0.12.0

- Upstream pdfcraft v0.12.0.
  > [!NOTE]
  > **FreeBSD aarch64 is dropped from this release.** We had to cut releases, and we cannot build this target within the 5-hour CI limit for any of the Craft apps, so we are dropping it for now. This is a known blocker, flagged for a future fix in storytold/craft-common-actions#39. FreeBSD x86_64 and every other platform are included.
  ## What's Changed
  * M7: lay out the Edit panel as Acrobat's by @abc2cb in https://github.com/storytold/pdfcraft/pull/2245
  * fix(engine): PNG export writes opaque pages as RGB at deflate level 3 (3–5× faster, same size) by @Piohrun in https://github.com/storytold/pdfcraft/pull/2425

## 0.11.0

- Upstream pdfcraft v0.11.0.
  ## What's Changed
  * Skip CodeRabbit reviews on Dependabot PRs by @echelon in https://github.com/storytold/pdfcraft/pull/3692
  * fix(redact): evaluate OCMD visibility during sanitization by @skylartaylor in https://github.com/storytold/pdfcraft/pull/3522
  * ci: switch to shared nix workflow from craft-common-actions by @dylanmtaylor in https://github.com/storytold/pdfcraft/pull/2753
  * Release: PdfCraft v0.11.0 by @echelon in https://github.com/storytold/pdfcraft/pull/3893

## 0.9.0

- Upstream pdfcraft v0.9.0.
  ## What's Changed
  * Make a missing external oracle loud instead of a silent pass by @tanish2k09 in https://github.com/storytold/pdfcraft/pull/2266
  * One way to write a synthetic PDF fixture by @tanish2k09 in https://github.com/storytold/pdfcraft/pull/2275
  * Measure rendering against Poppler, and turn the pixel test on by @tanish2k09 in https://github.com/storytold/pdfcraft/pull/2286
  * Say what the render-oracle score does and does not mean by @tanish2k09 in https://github.com/storytold/pdfcraft/pull/2290

## 0.8.1

- Upstream pdfcraft v0.8.0.
  ## What's Changed
  * Add a PR template, and record the review practice and external-PR bar by @tanish2k09 in https://github.com/storytold/pdfcraft/pull/223
  * F1 + F2: exact standard-14 metrics, and code-to-Unicode completeness by @tanish2k09 in https://github.com/storytold/pdfcraft/pull/357
  * Retire inherited resources by @not-a-feature in https://github.com/storytold/pdfcraft/pull/1348
  * Fix Lab Bradford adaptation to use LMS white-point ratios by @FredApps in https://github.com/storytold/pdfcraft/pull/1417

## 0.8.0

- Upstream pdfcraft v0.8.0.
  ## What's Changed
  * Add a PR template, and record the review practice and external-PR bar by @tanish2k09 in https://github.com/storytold/pdfcraft/pull/223
  * F1 + F2: exact standard-14 metrics, and code-to-Unicode completeness by @tanish2k09 in https://github.com/storytold/pdfcraft/pull/357
  * Retire inherited resources by @not-a-feature in https://github.com/storytold/pdfcraft/pull/1348
  * Fix Lab Bradford adaptation to use LMS white-point ratios by @FredApps in https://github.com/storytold/pdfcraft/pull/1417

## 0.7.2

- Upstream pdfcraft v0.7.1.
  ## What's Changed
  * Adopt shared panel docking with document-aware visibility by @skylartaylor in https://github.com/storytold/pdfcraft/pull/966
  * Print: Windows lists its printers and spools jobs by @Mawfyy in https://github.com/storytold/pdfcraft/pull/847
  * M10: Windows printing prints at actual size, off the UI thread (follow-up to #847) by @echelon in https://github.com/storytold/pdfcraft/pull/1007
  * ci: refactor workflows to use storytold/craft-common-actions by @dylanmtaylor in https://github.com/storytold/pdfcraft/pull/1073

## 0.7.1

- Upstream pdfcraft v0.7.1.
  ## What's Changed
  * Adopt shared panel docking with document-aware visibility by @skylartaylor in https://github.com/storytold/pdfcraft/pull/966
  * Print: Windows lists its printers and spools jobs by @Mawfyy in https://github.com/storytold/pdfcraft/pull/847
  * M10: Windows printing prints at actual size, off the UI thread (follow-up to #847) by @echelon in https://github.com/storytold/pdfcraft/pull/1007
  * ci: refactor workflows to use storytold/craft-common-actions by @dylanmtaylor in https://github.com/storytold/pdfcraft/pull/1073

## 0.6.0

- Upstream pdfcraft v0.6.0.
  ## What's Changed
  * M3: tiles skip paths that can't paint them (item 5 of #623) by @cyfung1031 in https://github.com/storytold/pdfcraft/pull/670
  * M3: harden render and CLI failure surfaces by @cyfung1031 in https://github.com/storytold/pdfcraft/pull/669
  * Print: list driverless CUPS destinations, not only permanent queues by @edurbs in https://github.com/storytold/pdfcraft/pull/658
  * Wayland: IME keeps working after a seat loses a capability (winit#4747) by @cyfung1031 in https://github.com/storytold/pdfcraft/pull/672

## 0.5.0

- Initial BifrOSt App Store package of upstream pdfcraft v0.5.0.
