## 0.5.1

- Opens as a native Wayland window (from the Dock, Spotlight and the App Store).

# Changelog

## 0.19.0

- Upstream filmcraft v0.19.0.
  ## What's Changed
  * Effect Controls: Scale Width is greyed out while Uniform Scale is on (#4381) by @ZIDAN44 in https://github.com/storytold/filmcraft/pull/4407
  * filmcraft-web: window.filmcraft takes its request functions from craft-web-api by @helenkwok in https://github.com/storytold/filmcraft/pull/4420
  * #65: CommandSpec carries which target keys a command takes by name by @vangel76 in https://github.com/storytold/filmcraft/pull/1413
  * audio-dsp: narrow the public API to what other crates use by @vangel76 in https://github.com/storytold/filmcraft/pull/3147

## 0.18.0

- Upstream filmcraft v0.18.0.
  ## What's Changed
  * web: check MP4 sample extent before allocating its buffer by @dylanmtaylor in https://github.com/storytold/filmcraft/pull/4102
  * avi: hold child chunks to their container so lists cannot overlap by @dylanmtaylor in https://github.com/storytold/filmcraft/pull/4106
  * security(dnx): cap decoded frame memory and reject repeated scan line indices by @dylanmtaylor in https://github.com/storytold/filmcraft/pull/4101
  * security(apv): size the auxiliary alpha frame only after its tiles decode by @dylanmtaylor in https://github.com/storytold/filmcraft/pull/4109

## 0.17.1

- Upstream filmcraft v0.17.1.
  **Full Changelog**: https://github.com/storytold/filmcraft/compare/v0.17.0...v0.17.1

## 0.17.0

- Upstream filmcraft v0.17.0.
  ## What's Changed
  * Ignore leading byte order mark in SRT parser by @Silvertree2010 in https://github.com/storytold/filmcraft/pull/3781
  * Read sub-dollar amounts without a zero major unit in TTS by @Silvertree2010 in https://github.com/storytold/filmcraft/pull/3783
  * i18n(sv): translate 49 new strings by @kler in https://github.com/storytold/filmcraft/pull/3784
  * Reject zero, negative and non-finite FPS in ALE import by @Silvertree2010 in https://github.com/storytold/filmcraft/pull/3790

## 0.16.0

- Upstream filmcraft v0.16.0.
  ## What's Changed
  * Updates: shared check-result dialog from craft-update by @aiongg in https://github.com/storytold/filmcraft/pull/3756
  * docs: tidy agent, contributor and asset docs by @aiongg in https://github.com/storytold/filmcraft/pull/3755
  * Integration: backlog round r1 (244 PRs) by @aiongg in https://github.com/storytold/filmcraft/pull/3746
  * Keep a clip's transitions when copying and pasting it by @Silvertree2010 in https://github.com/storytold/filmcraft/pull/3473

## 0.15.0

- Upstream filmcraft v0.15.0.
  ## What's Changed
  * Grant network access so update checks can reach GitHub by @dylanmtaylor in https://github.com/storytold/filmcraft/pull/3527
  * Honor CARGO_BUILD_TARGET in FreeBSD packaging for aarch64 release builds by @dylanmtaylor in https://github.com/storytold/filmcraft/pull/3534
  * CI: workflows only call craft-common-actions at @main (AGENTS.md; windows-arm64 unpinned) by @dylanmtaylor in https://github.com/storytold/filmcraft/pull/3645
  * Project panel: search finds names holding every typed word by @punkhop in https://github.com/storytold/filmcraft/pull/3327

## 0.14.0

- Upstream filmcraft v0.14.0.
  ## What's Changed
  * Keep draft-decoder counter assertions in one test by @lisn0 in https://github.com/storytold/filmcraft/pull/3286
  * Report parameter and accepted value from effects.setParam by @lisn0 in https://github.com/storytold/filmcraft/pull/3281
  * Frame workers: a replaced frame server's workers end and release its cache and media pool by @dotCooCoo in https://github.com/storytold/filmcraft/pull/3244
  * codecs: the rayon deadlock test allows 120 s between two answered frame requests by @dotCooCoo in https://github.com/storytold/filmcraft/pull/3241

## 0.13.0

- Upstream filmcraft v0.13.0.
  ## What's Changed
  * i18n test: leave installed right-to-left faces out of the Japanese check by @FredApps in https://github.com/storytold/filmcraft/pull/2576
  * CI: run cargo xtask ci on pull requests and pushes to main by @FredApps in https://github.com/storytold/filmcraft/pull/2563
  * M8: Fujifilm F-Log, F-Log2 and F-Log2 C colour spaces (F-Gamut / F-Gamut C) by @melchiaaronzedek in https://github.com/storytold/filmcraft/pull/2819
  * Require session-token authentication for desktop control requests by @coderabbitai[bot] in https://github.com/storytold/filmcraft/pull/2764

## 0.12.0

- Upstream filmcraft v0.12.0.
  > [!NOTE]
  > **FreeBSD aarch64 is dropped from this release.** We had to cut releases, and we cannot build this target within the 5-hour CI limit for any of the Craft apps, so we are dropping it for now. This is a known blocker, flagged for a future fix in storytold/craft-common-actions#39. FreeBSD x86_64 and every other platform are included.
  ## What's Changed
  * Reject TTML offset times that have no integer digits by @Silvertree2010 in https://github.com/storytold/filmcraft/pull/2753
  * Speak the minus sign of negative currency amounts in TTS normalizer by @Silvertree2010 in https://github.com/storytold/filmcraft/pull/2756

## 0.11.0

- Upstream filmcraft v0.11.0.
  > [!NOTE]
  > **FreeBSD aarch64 is dropped from this release.** We had to cut releases, and we cannot build this target within the 5-hour CI limit for any of the Craft apps, so we are dropping it for now. This is a known blocker, flagged for a future fix in storytold/craft-common-actions#39. FreeBSD x86_64 and every other platform are included.
  ## What's Changed
  * docs: add SonarCloud and CodeRabbit badges to README by @dylanmtaylor in https://github.com/storytold/filmcraft/pull/2368
  * Skip CodeRabbit reviews on Dependabot PRs by @echelon in https://github.com/storytold/filmcraft/pull/2594

## 0.10.0

- Upstream filmcraft v0.10.0.
  > [!NOTE]
  > **FreeBSD aarch64 is dropped from this release.** We had to cut releases, and we cannot build this target within the 5-hour CI limit for any of the Craft apps, so we are dropping it for now. This is a known blocker, flagged for a future fix in storytold/craft-common-actions#39. FreeBSD x86_64 and every other platform are included.
  ## What's Changed
  * Skip GPU goldens when Rgba32Float is not renderable by @vbasky in https://github.com/storytold/filmcraft/pull/1315
  * [agent] Dialog buttons show the pointing hand on hover by @alexng353 in https://github.com/storytold/filmcraft/pull/1777

## 0.9.0

- Upstream filmcraft v0.9.0.
  ## What's Changed
  * fix: preserve individual linked-clip selections through deletion by @haranishi in https://github.com/storytold/filmcraft/pull/1639
  * fix(interchange): avoid UTF-8 slice panics in file URLs by @nuclearspike in https://github.com/storytold/filmcraft/pull/1609
  * fix(media): preserve Unicode prefixes in image-sequence filenames by @nuclearspike in https://github.com/storytold/filmcraft/pull/1570
  * fix(export): prevent nested renderer-pool deadlocks by @earlvanze in https://github.com/storytold/filmcraft/pull/861

## 0.8.1

- Upstream filmcraft v0.8.0.
  ## What's Changed
  * Captions: preserve fractional TTML tick offsets by @webdevtodayjason in https://github.com/storytold/filmcraft/pull/1223
  * Export: 32-bit integer and 32-bit float audio by @JordyUSA in https://github.com/storytold/filmcraft/pull/1245
  * fix(apv): skip oracle tests when ffmpeg predates APV support by @AkashBhat14 in https://github.com/storytold/filmcraft/pull/1257
  * test(i18n): skip the Japanese language step when no Japanese font is available (#1251) by @MarekSpodBiedry in https://github.com/storytold/filmcraft/pull/1259

## 0.8.0

- Upstream filmcraft v0.8.0.
  ## What's Changed
  * Captions: preserve fractional TTML tick offsets by @webdevtodayjason in https://github.com/storytold/filmcraft/pull/1223
  * Export: 32-bit integer and 32-bit float audio by @JordyUSA in https://github.com/storytold/filmcraft/pull/1245
  * fix(apv): skip oracle tests when ffmpeg predates APV support by @AkashBhat14 in https://github.com/storytold/filmcraft/pull/1257
  * test(i18n): skip the Japanese language step when no Japanese font is available (#1251) by @MarekSpodBiedry in https://github.com/storytold/filmcraft/pull/1259

## 0.7.3

- Upstream filmcraft v0.7.2.
  ## What's Changed
  * Share docking transactions and preserve FilmCraft panel behavior by @skylartaylor in https://github.com/storytold/filmcraft/pull/881
  * ci: refactor workflows to use storytold/craft-common-actions by @dylanmtaylor in https://github.com/storytold/filmcraft/pull/1051
  * #853: Shift constrains pen mask segments to 45° steps by @vangel76 in https://github.com/storytold/filmcraft/pull/862
  * Reveal in Finder for Project clips with a local path by @vbasky in https://github.com/storytold/filmcraft/pull/889

## 0.7.2

- Upstream filmcraft v0.7.2.
  ## What's Changed
  * Share docking transactions and preserve FilmCraft panel behavior by @skylartaylor in https://github.com/storytold/filmcraft/pull/881
  * ci: refactor workflows to use storytold/craft-common-actions by @dylanmtaylor in https://github.com/storytold/filmcraft/pull/1051
  * #853: Shift constrains pen mask segments to 45° steps by @vangel76 in https://github.com/storytold/filmcraft/pull/862
  * Reveal in Finder for Project clips with a local path by @vbasky in https://github.com/storytold/filmcraft/pull/889

## 0.6.1

- Upstream filmcraft v0.6.0.
  ## What's Changed
  * CLI: a closed stderr no longer panics with exit 101 (#537) by @MarekSpodBiedry in https://github.com/storytold/filmcraft/pull/567
  * CLI export: forward a lone --start or --end so the engine reports the missing bound (#538) by @MarekSpodBiedry in https://github.com/storytold/filmcraft/pull/568
  * Roll: reversed clips take the cut from the opposite source end (#526) by @MarekSpodBiedry in https://github.com/storytold/filmcraft/pull/558
  * fix(edit): reversed trims use the media handle their playback direction consumes (#527) by @MarekSpodBiedry in https://github.com/storytold/filmcraft/pull/559

## 0.6.0

- Upstream filmcraft v0.6.0.
  ## What's Changed
  * CLI: a closed stderr no longer panics with exit 101 (#537) by @MarekSpodBiedry in https://github.com/storytold/filmcraft/pull/567
  * CLI export: forward a lone --start or --end so the engine reports the missing bound (#538) by @MarekSpodBiedry in https://github.com/storytold/filmcraft/pull/568
  * Roll: reversed clips take the cut from the opposite source end (#526) by @MarekSpodBiedry in https://github.com/storytold/filmcraft/pull/558
  * fix(edit): reversed trims use the media handle their playback direction consumes (#527) by @MarekSpodBiedry in https://github.com/storytold/filmcraft/pull/559

## 0.5.0

- Initial BifrOSt App Store package of upstream filmcraft v0.5.0.
