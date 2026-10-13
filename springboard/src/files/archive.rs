//! Archives of every kind the Files apps and Archive Utility handle: .zip with the built-in reader and writer
//! (`zip`), and tar (plain, gzip, bzip2, xz, zstd) and single-file compressed files through the system tools
//! (`tar`, `gzip`, `bzip2`, `xz`, `zstd`). The tools are run without a shell, so no file name is ever parsed as a
//! command. `progress(done_bytes, total_bytes)` is polled while a tool runs; returning false cancels it.

use super::{fs, zip};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

pub use zip::Extracted;

/// A kind of archive, told apart by its file name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Zip,
    Tar,
    TarGz,
    TarBz2,
    TarXz,
    TarZst,
    /// one file compressed on its own
    Gz,
    Bz2,
    Xz,
    Zst,
}

impl Format {
    /// The formats a new archive can be made in, in the order a picker lists them.
    pub const CREATABLE: [Format; 6] = [Format::Zip, Format::Tar, Format::TarGz, Format::TarBz2, Format::TarXz, Format::TarZst];

    pub fn label(self) -> &'static str {
        match self {
            Format::Zip => "ZIP",
            Format::Tar => "TAR",
            Format::TarGz => "TAR.GZ",
            Format::TarBz2 => "TAR.BZ2",
            Format::TarXz => "TAR.XZ",
            Format::TarZst => "TAR.ZST",
            Format::Gz => "GZ",
            Format::Bz2 => "BZ2",
            Format::Xz => "XZ",
            Format::Zst => "ZST",
        }
    }

    /// The file name ending, with its dot.
    pub fn extension(self) -> &'static str {
        match self {
            Format::Zip => ".zip",
            Format::Tar => ".tar",
            Format::TarGz => ".tar.gz",
            Format::TarBz2 => ".tar.bz2",
            Format::TarXz => ".tar.xz",
            Format::TarZst => ".tar.zst",
            Format::Gz => ".gz",
            Format::Bz2 => ".bz2",
            Format::Xz => ".xz",
            Format::Zst => ".zst",
        }
    }

    /// The format a file name says, longest ending first (".tar.gz" before ".gz").
    pub fn detect(name: &str) -> Option<Format> {
        let n = name.to_lowercase();
        let ends = |e: &str| n.ends_with(e);
        Some(if ends(".tar.gz") || ends(".tgz") {
            Format::TarGz
        } else if ends(".tar.bz2") || ends(".tbz2") || ends(".tbz") {
            Format::TarBz2
        } else if ends(".tar.xz") || ends(".txz") {
            Format::TarXz
        } else if ends(".tar.zst") || ends(".tzst") {
            Format::TarZst
        } else if ends(".tar") {
            Format::Tar
        } else if ends(".zip") {
            Format::Zip
        } else if ends(".gz") {
            Format::Gz
        } else if ends(".bz2") {
            Format::Bz2
        } else if ends(".xz") {
            Format::Xz
        } else if ends(".zst") {
            Format::Zst
        } else {
            return None;
        })
    }

    fn is_tar(self) -> bool {
        matches!(self, Format::Tar | Format::TarGz | Format::TarBz2 | Format::TarXz | Format::TarZst)
    }

    /// The tool and flag that undo a single-file compression.
    fn single_tool(self) -> Option<&'static str> {
        match self {
            Format::Gz => Some("gzip"),
            Format::Bz2 => Some("bzip2"),
            Format::Xz => Some("xz"),
            Format::Zst => Some("zstd"),
            _ => None,
        }
    }
}

/// Whether a file name is an archive this module can expand.
pub fn is_archive(name: &str) -> bool {
    Format::detect(name).is_some()
}

/// The name without its archive ending: "photos.tar.gz" gives "photos".
pub fn stem(name: &str) -> String {
    match Format::detect(name) {
        Some(f) => {
            let ext = match name.to_lowercase() {
                n if n.ends_with(f.extension()) => f.extension().len(),
                n => n.rsplit_once('.').map_or(0, |(_, e)| e.len() + 1), // .tgz and the other short endings
            };
            name[..name.len() - ext].to_string()
        }
        None => name.to_string(),
    }
}

