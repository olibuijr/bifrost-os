# Wave 5: User Notifications (gap rows)

Two gap rows of the User Notifications framework are built as pure Rust models. Neither is wired into the SDK
(`springboard/sdk/src/usernotif.rs`) or the shell (`springboard/src/shell/usernotif.rs`, `springboard/src/shell/notify/`)
yet, and neither was built or run: this wave ran no cargo, rustc or tests. The matrix stays with the lead.

## What was built

- `springboard/ui/src/notifservice.rs`: the content steps of a notification service extension. An `Extension` holds an
  ordered list of steps, each editing a copy of a `Content` (title, subtitle, body, thread, category, badge, custom
  data). `deliver` runs the steps within a time budget read from a clock. A failing step drops its own edits and the
  content before it is delivered. When the budget runs out before a step, the content so far is delivered, the same
  fallback the official page describes for an extension that does not finish in time. Helper steps: `truncate_body`
  (at most N characters, ellipsis last) and `set_info` (set or replace a custom-data key).
- `springboard/ui/src/notifcomm.rs`: the presentation of a communication notification. A `Message` (conversation,
  sender, text) yields a `Presentation`: sender name as title, group name as subtitle for a group only, thread
  `conversation:<id>` so one conversation's messages group together, and up to two upper-case initials of the sender
  as the avatar. Also `summary_text` ("1 message from X", "3 messages from X") and `validate` (empty conversation id or
  sender name is refused).

Both modules are registered with one `pub mod` line each in `springboard/ui/src/lib.rs`.

## Official API names mirrored

- `UNNotificationServiceExtension`, `didReceive(_:withContentHandler:)`, `serviceExtensionTimeWillExpire()`
  (https://developer.apple.com/documentation/usernotifications/unnotificationserviceextension)
- `UNNotificationContent` fields `title`, `subtitle`, `body`, `userInfo`, `badge`, `threadIdentifier`,
  `categoryIdentifier` (https://developer.apple.com/documentation/usernotifications/unnotificationcontent)
- Implementing communication notifications, participants and thread display
  (https://developer.apple.com/documentation/usernotifications/implementing-communication-notifications)

## REA evidence

- User Notifications landing page: `ev_d40679d065654af80ade48a75520649373430bdffcbb2cbb48981091f1931583`
- UNNotificationServiceExtension: `ev_19718e55334b997aaf1d77d453fec6c82d222308150c527dfcd5ae47b7694e49`
- Implementing communication notifications: `ev_38a1f696869a787f329aa7343082a47481686720cb403f746e02fd51cb1c74c0`

The service extension page states that the system shows the original content when the extension does not update it in
time, which is the fallback `deliver` mirrors.

## Matrix rows closed

| Framework | Feature | New status | BifrOSt path |
|---|---|---|---|
| User Notifications | Background push and notification service extension | partial | springboard/ui/src/notifservice.rs |
| User Notifications | Communication notifications | partial | springboard/ui/src/notifcomm.rs |

## Not built

- Remote push (device token, APNs-style registration and background wake-up) needs a push service and the network;
  the rules for this wave rule it out.
- The service extension has no delivery hook yet: nothing in the shell calls `Extension::deliver` before a banner is
  drawn, and no SDK call registers steps.
- Communication notifications have no SDK call: an app cannot pass a `Message` to the shell, and the shell draws
  no sender avatar or group subtitle.

## Tests

`notifservice.rs` has 7 tests: ordered steps, a failing step's edits dropped, budget expiry partway through, a zero
budget, an empty chain, `truncate_body`, and `set_info` replace-or-append. `notifcomm.rs` has 6 tests: direct and
group presentation, thread sharing, initials, summary text and validation. None have been run (no builds this wave).
