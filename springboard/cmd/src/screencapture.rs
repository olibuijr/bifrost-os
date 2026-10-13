//! `screencapture`: saves the screen as a PNG file, or puts it on the clipboard. The shell answers the control socket's
//! `shot` request with the whole panel as a raw `RIMG` frame (premultiplied RGBA, width and height as little-endian u32).
//! This tool reads it, crops it for `-R`, encodes it with the shell's PNG writer and writes the file or the clipboard.
//!
//! Supported: `-c` (clipboard), `-R x,y,w,h` (a rectangle in pixels of the panel), `-T seconds` (delay before the capture),
//! `-t png` (the only format), `-D 1` (the main display, the only one), and one file operand. `-x` (no sounds), `-m` (main
//! display only), `-r` (no DPI metadata) and `-a` (no attached windows) are accepted: sounds are never played, the panel is
//! the only display, the PNG has no DPI chunk, and a full-screen capture has no attached windows to leave out.
//! Without a file operand the capture goes to the current folder as `Screenshot-<unix seconds>.png`.
//! `-i` (a dragged selection) and `-w` (a window picked by its frame, with its shadow) start the shell's screenshot toolbar,
//! the same overlay as Shift-Cmd-5. The command returns once the toolbar is up; the capture is saved to Photos when the user
//! finishes, so `-i` and `-w` take no `-c`, `-R`, `-T` or file name.
//! `-v` records a movie of the screen (or of `-R`) through the desktop until the recording is stopped from the menu bar,
//! and `-V seconds` records for that long. `-T` waits before the recording starts. The movie goes to the file operand
//! (relative to the current folder), else to the desktop's dated file. Video takes no `-c`, `-i` or `-w`.
//! Refused with a message rather than run wrongly: the other selection modes (`-W -s -S -l`), the cursor
//! (`-C`), the sound and key-stroke options (`-G -g -k`), the built-in function strip (`-b`), the other formats, and the
//! options that open or send the picture somewhere (`-M -P -B -p -u -U -J -o -d`).
//! Gap: `-R` coordinates are panel pixels, not points, and the capture has no cursor.

use crate::control;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::io::Write;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const USAGE: &str = "usage: screencapture [-x] [-m] [-r] [-a] [-c] [-i | -w] [-v [-V seconds]] [-R x,y,w,h] [-T seconds] [-t png] [-D 1] [file]";
/// The MIME type the clipboard takes a picture under.
pub const PNG_MIME: &str = "image/png";
/// The shell's raw frame magic.
pub const FRAME_MAGIC: &[u8; 4] = b"RIMG";
const FRAME_HEADER: usize = 12;

/// A rectangle of the panel, in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// The interactive capture a `-i` or `-w` starts on the desktop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interactive {
    /// `-i`: a rectangle dragged over the screen.
    Select,
    /// `-w`: the window under the pointer.
    Window,
}

/// The parsed command line.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Options {
    /// `-i` or `-w`: the desktop's screenshot toolbar takes the capture.
    pub interactive: Option<Interactive>,
    /// `-c`: the picture goes to the clipboard instead of a file.
    pub clipboard: bool,
    /// `-R`: capture this rectangle instead of the whole panel.
    pub rect: Option<Rect>,
    /// `-T`: seconds to wait before the capture.
    pub delay: u32,
    /// `-v` or `-V`: record a movie instead of taking a picture.
    pub video: bool,
    /// `-V`: the length of the recording in seconds (without it, the recording runs until it is stopped).
    pub seconds: Option<u32>,
    /// The file to write (one file for the one display).
    pub file: Option<String>,
}

/// What the command line asks for: the help text, or a capture.
#[derive(Debug, PartialEq, Eq)]
pub enum Parsed {
    Help,
    Run(Options),
}

/// A decoded panel frame: premultiplied packed pixels, the low byte being the red channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u32>,
}

