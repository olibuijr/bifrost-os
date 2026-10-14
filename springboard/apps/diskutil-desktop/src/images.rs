//! Disk image operations that run as the user: attaching an image, creating a blank one (optionally with an empty file
//! system), building an ISO image from a folder, converting between image formats and resizing an image. Each
//! operation is a plan of commands (`bifrost_core::disk_commands::Command`, run by `disk_run::run_plan`), so the
//! logic is pure and tested; only the small writers at the end touch files. Paths come from the open and save panels
//! as open files, never as paths, so `file_path` reads the path of a granted file back from `/proc/self/fd`.

use bifrost_core::disk_commands::Command;
use bifrost_core::disk_format::{self, Personality};
use bifrost_core::disk_image::{self, BlockSize};
use bifrost_core::disk_run;
use std::fs::File;
use std::os::fd::AsRawFd;
use std::os::unix::fs::FileExt;
use std::path::Path;
use std::sync::mpsc::{channel, Receiver};

/// The image formats Convert writes. The names are what `qemu-img convert -O` takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageFormat {
    Raw,
    Qcow2,
    Vmdk,
    Vhdx,
    Vdi,
}

impl ImageFormat {
    pub const ALL: [ImageFormat; 5] = [ImageFormat::Raw, ImageFormat::Qcow2, ImageFormat::Vmdk, ImageFormat::Vhdx, ImageFormat::Vdi];

    pub fn word(self) -> &'static str {
        match self {
            ImageFormat::Raw => "raw",
            ImageFormat::Qcow2 => "qcow2",
            ImageFormat::Vmdk => "vmdk",
            ImageFormat::Vhdx => "vhdx",
            ImageFormat::Vdi => "vdi",
        }
    }

    /// The name a control shows.
    pub fn title(self) -> &'static str {
        match self {
            ImageFormat::Raw => "Raw",
            ImageFormat::Qcow2 => "QCOW2",
            ImageFormat::Vmdk => "VMDK",
            ImageFormat::Vhdx => "VHDX",
            ImageFormat::Vdi => "VDI",
        }
    }

    /// The file name extension of an image in this format.
    pub fn extension(self) -> &'static str {
        match self {
            ImageFormat::Raw => "img",
            other => other.word(),
        }
    }
}

/// A new blank image: its size, the file system to put on it (none leaves it empty) and that file system's label.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlankSpec {
    pub bytes: u64,
    pub fs: Option<Personality>,
    pub label: String,
}

/// A sentence for an error from the image model: its first letter capitalised and a full stop.
fn sentence(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => format!("{}{}.", first.to_uppercase(), chars.as_str().trim_end_matches('.')),
        None => String::new(),
    }
}

/// An image size such as `512`, `100m` or `2G` in bytes: a whole number of 512-byte blocks.
pub fn parse_size(text: &str) -> Result<u64, String> {
    let bytes = disk_image::parse_size(text).map_err(|e| sentence(&e.to_string()))?;
    disk_image::create_raw(bytes, BlockSize::Bytes512).map_err(|e| sentence(&e.to_string()))?;
    Ok(bytes)
}

/// A size as the shortest text `parse_size` reads back: `2g`, `100m`, `64k` or the byte count.
pub fn size_text(bytes: u64) -> String {
    for (unit, word) in [(1u64 << 40, "t"), (1 << 30, "g"), (1 << 20, "m"), (1 << 10, "k")] {
        if bytes >= unit && bytes % unit == 0 {
            return format!("{}{word}", bytes / unit);
        }
    }
    bytes.to_string()
}

/// Checks the label a new image's file system would get.
pub fn check_blank(spec: &BlankSpec) -> Result<(), String> {
    match spec.fs {
        Some(fs) => disk_format::check_label(&spec.label, fs).map_err(|r| r.sentence()),
        None if spec.label.is_empty() => Ok(()),
        None => Err("An image with no file system has no volume name.".to_string()),
    }
}

