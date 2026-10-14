# Changelog

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
