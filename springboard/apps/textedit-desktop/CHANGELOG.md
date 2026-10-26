# Changelog

## 1.13.12

- The save sheet takes Cmd+D for Don't Save, and Tab shows a focus ring that Space presses.

## 1.13.11

- The Window menu lists each document once, Open Recent brings an open document forward, File has one Print, the menu bar says TextEdit, and a text file dropped on the window opens.

## 1.13.10

- The red close button on a second document window asks about that document, like Cmd+W.

## 1.13.9

- The red close button asks before closing an edited document.

## 1.13.8

- An unsaved Untitled document autosaved at Quit opens again, still edited, at the next launch; the Preferences menu is now Settings.

## 1.13.7

- Rebuilt with the 0.10.40 SDK: a background app redraws at its new size after a resize.

## 1.13.6

- Closing the last document closes its window too, and Text Editor keeps running with no window, as the reference does (it used to leave an empty, disabled window). Clicking its Dock icon, choosing it in the app switcher or opening it again shows a new Untitled; File > New or a file opened meanwhile brings the window back with that document (with the matching system update).

## 1.13.5

- Undo of Make Plain Text makes the document rich text again: the format bar, the ruler and the formatting commands come back with the fonts. Redo makes it plain text again.
- The Make Plain Text confirmation is an alert sheet attached under the title bar, like the save confirmation, with OK (the default) stacked above Cancel.
- Closing the main window's document now draws the other document where its window was: the system moves the window on screen, not only in its records (with the matching system update).

## 1.13.4

- Format > Make Plain Text asks first, as the reference does: "Convert this document to plain text?", with OK and Cancel (it used to convert at once unless the document had attachments).
- A converted document takes the fixed-pitch plain text font instead of keeping Helvetica.
- Undo of Make Plain Text also restores the rich text default font, so text typed afterwards is no longer in the plain font.
- A plain text file opened from disk uses the plain text font too, so every plain document has one font.
- The save confirmation is a sheet attached to the window's title bar, with full-width stacked buttons.
- Closing the main window's document keeps the other document where its window was: the main window moves there instead of the document jumping into the main window's frame.

## 1.13.3

- A second document window shows one title row, like the first: it opens with full-size content, so the system draws no title bar over the window's own.
- The save confirmation is a sheet hanging from the title bar: the whole document name shows (the title wraps), and Save is the blue default button that Return presses.
- Plain text documents show no format bar; the page starts under the title row.
- The window reports where its caret is, so Look Up opens under the selection instead of under the menu bar.

## 1.13.2

- Each document has its own window: File > New (and Duplicate, or shared text) opens a second window and keeps the first; a file opened while the front window holds an empty Untitled takes its place; closing a document closes its window.
- The Services submenu lists the services for the selected text, and Look Up with only a caret defines the word at the caret: the document reports its selection to the system as text fields do.
- Closing an edited document, with File > Close or the window's close button, asks first: a sheet offers Don't Save, Cancel and Save, and the document closes only once it is answered. The close button used to take the window away and bring the document straight back.

## 1.13.1

- Rebuilt with the current renderer and SDK: Chinese, Japanese and Korean text draws whole, and a file opened with this app from Files or the Dock opens as a document.

## 1.13.0 — 2026-10-23

- Fonts (Cmd+T) and Colors panels; text draws in its chosen family; all installed families are listed.
- Find highlights matches live and Cmd+A works in its fields; the Format menu follows the reference with Font and Text submenus.
- The page and the Fonts panel scroll with the wheel; the Edited mark clears after autosave.

## 1.12.2 - 2026-10-20

- Cut, Copy, Paste, Select All, Undo and Redo from the keyboard act on the Find or Replace field, or on the sheet that is
  open, and never on the document behind it. A paste into the Find field puts the text in the field.
- Left, Right, Backspace and Delete move and remove a whole character as you see it: a flag, a family, a skin-tone emoji or
  a letter with its accent is one step.

## 1.12.1 - 2026-10-20

- Select All, Copy, Cut, Paste, Undo and Redo work from the keyboard (Cmd+A, C, X, V, Z and Shift+Cmd+Z); the shell used to
  drop them before they reached the app.
- Option+Backspace deletes the word before the caret, Cmd+Backspace everything back to the start of the line, and
  Option+Delete the word after it. Delete (Fn+Delete) deletes the character after the caret.
- Page Up and Page Down move the caret a window of lines, and extend the selection with Shift.
- Ctrl+A and Ctrl+E go to the start and end of the line, Ctrl+K deletes to the end of the paragraph, Ctrl+D and Ctrl+H delete
  forward and back, and Ctrl+F, B, N and P move by character and line.

## 1.12.0 - 2026-10-20

- Clicking in the text now places the caret where you click, drag selects, double-click selects a word and triple-click a
  paragraph. Before, a click on the first lines of the page did nothing and typing went in at the old caret.
- Shift with the arrow keys, Home or End extends the selection; Alt with an arrow moves by word and Cmd with Left or Right
  by line. Home and End move to the start and end of the line.
- The bold, italic, underline, size and other buttons act on the selected text, or on the text you type next when nothing
  is selected, and show the state of the selection.