/// The commands that put an empty file system on the new image at `path`, which already has its size. Empty when the
/// image stays unformatted.
pub fn blank_plan(spec: &BlankSpec, path: &str) -> Result<Vec<Command>, String> {
    check_blank(spec)?;
    Ok(spec.fs.map(|fs| fs.make_command(path, &spec.label)).into_iter().collect())
}

/// The plan that attaches an image file: `hdiutil attach` binds it to a loop device and mounts it, read-only unless
/// `writable` asks for read and write access (File > Open Disk Image for Writing).
pub fn attach_plan(path: &str, writable: bool) -> Result<Vec<Command>, String> {
    if !path.starts_with('/') {
        return Err("The image's location cannot be used.".to_string());
    }
    let access = if writable { "-readwrite" } else { "-readonly" };
    Ok(vec![Command { program: "hdiutil", args: vec!["attach".to_string(), access.to_string(), path.to_string()] }])
}

/// The command that turns `folder` into an ISO image at `scratch`.
pub fn folder_command(folder: &str, volname: &str, scratch: &str) -> Command {
    let mut args = vec!["create".to_string(), "-srcfolder".to_string(), folder.to_string()];
    if !volname.is_empty() {
        args.push("-volname".to_string());
        args.push(volname.to_string());
    }
    args.push("-o".to_string());
    args.push(scratch.to_string());
    Command { program: "hdiutil", args }
}

/// An ISO volume name from a folder name: capitals, digits and `_`, at most 32 characters. Anything else becomes `_`.
pub fn iso_volume_name(folder: &str) -> String {
    let name: String = base_name(folder)
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_uppercase() } else { '_' })
        .take(32)
        .collect();
    name.trim_matches('_').to_string()
}

/// The command that converts the image `src` into `dst` in `format`.
pub fn convert_command(src: &str, dst: &str, format: ImageFormat) -> Command {
    Command {
        program: "qemu-img",
        args: vec!["convert".to_string(), "-O".to_string(), format.word().to_string(), src.to_string(), dst.to_string()],
    }
}

/// Whether the first bytes of an image hold an ext2, ext3 or ext4 superblock (its magic number at byte 1080).
pub fn is_ext_image(head: &[u8]) -> bool {
    head.len() >= 1082 && head[1080] == 0x53 && head[1081] == 0xEF
}

/// The plan that gives the image at `path` the size `new`, from `old`. A growing image is extended and then its ext
/// file system is grown to fill it. A shrinking one has its file system shrunk first, so no data is cut off, and then
/// the file; only an ext image can shrink. The sizes are whole 4 KB blocks.
pub fn resize_plan(path: &str, old: u64, new: u64, ext: bool) -> Result<Vec<Command>, String> {
    if new == old {
        return Err("The image already has that size.".to_string());
    }
    if new == 0 || new % 4096 != 0 {
        return Err("The size must be a whole number of 4 KB blocks.".to_string());
    }
    let truncate = Command { program: "truncate", args: vec!["-s".to_string(), new.to_string(), path.to_string()] };
    let grow = new > old;
    if !ext {
        if !grow {
            return Err("Only an ext2, ext3 or ext4 image can shrink: the data of another file system would be cut off.".to_string());
        }
        return Ok(vec![truncate]);
    }
    // resize2fs insists on a check that has just run; -p repairs only what is safe, and -f checks a clean file system too
    let check = Command { program: "e2fsck", args: vec!["-f".to_string(), "-p".to_string(), path.to_string()] };
    let resize = Command { program: "resize2fs", args: vec![path.to_string(), format!("{}K", new / 1024)] };
    Ok(if grow { vec![truncate, check, resize] } else { vec![check, resize, truncate] })
}

/// The last component of a path.
pub fn base_name(path: &str) -> &str {
    path.trim_end_matches('/').rsplit('/').next().unwrap_or(path)
}

