# Changelog

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
