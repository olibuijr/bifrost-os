# Wiring Up Next clearing into Music (1.5.0)

Branch `wire5/music`, from `main` (release 0.5.42). Scope: `springboard/apps/music-desktop/` plus this note. No builds
were run in this round: the lead builds once. `Cargo.lock` still names `bifrost-music` 1.4.1 (lead updates it).

Earlier notes: `docs/parity/wire-app-music.md` (Now Playing session, remote commands) and `docs/parity/wire2-music.md`
(library change observation, load options, MIDI). Those stay as they are.

## What was already wired (checked in the code, not changed here)

- **Play next / play later and the Up Next list** (`music/upnext.rs` `UpNext`, `music.rs` `queue_add`, the song menu's
  Play Next and Play Last, reorder with `QUp`, remove with `QDel`). The queue page is `Page::Queue`.
- **Repeat and shuffle** (`nowplaying::Target::Shuffle` and `Target::Repeat` in the Now Playing sheet; `Music::control`
  sets the engine's repeat through `engine.rs` `set_repeat`; shuffle is `UpNext::set_shuffle`, done in the app).
- **Now Playing publishing and remote commands** (`music/nowplay.rs`, `Music::remote_command`), as in 1.3.0.

## What is now wired

1. **Clear Up Next.** `UpNext::clear_upcoming` takes every upcoming song out of the queue. The current song and what has
   played stay. A shuffled queue removes the same songs from its saved played order, so turning shuffle off restores
   the right list. `Music::queue_clear` then hands the queue to the engine (`push_queue`) at the current position and
   playing state, so the song that is playing keeps going.
   - The queue page shows a text row, "Clear Up Next" (tint colour `PINK`, the link colour), after the last waiting
     song. It appears only when something is waiting. Its hit target is `Target::QClear`, which is also in the
     scrolling set (`scrolls`), and its height is counted in `content_h`.

## How a user reaches it

- Play a song from a list, then open the Now Playing sheet and tap the queue button to open Up Next (the queue page).
- Add songs with the "..." menu (Play Next, Play Last). Scroll to the bottom of Up Next and tap "Clear Up Next".

## How to verify on the test PC (Midget)

1. `bifrost pc midget open-app is.olibuijr.music.desktop`, then play an album from its album page with Play.
2. Open the Now Playing sheet, then the queue. Confirm the rows after the current song and the "Clear Up Next" row
   under them.
3. Tap "Clear Up Next" (`bifrost pc midget tap-window is.olibuijr.music.desktop X Y`, with X and Y at the row's centre,
   window-local pixels). Expected: the list shows only the current song, the playing song keeps going from the same
   position, and the "Clear Up Next" row is gone.
4. Shuffle on, then Clear Up Next, then Shuffle off. Expected: the remaining song is the one playing, and the queue
   keeps its order.
5. With nothing waiting, the row is not drawn and a tap on that spot does nothing.

## REA evidence

- `ev_8ad60442956832c3427f9425132d74310f4b07e1af50fca60c441d8e9ca2c3df`: MPMusicPlayerControllerMutableQueue page
  (methods `insert(_:after:)` and `remove(_:)`, "A mutable queue containing the media items to play"). Taken in this
  round with `tools/parity/rea-page.sh`. Official DocC read with `python3 -I tools/parity/docc.py
  mediaplayer/mpmusicplayercontrollermutablequeue`.
- Clear Up Next has no direct Apple method. The page shows only per-item removal (`remove(_:)`), so the clear is our
  own action built from the same removal (the queue is replaced with the current song).

## Tests

Unit tests in `src/music/upnext.rs` (run by the lead with `cargo test -p bifrost-music`), worked out by hand:

- `clear_up_next_keeps_the_current_song_and_what_played`: a queue of a, b, c, d, e with b current. Clearing takes 3
  songs (c, d, e); the queue is a, b with current 1 (b); a second clear takes 0.
- `clear_up_next_with_nothing_upcoming_changes_nothing`: an empty queue and a queue whose current song is the last one
  both clear 0 songs and keep their items.
- `clear_up_next_while_shuffled_restores_the_played_order_when_shuffle_goes_off`: shuffle with seed 7 on a..e with b
  current, clear (3 removed), shuffle off gives a, b with current 1.

The queue page and its hit test are in `music.rs` and are not unit tested (the shell is not run in unit tests).

## Remaining gaps

1. **Library queries by Artists, Albums and Genres.** `springboard/ui/src/media_query.rs` (`Query`, `Grouping::Genre`)
   is not used by the app. The app's browse (`music/browse.rs`) groups songs itself and has no Genres tab. Genres need
   the scan to read the genre tag, which `music/library.rs` does not do yet. Closing this needs the tag reader and a
   Genres tab, a larger change than this round.
2. **Clear from the song menu.** Clear Up Next is on the queue page only. A menu item would touch the menu list (the
   compliance test checks menu rules), so it was left out.
3. **Reorder by drag.** Reorder is still the up-arrow button (`QUp`), as before.
4. **Now Playing info to the system lock card.** Unchanged from 1.4.1 (see `wire-app-music.md` gap 2).

## Matrix rows this touches

Paths are relative to the repo. Feature strings are exactly as in `docs/parity-matrix/data/<slug>.json`. Status stays
`partial`: none is complete on Midget yet.

| Slug | Feature (as in the matrix) | BifrOSt path |
|---|---|---|
| media-player | Player queues | springboard/apps/music-desktop/src/music/upnext.rs (clear_upcoming); springboard/apps/music-desktop/src/music.rs (queue_clear, Target::QClear, draw_queue) |
| media-player | Playback mode: repeat and shuffle | springboard/apps/music-desktop/src/music/upnext.rs (set_shuffle, the shuffled order kept through a clear) |
| media-player | Media item queries and filters | not wired here: springboard/ui/src/media_query.rs is not used by the app (see gap 1) |
