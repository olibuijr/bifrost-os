# Changelog

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