/// The archive name for a selection: the single item's name, else "Archive", with the format's ending.
pub fn archive_name(sources: &[PathBuf], format: Format) -> String {
    let base = match sources {
        [one] => one.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Archive".into()),
        _ => "Archive".into(),
    };
    format!("{base}{}", format.extension())
}

fn failed(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::Other, msg.into())
}

fn cancelled() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "Cancelled")
}

/// The arguments that make a tar of `sources` at `out`: one `-C parent name` pair per source.
pub fn tar_create_args(sources: &[PathBuf], out: &Path, format: Format) -> Vec<std::ffi::OsString> {
    let mut args: Vec<std::ffi::OsString> = vec![match format {
        Format::TarGz => "-czf",
        Format::TarBz2 => "-cjf",
        Format::TarXz => "-cJf",
        Format::TarZst => "--zstd",
        _ => "-cf",
    }
    .into()];
    if format == Format::TarZst {
        args.push("-cf".into());
    }
    args.push(out.into());
    for s in sources {
        let parent = s.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
        args.push("-C".into());
        args.push(parent.into());
        args.push(s.file_name().unwrap_or_default().into());
    }
    args
}

/// Runs a prepared command to its end, polling `progress` with the size of `watch` (the file being written) against
/// `total`; false from `progress` kills the tool. The tool's own error text is the error.
fn run(mut cmd: Command, watch: &Path, total: u64, progress: &mut dyn FnMut(u64, u64) -> bool) -> io::Result<()> {
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped()).spawn().map_err(|e| failed(format!("Could not start the tool: {e}")))?;
    loop {
        match child.try_wait()? {
            Some(status) => {
                if status.success() {
                    progress(total, total);
                    return Ok(());
                }
                let mut text = String::new();
                if let Some(mut e) = child.stderr.take() {
                    let _ = e.read_to_string(&mut text);
                }
                let text = text.lines().next().unwrap_or("The archive tool failed").to_string();
                return Err(failed(text));
            }
            None => {
                let done = std::fs::metadata(watch).map_or(0, |m| m.len()).min(total);
                if !progress(done, total) {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(cancelled());
                }
                std::thread::sleep(Duration::from_millis(40));
            }
        }
    }
}

fn tree_size(path: &Path) -> u64 {
    let Ok(md) = std::fs::symlink_metadata(path) else { return 0 };
    if md.is_dir() {
        std::fs::read_dir(path).map_or(0, |d| d.flatten().map(|e| tree_size(&e.path())).sum())
    } else {
        md.len()
    }
}

/// Makes a new archive of `sources` in `dest` (the name numbered when taken) and returns its path. A cancelled or
/// failed run leaves no partial file.
pub fn create(sources: &[PathBuf], dest: &Path, format: Format, progress: &mut dyn FnMut(u64, u64) -> bool) -> io::Result<PathBuf> {
    if sources.is_empty() {
        return Err(failed("Nothing to compress"));
    }
    if format == Format::Zip {
        return zip::create(sources, dest, progress);
    }
    if !format.is_tar() {
        return Err(failed("This format can only be expanded"));
    }
    let out = fs::unique_name(dest, &archive_name(sources, format));
    let total = sources.iter().map(|s| tree_size(s)).sum::<u64>().max(1);
    let mut cmd = Command::new("tar");
    cmd.args(tar_create_args(sources, &out, format));
    let result = run(cmd, &out, total, progress);
    match result {
        Ok(()) => Ok(out),
        Err(e) => {
            let _ = std::fs::remove_file(&out);
            Err(e)
        }
    }
}

