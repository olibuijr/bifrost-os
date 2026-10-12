# Parity wire-up round 4: Podcasts (media asset info)

Branch `wire4/podcasts` from `main` (0.5.40). App `springboard/apps/podcasts-desktop/`, now version 1.6.0 (manifest
and Cargo.toml; the earlier notes said 1.4.0 and 1.5.0, which are out of date). Nothing was built or run in this round:
the lead builds once on the host, then the app goes to Midget.

## What is now wired

1. **Episode info line in the Now Playing sheet.** Under the show name, one footnote line reads, for example,
   "Audio, MP3, 45:25, 05 Oct 2026". It has four parts:
   - the media type name, "Audio", from `bifrost_app::controls::media_type_names::type_name("soun", "en")`;
   - the codec name, such as "MP3", "AAC", "Opus", "FLAC" or "Linear PCM", from `subtype_name("soun", code)`, where the
     four-character code is read from the episode URL's file extension (`info::codec_code`);
   - the length: the audio service's length once it is known (`duration_ms`), else the length the feed gave;
   - the release date from the feed.
   Empty parts are left out. Pure logic is in the new `src/info.rs` (`codec_code`, `codec_name`, `summary`).
2. The sheet's draw code (`src/podcasts.rs` `draw_sheet`) finds the episode's date and feed length with the URL, so the
   line needs no new state.

## Not wired (gaps, with reasons)

- **Chapter navigation.** No chapter model exists in `springboard/ui/src` (no chapter list, no current-chapter lookup),
  and the feed parser (`src/feed.rs`) reads only the title, enclosure URL, date and duration. Chapters in podcast feeds
  come from a separate JSON file, so a fetch and parser are needed first. Not done in this round.
- **Episode artwork.** `feed.rs` does not read per-episode or per-show `itunes:image`, so the sheet still draws the
  app's own mark. The media-asset metadata model is not reachable from an app, so no artwork comes from an asset.
- **Codec and track data from the file itself.** The app does not read the container (tracks, sample rate, bit rate).
  Only the extension is used, so a file with a misleading extension shows the wrong codec. The audio service has no
  track-info call for apps.
- **Media asset object** (`AVAsset`-style tracks and metadata for apps): the shell's demux and tag readers are inside
  the shell crate (`springboard/src/media/`), and the audio service has no asset call. Not wired.
- The Info line is one line. The Info pane the focus asked for as a separate pane is not built; the sheet has no room
  for it at the current height.
- Chapter and artwork display in the Control Center tile (`src/media.rs`) are unchanged.

## Where the code is

- `src/info.rs` (new): `codec_code`, `codec_name`, `summary`, with unit tests worked out by hand (for example 2725 s
  shows as "45:25"; a NaN length with no feed length shows only "Audio").
- `src/main.rs`: `mod info;`.
- `src/podcasts.rs`: `use crate::info;`; `draw_sheet` finds (show, title, date, feed length) and draws the info line at
  `a.1 + a.3 + ui.pt(58.0)`, footnote size, `LABEL_2`, centred in the content column. Colours are the existing named
  constants; the content area stays `CONTENT_BG`. No `DETAIL_BG` is used.
- `Cargo.toml`, `manifest.toml`: 1.5.1 to 1.6.0. `CHANGELOG.md`: 1.6.0 entry at the top.

## How a user reaches it

1. Open Podcasts, add or open a show, and tap an episode so it plays.
2. Tap the player area to open the Now Playing sheet.
3. The line under the show name shows the type, codec, length and date.

## How to verify on the test PC (Midget)

Midget is the only test machine (the VM is skipped by request).

1. `bifrost pc publish`, then `bifrost pc midget ota` until `phase=accepted`. The app version should read 1.6.0.
2. `bifrost pc midget open-app is.olibuijr.podcasts.desktop`, then play an `.mp3` episode (a `.m4a` one shows "AAC").
3. Open the sheet with `bifrost pc midget tap-window Podcasts X Y` on the player area, then
   `bifrost pc midget screen /tmp/podcasts-info.png`. Check that the line reads "Audio, MP3, m:ss, <date>", that its
   length matches the clock once playback has started, and that it does not overlap the scrubber knob (the line sits
   at 58 pt below the artwork's bottom edge, the knob near 70 pt). Adjust the 58 pt in `podcasts.rs` if it does.
4. Check a long show title in a narrow window: the line should be cut to the column width, not wrap or overflow.

Host tests (for the lead): `cargo test --offline -p bifrost-podcasts info` in `springboard/`, then the full crate
tests (`cargo test --offline -p bifrost-podcasts`), including the compliance snapshot, which should be unchanged (the
sheet is not in the default snapshot). The root `springboard/Cargo.lock` version line for `bifrost-podcasts` updates
on the lead's build; it was not edited here.

## REA and official evidence

- Official DocC read: `python3 -I tools/parity/docc.py avfoundation/avasset --depth 1` lists AVAsset's `duration`,
  `tracks`, `metadata`, `commonMetadata`, `creationDate` and `isPlayable` (page
  https://developer.apple.com/documentation/avfoundation/avasset).
- REA rendered page: `ev_9c9440aa7e48cb95dfb2c694a2c44c21619d0d564b17b409f07927be35e3ad07` (AVAsset, the same page;
  it shows the asset as a container of tracks with metadata, and tracks as typed media).
- The type and codec names come from the shared table in `springboard/ui/src/media_type_names.rs`, whose rows are the
  `media-toolbox` matrix row; the official page for those names was not re-read in this round.

## Matrix rows

| Slug | Feature (as written in data/<slug>.json) | BifrOSt path | State |
|---|---|---|---|
| avfoundation-playback-editing | Media assets (AVAsset, tracks, metadata) | springboard/apps/podcasts-desktop/src/info.rs (duration and codec from the feed and the file extension) | wired in Podcasts for the info line only; still partial (no asset object, no chapters, no artwork) |
| media-toolbox | Localized media type and subtype names | springboard/ui/src/media_type_names.rs | wired in Podcasts (type and codec names in the sheet info line) |

## Remaining gaps

- Chapter list from the feed (needs a feed-chapters fetch and parser), and chapter navigation in the player.
- Episode and show artwork from the feed (`itunes:image`).
- Track and codec data read from the file (needs a container reader the app can call).
- A separate Info pane (the sheet has no room at this height).
- Audio settings (sample rate, bit rate) are not shown.
