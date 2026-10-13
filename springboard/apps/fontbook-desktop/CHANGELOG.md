# Changelog

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
