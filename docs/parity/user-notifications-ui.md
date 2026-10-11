# User Notifications UI: parity wave 2

Framework: https://developer.apple.com/documentation/usernotificationsui (matrix slug `user-notifications-ui`).

This round builds two pure models in `springboard/ui/src` (bifrost-ui). Neither is wired into the shell or the SDK
yet, so both rows move to `partial`, not `parity`.

## What was built

- `springboard/ui/src/notifmedia.rs`: the play and pause button of a notification card. It holds the frame (card
  points, pulled inside the card by `fitted_frame`), an optional RGB tint, the shown flag and the playing state.
  `media_play` and `media_pause` set the state; `glyph` returns play or pause; `tap` returns the command the app
  should run (the opposite of the current state) and does not flip the state itself, so the glyph only changes when
  the app confirms with `media_play` or `media_pause`.
- `springboard/ui/src/notifattach.rs`: a notification attachment. `Attachment::new(id, grant, options)` checks the
  identifier, the document grant, the clip rectangle (inside the unit square, positive size) and the kind. A type
  hint wins over the file extension. `thumbnail_rect(width, height)` gives the thumbnail inside an image, or `None`
  when the thumbnail is hidden. The file is named by a grant, never a path, as in the share sheet.

Both modules have `#[cfg(test)] mod tests` (7 tests in notifattach.rs, 6 in notifmedia.rs) and are registered in `springboard/ui/src/lib.rs`.

## Official API names mirrored

- `UNNotificationContentExtension`: `mediaPlayPauseButtonFrame`, `mediaPlayPauseButtonTintColor`, `mediaPlay()`,
  `mediaPause()`. The page lists `mediaPlayPauseButtonType` and the enum `UNNotificationContentExtensionMediaPlayPauseButtonType`,
  but the enum's case names were not retrieved (the DocC subpage did not resolve), so the model does not use them.
  It uses a `show`/`hide` flag instead.
- `UNNotificationAttachment`: `init(identifier:url:options:)`, `identifier`, `url` (as a grant), `type` (kind),
  and the options `UNNotificationAttachmentOptionsTypeHintKey`, `...ThumbnailHiddenKey`, `...ThumbnailClippingRectKey`,
  `...ThumbnailTimeKey`.

## REA evidence

- `ev_a940e67d73b043d40b9fce60019562a2c84cc56e7a8898fc8f00fabab81d3b3d`: UNNotificationContentExtension (rendered page,
  media playback topics and the note that the system blocks touch events on the extension's view).
- `ev_0177744ebe43676d9fabadcc697d1257d6de9e9a469caee04cdf91d67df2fae8`: UNNotificationAttachment (rendered page: the
  file must be on disk, the system validates attachments, thumbnail clipping and frame options, per-app storage limit).

Supported file types and maximum sizes are in the page's Table 1. The rendered text extraction did not include the
table, so this round does not model size limits and the accepted extension list in `notifattach.rs` is BifrOSt's own
(png, jpg, jpeg, gif; mp3, m4a, wav, aiff, caf; mp4, mov, m4v). It has not been checked against Table 1.

## Matrix rows closed

| Framework slug | Feature (as in the data file) | New status | BifrOSt path |
|---|---|---|---|
| user-notifications-ui | Media play and pause in notifications | partial | springboard/ui/src/notifmedia.rs |
| user-notifications | Notification attachments and rich media | partial | springboard/ui/src/notifattach.rs |

## Still open

- Wiring: `ToShell` and `NotifRequest` (springboard/proto/src/rich.rs) carry no attachments and no media control.
  The shell's cards (springboard/src/shell/notify/list.rs, banner.rs) draw no thumbnail and no button.
- Tint is RGB only; the card's default tint and the appearance pairs are not connected.
- The app-supplied notification view (UNNotificationContentExtension view controller) and the response options
  (UNNotificationContentExtensionResponseOption) remain gaps.
- No size limits, no per-app attachment storage quota.
