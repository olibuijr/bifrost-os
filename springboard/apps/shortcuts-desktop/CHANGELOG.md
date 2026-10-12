# Changelog

## 1.5.0 — 2026-10-12

- The Add Action list has an Intents section. It lists the actions other apps can ask Shortcuts to do, each with its name and a one-line description.
- Tapping an intent in that list runs it on the shortcut you are editing. Its result shows at the bottom of the window: the steps for "Plan Shortcut", or the reason for "Run Shortcut" (this device cannot run shortcut actions yet).
- Nothing else changes; the grid, the editor and the scripting and control socket behave as before.

## 1.4.1 — 2026-10-12

- Dark appearance: the main content area fills with the dark content grey instead of pure black, as stock dark content views do; sidebars and cards keep their colours.

## 1.4.0 — 2026-10-12

- The shortcut picker results are built from one typed list: each shortcut has its position as its id, its name, its colour and its step count, so searches from other apps read the same values as the window.
- The shortcut in front in the window (an open editor or picker) is the one used when "Run Shortcut" or "Plan Shortcut" is asked without a shortcut. A grid with no editor open still asks for one.
- "Plan Shortcut" works out its steps on a separate worker thread, so the window never waits for a plan.
- Shortcut parameters are optional in the intent declaration; a request with no shortcut and nothing in front says that a shortcut is required.
- Needs system 0.4.84, as before.

## 1.3.0 — 2026-10-12

- Shortcuts can be used from other apps and from automation. A shortcut is found by its id or its name; a name that two shortcuts share asks for the id, and a name that matches none says so.
- "Plan Shortcut" lists a shortcut's steps in order without running them. "Run Shortcut" finds the shortcut and then says that this device cannot run shortcut actions yet, so it never reports a run that did not happen.
- A scripting view of the list: a shortcut's name and colour can be read and changed, and its action count is read-only. Changes are checked, saved, and an open window shows them at once.
- A shortcut can have a daily reminder: the "Daily Reminder" row in its editor steps through a few times (07:00 to 22:30) and then Off. At that time the normal notification appears, and it says that this device cannot run shortcut actions yet.
- "Open at Login" in All Shortcuts asks the desktop to start Shortcuts at login. The row shows Needs Approval until the user allows it in Settings > General > Login Items.
- Needs system 0.4.84, as before.

## 1.2.2

- Compliance: grid tile sizes and the Remove badge's glyph come from named tokens; the badge's glyph bar is a capsule sized from the spacing token.

## 1.2.1

- Follows a Light/Dark switch immediately, including while the window is in the background.

## 1.2.0 — 2026-10-10

- Golden Gate layout: a Library sidebar (All Shortcuts with its count) beside the content in a full-size window, and a unified toolbar with the title, New Shortcut, Edit/Done, Run and Back in place of the iPad navigation bar and large title.
- Light and Dark follow the system appearance; colours come from the named system tokens.
- Toolbar buttons are published as controls so they stay clickable in the window's drag band. Needs system 0.4.84.

## 1.1.3 — 2026-10-10

- Opens again on BifrOSt 0.4.90 and later: rebuilt against the current SDK, whose app socket is `BIFROST_SOCK`.

## 1.1.2 — 2026-10-09

- Spacing follows the system 4-pt grid: 10 padding and gap values moved to the nearest standard step (at most 2 pt).
- Built for system 0.4.62.

## 1.1.1 — 2026-10-09

- Text that is shortened to fit now measures at the size it is drawn at, so labels are no longer cut off early.
- Follows the system accessibility settings (Increase Contrast, Reduce Transparency, Show Borders, Bold Text) on system 0.4.61.

## 1.1.0 — 2026-10-09

- Text sizes and spacing come from the shared macOS-style scale, so this app matches the others.
- No gap above the content for a status bar the desktop does not have.
- Fixes the coloured fringes on the action-count badges of the shortcut cards.

## 1.0.0 — 2026-10-08

- Shortcuts for the BifrOSt desktop: the tablet app's code, hosted in a window with keyboard and pointer input.

- Use the desktop package identity for the SDK handshake and supported PC permissions.
- Keep text-field focus without drawing an on-screen keyboard in PC app processes.
- Show when a shortcut cannot run instead of indicating successful execution.
