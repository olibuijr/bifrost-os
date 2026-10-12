# Connect: Browser app to the system services

The Browser (`springboard/apps/browser`, 0.10.0) now uses four shell services that were added for the parity work.
This is app-side glue only. The shell, the SDK and the other apps are unchanged.

## What is connected

| Service (SDK client) | Matrix slug | Feature | What the Browser does | Where |
|---|---|---|---|---|
| `pasteboard` (`bifrost_app::pasteboard_types`) | `application-services` (Pasteboard) | Copy a link as URL plus title | Cmd+Shift+C writes the current http(s) page to the general board: `public.url`, the URL as plain text, and `public.html` with the title as a link. A blank title uses the URL. | `system.rs` `link_item`, `send_copy`; `main.rs` `system_chord` |
| `pasteboard` | `core-transferable` (Copy and paste of typed items) | Copy an image | Cmd+Shift+K writes the newest finished PNG or JPEG download as its image type. Images up to 24 KB go through the service; larger ones use the synchronous pasteboard. | `system.rs` `newest_image`, `read_image`, `image_route` |
| `network` (`bifrost_app::network_path`) | `network` (Network path monitoring) | Offline page and retry | Asks for the path at start and on resume, and handles `pathUpdate`. While the network is down, a chosen address shows an "You are offline" page with a Try again link. When the network comes back, each waiting tab loads its address. | `system.rs` `Network`, `offline_url`; `main.rs` `set_path`, `load_chosen`, `service_event`, `service_reply` |
| `cursor` (`bifrost_app::cursor_service`) | `appkit-user-interactions` (Menus, Cursors, and the Dock) | I-beam over text | Pushes the I-beam while the pointer is over the address field (not while a sheet or page menu is up) and pops it when the pointer leaves. | `system.rs` `Pointer`; `main.rs` `update_pointer` |
| `dock` (`bifrost_app::dock_tile`) | `appkit-user-interactions` (Menus, Cursors, and the Dock) | Download progress | Shows the summed progress of running downloads on the dock icon. An unknown total shows an indeterminate bar. Clears when no download runs. A new value is sent only when it changes by 1 % or the kind changes. | `system.rs` `dock_progress`, `dock_changed`, `send_dock`; `main.rs` `tick` |

## How a user sees it

- **Copy link:** open a page, press Cmd+Shift+C, then paste in another app. A rich editor gets the title as a link; a plain text field gets the URL.
- **Copy image:** download a PNG or JPEG (the Browser saves it to Downloads), press Cmd+Shift+K, then paste it in an image editor or a document.
- **Offline:** turn off the network (unplug the Ethernet on Midget). Type an address and press Return: the tab shows "You are offline". Plug the network back in: the tab loads the address by itself. Pressing Try again also works.
- **Pointer:** move the pointer over the address field: it becomes an I-beam. Move it off: the arrow returns.
- **Dock:** start a large download. The Browser's dock icon shows a progress bar that fills until the download finishes.

## How to verify on the test PC (Midget)

Midget only. Build, publish and update as usual (`bifrost pc publish`, `bifrost pc midget ota` until `phase=accepted`), then:

1. Copy link: open `https://example.com`, Cmd+Shift+C, then paste into a text field of another app. Expect `https://example.com`. Pasting into a rich-text app should keep the title as a link.
2. Copy image: download a small PNG, Cmd+Shift+K, paste into an image-accepting app. Expect the image. Check the shell log for `pasteboard` write errors (none expected).
3. Offline: with Ethernet unplugged, enter `https://example.com` and press Return. Expect the offline page. Plug the cable in: the tab should load `https://example.com` within a few seconds (the shell sends `pathUpdate`).
4. Pointer: `bifrost pc midget screen` with the pointer over the address field; the shape is the I-beam.
5. Dock: start a download of a few MB, check the dock icon for the progress bar, and that it is gone after the download.
6. Older shell: `pasteboard`, `network`, `cursor` or `dock` answer "no service". Expect the Browser to keep working; a copy then goes to the plain clipboard.

Host tests (not run here, the lead builds once): `cargo test -p bifrost-browser system::` covers the link payload, image routing, newest-image choice, dock progress and threshold, the network state and held pages, the pointer transitions, and the offline page URL.

## Gaps

- **Copy a page image or link under the pointer:** the engine does not report which image or link is under the pointer, and the SDK web view has no hit-test event. The copy works on the page itself and on downloads only.
- **Cursor over page links:** a pointing hand over links needs hover data from the engine (the SDK has no cursor-change or hover-target event). Only the address field gets a shape.
- **Pointer leaving the window:** the SDK has no pointer-leave input, so an I-beam can stay after the pointer leaves through the window edge (the same limit as tab hover).
- **Offline detection:** only the path (no interface up) is used. A dead network behind a live interface still loads and fails in the engine. The Browser does not probe a host.
- **Offline page after a failed load:** the Browser has no load-failure event, so a load that fails while the path is up shows the engine's own error page.
- **Image formats:** PNG and JPEG only. WebP and GIF downloads are not copied. Images over 16 MB are not read.
- **Image size on the service:** images over 24 KB use the synchronous pasteboard path (`pasteboard::NativePasteboard::set_data`) because the service body is hex in a 64 KiB JSON message.
- **Named boards:** the Browser uses only the general board.
- **Tests:** the glue is tested through pure functions in `system.rs`. There are no `Ctx::testing` send tests, because the browser crate has no test font helper; the send helpers are one call each.
- **Not built:** written without cargo or tests, per the wiring rules.
