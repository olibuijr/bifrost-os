# Changelog

## 0.6.4

- The window follows the Golden Gate Dictionary: a sidebar of the matching headwords under the traffic lights (the word on screen selected), a top bar with the title and how many were found, Back and Forward and a text size pull-down in the kit toolbar, and the kit search field at the top right. The Back and Forward buttons no longer sit under the traffic lights.
- The entry shows the headword large, with its pronunciation when the dictionary gives one, the part of speech in grey and the senses as plain text. Only the terms of cross-reference lines (Synonyms, See also, Compare) that a dictionary defines are links, not every word of the definition.
- A window narrower than 560 pt has no sidebar and lists the matches in the content column; File > New Window opens at 720 by 560 pt.

## 0.6.3

- Rebuilt with the current renderer and SDK: Chinese, Japanese and Korean text draws whole, and a file opened with this app from Files or the Dock opens as a document.

## 0.6.2 — 2026-10-23

- Clicks in extra windows land under the pointer on scaled screens.

## 0.6.1

- BifrOSt 0.10.0 round (waves 14 and 15): see the system CHANGELOG for this app's changes.

## 0.6.0 — 2026-10-17

- Keys, Copy and Paste go to the key window, so a second window takes its own typing and Cmd+C copies from it; Cmd+W closes the window it is typed in (`window_input`, `extra_event`).
- The stored choices (dictionaries, order, labels, text size) are read back before each change and then saved, so one window's change no longer overwrites another's (`change_prefs`).

## 0.5.5 — 2026-10-17

- File > New Window (cmd+n) opens a second window with its own lookup, history, scope and find. Touch works in every window; the keyboard and the menus stay with the main window.

## 0.5.4 — 2026-10-14

- Add the generated app compliance harness.

## 0.5.3 — 2026-10-14

- Export the Dictionary icon at the store's required 180 × 180 size.

## 0.5.2 — 2026-10-14

- Run printing on the shared utility queue and declare sudden termination safe when the app configures.

## 0.5.1 — 2026-10-14

- Use the shared extra-small spacing token for suggestion label fitting.

## 0.5.0

- The model is a library target (`src/lib.rs`), so its behaviour is tested through the public API in `tests/parity_app-dictionary.rs`: history, scope, preferences, text size, find, cross-reference words, settings drag order, the copy and print text, the folder URL and the dict: and x-dictionary: URLs.
- The Dictionary matrix rows cite the test file; Services, New Window and the Midget checks stay open.

## 0.4.0

- Copy Definition and Print send the same text: each dictionary's name above its definition, from one tested routine.
- Open Dictionaries Folder percent-encodes the folder path, so folders with spaces open correctly.
- Headless tests cover the definition text and the folder URL in model.rs.

## 0.3.0

- The manifest registers the dict: and x-dictionary: URL schemes, so the shell offers Dictionary for those links.
- Find in Definition highlights each found word instead of the whole line, and Find Next/Previous step through every occurrence; the count is of occurrences.
- Words in a definition that have a definition of their own are drawn as links.
- Settings: drag a dictionary row to reorder the dictionaries (Up and Down stay).

## 0.2.0

- Back and Forward history (toolbar, Go menu, cmd+[ and cmd+]), a scope bar with Select Next/Previous Dictionary, and tapping a word in a definition to follow it.
- Find in Definition (cmd+f, cmd+g), Print (cmd+p), Open Dictionaries Folder, and dict: / x-dictionary: links.
- Text size buttons and View menu; a settings page to turn dictionaries on or off, reorder and rename them.

## 0.1.0

- New app: Dictionary. Reads StarDict dictionaries (.ifo, .idx, .dict) from the app's dictionaries folder and the system's stardict folder.
- Suggestions as you type, Return to look up, Look Up Clipboard, Copy Definition, and a scrolling list of every dictionary's definition.
- Look Up intent returns a definition to other callers. A word in the system word list with no definition is reported as such.
