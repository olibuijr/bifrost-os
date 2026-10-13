# Screen Sharing

Independent desktop App Store app `is.olibuijr.screensharing.desktop` (viewer). Build and publish with
`tools/pc-app.py build|publish springboard/apps/screen-sharing`.

Type the address of a computer that serves VNC (RFB 3.3, 3.7 or 3.8, no password) and press Connect or Return. Port
5900 is used when the address has none. The remote desktop shows scaled to fit (Scale to Fit) or at actual size.
Pointer and keyboard go to the remote computer unless View Only is on. Right-click sends a secondary click. Send
Clipboard sends this computer's text; text the server copies is put on this computer's clipboard.

The protocol is in `springboard/rfb` (`bifrost-rfb`). See `docs/parity/screen-sharing.md` for what is not yet
supported: password authentication, TLS, and encodings other than Raw and CopyRect.

Server side: the Remote app and Settings > Sharing > Screen Sharing (`docs/parity/screen-sharing.md`, Gaps).
