# Changelog

## 1.14.3

- Rebuilt with the current SDK: the app redraws after an action from the shell (Look Up, Services, Spotlight) and accepts the shell's account-credential connections.

## 1.14.2

- Rebuilt with the current SDK and renderer: shell actions such as Look Up and Services reach the app, and vibrant text draws inside clipped areas.

## 1.14.1

- Rebuilt with the current renderer and SDK: Chinese, Japanese and Korean text draws whole, and a file opened with this app from Files or the Dock opens as a document.

## 1.14.0 — 2026-10-23

- A paused radio station closes its connection; the player bar shows live or elapsed time and a volume slider; an Output submenu.
- Empty library sections explain how to add music; the MiniPlayer's clicks land under the pointer.

## 1.13.5 — 2026-10-20

- Music no longer repaints in a loop while the audio service is failing. A service that accepts the connection and
  closes it again was reconnected at once, again and again, and every round redrew the window; the client now waits
  1 s, 2 s, 4 s, up to 30 s between attempts after a connection that did not hold.
- A station that could not be tuned in because the service was away is tried again on the same schedule (1 s, 2 s,
  4 s, up to 30 s) and starts by itself when the service returns. Pausing, skipping or choosing another station or song
  ends the retries.
- The same failure message shown again no longer repaints the window, and a repaint that would draw the very
  pixels already on screen is not sent to the shell. `BIFROST_FRAME_STATS=1` logs, once a second, how many repaints the
  app asked for and from where (finished jobs, the audio service, motion).

## 1.13.4 — 2026-10-20

- Radio plays again. The app's audio service client was created lazily and never connected, so a station or a song
  was sent to nobody. The player now connects on start and tries again when a command is sent.
- A command the audio service cannot take is logged to stderr with its cause and shown as a short message at the
  foot of the Radio list; the tuned station's second line shows the stream's own error. The title "Radio (Offline)"
  now states why the station list could not be fetched.

## 1.13.3 — 2026-10-20

- Nothing else changed: this release only rewords the notes for the new app icon.

## 1.13.2 — 2026-10-19

- The app has a new icon.

## 1.13.1

- BifrOSt 0.10.0 round (waves 14 and 15): see the system CHANGELOG for this app's changes.

## Unreleased

- Controls > Crossfade Songs: one song fades into the next over 2 to 12 seconds (off by default). The choice is kept in
  `playback.txt` of the library store and sent to the audio service when the store opens.

## 1.13.0 — 2026-10-17

- Window > MiniPlayer (Option+Cmd+M) opens a small extra window with the artwork, title and artist, the transport, a
  scrubber and the volume. Closing it returns to the main window. Window > Keep MiniPlayer on Top floats it. The window
  is `music/miniplayer.rs`, drawn and routed from `app.rs` through the SDK's extra windows.
- The ReplayGain track tag is read from ID3 `TXXX` and MP4 `----` freeform items (`media/id3.rs`, `media/ilst.rs`) and
  maps to a Sound Check loudness. The engine does not use it yet.
- Crossfade and Sound Check are still not heard: no Settings page sets them and the audio service has one decoder and no
  gain stage per track.

## 1.12.0 — 2026-10-17

- Get Info writes an edited title, artist or album into the song's file through the shared tag writer (MP3 and ADTS
  ID3v2.4, M4A `ilst`), with the atomic temp-and-rename write. The other tags are kept. Files with chapters, and
  formats the writer does not take (FLAC, WAV), keep the edit in the sidecar only. Blank values clear the tag.
  An MP3 rewrite writes the fields the tag model holds (title, artist, album, album artist, track, year, lyrics,
  cover) and keeps every other ID3 frame unchanged; a tag that cannot be copied safely (ID3v2.2, compressed frames) is not
  rewritten.

## 1.11.0 — 2026-10-17

- Lyrics fall back to the lyrics tagged in the file (ID3 USLT) when there is no `.lrc` file beside the song. The tag is read by
  `springboard/src/media/id3.rs`, and `Lyrics::load` in `music/lyrics.rs` tries the sidecar first, then the tag.
- Lyrics tests cover plain and timed embedded text, and the USLT frame tests cover the language and descriptor skip in
  ID3 v2.3 and v2.4.

## 1.10.1 — 2026-10-14

- Use the shared spacing token for the artwork badge inset.

## 1.10.0 — 2026-10-14

- View > Sort Songs By > Descending reverses the order of any sort key in the Songs list. A recent list keeps its
  newest songs in either order. The order is `browse::view_in_order`, tested headless in `tests/parity_app-music_w10.rs`.
- The View menu carries the Descending item with its checked state, and `Command::SortDescending` is in the menu ids.

## 1.9.0 — 2026-10-14

