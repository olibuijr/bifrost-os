//! `hdiutil`: disk images from the command line. Create blank RAW images and ISO discs from a folder, attach and detach
//! them, and inspect, verify and convert them. The image model is `bifrost_core::disk_image`; attaching and detaching
//! go to the shell over its control socket (`image-attach`, `image-detach`), because only the shell holds the session
//! that privd checks. Files and Disk Utility use the same commands.

use bifrost_core::disk_image::{self as img, BlockSize, ImageContents};
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::time::Duration;

/// Longest control line the shell reads (the same limit `open` keeps).
const MAX_LINE: usize = 510;
const TIMEOUT: Duration = Duration::from_secs(60);
/// The head read for `info` and `verify`: the 32 KB system area plus the volume descriptor sector.
const HEAD_BYTES: u64 = 34 * 1024;
/// ISO writers tried in order when a folder becomes an ISO image.
const ISO_TOOLS: [&str; 3] = ["xorriso", "genisoimage", "mkisofs"];

pub const USAGE: &str = "usage:
  hdiutil create -size <n[k|m|g|t]> | -srcfolder <folder> [-volname <name>] [-format raw] [-blocksize 512|4096] -o <file>
  hdiutil attach [-readonly | -readwrite] <file>
  hdiutil detach <loop device or mount folder>
  hdiutil info <file>
  hdiutil verify <file>
  hdiutil convert <file> -format raw [-blocksize 512|4096] -o <file>";

/// What `create` makes from: a blank image of a size, or an ISO of a folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    Size(u64),
    Folder(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreateOptions {
    pub source: Source,
    pub volname: Option<String>,
    pub block_size: BlockSize,
    pub output: String,
}

/// The parsed command line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Help,
    Create(CreateOptions),
    Attach { path: String, read_only: bool },
    Detach(String),
    Info(String),
    Verify(String),
    Convert { src: String, block_size: BlockSize, output: String },
}

/// Reads the arguments. Every error names the problem and is shown with the usage text by the caller.
pub fn parse(args: &[String]) -> Result<Command, String> {
    let Some((verb, rest)) = args.split_first() else {
        return Ok(Command::Help);
    };
    match verb.as_str() {
        "help" | "-h" | "--help" => Ok(Command::Help),
        "create" => create(rest),
        "attach" => attach(rest),
        "detach" => one_operand(rest, "detach").map(Command::Detach),
        "info" => one_operand(rest, "info").map(Command::Info),
        "verify" => one_operand(rest, "verify").map(Command::Verify),
        "convert" => convert(rest),
        other => Err(format!("unknown command {other}")),
    }
}

/// The value after an option, which must be present.
fn value(it: &mut std::slice::Iter<'_, String>, flag: &str) -> Result<String, String> {
    it.next().cloned().ok_or_else(|| format!("{flag} needs a value"))
}

fn block_size_arg(text: Option<String>) -> Result<BlockSize, String> {
    match text {
        None => Ok(BlockSize::Bytes512),
        Some(t) => t.parse::<u64>().ok().and_then(BlockSize::from_bytes).ok_or_else(|| "the block size must be 512 or 4096".to_string()),
    }
}

fn check_format(text: Option<&str>) -> Result<(), String> {
    match text {
        None => Ok(()),
        Some(name) => img::parse_format(name).map(|_| ()).map_err(|e| e.to_string()),
    }
}

fn create(args: &[String]) -> Result<Command, String> {
    let (mut size, mut folder, mut volname, mut block, mut output, mut format) = (None, None, None, None, None, None);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-size" => size = Some(value(&mut it, a)?),
            "-srcfolder" => folder = Some(value(&mut it, a)?),
            "-volname" => volname = Some(value(&mut it, a)?),
            "-blocksize" => block = Some(value(&mut it, a)?),
            "-format" => format = Some(value(&mut it, a)?),
            "-o" => output = Some(value(&mut it, a)?),
            other => return Err(format!("unknown option {other}")),
        }
    }
    check_format(format.as_deref())?;
    let output = output.ok_or_else(|| "create needs -o <file>".to_string())?;
    let block_size = block_size_arg(block)?;
    let source = match (size, folder) {
        (Some(s), None) => Source::Size(img::parse_size(&s).map_err(|e| e.to_string())?),
        (None, Some(f)) => Source::Folder(f),
        _ => return Err("create needs either -size or -srcfolder, not both".into()),
    };
    Ok(Command::Create(CreateOptions { source, volname, block_size, output }))
}