- The window shows the document's name in its title, with a document icon and "Edited" while there are unsaved changes.
- The toolbar of text buttons (New, Open, Save, Undo, Redo, Find and Print) is gone: those commands are in the File and
  Edit menus. One format bar remains, with grouped icon controls: Bold, Italic, Underline and Strikethrough, the text
  colour, the font, style and size pop-ups, the four alignments, the list menu and the line spacing with a stepper.
- The format bar fits any window width: what does not fit moves into the button at its end.
- The save panel opens with the name of the document selected up to its extension, so typing replaces the name, and it
  hangs from the window as a sheet over a dimmed window.
- The help book describes selecting text and the format bar.

## 1.11.1

- BifrOSt 0.10.0 round (waves 14 and 15): see the system CHANGELOG for this app's changes.

## 1.11.0 — 2026-10-17

- RTFD packages open from the open panel: a package folder of the RTFD type is one file to the panel, chosen whole and not
  navigated into (the app declares `package_types = ["rtfd"]`).
- Save As and Rename to a package name (`Notes.rtfd`) write the package folder, which replaces an existing package only on
  commit; a package keeps its folder for the next save.
- Shared text opened on the Dock icon or from the share sheet becomes an untitled document (Input::Open with a data: URL).

## 1.10.0 — 2026-10-17

- Attachments are drawn inline: an attachment is as tall as its line and as wide as its picture, narrowed to the text
  width, and is selected, deleted and undone as one character.
- Insert > Image... opens a panel for PNG and JPEG files; image files dropped on the page are inserted at the drop point's
  caret (rich text only).
- Copy, cut and paste of pictures go through the general pasteboard as PNG or JPEG.
- Format > Make Plain Text asks before removing attachments (Remove Attachments, or Cancel).
- docx, odt, html and web archive saves leave attachments out instead of writing the attachment character.
- RTFD folder packages open and save through the document controller; Files shows a package folder as one file.
  Choosing a package in the open panel and opening one from Files are not wired yet.

## 1.9.1 — 2026-10-17

- Ships a help book (`help/`: writing, formatting, find and print), indexed by the shell help index for the Help menu and `help://is.olibuijr.textedit.desktop` links.

## 1.9.0 — 2026-10-17

- Rich text packages (RTFD) are read and written as a folder: `TXT.rtf` plus the files its attachments name, saved
  atomically. The SDK has the folder-package type (`bifrost_app::document_package`) and the manifest key `package_types`.
- An attachment is one character in the text (a NeXTGraphic group in RTF). Format > Make Plain Text is refused while the
  document has attachments, so none are dropped silently.

## 1.8.0 — 2026-10-17

- Services menu: "New Window Containing Selection" opens the selected text in a new untitled document, and "Open Selected
  File" opens the file whose path is selected (as an untitled document; Save asks for a place to write it).

## 1.7.0 — 2026-10-17

- Text > Speech: Start Speaking speaks the selection (else the whole document) through the shell speech service; Stop Speaking ends it.

## 1.6.2 — 2026-10-14

- Named document-format list indentation and bold-weight thresholds so exported document dimensions remain semantic, not resolution-like literals.

## 1.6.1 — 2026-10-14

- Text specific editing commands are in a Text menu; standard undo, redo, cut, copy, paste and select all use the shell Edit menu.
- Shared named colours and the Settings radius token replace literal palette values and radius constants.

## 1.6.0 — 2026-10-14

- Format > Text > Spacing: the space before and after a paragraph is kept in `.odt` (paragraph margins) and `.docx`
  (paragraph spacing), as well as RTF and HTML.
- Format > Text > Left to Right and Right to Left: the paragraph direction is kept in `.docx` (bidi paragraphs) and `.odt`
  (writing mode), as well as RTF.

## 1.5.0 — 2026-10-14

- Format > Text > Left to Right and Right to Left set the writing direction of the selected paragraphs (one undo entry).
  A left-aligned right to left paragraph starts its lines at the right edge, and the direction is kept in RTF.

## 1.4.0 — 2026-10-14

- Copy and paste keep formatting: a paste of text copied in this document keeps its character and paragraph formatting.
  Paste and Match Style, and any text copied from elsewhere, is plain text in the style at the caret.
- Edit > Use Selection for Find puts the selection on the find pasteboard, and the find field starts with the find
  pasteboard's text. Edit > Substitutions > Text Replacement turns a shortcut such as (c) or ... into its symbol.
- Edit > Find > Select Line opens a panel that takes a line number or a range. Edit > Spelling and Grammar > Show Spelling
  and Grammar opens a panel that steps through the misspelled words with Ignore, Learn, Change and Find Next.
- Copy Style also copies the paragraph's alignment, spacing and list. Format > Text > Copy Ruler and Paste Ruler copy the
  tab stop distance. Format > Text > Spacing sets the line spacing and the space before and after a paragraph (kept in
  RTF and HTML).
