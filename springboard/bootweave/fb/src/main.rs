//! Paints Bootweave frame 0 into `/dev/fb0` (docs/boot-architecture.md). Run from the initramfs before the root is
//! mounted; exits non-zero when nothing could be painted, and the boot goes on either way.

use bootweave_fb::{paint, query, FbInfo, Options, USAGE};
use std::fs::{File, OpenOptions};
use std::io;
use std::thread::sleep;
use std::time::{Duration, Instant};

/// Opens the framebuffer, retrying until `wait_ms` has passed: the display driver may still be probing.
fn open_fb(dev: &str, wait_ms: u64) -> io::Result<File> {
    let deadline = Instant::now() + Duration::from_millis(wait_ms);
    loop {
        match OpenOptions::new().read(true).write(true).open(dev) {
            Ok(f) => return Ok(f),
            Err(e) if Instant::now() >= deadline => return Err(e),
            Err(_) => sleep(Duration::from_millis(20)),
        }
    }
}

fn run(args: &[String]) -> Result<(), String> {
    let o = Options::parse(args).map_err(|e| format!("{e}\n{USAGE}"))?;
    if let (Some(path), Some((w, h))) = (&o.raw, o.size) {
        let file = OpenOptions::new().write(true).create(true).truncate(true).open(path).map_err(|e| format!("{path}: {e}"))?;
        return paint(&file, &FbInfo::raw(w, h, o.order), o.appearance).map_err(|e| format!("{path}: {e}"));
    }
    let fb = open_fb(&o.dev, o.wait_ms).map_err(|e| format!("{}: {e}", o.dev))?;
    let info = query(&fb).map_err(|e| format!("{}: {e}", o.dev))?;
    paint(&fb, &info, o.appearance).map_err(|e| format!("{}: {e}", o.dev))?;
    eprintln!("bootweave-fb: frame 0 on {} ({}x{}, stride {})", o.dev, info.width, info.height, info.line_bytes);
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(e) = run(&args) {
        eprintln!("bootweave-fb: {e}");
        std::process::exit(1);
    }
}
