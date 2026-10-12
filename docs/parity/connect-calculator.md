# Connect: Calculator to the pasteboard service

Status: wired in `springboard/apps/calculator-desktop` (version 1.4.0, the round's bump; not bumped again). Not built
here: host tests written, not run. The lead builds once.

Matrix rows this moves (`docs/parity-matrix/data/`, not edited here):

- `application-services.json`: "Pasteboard (NSPasteboard)" (partial). Calculator is a client of the `pasteboard` service.
- `core-transferable.json`: "Copy and paste of typed items" (partial). Calculator's Copy now writes a typed item (two
  representations) instead of a plain string.

Service notes: `docs/parity/wire-svc-pasteboard.md`. App notes from the previous round: `docs/parity/wire-app-calculator.md`.

## What is connected

- **Copy writes to the general board through the service.** Copy (the Edit menu's Copy, Cmd+C, and the touch menu's
  Copy) now calls the `pasteboard` service method `write` on the `general` board, with one item:
  - `public.utf8-plain-text`: the number as plain digits, for example `1234.5`. Same text as before, so a text field
    or another app's Paste gets the number as it did.
  - `public.html`: the same number in the calculator's language, for example `1234,5` in Icelandic, wrapped in a span.
    Characters that HTML reserves are escaped.
  - Source: `springboard/apps/calculator-desktop/src/copy_service.rs` (`item`, `send`, `span`).
- **The copy is sent only on a real event.** `calc.rs` `copy` records the copy; `main.rs` `send_copy` sends it once,
  after the key, touch or Edit-menu event that caused it. Nothing is sent every frame.
- **The shared clipboard sees the copy as a change.** The shell notifies other running apps of a `changed` event on
  the general board, with the new change count. This is the shell's behaviour and needs no Calculator code.
- **Fallback.** Each sent copy is remembered by its request id. When the reply comes back as an error (an older shell
  answers `no service pasteboard`, or the write fails), the number goes to the plain clipboard (`clipboard::set_text`),
  as it did before this change. The app keeps at most 16 unanswered copies.

Paste is unchanged: it still reads the plain clipboard synchronously, so a paste takes effect at once.

## How a user sees it

Copy a result (Cmd+C, or Copy in the touch menu) and paste it anywhere, as before. The number is the same plain
digits. Other apps that read the shared pasteboard now see the copy as a change, and the copy carries the number
in the calculator's language as HTML. No Settings page or menu item is added.

## How to verify on the test PC (Midget)

Midget only; the lab VM is not used for this round.

1. Build and publish as usual: `bifrost pc publish`, then `bifrost pc midget ota` until `phase=accepted`.
2. Check the service is registered: call the `system` service's `services` method from an app (or read the shell log);
   the list must include `pasteboard`.
3. `bifrost pc midget open-app is.olibuijr.calculator.desktop`, then `bifrost pc midget state` to find the Calculator
   window. Type `1234.5` on the keypad (or Paste a number first).
4. `bifrost pc midget tap-window Calculator X Y` on the touch menu's Copy (window-local pixels from a screenshot).
   Use `tap-window`, not `tap`.
5. Read the general board from a second app (or with `pasteboard read {"board":"general"}` through a control call):
   the item must hold `public.utf8-plain-text` = `1234.5` and `public.html` = `<span>1234.5</span>` (English) or
   `<span>1234,5</span>` (with Decimal set to Icelandic). The second app must see `changed` for `general`.
6. Paste into a text field in another app: the plain digits appear, as before.
7. Optional fallback check: on a shell without the service, Copy still fills the plain clipboard (paste to confirm).
8. `bifrost pc midget screen /tmp/calc-copy.png`. No UI change in this round, so the check is functional only.

Host tests (for the lead, once the build runs): `cargo test -p bifrost-calculator` in `springboard/` covers
`copy_service::tests` (item representations, HTML escaping, the one-write message to `pasteboard`, and the copy taken once).

## REA evidence

The model and the rules are the ones in `docs/parity/wire-svc-pasteboard.md` (the NSPasteboard pages it cites:
`ev_a56d232a88893f8c75a787655b47c8fcee754bfbfca75a880b0ce1fff09c3d10`). No new page was relied on for this wiring.

## Gaps

- **No number type.** The pasteboard model has no number UTI. "The number" is the plain-text digits, and the HTML
  representation carries the localized text. A consumer that wants a typed number must parse the text.
- **Flat general board.** The shell's general board keeps one representation per item. The shell publishes the
  text-like type (plain text first) to the Wayland selection; whether the HTML representation survives on the general
  board depends on the shell (see the wire note). The plain text representation is first so it is the one that
  survives.
- **Paste still uses the plain clipboard.** Paste does not read the pasteboard service, so the synchronous path stays
  in use. Reading the service would make paste asynchronous and was not needed for this round.
- **No reaction to other apps' copies.** The app does not subscribe to the `changed` event; the calculator does not
  show clipboard state, so there is nothing to refresh.
- **No access prompt.** Any app can read and write the general board (no access behaviour in the service yet).
- **Fallback depends on the error text.** Any error reply triggers the plain clipboard fallback; a service that
  accepted the write and then failed silently would not.
- **Not built or run.** The branch was written without a build. The lead's build and the Midget run are the first checks.
