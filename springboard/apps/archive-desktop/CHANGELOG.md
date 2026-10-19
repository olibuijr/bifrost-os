# Changelog

## 0.2.6

- BifrOSt 0.10.0 round (waves 14 and 15): see the system CHANGELOG for this app's changes.

## 0.2.5 — 2026-10-14

- Stage archive creation and extraction in hidden sibling paths, publishing complete outputs atomically without replacing existing files or folders.

## 0.2.4 — 2026-10-14

- Use the appearance-aware separator color in the progress bar.

## 0.2.3 — 2026-10-14

- Add the generated app compliance harness.

## 0.2.2 — 2026-10-14

- Use the appearance-aware separator for archive progress.

## 0.2.1 — 2026-10-14

- Schedule archive work through the shared utility queue.

## 0.2.0

- Expands .tar.xz, .tar.bz2, .tar.zst and single .gz/.bz2/.xz/.zst files through the shared archive module, the same one Files uses; xz is no longer refused.

## 0.1.0

- New app Archive (`is.olibuijr.archive.desktop`): expands zip, tar and gzipped tar (.tar.gz, .tgz) archives beside
  the file into a new folder, numbered when the name is taken; compresses a folder to a .zip beside it. Opening an
  archive in Files (Open With, or double-click when it is the default) starts the job; a progress bar follows it.
- Member paths that try to leave the folder (`..`, absolute) are skipped; links and devices in tar files are skipped
  and counted.
- Still missing: .tar.xz and other xz/bzip2/7z/rar formats (no decoder in the lock yet), cancel, and encrypted zips.
