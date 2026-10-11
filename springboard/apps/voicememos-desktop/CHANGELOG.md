# Changelog

## 1.2.3 — 2026-10-11

- The recordings sidebar is published to accessibility as a list (`Role::List`, "Recordings", with the recording count)
  even when it has no rows, so the empty state also exposes the source list's semantic control.
- Compliance: the first screen publishes its source list at 1200x780 without any recordings.

## 1.2.2 — 2026-10-11

- Recordings sidebar is the Golden Gate source list (`widgets::source_list`): rows, selection and scrolling come from the
  kit, and the column uses the kit sidebar width (Layout fraction, 180 to 280 pt).
- "All Recordings" is the kit large title; the fixed waveform and control positions are named tokens.
- Rename buttons, transport and record controls are published to accessibility with labels.
- Corner radii follow the prefs layout (card 13 pt for the rename field, button radius scaled to points).
- Unused icon module removed.

## 1.2.1

- Follows a Light/Dark switch immediately, including while the window is in the background.

## 1.2.0 — 2026-10-10

- Golden Gate window: full-size, with the traffic lights in the recordings sidebar and a unified toolbar over the
  detail pane holding Rename and Delete (Confirm Delete while asking).
- The sidebar uses the system sidebar colour and a neutral selection plate with a bold title; the waveform sits on a
  grey band and a live recording draws in red.
- Follows Light and Dark: every background, plate, text and hairline uses the system colour pairs.
- Toolbar buttons are published as controls, so the window does not drag from them. Needs system 0.4.84.

## 1.1.3 — 2026-10-10

- Opens again on BifrOSt 0.4.90 and later: rebuilt against the current SDK, whose app socket is `BIFROST_SOCK`.

## 1.1.2 — 2026-10-09

- Spacing follows the system 4-pt grid: 11 padding and gap values moved to the nearest standard step (at most 2 pt).
- 8 colours now use the system colour tokens (same values).
- Built for system 0.4.62.

## 1.1.1 — 2026-10-09

- Text that is shortened to fit now measures at the size it is drawn at, so labels are no longer cut off early.
- Follows the system accessibility settings (Increase Contrast, Reduce Transparency, Show Borders, Bold Text) on system 0.4.61.
- The recordings sidebar uses the full-height shaded column of macOS Golden Gate.

## 1.1.0 — 2026-10-09

- Text sizes and spacing come from the shared macOS-style scale, so this app matches the others.
- No gap above the content for a status bar the desktop does not have.

## 1.0.0 — 2026-10-08

- Voice Memos for the BifrOSt desktop: the tablet app's code, hosted in a window with keyboard and pointer input.

- Use the desktop package identity for the SDK handshake and supported PC permissions.
- Connect to the Linux audio service path.
- Keep text-field focus without drawing an on-screen keyboard in PC app processes.
- Support renaming recordings with the physical keyboard.