fn attach(args: &[String]) -> Result<Command, String> {
    let mut read_only = true;
    let mut path: Option<String> = None;
    for a in args {
        match a.as_str() {
            "-readonly" => read_only = true,
            "-readwrite" => read_only = false,
            flag if flag.starts_with('-') => return Err(format!("unknown option {flag}")),
            file => {
                if path.replace(file.to_string()).is_some() {
                    return Err("attach takes one image".into());
                }
            }
        }
    }
    Ok(Command::Attach { path: path.ok_or_else(|| "attach needs an image file".to_string())?, read_only })
}

fn convert(args: &[String]) -> Result<Command, String> {
    let (mut src, mut format, mut block, mut output) = (None, None, None, None);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-format" => format = Some(value(&mut it, a)?),
            "-blocksize" => block = Some(value(&mut it, a)?),
            "-o" => output = Some(value(&mut it, a)?),
            flag if flag.starts_with('-') => return Err(format!("unknown option {flag}")),
            file => {
                if src.replace(file.to_string()).is_some() {
                    return Err("convert takes one image".into());
                }
            }
        }
    }
    if format.is_none() {
        return Err("convert needs -format raw".into());
    }
    check_format(format.as_deref())?;
    Ok(Command::Convert {
        src: src.ok_or_else(|| "convert needs an image file".to_string())?,
        block_size: block_size_arg(block)?,
        output: output.ok_or_else(|| "convert needs -o <file>".to_string())?,
    })
}

fn one_operand(args: &[String], verb: &str) -> Result<String, String> {
    match args {
        [one] if !one.starts_with('-') => Ok(one.clone()),
        [] => Err(format!("{verb} needs one operand")),
        _ => Err(format!("{verb} takes exactly one operand")),
    }
}

/// The argument list for an ISO writer that turns `folder` into `output`. `xorriso` runs its `mkisofs` mode.
pub fn mkisofs_args(tool: &str, volname: Option<&str>, output: &str, folder: &str) -> Vec<String> {
    let mut a: Vec<String> = Vec::new();
    if tool == "xorriso" {
        a.push("-as".into());
        a.push("mkisofs".into());
    }
    if let Some(v) = volname {
        a.push("-V".into());
        a.push(v.to_string());
    }
    a.push("-o".into());
    a.push(output.to_string());
    a.push(folder.to_string());
    a
}

/// The first ISO writer found on `PATH`.
fn find_iso_tool() -> Option<&'static str> {
    let path = std::env::var("PATH").unwrap_or_default();
    ISO_TOOLS.into_iter().find(|tool| path.split(':').any(|dir| !dir.is_empty() && Path::new(dir).join(tool).is_file()))
}

/// The head of an image file and its length: what `info` and `verify` read.
fn read_head(path: &str) -> Result<(Vec<u8>, u64), String> {
    let mut f = File::open(path).map_err(|e| format!("{path}: {e}"))?;
    let len = f.metadata().map_err(|e| format!("{path}: {e}"))?.len();
    let mut head = Vec::new();
    f.by_ref().take(HEAD_BYTES).read_to_end(&mut head).map_err(|e| format!("{path}: {e}"))?;
    Ok((head, len))
}

/// The lines `info` prints for an image.
pub fn info_text(path: &str, head: &[u8], len: u64) -> String {
    let i = img::inspect(head, len);
    let mut lines = vec![format!("path: {path}"), "format: raw".to_string(), format!("size: {len} bytes")];
    lines.push(match i.contents {
        ImageContents::Iso9660 => "contents: ISO 9660 disc image".to_string(),
        ImageContents::Raw => "contents: raw image".to_string(),
    });
    if let Some(name) = &i.volume_name {
        lines.push(format!("volume name: {name}"));
    }
    if let Some(size) = i.declared_size() {
        lines.push(format!("declared size: {size} bytes"));
        lines.push(format!("complete: {}", if i.is_complete() { "yes" } else { "no, the file is shorter than its volume" }));
    }
    lines.join("\n")
}

