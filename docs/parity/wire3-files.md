# Wire 3: Files app (wire3/files)

Scope: `springboard/apps/files/` (Files 1.19.0). Two items from the wire2 gap list were wired: the data-based
preview registry for pictures, and Connect to Server for ftp:// addresses with a remote folder browser. Nothing was
built or run in this round (the lead builds once); every expected value below was worked out by hand.

## What is now wired

1. **Data-based preview registry for pictures.** The preview panel asks a `bifrost_app::peek::Registry` which provider
   previews the focused file. The picture provider (`PictureFile`, a `PreviewProvider`) is registered for
   `public.image`, so the image type tree decides (PNG and JPEG are covered through their parent type). Its reply is
   `Content::Bitmap` with the decoded pixel size, which `Reply::new` validates (zero or above 16384 pixels is refused,
   and the panel then shows the file's details instead). The decoded picture is kept for the panel to draw.
   Code: `apps/files/src/peek.rs` (`PictureFile`, `picture_registry`, `Files::peek`).
   Text and folders still use the in-app path; the SDK's `PreviewView` (`sdk/src/peek.rs`) is not placed by Files yet.
2. **Connect to Server (ftp://).** Go > Connect to Server, Command-K. The address sheet takes an ftp:// address (a
   bare host name gets `ftp://` added). Return connects in the background: anonymous login (or the user and password
   in the URL), `TYPE A`, `PASV` for the data address (`bifrost_app::ftp::pasv_address`), `LIST` over the data
   connection, then `bifrost_app::ftp::parse_listing`. A connect, reply or transfer that waits longer than 15 seconds
   reports why. Connected, the browser sheet lists the folder (folders first, `.` and `..` hidden). Return on a folder
   opens it, Delete goes up one level, Escape disconnects. A remote file reports "Files can't open remote files yet."
   Code: `apps/files/src/server.rs` (`fetch`, `read_reply`, `Files::open_connect`, `start_connect`, `poll_server`,
   `address_input`, `browse_input`, `paint_address`, `paint_browse`); the chord is in `apps/files/src/go.rs`
   (`go_chord`, `(37, false, false)`).
3. **Recent servers.** Up to eight addresses, newest first, kept in `$XDG_CONFIG_HOME/bifrost/files-servers.txt`
   (or `~/.config/bifrost/files-servers.txt`). The address sheet lists them under the field. Down picks one (Up goes
   back to the field), Return connects to the picked one. The list is saved when a listing succeeds from the sheet, and
   a failed write only loses the list for the next launch.
4. The server sheets are modal: keys go to them, and clicks and right-clicks are swallowed so they do not reach the
   folder behind. They draw in the same place as Go to Folder, with the same sheet widths and the `CARD_BG` card.

Pure helpers with host tests (`apps/files/src/server.rs` `mod tests`): `normalize`, `remember` (move to front, cap at
eight), `parse_recent` (blanks and repeats dropped), `remote_parent` and `remote_join`, `read_reply` (single line,
multi-line through the closing `NNN ` line, closed connection is an error) and `sorted` (folders first, `.` and `..`
removed) on a `parse_listing` sample.

## How a user reaches it

- Picture preview: select a PNG or JPEG in Files and press Space or open the preview panel; the picture is drawn in the
  panel as before. Pictures the registry refuses (too large, or not decodable) show their details instead.
- Connect to Server: press Command-K in Files (Go > Connect to Server). Type `files.example.is/pub` or
  `ftp://user:password@host:2121/path`, press Return. Down and Return reconnect to a recent server.

## How to verify on Midget

1. `bifrost pc publish`, then `bifrost pc midget ota` until `phase=accepted`.
2. Preview: put a PNG and a JPEG in a folder on Midget, select each and open the preview panel. Expected: the picture
   fits the panel. Put a 1 x 1 PNG and a file named `broken.png` containing text: the first shows its picture, the second
   shows its details (name, kind, size), not a blank panel.
3. Connect: run a local FTP server on another machine (for example `python3 -m pyftpdlib -p 2121 -d /srv/ftp -w` for
   an anonymous or user login, or any anonymous FTP server). Press Command-K, type `ftp://192.168.1.10:2121/`, Return.
   Expected: "Connecting…" then the folder list, folders first. Return on a folder opens it; Delete goes up; Escape
   closes. Then Command-K again: the address appears under Recent Servers; Down, Return reconnects.
4. Failure: type `ftp://127.0.0.1:9/` (nothing listens) and Return. Expected: a message under the field within 15 s,
   the sheet stays open for another address.
5. Dark and Light: the sheets are `CARD_BG` cards over the Files content; check both appearances.

## REA evidence

- `ev_4d1d5fef7d13600ae15a2632e17064bc0a9907fa72207d552e32d29ef650418e`: rendered official page
  `https://developer.apple.com/documentation/quicklook/data-based-preview-extensions` (Quick Look data-based preview
  extensions; QLPreviewProvider and QLPreviewReply).
- `ev_1cf971ce7e074be5ee2648c6e3640036b987c62cb3d8e34d4bbf3548567da1a4`: rendered official page
  `https://developer.apple.com/documentation/cfnetwork/ftp` (CFNetwork FTP transfers, deprecated in favour of URL
  loading).
- The `docc.py` JSON endpoints for both paths failed in this session; the rendered pages above are the evidence.

## Remaining gaps

- **Quick Look space-bar panel.** The Files preview panel still draws its own pictures and text. The SDK `PreviewView`
  (`sdk/src/peek.rs`) is not placed by Files, and text, PDF and video previews do not use the registry. Third-party
  providers cannot register a file type yet (no extension host).
- **Remote files.** Files cannot open, download, copy, or drag remote files. The browser lists folders and files only.
  The server is not mounted in the sidebar, and it is not in the Go menu bar: Files has no main-menu model in this
  code base, so Go > Connect to Server is reachable only by Command-K (as Go to Folder is only by Shift+Command-G).
- **Protocol.** Plain FTP only: no TLS (FTPS), no EPSV, no ASCII/binary listing options, no reconnect after a dropped
  control connection. The passive address must be reachable; a server that reports a private address behind NAT fails.
- **Recent servers** keep the typed address, so a password typed into the URL is stored in the file. Use the anonymous
  form or a user name only, until a keychain store exists.
- The Files Cargo.toml version is 1.19.0 and `springboard/Cargo.lock` still names 1.18.0 for `bifrost-files`; cargo
  updates the lock at the lead's build (this round did not edit files outside the app folder).
- Not verified: no build or test run (the lead builds once). The snapshots `apps/files/tests/snapshots/` do not cover
  the new sheets; no bless is needed for the picture change unless a snapshot shows a picture preview.

## Matrix rows

| Slug | Feature (as in `docs/parity-matrix/data/<slug>.json`) | BifrOSt path |
|---|---|---|
| quick-look | Data-based and view-controller preview extensions | springboard/apps/files/src/peek.rs (PictureFile, picture_registry, Files::peek) |
| quicklook-ui | Data-based preview extensions (QLPreviewProvider) | springboard/apps/files/src/peek.rs (PictureFile, picture_registry) |
| cfnetwork | FTP transfers | springboard/apps/files/src/server.rs (fetch, read_reply, Files::open_connect) |

Rows are not promoted to `parity` yet: each needs the Midget check above first (`bifrost pc parity` update after
Midget passes). The Quick Look row stays `Partial` until the space-bar panel uses the registry.
