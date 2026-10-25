## 0.2.2 — 2026-10-19

- The web viewer uses the system font; it no longer bundles a font BifrOSt may not redistribute.

## 0.2.4

- Rebuilt with the current SDK and renderer: shell actions such as Look Up and Services reach the app, and vibrant text draws inside clipped areas.

## 0.2.3

- Rebuilt with the current renderer and SDK: Chinese, Japanese and Korean text draws whole, and a file opened with this app from Files or the Dock opens as a document.

## 0.2.0 — 2026-10-12

- Nearby computers: Remote looks on the local network for other computers that share a desktop and lists them. Tap one to see its address.
- Network: the window says whether this computer is connected by Ethernet, Wi-Fi or mobile data, or has no connection yet.
- Not yet: this computer does not announce itself on the network, so other computers cannot list it.
- Network changes arrive from the system as they happen, so the window updates when the connection drops or comes back. Older systems still check the link every few seconds.
- Desktop size: the intro line shows the size of the desktop being shared.
- Copy address: tap the address (yours, or the selected computer's) to copy it, then paste it into another app.

## 0.2.1 — 2026-10-12

- Rebuilt on the BifrOSt 0.6.4 SDK: the shared shell/app wire codec, frame and list caps, and the window materials API.

## 0.1.9 — 2026-10-11

- Full-size window (`window = "full-size"`, min system 0.4.84): the large title sits below the traffic-light band.
- Colours follow Light and Dark through the shared Pair tokens; type uses the macOS text sizes; Stop Sharing is a quiet secondary button.
- Enable sudden termination (nothing in the app is unsaved). Bundle an app icon.

## 0.1.8 — 2026-10-10

- Follows the system Light and Dark appearance instead of always drawing dark.

## 0.1.7 — 2026-10-10

- Opens again on BifrOSt 0.4.90 and later: rebuilt against the current SDK, whose app socket is `BIFROST_SOCK`.

## 0.1.5 — 2026-10-08

## 0.1.6 — 2026-10-08

- Rebuild against the reviewed shared PC app foundations.

- Keep WayVNC control sockets in writable private service storage.

## 0.1.4 — 2026-10-08

- Run the UI inside the central app sandbox as the local account.
- Request Start/Stop Sharing through the authenticated system broker; remove direct privileged systemd launches from the app.

## 0.1.3 — 2026-10-08

- Add desktop and mobile browser controls: fullscreen, fit, actual size, zoom, pinch and pan, input/view-only modes, keyboard, clipboard and reconnect.
- Use SF UI and the shared BifrOSt dark palette with accessible controls.
- Keep the web interface available while Wayland restarts; reconnect WayVNC without exhausting service restart limits.

# Remote

## 0.1.2 — 2026-10-07

- Reconnect serving after desktop session restarts; redraw the control window only when status changes.

## 0.1.1 — 2026-10-07

- Detect the browser address from the active network interface.

## 0.1.0 — 2026-10-07

- Browser remote desktop on port 12000, with a private access password and start/stop control.