- Playlist folders can be renamed and deleted in the model. A renamed folder keeps its playlists, and a deleted folder
  leaves its playlists in no folder. The folder page that offers these buttons is not built yet.
- Playback settings model: a crossfade length of up to 12 seconds, Sound Check (a gain that moves each song toward
  -16 dB, limited to 9 dB either way), and an equal-power crossfade curve. The settings are kept as text in the same
  form the settings file uses. No Settings page sets them yet, and the audio service does not take them.
- Visualizer model: the level of each of the ten equalizer bands in a block of audio, and the rise and fall of a bar.
  No window draws the bars yet, because the audio service publishes no spectrum.

## 1.8.0 — 2026-10-14

- Get Info. Song > Get Info, and Get Info in a song's "..." menu, opens a page with the song's Title, Artist and Album.
  Tap a field to type a new value; an empty value takes the edit away, and Restore Original Tags drops them all. The
  edits are kept in `info.txt` in the Music data folder by file path and laid over the tags at every scan. The audio
  files are not written, so the changes show in Music only.
- Playlist folders. File > New Playlist Folder, or New Playlist Folder in the Playlists list, makes a folder. A
  playlist page's Move to Folder button (or its folder menu) puts the playlist in a folder; folders list their playlists
  nested under them and Show or Hide them. The folders are kept in `Playlists/folders.txt`.
- The mini player bar has a route button left of Play that lists the output devices, as the Now Playing sheet does.

## 1.7.0 — 2026-10-13

- Song and Controls menus in the menu bar: Play/Pause, Next, Previous, Shuffle, Repeat and volume with shortcuts;
  Favourite and a 1 to 5 star Rating for the playing song.
- Favourites show a heart in the song rows and the song menu has Favourite.
- New source list entries: Recently Added, Recently Played, Favourite Songs and Top Rated. Marks and the play log are
  kept in the Music data folder.

## 1.7.0 — 2026-10-13

- Ratings and favourites. A song's "..." menu steps its rating (none, 1 to 5 stars) and adds it to or takes it off the
  favourites. Rated songs show their stars under the length and favourites show a heart. The sort button gains Rating,
  Plays and Recently Played.
- New lists under Songs in the source list: Recently Added, Recently Played and Favourite Songs. Music counts a play once
  a song has been heard for 30 seconds (half of a shorter one) and keeps ratings, favourites, play counts and the history
  in `stats.txt` next to the queue.
- Smart playlists. "New Smart Playlist" in the Playlists list opens a rules editor: pick Title, Artist, Album, Year,
  Rating, Plays, days since added or played, or Favourite, an operator and a value, and choose whether all rules or any
  rule must match. The playlist fills itself from the library and follows new ratings and plays. Smart playlists are
  files in `Smart/` and appear in the source list.
- Up Next: drag a row by its handle to move it anywhere in the queue, and open History (the songs that played lately,
  most recent first) from the Up Next toolbar or the View menu.
- Lyrics. A song with a `.lrc` file beside it gets a Lyrics switch on the Now Playing sheet: the line being sung is
  highlighted and follows the song. Plain text lyrics are listed without highlight.
- App menus: File > Import, Song (Play Next, Play Last, Add to Playlist, Rating, Favourite), Controls (Play or Pause,
  Next, Previous, Shuffle, Repeat, Volume) and View (Artists to Playlists, the new lists, Up Next, History, Lyrics, Sort
  Songs By), with key equivalents.
- File > Import copies chosen songs into `Music/Imported` and scans the library again.
- The route button on the Now Playing sheet lists the output devices and makes the chosen one the default output.
- Music asks for the `documents` permission, for the Import open panel.

## 1.6.2 — 2026-10-13

- The Now Playing sheet slides on the shared `PanelReveal` motion from the SDK kit (`bifrost_app::kit::motion`), so it follows
  the system durations and snaps under Reduce Motion.

## 1.6.1 — 2026-10-12

- Rebuilt on the BifrOSt 0.6.4 SDK: the shared shell/app wire codec, frame and list caps, and the window materials API.

## 1.6.0 — 2026-10-12

- The Songs list shows each song's tempo in beats per minute, in a column left of its length. Music reads the start of
  each song in the background while the list is on screen, two songs at a time, so the numbers fill in over a few
  minutes for a large library. A song with no clear beat, and a MIDI file, shows no number.

## 1.5.0 — 2026-10-12

- Up Next can be cleared. A "Clear Up Next" line sits at the end of the queue when songs are waiting. It takes every
  waiting song out and keeps the song that is playing, where it is. A shuffled queue keeps its played order for when
  shuffle is turned off.

## 1.4.1 — 2026-10-12

- Dark appearance: the main content area fills with the dark content grey instead of pure black, as stock dark content views do; sidebars and cards keep their colours.

