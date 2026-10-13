# Changelog

## 0.1.0

- New app Archive (`is.olibuijr.archive.desktop`): expands zip, tar and gzipped tar (.tar.gz, .tgz) archives beside
  the file into a new folder, numbered when the name is taken; compresses a folder to a .zip beside it. Opening an
  archive in Files (Open With, or double-click when it is the default) starts the job; a progress bar follows it.
- Member paths that try to leave the folder (`..`, absolute) are skipped; links and devices in tar files are skipped
  and counted.
- Still missing: .tar.xz and other xz/bzip2/7z/rar formats (no decoder in the lock yet), cancel, and encrypted zips.