/// The line `verify` prints when the image passes, or the reason it does not.
pub fn verify_text(path: &str, head: &[u8], len: u64) -> Result<String, String> {
    let i = img::verify(head, len).map_err(|e| format!("{path}: {e}"))?;
    Ok(match i.contents {
        ImageContents::Iso9660 => format!("{path}: verified, ISO 9660 disc image of {len} bytes"),
        ImageContents::Raw => format!("{path}: verified, raw image of {len} bytes"),
    })
}

/// Writes a blank image of `props` bytes, sparse, at `output`. Refuses an existing file.
pub fn write_blank(output: &str, byte_size: u64, block_size: BlockSize) -> Result<String, String> {
    let props = img::create_raw(byte_size, block_size).map_err(|e| e.to_string())?;
    if Path::new(output).exists() {
        return Err(format!("{output} already exists"));
    }
    let f = File::create(output).map_err(|e| format!("{output}: {e}"))?;
    f.set_len(props.byte_size()).map_err(|e| format!("{output}: {e}"))?;
    Ok(format!("created {output} ({} bytes, {} blocks)", props.byte_size(), props.block_count))
}

fn create_iso(folder: &str, volname: Option<&str>, output: &str) -> Result<String, String> {
    let tool = find_iso_tool().ok_or_else(|| "no ISO writer found: install xorriso".to_string())?;
    if Path::new(output).exists() {
        return Err(format!("{output} already exists"));
    }
    let out = std::process::Command::new(tool).args(mkisofs_args(tool, volname, output, folder)).output().map_err(|e| format!("{tool}: {e}"))?;
    if !out.status.success() {
        let _ = std::fs::remove_file(output);
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("the ISO writer failed").trim().to_string());
    }
    Ok(format!("created {output} from {folder}"))
}

/// Sends one control line to the shell and returns the body of its `ok` answer.
fn ask(line: &str, port: u16) -> Result<String, String> {
    if line.len() > MAX_LINE {
        return Err("the path is too long".into());
    }
    if line.contains(['\n', '\r']) {
        return Err("a path with a line break cannot be sent".into());
    }
    let mut stream = TcpStream::connect(("127.0.0.1", port)).map_err(|e| format!("the desktop is not answering on port {port}: {e}"))?;
    stream.set_read_timeout(Some(TIMEOUT)).map_err(|e| e.to_string())?;
    stream.set_write_timeout(Some(TIMEOUT)).map_err(|e| e.to_string())?;
    stream.write_all(format!("{line}\n").as_bytes()).map_err(|e| e.to_string())?;
    let mut answer = String::new();
    BufReader::new(stream).read_line(&mut answer).map_err(|e| e.to_string())?;
    reply_body(&answer)
}

/// The body of a shell answer: `ok` (with an optional body) or `error: <why>`.
pub fn reply_body(answer: &str) -> Result<String, String> {
    let t = answer.trim();
    if t == "ok" {
        Ok(String::new())
    } else if let Some(body) = t.strip_prefix("ok ") {
        Ok(body.to_string())
    } else if t.is_empty() {
        Err("no answer from the desktop".into())
    } else {
        Err(t.strip_prefix("error: ").unwrap_or(t).to_string())
    }
}