- File > Print > Rewrap to Fit Page: when it is off, the lines print where the window breaks them, scaled to the page.
- View > Use Dark Background for Windows: the page is dark and the text without a colour of its own is white.
- Preferences opens a panel with the new document format, the default font, the plain text encoding, the markup
  setting, the spelling, substitution, ruler, wrapping, header and footer and printing options, and the dark background.
  Preferences > Customize Encodings List chooses which encodings the Text Encoding menu lists.
- Format > Wrap to Page sits in the Format menu as well as View.
- Line breaks: a line break inside a paragraph stays inside it in HTML and `.odt` files (it was read as a new paragraph).
- Properties: HTML files keep the title, author, subject (as the description), keywords and copyright in their head.
- Web archives open and save (`.webarchive`): the page is kept as the main resource of a binary property list.

## 1.3.0 — 2026-10-13

- Plain text encodings: a plain text file opens in the encoding it is in (a byte order mark, UTF-8, UTF-16 without a
  mark, or else Western single-byte text) and saves in the same one. Format > Text Encoding changes the encoding a plain
  text document is saved in (UTF-8, UTF-16 in either byte order, ISO Latin 1, Windows Latin 1, Classic Roman, ASCII) and
  refuses to save text the encoding cannot hold; Format > Reinterpret As reads the text again in another encoding when
  the guess was wrong. Preferences > Plain Text Encoding sets the encoding of new documents.
- File > Show Properties: title, author, subject, keywords and copyright, kept in the RTF info group and in the
  `.docx` and `.odt` package metadata (those two have no copyright field).
- Edit > Add Link (Cmd+K), Open Link and Remove Link: text can link to a web, mail, phone or file transfer address. Links
  are drawn in the link colour with an underline, are kept by RTF (hyperlink fields), HTML, `.docx` and `.odt`, print,
  and open with a click in a document that cannot be edited. Addresses that run code or read local files are refused.
- Edit > Substitutions > Smart Links: a web address (`http://`, `https://`, `www.`), a `mailto:` address or a mail address
  becomes a link when the word ends with a blank. Kept in the preferences.
- Edit > Insert: Page Break, Line Break and Paragraph Break. A line break starts a new line inside the paragraph, and a
  page break starts a new page when printing (RTF `\line` and `\page`, a docx break). A plain text document writes a line
  break as a line ending. HTML and `.odt` files write both as a line break, which those readers take as a paragraph
  break.
- Preferences > Show RTF and HTML Files as Code opens those files as plain text showing their markup.

## 1.2.0 — 2026-10-13

- File: Save All, Export as PDF (a save panel, then the same PDF the printer gets), and Print Header and Footer (the
  title and "Page n of m" in the margins).
- Word processing files: documents open and save as `.docx` and `.odt` (paragraphs, runs, bold, italic, underline,
  strikethrough, size, colour, highlight, baseline shift, font family, alignment, line spacing, bullet and numbered
  lists), chosen by the file's extension in Save As.
- Edit: Paste and Match Style, Delete, Complete (steps through the words of the document that start the same), Use
  Selection for Find (Cmd+E), Jump to Selection (Cmd+J), Select Line (Cmd+L, a number or a range such as 12-20 typed in
  the find field), Transformations (upper case, lower case, capitalize), Substitutions (Smart Quotes, Smart Dashes) and
  Correct Spelling Automatically.
- Format: Strikethrough, Baseline (superscript, subscript), Highlight colours, Copy Style and Paste Style, and Prevent
  Editing / Allow Editing. RTF and HTML carry the new character styles.
- View menu: Show or Hide Ruler, Wrap to Window or Page, Zoom In, Zoom Out and Actual Size.
- Preferences menu: the format of new documents; the spelling, substitution, ruler, wrapping and header options are kept
  between launches.

## 1.1.1 — 2026-10-13

- Printing goes through the shared print operation: the PDF is written by the shared PDF writer and submitted to CUPS
  as a job with copies, paper, orientation and page range. The app's private PDF writer is gone.

## 1.1.0 — 2026-10-13

- Documents open and save as RTF, HTML and plain text, chosen by the file's extension in Save As; a plain text document
  stays plain, and Format > Make Rich Text and Make Plain Text switch a document's mode.
- Paragraph styles: line spacing, bullet and numbered lists, and tab stops set by clicking the ruler above the page.
  Colours from a palette in the format bar and Format > Color.
- Spelling: misspelled words are underlined while typing, Check Document Now selects the next one, Correct Spelling,
  Learn Spelling and Ignore Spelling, and learned words are kept in the app's preferences.
- Printing: File > Print sends the document, paginated for the page setup, to the default printer. Page setup offers
  A4, US Letter, US Legal, A5 and A3 in portrait or landscape.

## 1.0.0 — 2026-10-13

- New app: Text Editor. Opens and saves plain text and RTF through the shell's document panels (Open, Save, Save As,
  Revert, Duplicate), with the document controller's File menu and recent documents.
- Formatting: bold, italic, underline, size (Bigger and Smaller), font family from Format > Font, and alignment for
  each paragraph.
- Find, Find Next and Find Previous, Replace, and Replace All.
- Undo and redo with the names of the edits ("Undo Typing"), and a word and character count at the bottom.