/// Why an option is refused, or what it means for the capture.
fn refusal(c: char) -> String {
    match c {
        'W' | 's' | 'S' | 'l' => format!("-{c}: this selection mode is not supported yet"),
        'C' => "-C: the cursor is not part of the desktop's frame yet".into(),
        'G' | 'g' | 'k' => format!("-{c}: sound and key-stroke capture are not supported yet"),
        'b' => "-b: the function strip is not part of this desktop".into(),
        'M' | 'P' | 'B' | 'p' | 'u' | 'U' | 'J' | 'o' | 'd' => format!("-{c} is not supported yet"),
        other => format!("unknown option -{other}"),
    }
}

/// Reads the arguments. Options bundle as `-cx`, and the value options take the rest of their word or the next argument.
pub fn parse(args: &[String]) -> Result<Parsed, String> {
    let mut o = Options::default();
    let mut operands = Vec::new();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        if arg == "-help" || arg == "--help" || arg == "-h" {
            return Ok(Parsed::Help);
        }
        if !arg.starts_with('-') || arg == "-" {
            operands.push(arg.clone());
            continue;
        }
        let mut chars = arg[1..].chars();
        while let Some(c) = chars.next() {
            match c {
                'c' => o.clipboard = true,
                'v' => o.video = true,
                'i' | 'w' => {
                    if o.interactive.is_some() {
                        return Err("-i and -w pick one mode".into());
                    }
                    o.interactive = Some(if c == 'i' { Interactive::Select } else { Interactive::Window });
                }
                'x' | 'm' | 'r' | 'a' => {}
                'T' | 'R' | 't' | 'D' | 'V' => {
                    let rest: String = chars.by_ref().collect();
                    let value = if rest.is_empty() { it.next().cloned().ok_or(format!("-{c} needs a value"))? } else { rest };
                    apply_value(&mut o, c, &value)?;
                    break;
                }
                other => return Err(refusal(other)),
            }
        }
    }
    if operands.len() > 1 {
        return Err("one file for the one display: give a single file name".into());
    }
    o.file = operands.pop();
    if o.interactive.is_some() && (o.clipboard || o.rect.is_some() || o.delay > 0 || o.file.is_some()) {
        return Err("-i and -w save to Photos from the desktop: they take no -c, -R, -T or file name".into());
    }
    if o.video && (o.clipboard || o.interactive.is_some()) {
        return Err("-v records a movie file: it takes no -c, -i or -w".into());
    }
    Ok(Parsed::Run(o))
}

/// The control request that starts the desktop's capture for `mode`.
pub fn interactive_request(mode: Interactive) -> &'static str {
    match mode {
        Interactive::Select => "capture select",
        Interactive::Window => "capture window",
    }
}

/// Asks the desktop to open its screenshot toolbar for `mode` and reads its answer.
pub fn start_interactive(mode: Interactive, port: u16) -> Result<(), String> {
    let reply = control::request(interactive_request(mode), port)?;
    let text = String::from_utf8_lossy(&reply);
    let text = text.trim();
    if text == "ok" {
        Ok(())
    } else {
        Err(text.strip_prefix("error: ").unwrap_or("the desktop did not start the capture").to_string())
    }
}

fn apply_value(o: &mut Options, c: char, value: &str) -> Result<(), String> {
    match c {
        'T' => o.delay = value.trim().parse().map_err(|_| format!("-T needs a number of seconds, not {value}"))?,
        'R' => o.rect = Some(parse_rect(value)?),
        't' if value == "png" => {}
        't' => return Err(format!("the {value} format is not supported yet (png only)")),
        'D' if value == "1" => {}
        'D' => return Err(format!("display {value} is not reachable: the desktop drives one panel (-D 1)")),
        'V' => {
            o.video = true;
            o.seconds = Some(value.trim().parse().map_err(|_| format!("-V needs a number of seconds, not {value}"))?);
        }
        _ => unreachable!("only value options reach apply_value"),
    }
    Ok(())
}

/// Parses `x,y,w,h`. The width and height must be at least one pixel.
pub fn parse_rect(s: &str) -> Result<Rect, String> {
    let parts: Vec<u32> = s.split(',').map(|p| p.trim().parse::<u32>()).collect::<Result<_, _>>().map_err(|_| format!("-R needs x,y,width,height, not {s}"))?;
    match parts[..] {
        [x, y, w, h] if w > 0 && h > 0 => Ok(Rect { x, y, w, h }),
        [_, _, _, _] => Err("-R needs a width and height of at least one pixel".into()),
        _ => Err(format!("-R needs x,y,width,height, not {s}")),
    }
}

