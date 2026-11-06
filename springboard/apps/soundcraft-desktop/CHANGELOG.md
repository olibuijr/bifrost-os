## 0.4.1

- Opens as a native Wayland window (from the Dock, Spotlight and the App Store).

# Changelog

## 0.17.1

- Upstream soundcraft v0.17.1.
  **Full Changelog**: https://github.com/storytold/soundcraft/compare/v0.17.0...v0.17.1

## 0.17.0

- Upstream soundcraft v0.17.0.
  ## What's Changed
  * Keep score export notes when bars are shorter than a sixteenth by @Silvertree2010 in https://github.com/storytold/soundcraft/pull/1234
  * Export only the MIDI notes a trimmed clip still plays by @Silvertree2010 in https://github.com/storytold/soundcraft/pull/1032
  * AIFF: test that a negative channel count is rejected by @TheGalitube in https://github.com/storytold/soundcraft/pull/1223
  * Accept VST3 multi-out instruments with up to 64 audio buses by @Silvertree2010 in https://github.com/storytold/soundcraft/pull/1248

## 0.16.0

- Upstream soundcraft v0.16.0.
  ## What's Changed
  * Updates: shared check-result dialog from craft-update by @aiongg in https://github.com/storytold/soundcraft/pull/1219
  * docs: tidy agent, contributor and release docs by @aiongg in https://github.com/storytold/soundcraft/pull/1218
  * Clip menu: act on the whole edit selection when the right-clicked clip is part of it by @ydking0911 in https://github.com/storytold/soundcraft/pull/1196
  * Zoom Toggle: Alt+Shift+E and Alt-click cancel it and keep the current view by @dotCooCoo in https://github.com/storytold/soundcraft/pull/1101

## 0.15.0

- Upstream soundcraft v0.15.0.
  ## What's Changed
  * Grant network access so update checks can reach GitHub by @dylanmtaylor in https://github.com/storytold/soundcraft/pull/1163
  * Honor CARGO_BUILD_TARGET in FreeBSD packaging for aarch64 release builds by @dylanmtaylor in https://github.com/storytold/soundcraft/pull/1167
  * Reject out-of-range timecode fields so Min:Secs strings fall through by @Silvertree2010 in https://github.com/storytold/soundcraft/pull/1024
  * Count only audio-track clips in session text export by @Silvertree2010 in https://github.com/storytold/soundcraft/pull/1025

## 0.14.0

- Upstream soundcraft v0.14.0.
  ## What's Changed
  * Migrate SoundCraft to egui 0.37 by @skylartaylor in https://github.com/storytold/soundcraft/pull/1063
  * Follow-ups to #1058: Auto Pan keeps the stereo image, paste and Strip Silence fixes by @aiongg in https://github.com/storytold/soundcraft/pull/1060
  * Meters: keep every block's peak until the UI takes it by @forwardaudio in https://github.com/storytold/soundcraft/pull/1045
  * feat: startup update prompt modal and direct self-updating by @dylanmtaylor in https://github.com/storytold/soundcraft/pull/1073

## 0.13.1

- Upstream soundcraft v0.13.1.
  ## What's Changed
  * chore(deps): bump the cargo-dependencies group across 1 directory with 11 updates by @dependabot[bot] in https://github.com/storytold/soundcraft/pull/767
  * style: normalize the prores DCT basis literals (SonarCloud rust:S7454) by @dylanmtaylor in https://github.com/storytold/soundcraft/pull/962
  * style: use [[ ]], named locals and explicit returns in packaging scripts (SonarCloud S7688/S7679/S7682) by @dylanmtaylor in https://github.com/storytold/soundcraft/pull/950
  * Use chrono instead of hand-rolled UTC conversion by @dylanmtaylor in https://github.com/storytold/soundcraft/pull/844

## 0.12.1

- Upstream soundcraft v0.12.1.
  > [!NOTE]
  > **FreeBSD aarch64 is dropped from this release.** We had to cut releases, and we cannot build this target within the 5-hour CI limit for any of the Craft apps, so we are dropping it for now. This is a known blocker, flagged for a future fix in storytold/craft-common-actions#39. FreeBSD x86_64 and every other platform are included.
  ## What's Changed
  * security: enforce HTTPS on curl downloads (SonarCloud S6506) by @dylanmtaylor in https://github.com/storytold/soundcraft/pull/860
  * README: the app icon above the title by @bflatastic in https://github.com/storytold/soundcraft/pull/934

## 0.11.0

- Upstream soundcraft v0.11.0.
  > [!NOTE]
  > **FreeBSD aarch64 is dropped from this release.** We had to cut releases, and we cannot build this target within the 5-hour CI limit for any of the Craft apps, so we are dropping it for now. This is a known blocker, flagged for a future fix in storytold/craft-common-actions#39. FreeBSD x86_64 and every other platform are included.
  ## What's Changed
  * Skip CodeRabbit reviews on Dependabot PRs by @echelon in https://github.com/storytold/soundcraft/pull/821
  **Full Changelog**: https://github.com/storytold/soundcraft/compare/v0.10.0...v0.11.0

