# Remote

Independent desktop App Store app `is.olibuijr.remote.desktop`.
Build/publish with `tab pc app build|publish remote`.

Open Remote and select Start Sharing. It serves a password login and noVNC
browser client on `0.0.0.0:12000`. Use the address and password displayed by
the app. Stop Sharing stops the server; closing the app leaves it running.
The service runs as the local desktop account, captures the real labwc session
through WayVNC, and exposes VNC only through a private Unix socket. Browser
WebSocket access requires an authenticated session and matching Origin.

Runtime dependencies: `wayvnc`, `python-aiohttp`, provided by the PC system
package and installer. Bundled client is noVNC 1.6.0; license in assets/novnc.
Remote serving currently starts on demand rather than automatically at boot.

Version 0.1.3 adds SF UI desktop/mobile controls, fullscreen, local pinch zoom
and pan, view-only mode, keyboard and clipboard. HTTP authentication remains
available while the WayVNC supervisor waits for a replacement desktop session.
A sharing service retries after desktop restarts; it still starts on demand.