/// Reads a `RIMG` frame. The body must be exactly `w * h` pixels, four bytes each.
pub fn parse_frame(bytes: &[u8]) -> Result<Frame, String> {
    if bytes.len() < FRAME_HEADER || &bytes[..4] != FRAME_MAGIC {
        let text = String::from_utf8_lossy(bytes);
        return Err(text.trim().strip_prefix("error: ").unwrap_or("the desktop did not send a frame").to_string());
    }
    let w = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
    let h = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as usize;
    let body = &bytes[FRAME_HEADER..];
    if w == 0 || h == 0 || w.checked_mul(h).and_then(|n| n.checked_mul(4)) != Some(body.len()) {
        return Err("the desktop sent a frame of the wrong size".into());
    }
    let px = body.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
    Ok(Frame { w, h, px })
}

/// Asks the shell for the whole panel at full resolution.
pub fn grab(port: u16) -> Result<Frame, String> {
    parse_frame(&control::request("shot 1", port)?)
}

/// The part of `frame` inside `r`. A rectangle that reaches past the panel is an error.
pub fn crop(frame: &Frame, r: Rect) -> Result<Frame, String> {
    let (x, y, w, h) = (r.x as usize, r.y as usize, r.w as usize, r.h as usize);
    if x.checked_add(w).is_none_or(|v| v > frame.w) || y.checked_add(h).is_none_or(|v| v > frame.h) {
        return Err(format!("-R {},{},{},{} is outside the {}x{} panel", r.x, r.y, r.w, r.h, frame.w, frame.h));
    }
    let mut px = Vec::with_capacity(w * h);
    for row in y..y + h {
        px.extend_from_slice(&frame.px[row * frame.w + x..row * frame.w + x + w]);
    }
    Ok(Frame { w, h, px })
}

/// The PNG bytes for a frame, from the shell's PNG writer.
pub fn to_png(frame: &Frame) -> Vec<u8> {
    springboard::pngenc::encode_rgb(frame.w, frame.h, &frame.px)
}

/// The default file name for a capture taken at `unix` seconds.
pub fn default_name(unix: u64) -> String {
    format!("Screenshot-{unix}.png")
}

