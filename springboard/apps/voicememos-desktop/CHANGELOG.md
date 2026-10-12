# Changelog

## 1.6.0 — 2026-10-12

- Split at Playhead: the Recording menu has a new item that cuts the take that is playing into two recordings,
  "<title> part 1" and "<title> part 2", at the point it has reached. The original is kept. Playback stops, and the
  first part is selected. The item is greyed out unless the selected take is playing, and a split too close to either
  end (under a tenth of a second) is refused with a message.
- Skip Silence: a check mark in the Recording menu. When it is on, playback jumps over the quiet parts of a take and
  plays only the speech. A take with no speech plays as before. Turning it on during playback takes effect at once.
- Recording quality, playback speed and enhanced levels are not in this version: the audio service still records one
  fixed format and does not take a quality setting.

## 1.5.0 — 2026-10-12

- Input meter: while you record, a bar under the timer shows the microphone level on a decibel scale, with the level
  read out in dB ("-6 dB"), or "Silent" when nothing is coming in. The bar is blue, as the app's one tint.
- Playback volume: a volume slider sits under the playback buttons. Drag it, or press a point on it, to set the output
  level. It is the same level Control Center shows, and it follows the level when that changes elsewhere.
- The ring around the record button is now the content grey, so it matches the window in Dark appearance.
- Recording quality and playback speed are not in this version: the audio service does not take a sample rate or a
  playback speed yet.

## 1.4.1 — 2026-10-12

- Dark appearance: the main content area fills with the dark content grey instead of pure black, as stock dark content views do; sidebars and cards keep their colours.

## 1.4.0 — 2026-10-12

- Recording menu: a new menu between the app menu and the standard menus with Trim, Notes, Rename and Export to Downloads.
  The items are greyed out when no recording is selected or while one is being recorded or edited.
- Export to Downloads copies the selected recording as a WAV file into the Downloads folder (numbered when the name is
  taken). The recording itself is left as it was.
- Notes: a Notes button in the toolbar opens a notes card for the selected recording, a multi-line text view with a
  caret, selection by dragging, arrow keys and Enter for a new line. Done saves the notes beside the recording; Cancel
  discards the changes. Notes follow the recording when it is renamed and are removed with it.
- The arrow keys move the caret in the notes card.

## 1.3.0 — 2026-10-12

- Trim a recording: the new Trim button in the toolbar opens a trim view over the waveform. Drag the start or end handle
  (the nearer one moves where you press). Done saves the kept part as a new recording named "<title> trimmed"; the
  original is left as it was. Cancel discards the change. Playback stops while you trim.
- Recordings that are too short to trim say so instead of opening an empty trim.
- The detail subtitle shows the recording's format: sample rate and channels, for example "8 kHz mono".
- Microphone permission: Record checks the microphone permission first. When it is turned off or restricted, the app says so
  and does not record. A line under the record button shows the microphone state and the input device.
- A recording that is taken while the permission is switched off stops with a message.
- Input device: the app shows the default input's name and says when the input changes during a recording.
- Now Playing: playing a recording shows it on the media tile, with its title and length. Play, pause, skip 15 seconds
  and seek work from the tile and the keyboard media keys, and the tile is removed when playback ends.

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