## 1.4.0

- The library follows the music folders. Every few seconds Music checks the files it holds: a song that was added,
  removed or changed (its size or modification time) starts a new scan, and the Songs, Albums and Artists lists update
  on their own. Nothing needs a restart.
- MIDI files (.mid, .midi) are listed in the library with their length, read from the file. They are shown under the
  artist "MIDI file". Music cannot play them yet: there is no sound bank, so a MIDI song is not put in the queue.
- Music folders can be left out of the library. A folder directly inside ~/Music can be excluded, and the folders
  loaded can be limited to a list. The options are the defaults keys `music.library.loadFolders`,
  `music.library.include` and `music.library.exclude` (one folder path per line).

## 1.3.0

- Now Playing: Music keeps a system Now Playing session in step with what is playing. The current song (title, artist,
  album, length) or station (a live item) is the session's content, and the playback state and position follow the
  audio service.
- Transport commands are checked before they run: Play, Pause, Toggle, Next and Previous are offered only when they can
  act now (Next only with a following song, or with Repeat All; Pause only while playing). Stop and skips are not offered.
- A system playback command (play, pause, toggle, next, previous) runs the same transport as the buttons do.
- Control Center's Now Playing tile shows the song or station Music is playing. Its play, pause, next, previous and seek
  taps now reach Music. The tile is updated when the song or the state changes, and after a seek.
- Unplugging headphones or a Bluetooth output pauses Music, as the speaker takes over.
- While the library is being scanned, the Music Dock icon shows a progress bar.

## 1.2.3

- The Songs search field is always open in the toolbar, as Music's is: a kit search field (a labelled text field
  on the root screen), no longer a collapsed magnifier button. Clicking it starts a search; Esc or the clear button ends it.
- Colours come from named tokens (`ui::DESKTOP_MENU_SOLID`, `scale(WHITE|BLACK, a)` for the Now Playing dims and
  scrims) instead of literal `rgba`/`pack` values; layout reserves are named constants.
- Album grid caption space is `GRID_LABEL_PT`; the station row constant is `STATION_ROW_PT`.

## 1.2.2

- Background work (library scan, artwork, logos, the station list) runs on the shared dispatch queues instead of a
  private worker pool; results reach the window on the main queue.
- Quitting is sudden: the play queue and playlists are written when they change, so the app keeps no unsaved state.
- Play and Shuffle on the album and playlist pages, and the sidebar rows, are published as labelled controls.
- Menu, artwork and button corners use the system radii (cards 13 pt, icon tiles 5 pt, buttons are capsules).
- Empty-state text is centred on the content area instead of a fixed offset from its top.
- Compliance test (`tests/compliance.rs`): the root renders at 1200 x 780 pt, scale 1.5625, in Light and Dark, with the
  sidebar and toolbar published and every control labelled.

## 1.2.1

- Follows a Light/Dark switch immediately, including while the window is in the background.

## 1.2.0 — 2026-10-10

- Golden Gate window: a source list at the left (Library, Radio, Playlists with each playlist) and a unified toolbar
  over the content (Back inside a list, the title, Sort, Up Next, a search field on Songs, Rename and Delete on a
  playlist), drawn with the system kit in a full-size window. Replaces the iPad large title, segmented tabs and back
  chevrons.
- Light and Dark: the library, lists, album pages, song menu and mini player follow the system appearance.
- Toolbar buttons are published as controls, so the window does not drag over them.
- Needs system 0.4.84.

## 1.1.3 — 2026-10-10

- Opens again on BifrOSt 0.4.90 and later: rebuilt against the current SDK, whose app socket is `BIFROST_SOCK`.

## 1.1.2 — 2026-10-09

- Spacing follows the system 4-pt grid: 43 padding and gap values moved to the nearest standard step (at most 2 pt).
- 4 colours now use the system colour tokens (same values).
- Built for system 0.4.62.

## 1.1.1 — 2026-10-09

- Text that is shortened to fit now measures at the size it is drawn at, so labels are no longer cut off early.
- Follows the system accessibility settings (Increase Contrast, Reduce Transparency, Show Borders, Bold Text) on system 0.4.61.

## 1.1.0 — 2026-10-09

- Text sizes and spacing come from the shared macOS-style scale, so this app matches the others.
- No gap above the content for a status bar the desktop does not have.
- The accent is the system blue like every other app (it was Apple Music red).

## 1.0.0 — 2026-10-08

- Music for the BifrOSt desktop: the tablet app's code, hosted in a window with keyboard and pointer input.

- Use the desktop package identity for the SDK handshake and supported PC permissions.
- Connect to the Linux audio service path.
- Scan the local account’s Music directory.
- Keep text-field focus without drawing an on-screen keyboard in PC app processes.
