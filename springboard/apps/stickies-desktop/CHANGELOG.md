# Changelog

## 0.8.3 — 2026-10-14

- Use shared radius and spacing tokens; use the named white color for text on dark custom notes.

## 0.8.2

- Standard editing commands now come from the shell Edit menu; app specific text actions are in Text. Added the SDK and design compliance test harness.

## 0.8.1

- Exported the app icon at the required 180 x 180 pixel size.

## 0.8.0

- File > Import Text keeps the list items of a rich text file: each item starts with its marker (a bullet or a number), typed in the default style, so the list reads the same in the note.

## 0.7.0

- Edit > Whole Word (off by default, kept across launches) makes Find, Find Next, Replace and Replace All match only whole words, so "cat" does not match inside "category".
- File > Paper Size and File > Orientation are kept across launches, not only for the session.
- Black text stays readable on a dark custom note colour: it turns white on such a colour, and keeps black on light ones.

## 0.6.0

- Edit > Ignore Case (on by default, kept across launches) makes Find, Find Next, Replace and Replace All match letters of either case, or only the same case when it is off.
- Edit > Replace All in Current Note replaces every match in the focused note and leaves the other notes alone. It is one undo step.
- File > Import Text keeps the text colours of a rich text file: red and orange text becomes Red, blue and green keep their colour, and yellow or any other colour becomes Black.
- File > Paper Size (A4, Letter, Legal, A5, A3) and File > Orientation (Portrait or Landscape) choose how Print lays out the note. The choice lasts for the session.

## 0.5.0

- Edit > Replace... opens the find bar with a replace field (Tab switches between the two). Return replaces the current match and steps to the next; Edit > Replace All replaces every match in every note.
- Font > Outline rings the letters of a note in the text colour. Font > Baseline sets superscript, subscript or the default position.

## 0.4.0

- Window > Arrange By Location orders the notes by where they sit now (top to bottom, then left to right) and lays them out in rows.

## 0.5.0

- Styles apply to a selection. Bold, Italic, Underline, Strikethrough, Outline and Text Colour change only the selected text (or the whole note when nothing is selected), and typed text takes the style before the caret. Each run is drawn with its own weight, slant, outline and decorations.
- Font > Outline draws the letters ringed in the text colour. Font > Baseline sets Superscript or Subscript, and Font > Kern and Font > Ligature turn kerning and ligatures off or on for the run.
- Italic text is drawn leaning, not only marked.
- Copy Style and Paste Style take the style at the caret and keep the size of the note they land in.
- Spelling and Grammar: Check Spelling While Typing marks misspelled words with a wavy rule, Check Document Now selects the next one, Correct Spelling replaces the word at the caret, Learn Spelling and Ignore Spelling act on it, and Correct Spelling Automatically fixes a word when a space or punctuation follows it. Spelling and Grammar switches are kept across launches.
- Substitutions: Smart Quotes turns typed quotes into curly quotes, and Smart Dashes turns a second typed hyphen into an em dash.
- Data Detectors: Dates, Addresses and Phone Numbers are marked with a dotted rule. Add Link inserts a typed web address, which the note underlines.
- Command arrows: Cmd-Left and Cmd-Right move to the ends of the line, Cmd-Up and Cmd-Down to the ends of the text, and Shift with them extends the selection.
- File > Import Text reads rich text files: bold, italic, underline and strikethrough are kept.
- Shift+Cmd+Y makes a new sticky note from the selected text (the Make New Sticky Note service).

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