/// A file name for the save panel: `name` without slashes or control characters, `Untitled` when nothing is left, and
/// ending in `.extension`.
pub fn suggested_file(name: &str, extension: &str) -> String {
    let clean: String = name.chars().filter(|c| *c != '/' && !c.is_control()).collect();
    let clean = clean.trim().trim_start_matches('.').to_string();
    let stem = if clean.is_empty() { "Untitled".to_string() } else { clean };
    let suffix = format!(".{extension}");
    if stem.to_lowercase().ends_with(&suffix) {
        stem
    } else {
        format!("{stem}{suffix}")
    }
}

/// The name of an image file without its extension, for the name of a converted copy.
pub fn stem(name: &str) -> &str {
    match name.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem,
        _ => name,
    }
}

/// The absolute path of an open file, read back from `/proc/self/fd`. Refused for a file that has no usable path (a
/// deleted file, an in-memory one).
pub fn file_path(file: &File) -> Result<String, String> {
    let link = std::fs::read_link(format!("/proc/self/fd/{}", file.as_raw_fd())).map_err(|e| format!("The location of the file could not be read: {e}."))?;
    let text = link.to_str().ok_or("The location of the file is not text.")?;
    if !Path::new(text).is_absolute() || text.ends_with(" (deleted)") {
        return Err(format!("The location {text} cannot be used."));
    }
    Ok(text.to_string())
}

/// Up to `len` bytes from the start of an open file.
pub fn read_head(file: &File, len: usize) -> Vec<u8> {
    let mut buf = vec![0u8; len];
    let mut got = 0;
    while got < len {
        match file.read_at(&mut buf[got..], got as u64) {
            Ok(0) | Err(_) => break,
            Ok(n) => got += n,
        }
    }
    buf.truncate(got);
    buf
}

/// Runs `job` on a new thread. The receiver gets its result.
pub fn spawn<F>(job: F) -> Receiver<Result<(), String>>
where
    F: FnOnce() -> Result<(), String> + Send + 'static,
{
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let _ = tx.send(job());
    });
    rx
}

/// Sizes the new image file the save panel made, then formats it.
pub fn write_blank(file: File, spec: BlankSpec) -> Result<(), String> {
    let path = file_path(&file)?;
    file.set_len(spec.bytes).map_err(|e| format!("The image could not be sized: {e}."))?;
    let plan = blank_plan(&spec, &path)?;
    disk_run::run_plan(&plan, false).map(|_| ())
}

/// Builds an ISO image of `folder` next to the new file, copies it into the file and removes the build.
pub fn write_folder_image(mut out: File, folder: &str, volname: &str) -> Result<(), String> {
    let path = file_path(&out)?;
    let scratch = format!("{path}.partial");
    let result = (|| -> Result<(), String> {
        disk_run::run_plan(&[folder_command(folder, volname, &scratch)], false)?;
        let mut built = File::open(&scratch).map_err(|e| format!("The image could not be read back: {e}."))?;
        std::io::copy(&mut built, &mut out).map_err(|e| format!("The image could not be saved: {e}."))?;
        out.sync_all().map_err(|e| format!("The image could not be saved: {e}."))
    })();
    let _ = std::fs::remove_file(&scratch);
    result
}

