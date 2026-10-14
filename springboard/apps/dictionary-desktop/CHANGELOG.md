# Changelog

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
