# Changelog

## 1.6.10

- Rebuilt with the 0.10.40 SDK: a background app redraws at its new size after a resize.

## 1.6.9

- Rebuilt with the current SDK: the app redraws after an action from the shell (Look Up, Services, Spotlight) and accepts the shell's account-credential connections.

## 1.6.8

- Rebuilt with the current SDK and renderer: shell actions such as Look Up and Services reach the app, and vibrant text draws inside clipped areas.

## 1.6.7

- Rebuilt with the current renderer and SDK: Chinese, Japanese and Korean text draws whole, and a file opened with this app from Files or the Dock opens as a document.

## 1.6.6 — 2026-10-20

- Nothing else changed: this release only rewords the notes for the new app icon.

## 1.6.5 — 2026-10-19

- The app has a new icon.

## 1.6.4

- BifrOSt 0.10.0 round (waves 14 and 15): see the system CHANGELOG for this app's changes.

## 1.6.3 — 2026-10-14

- Translate Article and Share Article are in an Article menu, avoiding a duplicate shell Edit menu.

## 1.6.2 — 2026-10-12

- Rebuilt on the BifrOSt 0.6.4 SDK: the shared shell/app wire codec, frame and list caps, and the window materials API.

## 1.6.1 — 2026-10-12

- Removed the unused procedural icon and the unused state helpers (describe, needs_frame, online).

## 1.6.0 — 2026-10-12

- Search: a search field sits at the trailing end of the toolbar. Typing filters the list to the headlines that contain the text (ignoring case), in Today, the Reading List and each feed. Escape or the clear button empties it, and "No matching articles" shows when nothing matches. The on-screen keyboard is asked for only while the field has focus.

## 1.5.0 — 2026-10-12

- Faster feed refreshes: News keeps the feeds and pictures it has loaded and asks the server only whether they changed (with the ETag and Last-Modified values it was sent). A copy that the server says is still fresh is used without a request, and an unchanged feed answers "not modified" with no new download. Nothing is kept for responses the server marks as not to be stored.
- Share Article: Edit > Share Article sends the open article's web address to the system's share menu. It is greyed out until an article with a web address is open.
- Larger Text and Smaller Text in the View menu change the size of the article's body text in five steps. The title keeps its size, and the size stays the same when you open another article.

## 1.4.0 — 2026-10-12

- Translate Article: Edit > Translate Article translates the open article's title and preview into the other of English and Icelandic. It is greyed out until an article is open. BifrOSt does not ship translation language packs yet, so for now News says that the languages are not installed on this PC; articles in other languages say that the source language is not supported.
- The reader's background is the content colour (white in Light, dark grey in Dark) instead of the pure black used for the window's frame.

- Reading List: a new Reading List row in the source list. The toolbar button saves the open article to it (or removes it again); the list is kept in the app's data folder and survives a restart. Pages without a web address cannot be saved and show the button dimmed.
- Each article shows its language (for example Icelandic or English), word count and reading time under the source and date. The language is only shown when the text has recognisable letters.
- The link to the original page is now a card with the article's title and the feed's name, in place of the bare address.
- Offline reading: News asks the system whether the network is up and follows changes. Offline it stops fetching feeds and pictures, keeps showing the saved headlines and articles, and says "You are offline" when there is nothing saved to show. Coming back online refreshes the feeds at once.
- Widget timeline: the top four headlines (Today's order) are given to the system's widget area, one after another every 30 minutes. The timeline is refreshed whenever the headlines change or the system asks for it.

## 1.2.2 — 2026-10-11

- Compliance: feeds and pictures load on the shared dispatch pool (`bifrost_app::dispatch`) instead of a worker pool with a poll loop; the loop sleeps until the next feed is due and a finished job wakes it through the main queue.
- The app enables sudden termination (it keeps no unsaved state), so the shell can quit it at once.
- The source list rows (Today and each feed) are published as accessibility controls, and each can be activated.
- Corner radii come from the Settings card and plate tokens; the icon colours are named artwork constants.

## 1.2.1

- Follows a Light/Dark switch immediately, including while the window is in the background.

## 1.2.0 — 2026-10-10

- Looks like News on the Mac: a full-size window with a source list (Today, then each feed under Sources) in place of the tab strip, and a unified toolbar with Back and Refresh in place of the iPad navigation bar.
- Follows Light and Dark: every colour is a system Pair token; the toolbar buttons are published as controls.
- Needs system 0.4.84.

## 1.1.3 — 2026-10-10

- Opens again on BifrOSt 0.4.90 and later: rebuilt against the current SDK, whose app socket is `BIFROST_SOCK`.

## 1.1.2 — 2026-10-09

- Spacing follows the system 4-pt grid: 3 padding and gap values moved to the nearest standard step (at most 2 pt).
- Built for system 0.4.62.

## 1.1.1 — 2026-10-09

- Text that is shortened to fit now measures at the size it is drawn at, so labels are no longer cut off early.
- Follows the system accessibility settings (Increase Contrast, Reduce Transparency, Show Borders, Bold Text) on system 0.4.61.

## 1.1.0 — 2026-10-09

- Text sizes and spacing come from the shared macOS-style scale, so this app matches the others.
- No gap above the content for a status bar the desktop does not have.

## 1.0.0 — 2026-10-08

- News for the BifrOSt desktop: the tablet app's code, hosted in a window with keyboard and pointer input.

- Use the desktop package identity for the SDK handshake and supported PC permissions.
- Keep text-field focus without drawing an on-screen keyboard in PC app processes.