/// Converts the image `src` into the new file.
pub fn write_converted(file: File, src: &str, format: ImageFormat) -> Result<(), String> {
    let path = file_path(&file)?;
    disk_run::run_plan(&[convert_command(src, &path, format)], false).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(plan: &[Command]) -> Vec<(&'static str, Vec<String>)> {
        plan.iter().map(|c| (c.program, c.args.clone())).collect()
    }

    fn s(w: &[&str]) -> Vec<String> {
        w.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn sizes_read_with_units_and_must_be_whole_blocks() {
        assert_eq!(parse_size("100m"), Ok(100 << 20));
        assert_eq!(parse_size("2G"), Ok(2 << 30));
        assert_eq!(parse_size("1024"), Ok(1024));
        assert!(parse_size("1000").unwrap_err().contains("whole number of blocks"));
        assert!(parse_size("big").unwrap_err().ends_with('.'));
        assert!(parse_size("0").is_err());
        assert!(parse_size("").is_err());
    }

    #[test]
    fn a_size_is_written_in_the_largest_exact_unit() {
        assert_eq!(size_text(2 << 30), "2g");
        assert_eq!(size_text(100 << 20), "100m");
        assert_eq!(size_text(64 << 10), "64k");
        assert_eq!(size_text(1000), "1000");
        assert_eq!(parse_size(&size_text(3 << 30)), Ok(3 << 30));
    }

    #[test]
    fn a_blank_image_is_formatted_with_the_chosen_file_system_and_label() {
        let spec = BlankSpec { bytes: 64 << 20, fs: Some(Personality::Ext4), label: "SCRATCH".into() };
        let plan = blank_plan(&spec, "/home/me/a.img").unwrap();
        assert_eq!(words(&plan), vec![("mkfs.ext4", s(&["-F", "-q", "-L", "SCRATCH", "/home/me/a.img"]))]);
        let bare = BlankSpec { bytes: 64 << 20, fs: None, label: String::new() };
        assert_eq!(blank_plan(&bare, "/home/me/a.img"), Ok(Vec::new()));
    }

    #[test]
    fn a_label_the_file_system_refuses_is_caught_before_the_save_panel() {
        let long = BlankSpec { bytes: 64 << 20, fs: Some(Personality::Fat32), label: "ABCDEFGHIJKL".into() };
        assert!(check_blank(&long).unwrap_err().contains("not valid"));
        let orphan = BlankSpec { bytes: 64 << 20, fs: None, label: "X".into() };
        assert!(check_blank(&orphan).unwrap_err().contains("no file system"));
    }

    #[test]
    fn attach_runs_the_image_tool_on_an_absolute_path_only() {
        assert_eq!(words(&attach_plan("/home/me/disc.iso", false).unwrap()), vec![("hdiutil", s(&["attach", "-readonly", "/home/me/disc.iso"]))]);
        assert!(attach_plan("disc.iso", false).is_err());
    }

    #[test]
    fn an_image_opened_for_writing_is_attached_read_and_write() {
        assert_eq!(words(&attach_plan("/home/me/disc.img", true).unwrap()), vec![("hdiutil", s(&["attach", "-readwrite", "/home/me/disc.img"]))]);
        assert!(attach_plan("disc.img", true).is_err());
    }

    #[test]
    fn a_folder_becomes_an_iso_through_the_image_tool() {
        assert_eq!(
            folder_command("/home/me/Photos", "PHOTOS", "/home/me/p.iso.partial").args,
            s(&["create", "-srcfolder", "/home/me/Photos", "-volname", "PHOTOS", "-o", "/home/me/p.iso.partial"])
        );
        assert_eq!(folder_command("/a", "", "/b").args, s(&["create", "-srcfolder", "/a", "-o", "/b"]));
    }

    #[test]
    fn an_iso_volume_name_keeps_capitals_digits_and_underscores() {
        assert_eq!(iso_volume_name("/home/me/My Photos 2026/"), "MY_PHOTOS_2026");
        assert_eq!(iso_volume_name("/mnt/-x-"), "X");
        assert_eq!(iso_volume_name("/a/").len(), 1);
        assert_eq!(iso_volume_name(&format!("/{}", "a".repeat(80))).len(), 32);
    }

    #[test]
    fn a_conversion_names_the_target_format() {
        assert_eq!(words(&[convert_command("/a.img", "/b.qcow2", ImageFormat::Qcow2)]), vec![("qemu-img", s(&["convert", "-O", "qcow2", "/a.img", "/b.qcow2"]))]);
        assert_eq!(ImageFormat::Raw.extension(), "img");
        assert_eq!(ImageFormat::Vhdx.extension(), "vhdx");
        assert_eq!(ImageFormat::ALL.len(), 5);
    }

    #[test]
    fn an_ext_superblock_is_recognised_by_its_magic() {
        let mut head = vec![0u8; 2048];
        assert!(!is_ext_image(&head));
        head[1080] = 0x53;
        head[1081] = 0xEF;
        assert!(is_ext_image(&head));
        assert!(!is_ext_image(&head[..1000]));
    }

    #[test]
    fn growing_an_ext_image_extends_the_file_then_the_file_system() {
        let plan = resize_plan("/home/me/a.img", 64 << 20, 128 << 20, true).unwrap();
        assert_eq!(
            words(&plan),
            vec![
                ("truncate", s(&["-s", "134217728", "/home/me/a.img"])),
                ("e2fsck", s(&["-f", "-p", "/home/me/a.img"])),
                ("resize2fs", s(&["/home/me/a.img", "131072K"])),
            ]
        );
    }

    #[test]
    fn shrinking_an_ext_image_shrinks_the_file_system_before_the_file() {
        let plan = resize_plan("/a.img", 128 << 20, 64 << 20, true).unwrap();
        let programs: Vec<&str> = plan.iter().map(|c| c.program).collect();
        assert_eq!(programs, vec!["e2fsck", "resize2fs", "truncate"]);
    }

    #[test]
    fn a_raw_image_of_another_file_system_can_only_grow() {
        assert_eq!(words(&resize_plan("/a.img", 1 << 20, 2 << 20, false).unwrap()), vec![("truncate", s(&["-s", "2097152", "/a.img"]))]);
        assert!(resize_plan("/a.img", 2 << 20, 1 << 20, false).unwrap_err().contains("can shrink"));
    }

    #[test]
    fn a_resize_needs_a_new_whole_block_size() {
        assert!(resize_plan("/a.img", 1 << 20, 1 << 20, true).unwrap_err().contains("already"));
        assert!(resize_plan("/a.img", 1 << 20, (2 << 20) + 512, true).unwrap_err().contains("4 KB"));
        assert!(resize_plan("/a.img", 1 << 20, 0, true).is_err());
    }

    #[test]
    fn names_for_the_save_panel_are_cleaned_and_get_their_extension() {
        assert_eq!(suggested_file("Backup", "img"), "Backup.img");
        assert_eq!(suggested_file("Backup.IMG", "img"), "Backup.IMG");
        assert_eq!(suggested_file("a/b", "iso"), "ab.iso");
        assert_eq!(suggested_file("  ", "iso"), "Untitled.iso");
        assert_eq!(suggested_file(".hidden", "img"), "hidden.img");
        assert_eq!(stem("disk.img"), "disk");
        assert_eq!(stem("archive"), "archive");
        assert_eq!(stem(".img"), ".img");
        assert_eq!(base_name("/home/me/Photos/"), "Photos");
        assert_eq!(base_name("plain"), "plain");
    }

    #[test]
    fn the_path_of_an_open_file_is_read_back_and_a_deleted_file_has_none() {
        let dir = std::fs::canonicalize(std::env::temp_dir()).unwrap();
        let path = dir.join(format!("bifrost-diskutil-test-{}.img", std::process::id()));
        let file = File::create(&path).unwrap();
        assert_eq!(file_path(&file).unwrap(), path.to_str().unwrap());
        std::fs::remove_file(&path).unwrap();
        assert!(file_path(&file).is_err(), "a deleted file has no usable path");
    }

    #[test]
    fn the_head_of_a_file_is_read_up_to_its_length() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("bifrost-diskutil-head-{}.img", std::process::id()));
        std::fs::write(&path, b"abcdef").unwrap();
        let file = File::open(&path).unwrap();
        assert_eq!(read_head(&file, 4), b"abcd");
        assert_eq!(read_head(&file, 100), b"abcdef");
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn a_job_reports_its_result_on_the_channel() {
        assert_eq!(spawn(|| Ok(())).recv().unwrap(), Ok(()));
        assert_eq!(spawn(|| Err("no".to_string())).recv().unwrap(), Err("no".to_string()));
    }
}
