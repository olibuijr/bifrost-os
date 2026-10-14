# Changelog

## 1.12.3 — 2026-10-14

- Name the one-second playback-service wake interval explicitly.

## 1.12.2 — 2026-10-14

- Let the source-list sidebar shrink with compact windows while retaining its desktop width.

## 1.12.1 — 2026-10-14

- Use shared spacing values in the episode list, transcript rows, chapter indicator, and notes view.

## 1.12.0 — 2026-10-14

- Episode menu: Mark All as Played (or Mark All as Unplayed, when every episode of the show is played) marks every
  episode of the show on screen in one step. It changes only the episodes whose flag differs and keeps the flags of
  other shows. The item is off on the New Episodes, Up Next and other lists that are not a single show.

## 1.11.0 — 2026-10-14

- Parity records: the Podcasts rows for downloads, Up Next, refresh, search, New Episodes, notes, chapters, show and
  app settings, OPML, transcripts and menus now cite the code that does the work (feed refresh, the download queue,
  Up Next moves, the search, the settings choices, the OPML and transcript parsers, and the main menu). No behaviour
  change in this release.

## 1.10.0 — 2026-10-14

- Up Next can be reordered by hand: press and hold an episode for half a second, then drag it to a new place. The
  list keeps its order when the app restarts. A drag that starts as a quick swipe still scrolls the list.
- Episode notes keep links: a link shows its text followed by its address in parentheses, unless the text is the
  address itself.

## 1.8.0 — 2026-10-13

- Library: the sidebar has Search, New Episodes (unplayed episodes of every show, newest first), Up Next, Downloaded,
  Shows and Settings. Unplayed episodes carry a dot; played ones are dimmed. Mark as Played or Unplayed from the Episode
  menu, the context menu or the episode page; an episode that finishes is marked played.
- Up Next: Play Next and Play Last put an episode in the queue; the queue plays on when an episode ends and Next Episode
  skips ahead. Move episodes up or down and clear the queue from the Episode menu.
- Episode page (Show Episode Details): notes as plain text, chapters, and buttons for Play, Download, Play Next and
  Mark Played. The Now Playing sheet gains Chapters (tap one to jump) and Transcript panels; transcripts come from the
  feed's transcript link (WebVTT, SubRip or plain text) and follow the playing position.
- Downloads: Download and Remove Download for any episode, a Downloaded list, and a download queue.
- Per-show settings (the toolbar button on a show): download new episodes automatically, how many downloads to keep, and
  the episode order. App Settings: skip back and forward intervals (10 to 60 seconds) and the feed refresh period.
- Search finds shows in an open podcast directory and follows them. File > Import Subscriptions and Export Subscriptions
  read and write OPML. View > Refresh (and a toolbar button) updates every show; feeds also refresh on the chosen period.
- Menus: File, View, Controls (play, skip, next, speed, sleep timer) and Episode, with key equivalents. Arrow keys move
  the selection, Return plays, a second click on an episode plays it, Escape leaves a panel.
- The app now asks for the documents permission (the import and export panels).

## 1.7.2 — 2026-10-12

- Rebuilt on the BifrOSt 0.6.4 SDK: the shared shell/app wire codec, frame and list caps, and the window materials API.

## 1.7.1 — 2026-10-12

- The feed refresh status line names the feeds that failed while a batch is still running, for example "Updating feeds,
  1 of 3, 1 failed". The failure count was tracked but never shown.

## 1.7.0 — 2026-10-12

- The Speed button in the Now Playing sheet opens a menu with the rates 0.5x, 0.75x, 1x, 1.25x, 1.5x, 1.75x and 2x.
  The current rate is highlighted. Pick a rate to set it, or tap Speed again to close the menu. The rate is kept for the
  session; the audio engine still plays at normal speed until the audio service can change speed.
- Back skips are 15 seconds everywhere: the mini player's back button now reads "-15s", the Left arrow key and the
  system's skip-back command also move 15 seconds. Forward skips stay at 30 seconds.
- Album art and per-episode pictures are not in this release. Podcasts does not read pictures from feeds yet.

## 1.6.0 — 2026-10-12

- The Now Playing sheet shows a line about the episode under its show name: the kind of audio, the codec (MP3, AAC,
  Opus, FLAC or Linear PCM, read from the file name), the length and the release date. The length is the one the
  audio service reports once it knows it, and the length from the feed until then.