## 0.8.1

- Upstream soundcraft v0.8.0.
  ## What's Changed
  * Engine tests compile on Windows: borrow the path in the case test, allow the unix-only mut by @Testsubject1337 in https://github.com/storytold/soundcraft/pull/571
  * ci: add nix check workflow by @dylanmtaylor in https://github.com/storytold/soundcraft/pull/649
  * Follow stored paths only on this computer or the session's share by @skylartaylor in https://github.com/storytold/soundcraft/pull/726
  ## New Contributors

## 0.8.0

- Upstream soundcraft v0.8.0.
  ## What's Changed
  * Engine tests compile on Windows: borrow the path in the case test, allow the unix-only mut by @Testsubject1337 in https://github.com/storytold/soundcraft/pull/571
  * ci: add nix check workflow by @dylanmtaylor in https://github.com/storytold/soundcraft/pull/649
  * Follow stored paths only on this computer or the session's share by @skylartaylor in https://github.com/storytold/soundcraft/pull/726
  ## New Contributors

## 0.7.1

- Upstream soundcraft v0.7.0.
  ## What's Changed
  * fix(dsp): a huge finite sample rate is no pitch, not an overflow (#241) by @marcoarnulfo in https://github.com/storytold/soundcraft/pull/269
  * fix(track): vca: null clears a VCA assignment, as documented (#237) by @marcoarnulfo in https://github.com/storytold/soundcraft/pull/270
  * fix(control): apply the engine.commands filter to UI commands (#255) by @marcoarnulfo in https://github.com/storytold/soundcraft/pull/271
  * fix(window): keep the MIDI Editor open on Bring to Front (#254) by @marcoarnulfo in https://github.com/storytold/soundcraft/pull/272

## 0.7.0

- Upstream soundcraft v0.7.0.
  ## What's Changed
  * fix(dsp): a huge finite sample rate is no pitch, not an overflow (#241) by @marcoarnulfo in https://github.com/storytold/soundcraft/pull/269
  * fix(track): vca: null clears a VCA assignment, as documented (#237) by @marcoarnulfo in https://github.com/storytold/soundcraft/pull/270
  * fix(control): apply the engine.commands filter to UI commands (#255) by @marcoarnulfo in https://github.com/storytold/soundcraft/pull/271
  * fix(window): keep the MIDI Editor open on Bring to Front (#254) by @marcoarnulfo in https://github.com/storytold/soundcraft/pull/272

## 0.6.2

- Upstream soundcraft v0.6.1.
  ## What's Changed
  * Add shared docking with SoundCraft theme and MIDI sizing by @skylartaylor in https://github.com/storytold/soundcraft/pull/167
  * ci: refactor workflows to use storytold/craft-common-actions by @dylanmtaylor in https://github.com/storytold/soundcraft/pull/184
  * Keep each consolidation in its own audio file. by @vbasky in https://github.com/storytold/soundcraft/pull/123
  * Save a session copy without retargeting the open document. by @vbasky in https://github.com/storytold/soundcraft/pull/124

## 0.6.1

- Upstream soundcraft v0.6.1.
  ## What's Changed
  * Add shared docking with SoundCraft theme and MIDI sizing by @skylartaylor in https://github.com/storytold/soundcraft/pull/167
  * ci: refactor workflows to use storytold/craft-common-actions by @dylanmtaylor in https://github.com/storytold/soundcraft/pull/184
  * Keep each consolidation in its own audio file. by @vbasky in https://github.com/storytold/soundcraft/pull/123
  * Save a session copy without retargeting the open document. by @vbasky in https://github.com/storytold/soundcraft/pull/124

## 0.5.1

- Upstream soundcraft v0.5.0.
  ## What's Changed
  * Nudge clip gain with Ctrl+Shift+Up/Down on Windows and Linux too by @grovebit in https://github.com/storytold/soundcraft/pull/105
  * Keep the MIDI editor dock at its resized height by @reoniazx in https://github.com/storytold/soundcraft/pull/113
  * Stop playback when replacing the session by @reoniazx in https://github.com/storytold/soundcraft/pull/110
  * fix(ui): follow live system appearance changes by @coygeek in https://github.com/storytold/soundcraft/pull/85

## 0.5.0

- Upstream soundcraft v0.5.0.
  ## What's Changed
  * Nudge clip gain with Ctrl+Shift+Up/Down on Windows and Linux too by @grovebit in https://github.com/storytold/soundcraft/pull/105
  * Keep the MIDI editor dock at its resized height by @reoniazx in https://github.com/storytold/soundcraft/pull/113
  * Stop playback when replacing the session by @reoniazx in https://github.com/storytold/soundcraft/pull/110
  * fix(ui): follow live system appearance changes by @coygeek in https://github.com/storytold/soundcraft/pull/85

## 0.4.0

- Initial BifrOSt App Store package of upstream soundcraft v0.4.0.
