# CLI: open, pbcopy, pbpaste (slice cli-open-clipboard)

Reference: the shipped man pages `open.1`, `pbcopy.1` and `pbpaste.1` under
`~/Projects/macos-ref/extract/ref27/usr/share/man/man1/` (read with `man -l`). No REA evidence was used; the
behaviour comes from the man pages and the existing shell control socket.

Matrix entries (not edited here, the lead closes them): `path:/usr/bin/open` (cli/m-o.json),
`path:/usr/bin/pbcopy` and `path:/usr/bin/pbpaste` (cli/p-r.json).

## Code

- `springboard/cmd/src/open.rs`, binary `springboard/cmd/src/bin/open.rs`.
- `springboard/cmd/src/pasteboard.rs`, binaries `springboard/cmd/src/bin/pbcopy.rs` and `bin/pbpaste.rs`.
- `springboard/cmd/src/lib.rs`: two `pub mod` lines.

## open

| Man page option | Here | Path |
|---|---|---|
| `-a application`, `-b bundle_identifier` | done: `open-app <id>` with no operands, `open-file <id> <file>` for each file operand | shell control socket, `src/bin/shell.rs` `OpenApp`, `OpenFile`, `desk.rs` `desktop_open_file` |
| `-e`, `-t` (text editor) | done: `open-app is.olibuijr.textedit.desktop`, or `open-file` with that app for each file operand | same |
| `-R` (reveal in Files) | done: a file is shown selected in its folder (`reveal-file is.olibuijr.files.desktop <file>`), a folder is shown itself (`open-folder`) | `RevealFile`, `OpenFolder`, `desk.rs` `desktop_reveal_file`, `desktop_open_folder` |
| `-u URL` | done: `open-url <url>` | `OpenUrl`, `defaultapps.rs` `desktop_open_link` |
| URL operand, `file://` URL | done (`file://` becomes a path) | as above |
| file operand, no app named | done: `open-document <file>`, the shell picks the default app for the type | `OpenDocument`, `desk.rs` `desktop_open_document`, `defaultapps.rs` `default_app_for_file` |
| folder operand | done: shown in Files | `OpenFolder` |
| `-W -n -g -j -F -f -h -s --arch --env --stdin --stdout --stderr --args` | refused with "option X is not supported yet" | |
| file operand opened with its default app | refused with a message | needs a shell request that picks the default app |
| folder or URL passed to `-a`/`-e` | refused with a message (a folder with `-a is.olibuijr.files.desktop` is shown in Files) | |

Protocol: one line per connection to the Unix socket `/run/bifrost/shell-control.sock` (`SPRINGBOARD_CONTROL_SOCKET` overrides it), answer `ok` or
`error: <why>`. Same channel as `packaging/arch/wayland/open-url`. Request lines are capped at 510 bytes and refuse line breaks.

Still needs Midget checking: `open -a`, `open -e`, `open -R` on a file, `open https://...` and folder operands against a
live shell.

## pbcopy and pbpaste

- `-pboard general` only. `ruler`, `find` and `font` are refused: the shell's pasteboard service holds named boards,
  and the command line cannot reach it yet.
- Data goes through `wl-copy` and `wl-paste`, the Wayland selection that the shell already offers typed items on
  (`src/shell/pasteboard_sync.rs`, over the window server's data-control globals). Empty input to `pbcopy` clears the board.
- `pbcopy` picks the type from the input header: `{\rtf` gives `text/rtf`, `%!PS-Adobe` gives
  `application/postscript`, anything else is plain UTF-8 text.
- `pbpaste -Prefer txt|ascii|rtf|ps` tries the preferred type first and the others after it, as the man page says.
  `ascii` is accepted as `txt`.
- `-help` prints the usage line.

Gaps: the shell's pasteboard service keeps item grouping and several representations per copy, and general writes do
not re-broadcast to other apps' mirrors. This tool reads and writes one flat item. A plain-text copy from this tool
reaches the shell only if the compositor's Wayland selection reaches the shell's plain clipboard. That path was not
measured on Midget.

Still needs Midget checking: `echo hi | pbcopy` then paste in a BifrOSt app, and a copy from an app then `pbpaste`.

## Shell-side gaps the lead should know about

- `open-file <app id> <path>` on the control socket opens a file in a named app with the app's document grant
  (`-a`, `-e`, `-t` with file operands). `open-document <path>` is the same request without an app: the shell picks
  the default app and applies the same checks (a file in the home folder, the malware check).
- `open-app` and `open-folder` need the desktop to be active, as they already are.

## Tests

Unit tests in `open.rs` and `pasteboard.rs` cover option parsing, the request plan, URL and `file://` shapes, the
control-line format, reply mapping, a loopback round trip through `send`, and the pasteboard type choices. Nothing was
built or run, per the round rules; the lead runs them.