/// Runs a parsed command and returns the text to print.
pub fn run(cmd: &Command) -> Result<String, String> {
    match cmd {
        Command::Help => Ok(USAGE.to_string()),
        Command::Create(o) => match &o.source {
            Source::Size(bytes) => write_blank(&o.output, *bytes, o.block_size),
            Source::Folder(folder) => create_iso(folder, o.volname.as_deref(), &o.output),
        },
        Command::Attach { path, read_only } => {
            let abs = std::fs::canonicalize(path).map_err(|e| format!("{path}: {e}"))?;
            let abs = abs.to_str().ok_or_else(|| format!("{path}: the path is not text"))?;
            let mode = if *read_only { "ro" } else { "rw" };
            let body = ask(&format!("image-attach {mode} {abs}"), crate::open::port())?;
            let (device, point) = body.split_once(' ').ok_or_else(|| "the desktop gave no mount folder".to_string())?;
            Ok(format!("{device} {point}"))
        }
        Command::Detach(target) => {
            let device = ask(&format!("image-detach {target}"), crate::open::port())?;
            Ok(format!("released {device}"))
        }
        Command::Info(path) => {
            let (head, len) = read_head(path)?;
            Ok(info_text(path, &head, len))
        }
        Command::Verify(path) => {
            let (head, len) = read_head(path)?;
            verify_text(path, &head, len)
        }
        Command::Convert { src, block_size, output } => {
            if Path::new(output).exists() {
                return Err(format!("{output} already exists"));
            }
            let len = std::fs::metadata(src).map_err(|e| format!("{src}: {e}"))?.len();
            let props = img::open_raw(len, *block_size, true).map_err(|e| format!("{src}: {e}"))?;
            if let Err(e) = std::fs::copy(src, output) {
                let _ = std::fs::remove_file(output);
                return Err(format!("{output}: {e}"));
            }
            Ok(format!("converted {src} to {output} ({} bytes, {} blocks)", props.byte_size(), props.block_count))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("bifrost-hdiutil-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn no_arguments_or_help_print_the_usage() {
        assert_eq!(parse(&[]), Ok(Command::Help));
        assert_eq!(parse(&args(&["help"])), Ok(Command::Help));
        assert!(USAGE.contains("hdiutil create"));
    }

    #[test]
    fn create_reads_a_size_a_block_size_and_an_output() {
        assert_eq!(
            parse(&args(&["create", "-size", "10m", "-blocksize", "4096", "-o", "/home/me/a.img"])),
            Ok(Command::Create(CreateOptions {
                source: Source::Size(10 << 20),
                volname: None,
                block_size: BlockSize::Bytes4096,
                output: "/home/me/a.img".into(),
            }))
        );
    }

    #[test]
    fn create_from_a_folder_takes_a_volume_name() {
        assert_eq!(
            parse(&args(&["create", "-srcfolder", "/home/me/Pics", "-volname", "PICS", "-o", "/home/me/pics.iso"])),
            Ok(Command::Create(CreateOptions {
                source: Source::Folder("/home/me/Pics".into()),
                volname: Some("PICS".into()),
                block_size: BlockSize::Bytes512,
                output: "/home/me/pics.iso".into(),
            }))
        );
    }

    #[test]
    fn create_refuses_bad_combinations_and_values() {
        assert!(parse(&args(&["create", "-o", "x.img"])).is_err(), "no source");
        assert!(parse(&args(&["create", "-size", "1m", "-srcfolder", "d", "-o", "x"])).is_err(), "two sources");
        assert!(parse(&args(&["create", "-size", "1m"])).is_err(), "no output");
        assert!(parse(&args(&["create", "-size", "1m", "-format", "qcow2", "-o", "x"])).is_err());
        assert!(parse(&args(&["create", "-size", "1m", "-blocksize", "1024", "-o", "x"])).is_err());
        assert!(parse(&args(&["create", "-size", "1m", "-bogus", "-o", "x"])).is_err());
    }

    #[test]
    fn attach_is_read_only_unless_asked() {
        assert_eq!(parse(&args(&["attach", "/home/me/a.iso"])), Ok(Command::Attach { path: "/home/me/a.iso".into(), read_only: true }));
        assert_eq!(
            parse(&args(&["attach", "-readwrite", "/home/me/a.img"])),
            Ok(Command::Attach { path: "/home/me/a.img".into(), read_only: false })
        );
        assert!(parse(&args(&["attach"])).is_err());
        assert!(parse(&args(&["attach", "a.iso", "b.iso"])).is_err());
    }

    #[test]
    fn detach_info_and_verify_take_one_operand() {
        assert_eq!(parse(&args(&["detach", "/dev/loop3"])), Ok(Command::Detach("/dev/loop3".into())));
        assert_eq!(parse(&args(&["info", "a.iso"])), Ok(Command::Info("a.iso".into())));
        assert_eq!(parse(&args(&["verify", "a.iso"])), Ok(Command::Verify("a.iso".into())));
        assert!(parse(&args(&["info"])).is_err());
        assert!(parse(&args(&["verify", "a", "b"])).is_err());
        assert!(parse(&args(&["detach", "-x"])).is_err());
    }

    #[test]
    fn convert_needs_the_raw_format_and_an_output() {
        assert_eq!(
            parse(&args(&["convert", "a.img", "-format", "raw", "-blocksize", "4096", "-o", "b.img"])),
            Ok(Command::Convert { src: "a.img".into(), block_size: BlockSize::Bytes4096, output: "b.img".into() })
        );
        assert!(parse(&args(&["convert", "a.img", "-o", "b.img"])).is_err(), "no format");
        assert!(parse(&args(&["convert", "a.img", "-format", "asif", "-o", "b.img"])).is_err());
        assert!(parse(&args(&["convert", "a.img", "-format", "raw"])).is_err(), "no output");
    }

    #[test]
    fn unknown_commands_are_named() {
        assert_eq!(parse(&args(&["burn", "x"])), Err("unknown command burn".into()));
    }

    #[test]
    fn the_shell_answer_gives_its_body_or_its_reason() {
        assert_eq!(reply_body("ok /dev/loop3 /run/media/me/My Disk\n").as_deref(), Ok("/dev/loop3 /run/media/me/My Disk"));
        assert_eq!(reply_body("ok\n").as_deref(), Ok(""));
        assert_eq!(reply_body("error: the image is not owned by the account\n"), Err("the image is not owned by the account".into()));
        assert!(reply_body("").is_err());
    }

    #[test]
    fn iso_writers_get_their_own_argument_order() {
        assert_eq!(
            mkisofs_args("xorriso", Some("PICS"), "/h/p.iso", "/h/Pics"),
            vec!["-as", "mkisofs", "-V", "PICS", "-o", "/h/p.iso", "/h/Pics"]
        );
        assert_eq!(mkisofs_args("genisoimage", None, "/h/p.iso", "/h/Pics"), vec!["-o", "/h/p.iso", "/h/Pics"]);
    }

    #[test]
    fn a_blank_image_is_a_file_of_the_asked_size() {
        let dir = scratch("blank");
        let out = dir.join("blank.img");
        let text = write_blank(out.to_str().unwrap(), 1 << 20, BlockSize::Bytes512).unwrap();
        assert!(text.contains("2048 blocks"));
        assert_eq!(std::fs::metadata(&out).unwrap().len(), 1 << 20);
        assert!(write_blank(out.to_str().unwrap(), 1 << 20, BlockSize::Bytes512).is_err(), "never overwrites");
        assert!(write_blank(dir.join("bad.img").to_str().unwrap(), 1000, BlockSize::Bytes512).is_err());
    }

    #[test]
    fn info_names_a_raw_image_and_its_size() {
        let text = info_text("/h/a.img", &[7u8; 100], 100);
        assert!(text.contains("contents: raw image"));
        assert!(text.contains("size: 100 bytes"));
        assert!(!text.contains("declared size"));
    }

    #[test]
    fn verify_fails_a_truncated_iso_and_passes_a_raw_image() {
        // An ISO head that declares 1000 blocks of 2048 bytes, in a file of 1024 bytes.
        let mut head = vec![0u8; 32768 + 2048];
        head[32768] = 1;
        head[32769..32774].copy_from_slice(b"CD001");
        head[32774] = 1;
        head[32768 + 80..32768 + 84].copy_from_slice(&1000u32.to_le_bytes());
        head[32768 + 128..32768 + 130].copy_from_slice(&2048u16.to_le_bytes());
        let err = verify_text("/h/a.iso", &head, 1024).unwrap_err();
        assert!(err.starts_with("/h/a.iso: image is 1024 bytes"), "{err}");
        assert!(verify_text("/h/a.img", &[1u8; 64], 512).unwrap().contains("verified, raw image of 512 bytes"));
    }

    #[test]
    fn convert_copies_a_whole_block_image_and_refuses_a_partial_one() {
        let dir = scratch("convert");
        let src = dir.join("src.img");
        std::fs::write(&src, vec![3u8; 4096]).unwrap();
        let out = dir.join("out.img");
        let text = run(&Command::Convert { src: src.to_str().unwrap().into(), block_size: BlockSize::Bytes4096, output: out.to_str().unwrap().into() }).unwrap();
        assert!(text.contains("1 blocks"));
        assert_eq!(std::fs::read(&out).unwrap(), vec![3u8; 4096]);
        let odd = dir.join("odd.img");
        std::fs::write(&odd, vec![0u8; 1000]).unwrap();
        assert!(run(&Command::Convert { src: odd.to_str().unwrap().into(), block_size: BlockSize::Bytes512, output: dir.join("o2.img").to_str().unwrap().into() }).is_err());
    }

    #[test]
    fn help_text_is_returned_by_run() {
        assert_eq!(run(&Command::Help).unwrap(), USAGE);
    }
}
