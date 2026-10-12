# Changelog

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
