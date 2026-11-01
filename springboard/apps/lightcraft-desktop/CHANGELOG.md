## 0.5.1

- Opens as a native Wayland window (from the Dock, Spotlight and the App Store).

# Changelog

## 0.16.0

- Upstream lightcraft v0.16.0.
  ## What's Changed
  * Grant network access so update checks can reach GitHub by @dylanmtaylor in https://github.com/storytold/lightcraft/pull/3269
  * Honor CARGO_BUILD_TARGET in FreeBSD packaging for aarch64 release builds by @dylanmtaylor in https://github.com/storytold/lightcraft/pull/3275
  * deps: bump tar, zip, roxmltree, rustls-webpki and symphonia to latest by @dylanmtaylor in https://github.com/storytold/lightcraft/pull/3358
  * CI: workflows only call craft-common-actions at @main (AGENTS.md; windows-arm64 unpinned) by @dylanmtaylor in https://github.com/storytold/lightcraft/pull/3356

## 0.15.0

- Upstream lightcraft v0.15.0.
  ## What's Changed
  * Migrate LightCraft to egui 0.37 with current main preserved by @skylartaylor in https://github.com/storytold/lightcraft/pull/2913
  * feat(update): add startup update prompt modal and self-update by @dylanmtaylor in https://github.com/storytold/lightcraft/pull/2933
  * security: bound heic-rs 0.1.4 work around hostile HEIF containers by @dylanmtaylor in https://github.com/storytold/lightcraft/pull/2770
  * security/raw: bound hostile-memory paths in the raw decoders by @dylanmtaylor in https://github.com/storytold/lightcraft/pull/2780

## 0.14.0

- Upstream lightcraft v0.14.0.
  ## What's Changed
  * fix: document the deliberate buffer leak during thread teardown (SonarCloud) by @dylanmtaylor in https://github.com/storytold/lightcraft/pull/2700
  * packaging: bash tests, named positional parameters and stderr errors (SonarCloud S7688/S7679/S7682/S7677) by @dylanmtaylor in https://github.com/storytold/lightcraft/pull/2655
  * Packaging: pin the signing client and appimagetool, and check their hashes by @aiongg in https://github.com/storytold/lightcraft/pull/2413
  * CI: one test binary per crate, line tables only, one WASM check by @aiongg in https://github.com/storytold/lightcraft/pull/2710

## 0.13.0

- Upstream lightcraft v0.13.0.
  > [!NOTE]
  > **FreeBSD aarch64 is dropped from this release.** We had to cut releases, and we cannot build this target within the 5-hour CI limit for any of the Craft apps, so we are dropping it for now. This is a known blocker, flagged for a future fix in storytold/craft-common-actions#39. FreeBSD x86_64 and every other platform are included.
  ## What's Changed
  * security: validate CLI paths/args in dev tools (SonarCloud S2083/S8707/S8705) by @dylanmtaylor in https://github.com/storytold/lightcraft/pull/2494
  **Full Changelog**: https://github.com/storytold/lightcraft/compare/v0.12.0...v0.13.0

## 0.11.0

- Upstream lightcraft v0.11.0.
  ## What's Changed
  * docs: add SonarCloud and CodeRabbit badges to README by @dylanmtaylor in https://github.com/storytold/lightcraft/pull/2240
  * Skip CodeRabbit reviews on Dependabot PRs by @echelon in https://github.com/storytold/lightcraft/pull/2407
  * ci: switch to shared nix workflow from craft-common-actions by @dylanmtaylor in https://github.com/storytold/lightcraft/pull/1980
  * Integration: 43 reviewed PRs (round 1) by @aiongg in https://github.com/storytold/lightcraft/pull/2470

## 0.9.1