- Chapter navigation and episode artwork are not in this release. Podcasts does not read chapter lists or per-episode
  pictures from feeds yet.

## 1.5.1 — 2026-10-12

- Dark appearance: the main content area fills with the dark content grey instead of pure black, as stock dark content views do; sidebars and cards keep their colours.

## 1.5.0

- While feeds load, the list says how many are done, for example "Updating feeds, 1 of 3", instead of only
  "Loading...". The count resets for each new batch, and a feed that fails to load is counted as finished.
  The count comes through the shared async publisher sequence, which the app pulls on each frame.
- Trimming an episode and flashing-light dimming are not in this release. Podcasts plays audio only, and the
  audio service has no call to cut a file.

## 1.4.0

- Skips stop at the ends of an episode: a back skip never goes before the start, and a forward skip stops at the end
  once the length is known. The skip buttons use the shared skip-interval rules.
- The speed button steps through 0.5x to 2x and wraps back to 0.5x after 2x. The chosen speed is shown; the engine
  still plays at normal speed until the audio service takes a rate.
- Keyboard transport: Space plays or pauses, and the Left and Right arrows skip back and forward 30 seconds, when no
  text field has focus.
- Control Center's Now Playing tile shows the episode's title, its show and its length while an episode plays or is
  paused. Its taps (play or pause, stop, skip back and forward, and seek) reach Podcasts. The tile is updated when playback
  starts, pauses, resumes or ends, not every second.
- The Dock icon shows a bar while an episode downloads: a sweep until the size is known, then the percent done. When a
  download finishes while Podcasts is in the background, the icon bounces once.

## 1.3.3 — 2026-10-11

- Compliance: feed and download results reach the app loop through a shared inbox filled on the main queue, instead of
  an mpsc channel the loop polled. The Now Playing artwork is sized from the window height, the empty-state text fills
  the list area, and the feed URL field is a capsule. The app icon uses the named system purple and white.

## 1.3.2 — 2026-10-10

- Snow Leopard: feeds, episode downloads and the playback clock no longer use raw threads or polling for results.
  Work runs on the dispatch pool (Utility) and reaches the app loop through the main queue, so the loop wakes when a
  result arrives. The app enables sudden termination: its subscriptions and resume positions are written as they change.

## 1.3.1

- Follows a Light/Dark switch immediately, including while the window is in the background.

## 1.3.0 — 2026-10-10

- Golden Gate layout: a full-size window with a source-list sidebar (Shows and one row per subscription), a unified
  toolbar (title, Add Podcast, Remove Show) and a floating mini player; the iPad back button, large title and bottom
  bar are gone. Light and Dark both follow the system appearance. Needs BifrOSt 0.4.84.

## 1.2.3 — 2026-10-10

- Opens again on BifrOSt 0.4.90 and later: rebuilt against the current SDK, whose app socket is `BIFROST_SOCK`.

## 1.2.2 — 2026-10-09

- Spacing follows the system 4-pt grid: 9 padding and gap values moved to the nearest standard step (at most 2 pt).
- 1 colour now uses the system colour token (same values).
- Built for system 0.4.62.

## 1.2.1 — 2026-10-09

- Text that is shortened to fit now measures at the size it is drawn at, so labels are no longer cut off early.
- Follows the system accessibility settings (Increase Contrast, Reduce Transparency, Show Borders, Bold Text) on system 0.4.61.

## 1.2.0 — 2026-10-09

- Text sizes and spacing come from the shared macOS-style scale, so this app matches the others.
- No gap above the content for a status bar the desktop does not have.

## 1.1.0 — 2026-10-08

- The show list has a large "Podcasts" title under the bar, as Books does. Episodes keep their inline title with Back.

## 1.0.0 — 2026-10-08

- Podcasts for the BifrOSt desktop: the tablet app's code, hosted in a window with keyboard and pointer input.

- Use the desktop package identity for the SDK handshake and supported PC permissions.
- Connect to the Linux audio service path.
- Keep text-field focus without drawing an on-screen keyboard in PC app processes.
- Route physical text, Backspace and Return keys to existing field editors.

## 1.9.0 — 2026-10-13

- Chapters: when a feed links to a chapters file (the `podcast:chapters` JSON format) instead of listing the marks
  itself, Podcasts reads it when the episode plays or its page opens. Chapters marked as hidden are left out.
- Transcript: tap a timed line to jump to it.
