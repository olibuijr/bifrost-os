# Changelog

## 0.4.0

- Custom colours: Colour, Custom Colour... opens a bar for a `#RRGGBB` value with a live swatch; Return applies it. A custom colour is saved with the note, undoable, and carried to new notes by Use as Default.
- Make New Sticky Note in the Services menu now creates a note from the selected text, also when Sticky Notes is not running.
- Edit, Speech: Start Speaking reads the selection (or the whole note) aloud through the system speech engine (espeak-ng), Stop Speaking ends it. Nothing speaks unless asked.
- Edit, Replace and Replace All: Tab in the find bar switches between the find and the replace field, Return replaces the selected match and moves to the next. One undo step each.
- Window, Arrange By, Location: lays the notes out in reading order by where they sit now.

## 0.3.0

- A caret and a selection in every note: typing goes in at the caret, click to place it, drag to select, double-press to select a word, arrow keys, Home, End and forward Delete. Cut, Copy, Paste and Select All work on the selection.
- Edit menu: Undo and Redo of every change (named, for example "Undo Arrange"; a run of typing is one step), Paste and Match Style, Find (Cmd+F) with Find Next, Find Previous and Use Selection for Find across all notes, Open Link, and Transformations (upper case, lower case, capitalise).
- Web addresses in a note are underlined; double-press one, or choose Open Link, to open it.
- File > Print prints the focused note. Delete Sticky Note asks first when the note has text.
- Font menu: Underline, Strikethrough, Bigger and Smaller (Cmd+= and Cmd+-), text colour, Copy Style and Paste Style, and Use as Default for the colour and style of new notes.
- Window menu: Collapse All, Expand All, a Translucent toggle per note, Zoom, Arrange By colour, content or date (with undo), and the focused note's created and modified dates. A double press on a title bar collapses or expands the note.

## 0.2.0

- Duplicate Sticky Note (cmd+d): the copy sits offset from the original and takes focus.
- Escape leaves the focused note, so typing no longer goes into it until a note is pressed again.

## 0.1.0

- New app: Sticky Notes. Floating notes in six colours, collapse to the title bar, Bold and Italic, three text sizes.
- Notes save to the app's data directory on every change and restore on launch.
- Inactive notes are translucent (shared visual effect model).
- Import and export as plain text.