- Upstream lightcraft v0.9.1.
  ## What's Changed
  * Fix Lightroom catalog import results and reduce CPU and memory use by @skylartaylor in https://github.com/storytold/lightcraft/pull/1406
  * ci: add comfortable runtime limits to unbounded jobs by @skylartaylor in https://github.com/storytold/lightcraft/pull/1836
  * Integration batch 36: 110 reviewed community PRs (slideshow, camera verification, translations, never-crash, UI) by @echelon in https://github.com/storytold/lightcraft/pull/1843
  * DNG export and Convert / Copy as DNG write the colour a raw without matrices opens with by @marcoarnulfo in https://github.com/storytold/lightcraft/pull/1750

## 0.8.1

- Upstream lightcraft v0.8.0.
  ## What's Changed
  * Integration batch 32a: 8 reviewed contributor PRs by @echelon in https://github.com/storytold/lightcraft/pull/1426
  * Issue #989: clippy 1.99, fill_box reads its buffer with as_chunks::<3>() (item 4) by @snofla in https://github.com/storytold/lightcraft/pull/997
  * LR-LIB-ALBUM: "Include selected photos" option in New Album by @MarkusFriedel in https://github.com/storytold/lightcraft/pull/1250
  * Scroll the filmstrip when the same photo is opened in another source. by @vbasky in https://github.com/storytold/lightcraft/pull/1253

## 0.8.0

- Upstream lightcraft v0.8.0.
  ## What's Changed
  * Integration batch 32a: 8 reviewed contributor PRs by @echelon in https://github.com/storytold/lightcraft/pull/1426
  * Issue #989: clippy 1.99, fill_box reads its buffer with as_chunks::<3>() (item 4) by @snofla in https://github.com/storytold/lightcraft/pull/997
  * LR-LIB-ALBUM: "Include selected photos" option in New Album by @MarkusFriedel in https://github.com/storytold/lightcraft/pull/1250
  * Scroll the filmstrip when the same photo is opened in another source. by @vbasky in https://github.com/storytold/lightcraft/pull/1253

## 0.7.2

- Upstream lightcraft v0.7.1.
  ## What's Changed
  * Add shared docking while preserving photo tools and catalog drops by @skylartaylor in https://github.com/storytold/lightcraft/pull/869
  * Integration batch 26: 31 reviewed contributor PRs by @bflatastic in https://github.com/storytold/lightcraft/pull/975
  * Brush mask: holding the brush still adds no coverage; flow builds up by passes (#518) by @bitshiftza in https://github.com/storytold/lightcraft/pull/767
  * KEY-ESC: Esc leaves the crop tool before leaving Detail by @PiMinusMax in https://github.com/storytold/lightcraft/pull/772

## 0.7.1

- Upstream lightcraft v0.7.1.
  ## What's Changed
  * Add shared docking while preserving photo tools and catalog drops by @skylartaylor in https://github.com/storytold/lightcraft/pull/869
  * Integration batch 26: 31 reviewed contributor PRs by @bflatastic in https://github.com/storytold/lightcraft/pull/975
  * Brush mask: holding the brush still adds no coverage; flow builds up by passes (#518) by @bitshiftza in https://github.com/storytold/lightcraft/pull/767
  * KEY-ESC: Esc leaves the crop tool before leaving Detail by @PiMinusMax in https://github.com/storytold/lightcraft/pull/772

## 0.6.0

- Upstream lightcraft v0.6.0.
  ## What's Changed
  * Integrate 59 community PRs (batches 21–23) by @echelon in https://github.com/storytold/lightcraft/pull/657
  * LR-IMP-CAMERA-COVERAGE: Sony ILCE-7CR samples in the corpus by @AlexChambers in https://github.com/storytold/lightcraft/pull/524
  * LR-PROF-CAMERACOLOR: bundled Sony ILCE-7CR camera profile by @AlexChambers in https://github.com/storytold/lightcraft/pull/526
  * LR-PRE-AMOUNT: presets with the Grain group apply at any amount by @AlexChambers in https://github.com/storytold/lightcraft/pull/530

## 0.5.0

- Initial BifrOSt App Store package of upstream lightcraft v0.5.0.