/// Puts PNG bytes on the Wayland clipboard as an image.
pub fn copy_png(png: &[u8]) -> Result<(), String> {
    let mut child = Command::new("wl-copy").args(["--type", PNG_MIME]).stdin(Stdio::piped()).stdout(Stdio::null()).spawn().map_err(|e| format!("wl-copy: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(png).map_err(|e| format!("wl-copy: {e}"))?;
    }
    let status = child.wait().map_err(|e| format!("wl-copy: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("wl-copy failed ({status})"))
    }
}

/// Sends a control request and returns the desktop's text answer, or its error.
fn ask(line: &str, port: u16) -> Result<String, String> {
    let answer = String::from_utf8_lossy(&control::request(line, port)?).trim().to_string();
    match answer.strip_prefix("error: ") {
        Some(e) => Err(e.to_string()),
        None => Ok(answer),
    }
}

/// Records the screen through the desktop: for `-V` seconds, else until the recording is stopped from the menu bar.
/// The command returns once the recording has ended.
fn record(o: &Options, port: u16) -> Result<(), String> {
    if o.delay > 0 {
        std::thread::sleep(Duration::from_secs(o.delay as u64));
    }
    let area = match o.rect {
        Some(r) => format!(" {},{},{},{}", r.x, r.y, r.w, r.h),
        None => " screen".to_string(),
    };
    let file = match &o.file {
        Some(f) => {
            let path = std::env::current_dir().map_err(|e| e.to_string())?.join(f);
            format!(" {}", springboard::bare::control::pct_encode(&path.to_string_lossy()))
        }
        None => String::new(),
    };
    ask(&format!("rec start{area}{file}"), port)?;
    match o.seconds {
        Some(s) => {
            std::thread::sleep(Duration::from_secs(s as u64));
            ask("rec stop", port)?;
        }
        None => loop {
            std::thread::sleep(Duration::from_millis(500));
            if !ask("rec status", port)?.starts_with("recording") {
                break;
            }
        },
    }
    Ok(())
}

/// Runs a capture: waits the delay, grabs the panel, crops it, then writes the file or the clipboard.
pub fn run(o: &Options, port: u16) -> Result<PathBuf, String> {
    if o.video {
        record(o, port)?;
        return Ok(PathBuf::from("recording"));
    }
    if let Some(mode) = o.interactive {
        start_interactive(mode, port)?;
        return Ok(PathBuf::from("Photos"));
    }
    if o.delay > 0 {
        std::thread::sleep(Duration::from_secs(o.delay as u64));
    }
    let mut frame = grab(port)?;
    if let Some(r) = o.rect {
        frame = crop(&frame, r)?;
    }
    let png = to_png(&frame);
    if o.clipboard {
        copy_png(&png)?;
        return Ok(PathBuf::from("clipboard"));
    }
    let unix = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let path = PathBuf::from(o.file.clone().unwrap_or_else(|| default_name(unix)));
    std::fs::write(&path, &png).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn frame_bytes(w: u32, h: u32, px: &[u32]) -> Vec<u8> {
        let mut out = FRAME_MAGIC.to_vec();
        out.extend(w.to_le_bytes());
        out.extend(h.to_le_bytes());
        for p in px {
            out.extend(p.to_le_bytes());
        }
        out
    }

    #[test]
    fn options_bundle_and_take_values() {
        let Parsed::Run(o) = parse(&args(&["-cx", "-T", "3", "-R", "10,20,30,40", "shot.png"])).unwrap() else { panic!("help") };
        assert!(o.clipboard);
        assert_eq!(o.delay, 3);
        assert_eq!(o.rect, Some(Rect { x: 10, y: 20, w: 30, h: 40 }));
        assert_eq!(o.file.as_deref(), Some("shot.png"));
        let Parsed::Run(attached) = parse(&args(&["-T5", "-tpng"])).unwrap() else { panic!("help") };
        assert_eq!(attached.delay, 5);
    }

    #[test]
    fn video_options_record_until_stopped_or_for_a_length() {
        let Parsed::Run(v) = parse(&args(&["-v"])).unwrap() else { panic!("help") };
        assert!(v.video);
        assert_eq!(v.seconds, None);
        let Parsed::Run(timed) = parse(&args(&["-V", "5", "clip.mov"])).unwrap() else { panic!("help") };
        assert!(timed.video);
        assert_eq!(timed.seconds, Some(5));
        assert_eq!(timed.file.as_deref(), Some("clip.mov"));
        let Parsed::Run(attached) = parse(&args(&["-V5"])).unwrap() else { panic!("help") };
        assert_eq!(attached.seconds, Some(5));
        assert!(parse(&args(&["-V", "soon"])).unwrap_err().contains("seconds"));
        assert!(parse(&args(&["-v", "-c"])).unwrap_err().contains("movie"));
        assert!(parse(&args(&["-v", "-i"])).unwrap_err().contains("movie"));
    }

    #[test]
    fn interactive_and_video_modes_are_refused_by_name() {
        assert!(parse(&args(&["-W"])).unwrap_err().contains("-W"));
        assert!(parse(&args(&["-G"])).unwrap_err().contains("sound"));
        assert!(parse(&args(&["-t", "jpg"])).unwrap_err().contains("png only"));
        assert!(parse(&args(&["-D", "2"])).unwrap_err().contains("-D 1"));
        assert!(parse(&args(&["-q"])).unwrap_err().contains("unknown option -q"));
    }

    #[test]
    fn interactive_modes_ask_the_desktop_toolbar() {
        let Parsed::Run(sel) = parse(&args(&["-i"])).unwrap() else { panic!("help") };
        assert_eq!(sel.interactive, Some(Interactive::Select));
        let Parsed::Run(win) = parse(&args(&["-x", "-w"])).unwrap() else { panic!("help") };
        assert_eq!(win.interactive, Some(Interactive::Window));
        assert_eq!(interactive_request(Interactive::Select), "capture select");
        assert_eq!(interactive_request(Interactive::Window), "capture window");
        assert!(parse(&args(&["-i", "-w"])).unwrap_err().contains("one mode"));
        assert!(parse(&args(&["-i", "-c"])).unwrap_err().contains("Photos"));
        assert!(parse(&args(&["-w", "-T", "3"])).unwrap_err().contains("Photos"));
        assert!(parse(&args(&["-i", "shot.png"])).unwrap_err().contains("Photos"));
    }

    #[test]
    fn help_and_operand_count() {
        assert_eq!(parse(&args(&["-h"])), Ok(Parsed::Help));
        assert!(parse(&args(&["a.png", "b.png"])).unwrap_err().contains("single file"));
        assert_eq!(parse(&args(&[])), Ok(Parsed::Run(Options::default())));
    }

    #[test]
    fn rectangles_need_four_numbers_and_a_size() {
        assert_eq!(parse_rect("0,0,1,1"), Ok(Rect { x: 0, y: 0, w: 1, h: 1 }));
        assert!(parse_rect("1,2,3").is_err());
        assert!(parse_rect("1,2,0,4").unwrap_err().contains("at least one pixel"));
        assert!(parse_rect("a,2,3,4").is_err());
    }

    #[test]
    fn frames_round_trip_and_bad_frames_are_named() {
        let f = parse_frame(&frame_bytes(2, 1, &[0xff00_00ff, 0x0102_0304])).unwrap();
        assert_eq!((f.w, f.h), (2, 1));
        assert_eq!(f.px, vec![0xff00_00ff, 0x0102_0304]);
        assert_eq!(parse_frame(b"error: no frame yet\n"), Err("no frame yet".into()));
        let mut short = frame_bytes(2, 2, &[1, 2, 3, 4]);
        short.truncate(short.len() - 4);
        assert!(parse_frame(&short).unwrap_err().contains("wrong size"));
    }

    #[test]
    fn crop_takes_the_rectangle_and_refuses_what_is_outside() {
        let f = Frame { w: 3, h: 2, px: vec![1, 2, 3, 4, 5, 6] };
        let c = crop(&f, Rect { x: 1, y: 0, w: 2, h: 2 }).unwrap();
        assert_eq!((c.w, c.h, c.px), (2, 2, vec![2, 3, 5, 6]));
        assert!(crop(&f, Rect { x: 2, y: 0, w: 2, h: 1 }).unwrap_err().contains("outside"));
        assert!(crop(&f, Rect { x: u32::MAX, y: 0, w: 2, h: 1 }).is_err());
    }

    #[test]
    fn the_png_has_its_signature_and_the_default_name_is_dated() {
        let png = to_png(&Frame { w: 1, h: 1, px: vec![0xff11_2233] });
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(default_name(1_791_073_395), "Screenshot-1791073395.png");
    }

    #[test]
    fn a_capture_writes_the_cropped_file() {
        let dir = std::env::temp_dir().join(format!("bifrost-screencapture-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("crop.png");
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let bytes = frame_bytes(2, 2, &[1, 2, 3, 4]);
        let server = std::thread::spawn(move || {
            let (mut conn, _) = std::net::TcpListener::accept(&listener).unwrap();
            use std::io::{Read, Write};
            let mut buf = [0u8; 32];
            let n = conn.read(&mut buf).unwrap();
            conn.write_all(&bytes).unwrap();
            String::from_utf8_lossy(&buf[..n]).into_owned()
        });
        let o = Options { rect: Some(Rect { x: 0, y: 0, w: 1, h: 1 }), file: Some(file.to_string_lossy().into_owned()), ..Options::default() };
        let path = run(&o, port).unwrap();
        assert_eq!(path, file);
        assert_eq!(server.join().unwrap(), "shot 1\n");
        assert_eq!(&std::fs::read(&file).unwrap()[..8], b"\x89PNG\r\n\x1a\n");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
