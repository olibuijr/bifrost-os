# Changelog

## 0.6.10

- Rebuilt for 0.10.44: every touch says whether a mouse or a finger made it, so a mouse drag no longer scrolls lists on the desktop (the wheel, trackpad and scroll bar still do).

## 0.6.9

- The sidebar shows a hint before the first search.

## 0.6.8

- Rebuilt with the 0.10.40 SDK: a background app redraws at its new size after a resize.

## 0.6.7

- The search field edits like a standard text field: Select All (Cmd+A) selects the word so typing replaces it instead of appending, Left, Right, Home and End move the caret, Delete removes the character after it, and Cut, Copy, Paste, Undo and Redo act on the field (Copy with no selection still copies the definition).

## 0.6.6

- The sidebar lists every match again: the rows that are not selected were invisible under the rows clip (their vibrant text blended into nothing), so only the selected word showed.
- Up and Down move the sidebar's selection and show that word, scrolling its row into view.
- The sidebar's rows scroll no further than their end; a list that fits the column does not scroll.

## 0.6.5

- Typing a word selects the first match in the sidebar and shows its entry at once, as the reference does; Return chooses it. The search field keeps what was typed.
- The sidebar's rows are clipped below the traffic lights' band, so a scrolled list no longer draws under them.
- The subtitle under the title reads "151,378 words" instead of a dictionary count that was cut off.
- A word asked for while Dictionary is not running (a Spotlight dictionary hit, Open in Dictionary, the Services menu) is shown once the app has started, instead of an empty search. The app also takes a word or a `dict:` URL as its launch argument.

## 0.6.4

- The window follows the Golden Gate Dictionary: a sidebar of the matching headwords under the traffic lights (the word on screen selected), a top bar with the title and how many were found, Back and Forward and a text size pull-down in the kit toolbar, and the kit search field at the top right. The Back and Forward buttons no longer sit under the traffic lights.
- The entry shows the headword large, with its pronunciation when the dictionary gives one, the part of speech in grey and the senses as plain text. Only the terms of cross-reference lines (Synonyms, See also, Compare) that a dictionary defines are links, not every word of the definition.
- A window narrower than 560 pt has no sidebar and lists the matches in the content column; File > New Window opens at 720 by 560 pt.
- Cross-reference links stay on their terms when a line breaks after a hyphen, between Chinese, Japanese or Korean characters, or inside a long word.
- Choosing a word in the sidebar keeps the list of matches for the search. The sidebar scrolls with the wheel and by dragging when the matches do not fit, and keeps the selected word in view.

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
