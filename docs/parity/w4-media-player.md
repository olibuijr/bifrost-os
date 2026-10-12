# Wave 4: Media Player parity

Framework: Media Player (`docs/parity-matrix/data/media-player.json`). Official doc:
https://developer.apple.com/documentation/mediaplayer

This wave closes three matrix rows: one `os` row (the system already sends transport commands, but apps had no API
for them) and two `gap` rows. All three are pure models in `springboard/ui` (bifrost-ui, `std` only, no hardware, no
network). Nothing was built or run in this wave; the tests were written by hand and have not been executed.

## What was built

1. **Remote command centre** (`springboard/ui/src/remote_command.rs`, `CommandCenter`). The file already held the
   playback rate, rating and feedback commands. The new `RemoteAction` enum names the transport actions (play, pause,
   toggle, stop, next track, previous track, change playback position). `CommandCenter` keeps the handlers per action
   and the actions switched off. It stores no closures: `add_handler` returns a `HandlerToken`, the app keeps its handler
   under that token, and `dispatch` returns the token to run. A switched-off action is refused with `Disabled`, and an
   action with no handler with `NoHandler` (a new `CommandError` variant). Dispatch picks the newest handler for an action.
   The official pages do not state the invocation order when several handlers are registered for one command; the
   newest-first choice is ours and is a decision, not a copy of Apple's behaviour.
   SDK surface: `springboard/sdk/src/lib.rs` re-exports the module as `bifrost_app::remote_command`.

2. **Playlists** (`springboard/ui/src/media_playlist.rs`, new). `Playlist` is a named, ordered list of item identifiers
   with an optional description and author. `PlaylistMetadata` is the creation input and needs a non-blank name.
   Items are added at the end (`add`, `add_all`), as the framework's add calls do. `add_all` checks every identifier
   before it appends, so one blank entry leaves the playlist unchanged. Duplicates are kept.

3. **Library change records** (`springboard/ui/src/media_changes.rs`, new). `MediaLibrary` compares a fetched listing
   (`Stamp`: identifier and revision) with the previous one and returns a `LibraryChange` (inserted, removed, changed,
   each sorted). It records the time of the last change. Change notifications are switched on and off with
   `begin_generating` and `end_generating`. While they are off, the snapshot and time still advance, so switching
   them on later does not replay old changes. This is a decision: the official page does not say what happens to
   changes that occur while notifications are off.

Module registration: `springboard/ui/src/lib.rs` has one `pub mod` line per new module (`media_playlist`,
`media_changes`). `remote_command` was already registered.

## Official API names mirrored

| Module | Official names (Media Player) |
|---|---|
| `remote_command::CommandCenter` | `MPRemoteCommandCenter` (shared instance and the playback, track, and language commands), `MPRemoteCommand` (`addTarget`, `removeTarget`, `isEnabled`) |
| `media_playlist::Playlist` | `MPMediaPlaylist` (`name`, `descriptionText`, `authorDisplayName`, `add(_:)`, `addItem(withProductID:)`), creation metadata |
| `media_changes::MediaLibrary` | `MPMediaLibrary` (`beginGeneratingLibraryChangeNotifications`, `endGeneratingLibraryChangeNotifications`, `lastModifiedDate`) |

Not mirrored: the handler status values (`MPRemoteCommandHandlerStatus`), skip-interval, seek, rating and feedback
commands in the centre, language options, persistent and cloud identifiers, playlist attributes, seed items,
authorization (`requestAuthorization`), and the library query and playlist retrieval calls.

## REA evidence (rendered official pages)

| Page | REA evidence id |
|---|---|
| https://developer.apple.com/documentation/mediaplayer/mpremotecommandcenter | `ev_509305c6d87f21b3784684e78b86b263c5376afb2656a7748536e73582229c83` |
| https://developer.apple.com/documentation/mediaplayer/mpremotecommand | `ev_d30713d64f46cd20ad847e1ec5720aa8960dea6ceaa71068bdcacd807f4606c5` |
| https://developer.apple.com/documentation/mediaplayer/mpmediaplaylist | `ev_d3921a3a79485bcd1be64d0a8e9486a9565a45c7e4d958da48f38ae9c692d775` |
| https://developer.apple.com/documentation/mediaplayer/mpmedialibrary | `ev_ec0d580ecb6983b0e60e39d8b9d371b859a400321cca48f6a03bbbd54136faa5` |

## Matrix rows closed

| Framework | Feature (as in `docs/parity-matrix/data/media-player.json`) | Previous status | New status | BifrOSt path |
|---|---|---|---|---|
| Media Player | Remote command center for apps | os | partial | `springboard/ui/src/remote_command.rs` (`CommandCenter`), `springboard/sdk/src/lib.rs` (`remote_command`) |
| Media Player | Media items, collections and playlists | gap | partial | `springboard/ui/src/media_playlist.rs` (`Playlist`) |
| Media Player | Media library synchronization | gap | partial | `springboard/ui/src/media_changes.rs` (`MediaLibrary`) |

Status is `partial`, not `parity`. The centre is not wired into the audio service or the shell, so the system does not
yet deliver Control Center or lock-screen commands to app handlers. The playlist model is not used by the Music app,
which still keeps its own `.m3u` files. The library tracker is not fed by the Music app's scan.

## Tests (written, not run)

- `remote_command.rs`: five new tests on `CommandCenter` (newest handler wins; no handler is refused; switched-off
  action refused until switched on; removing the newest falls back to the older handler; routing per action and no
  token reuse).
- `media_playlist.rs`: five tests (name trimming and blank names; blank description and author become None; append
  order with duplicates; empty item refused; `add_all` atomic).
- `media_changes.rs`: six tests (first snapshot reports nothing while off; insert, remove and revision change sorted;
  unchanged listing keeps the time; changes while off update time only; no replay after switching on; duplicate
  identifier takes the later stamp).

No new crates, no `unsafe`, no Apple platform or product names in the `.rs` files (`python3 tools/check-names.py`).