/// Expands `archive` into a new folder named after it inside `dest` (numbered when taken). A single compressed file
/// becomes the one file in that folder.
pub fn extract(archive: &Path, dest: &Path, progress: &mut dyn FnMut(u64, u64) -> bool) -> io::Result<Extracted> {
    let name = archive.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let format = Format::detect(&name).ok_or_else(|| failed("This is not an archive"))?;
    if format == Format::Zip {
        return zip::extract(archive, dest, progress);
    }
    let folder = Some(stem(&name)).filter(|s| !s.is_empty()).unwrap_or_else(|| "Archive".into());
    let out = fs::unique_name(dest, &folder);
    std::fs::create_dir(&out)?;
    let size = std::fs::metadata(archive).map_or(1, |m| m.len().max(1));
    // the expanded size is not known up front; the compressed size stands in so the bar moves
    let result = if format.is_tar() {
        let mut cmd = Command::new("tar");
        cmd.arg("-xf").arg(archive).arg("-C").arg(&out).arg("--no-same-owner");
        run(cmd, &out, size, progress)
    } else {
        let tool = format.single_tool().ok_or_else(|| failed("Unknown format"))?;
        let target = out.join(stem(&name));
        let file = std::fs::File::create(&target)?;
        let mut cmd = Command::new(tool);
        cmd.arg("-dc").arg(archive);
        // the tool's output goes straight into the file
        run_into(cmd, file, &target, size, progress)
    };
    match result {
        Ok(()) => Ok(Extracted { dir: out.clone(), files: count_files(&out), skipped: Vec::new() }),
        Err(e) => {
            let _ = fs::delete(&out);
            Err(e)
        }
    }
}

fn run_into(mut cmd: Command, file: std::fs::File, watch: &Path, total: u64, progress: &mut dyn FnMut(u64, u64) -> bool) -> io::Result<()> {
    cmd.stdout(Stdio::from(file));
    let mut child = cmd.stdin(Stdio::null()).stderr(Stdio::piped()).spawn().map_err(|e| failed(format!("Could not start the tool: {e}")))?;
    loop {
        match child.try_wait()? {
            Some(status) if status.success() => {
                progress(total, total);
                return Ok(());
            }
            Some(_) => {
                let mut text = String::new();
                if let Some(mut e) = child.stderr.take() {
                    let _ = e.read_to_string(&mut text);
                }
                return Err(failed(text.lines().next().unwrap_or("The archive tool failed").to_string()));
            }
            None => {
                let done = std::fs::metadata(watch).map_or(0, |m| m.len()).min(total);
                if !progress(done, total) {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(cancelled());
                }
                std::thread::sleep(Duration::from_millis(40));
            }
        }
    }
}

fn count_files(dir: &Path) -> usize {
    std::fs::read_dir(dir).map_or(0, |d| {
        d.flatten()
            .map(|e| match e.file_type() {
                Ok(t) if t.is_dir() => count_files(&e.path()),
                _ => 1,
            })
            .sum()
    })
}

/// One member of an archive, for a listing.
#[derive(Clone, Debug, PartialEq)]
pub struct Member {
    pub name: String,
    pub size: u64,
    pub is_dir: bool,
}

/// The members of an archive. A single compressed file lists as itself, without a size.
pub fn list(archive: &Path) -> io::Result<Vec<Member>> {
    let name = archive.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let format = Format::detect(&name).ok_or_else(|| failed("This is not an archive"))?;
    if format == Format::Zip {
        let ar = zip::Archive::open(std::io::BufReader::new(std::fs::File::open(archive)?))?;
        return Ok(ar.members.iter().map(|m| Member { name: m.name.trim_end_matches('/').to_string(), size: m.size, is_dir: m.is_dir }).collect());
    }
    if !format.is_tar() {
        return Ok(vec![Member { name: stem(&name), size: 0, is_dir: false }]);
    }
    let out = Command::new("tar").arg("-tvf").arg(archive).output().map_err(|e| failed(format!("Could not start tar: {e}")))?;
    if !out.status.success() {
        return Err(failed(String::from_utf8_lossy(&out.stderr).lines().next().unwrap_or("Could not read the archive").to_string()));
    }
    Ok(parse_tar_listing(&String::from_utf8_lossy(&out.stdout)))
}

