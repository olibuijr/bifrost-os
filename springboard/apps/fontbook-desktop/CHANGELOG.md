# Changelog

## 0.6.1

- Sidebar and list selections use the neutral scheme-aware Golden Gate plate; separators and disabled controls follow Light and Dark.

## 0.6.0 — 2026-10-14

- Font collections (.ttc) are validated: the table directory of the first face is checked the same way as a single
  face, and the report warns that the other faces are not checked. Validate File... offers .ttc files in its panel.
  Installing a collection is still refused, because the install and the family reader accept only .ttf and .otf.
- Tests for collections (valid first face, broken first face, empty and cut-short headers) in validate.rs.
- Still missing: installing collections, Type 1, a Libraries sidebar section, a slider for the sample size, rules on
  style or language, and a choice of install location.

## 0.5.0 — 2026-10-14

- Validate File...: the Font menu item opens a panel for a .ttf or .otf file and shows its
  validation (pass, warnings or each problem) without installing it. Any list or sidebar press, Escape or Validate
  Font returns to the selected family.
- Export Fonts to Folder: copies every family the list shows into a folder chosen in a panel. Names never overwrite:
  a file that already exists in the folder is numbered (`Name 2.ttf`). Files that cannot be copied are counted in the
  status line.
- The unused library module is removed: the libraries are the model's font sets, switched from the Font menu's
  Library submenu. The export naming moved to `export.rs` with tests.
- Still missing: Type 1 and font collections, a Libraries sidebar section, a slider for the sample size, rules on
  style or language, and a choice of install location.

## 0.4.0 — 2026-10-13

- Libraries: Font > Library switches the list between Computer (every font), User (your installs) and your own
  libraries. Add Library Folder turns a folder chosen in the folder panel into a library named after it; a folder with
  the name of an existing library joins it. Remove Library forgets a library and leaves its fonts on disk. The choice
  and the libraries are saved in the app's data dir.
- Export Font (Cmd+E): the save panel writes a copy of the selected font file.
- Print Sample Sheet (Cmd+P): sends the family, its details, the sample line and the character set to the default
  printer as plain text.
- Show in Files: reveals the selected font's file. Only fonts inside your home folder can be shown.
- Validation has a warning level: a font that installs but lacks a name, OS/2 or post table, has no outline table or
  does not cover Basic Latin passes with warnings, listed in the Validate Font view and the install preview.
- Still missing: Type 1 and font collections, per-style listing, a Validate File command, export of several fonts.

## 0.3.0 — 2026-10-13

- Font menu in the menu bar: Add Fonts (Cmd+O, open panel), New Collection, New Smart Collection (fixed width, variable,
  static, TrueType, OpenType, name contains the search text), Rename and Delete Collection, Turn Font On/Off, Remove Font,
  Validate Font, Resolve Duplicates, the three preview modes, Edit Sample Text, Bigger and Smaller.
- Install preview: a font from Add Fonts or Open With shows its validation and a sample, with Install and Cancel.
- Settings: "Validate Fonts Before Installing" (on by default), saved with the sample text and size.
- Preview modes Sample, Repertoire (coverage and a character grid) and Info, chosen with tabs; a Validation view shows
  pass or fail with each problem.
- Custom sample text (click the field or Cmd+T) and a size control, saved in the app's data dir.
- Smart collections saved as rules; collections can be renamed in the sidebar and deleted.
- Resolve Duplicates: finds families with several files among your installs and keeps the newest after a second confirm.
- `fontbook:` links select a family (`fontbook://Inter`).
- Still missing: Libraries, export and print of samples, Show in Files, Type 1 and font collections, per-style listing.

## 0.2.0 — 2026-10-13

- Validation: an install is refused, with the reason, when the file is not a complete TrueType or OpenType font (header,
  tables, glyphs, a readable character map). Font collections (.ttc) are refused for now.
- Repertoire in the Info panel: characters covered, glyphs, the Unicode blocks covered, and the validation result.
- Remove Font for the user's own installs (files under ~/.local/share/fonts only); system and library fonts stay.
- Collections: New Collection in the sidebar, and one row per collection in the preview to add the font to it or take it out.
- Still missing: renaming and deleting a collection, editable preview text, and per-style listing.

## 0.1.0

- New app Fonts (`is.olibuijr.fontbook.desktop`): lists the families under the system fonts, the user's installs and the
  shell's font library; search by family or file name; All, Off and collection lists; preview at four sizes; Info rows;
  a per-user on/off switch saved in the app's data dir; installs into `~/.local/share/fonts` (Open With on a .ttf/.otf).
- Still missing: per-style listing and repertoire (the font code reads one face per family), font collections as
  Apple's shared collections, and the shell does not yet read the off list.