/// Reads `tar -tv` lines: "-rw-r--r-- user/group 1234 2026-10-12 14:03 path/name".
pub fn parse_tar_listing(text: &str) -> Vec<Member> {
    text.lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            if fields.len() < 6 {
                return None;
            }
            let mode = fields[0];
            let size = fields[2].parse::<u64>().ok()?;
            // the name starts after the time field; links print "name -> target"
            let rest = fields[5..].join(" ");
            let name = rest.split(" -> ").next().unwrap_or(&rest).trim_end_matches('/').to_string();
            Some(Member { name, size, is_dir: mode.starts_with('d') })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_tell_the_format() {
        assert_eq!(Format::detect("a.tar.gz"), Some(Format::TarGz));
        assert_eq!(Format::detect("A.TGZ"), Some(Format::TarGz));
        assert_eq!(Format::detect("a.gz"), Some(Format::Gz));
        assert_eq!(Format::detect("a.tar.zst"), Some(Format::TarZst));
        assert_eq!(Format::detect("a.zip"), Some(Format::Zip));
        assert_eq!(Format::detect("a.txt"), None);
        assert!(is_archive("x.tar.xz"));
    }

    #[test]
    fn stems_lose_the_whole_ending() {
        assert_eq!(stem("photos.tar.gz"), "photos");
        assert_eq!(stem("photos.tgz"), "photos");
        assert_eq!(stem("notes.txt.gz"), "notes.txt");
        assert_eq!(stem("plain"), "plain");
    }

    #[test]
    fn archive_names_follow_the_selection_and_format() {
        assert_eq!(archive_name(&[PathBuf::from("/x/Docs")], Format::TarXz), "Docs.tar.xz");
        assert_eq!(archive_name(&[PathBuf::from("/x/a"), PathBuf::from("/x/b")], Format::Zip), "Archive.zip");
    }

    #[test]
    fn tar_arguments_name_each_parent() {
        let args = tar_create_args(&[PathBuf::from("/a/x"), PathBuf::from("/b/y")], Path::new("/o/out.tar.gz"), Format::TarGz);
        let text: Vec<String> = args.iter().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!(text, ["-czf", "/o/out.tar.gz", "-C", "/a", "x", "-C", "/b", "y"]);
        let z = tar_create_args(&[PathBuf::from("/a/x")], Path::new("/o/out.tar.zst"), Format::TarZst);
        assert_eq!(z[0].to_string_lossy(), "--zstd");
        assert_eq!(z[1].to_string_lossy(), "-cf");
    }

    #[test]
    fn tar_listings_parse() {
        let text = "drwxr-xr-x user/user 0 2026-10-12 14:03 Docs/\n-rw-r--r-- user/user 1234 2026-10-12 14:03 Docs/a b.txt\n";
        let m = parse_tar_listing(text);
        assert_eq!(m, vec![Member { name: "Docs".into(), size: 0, is_dir: true }, Member { name: "Docs/a b.txt".into(), size: 1234, is_dir: false }]);
    }

    #[test]
    fn a_tar_round_trip_keeps_the_files() {
        if Command::new("tar").arg("--version").output().is_err() {
            return;
        }
        let root = std::env::temp_dir().join(format!("bifrost-archive-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src/sub")).unwrap();
        std::fs::write(root.join("src/sub/a.txt"), "hello").unwrap();
        let made = create(&[root.join("src")], &root, Format::TarGz, &mut |_, _| true).unwrap();
        assert_eq!(made.file_name().unwrap(), "src.tar.gz");
        let listed = list(&made).unwrap();
        assert!(listed.iter().any(|m| m.name == "src/sub/a.txt" && m.size == 5));
        let out = extract(&made, &root, &mut |_, _| true).unwrap();
        assert_eq!(std::fs::read_to_string(out.dir.join("src/sub/a.txt")).unwrap(), "hello");
        let _ = std::fs::remove_dir_all(&root);
    }
}
