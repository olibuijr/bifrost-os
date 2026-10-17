//! `bifrostd [root]`: root defaults to `BIFROST_ROOT` or `/var/lib/springboard`.
//! `bifrostd desktop-ready [marks]`: waits for the first desktop frame and starts `bifrost-after-desktop.target`.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

static QUIT: AtomicBool = AtomicBool::new(false);

extern "C" {
    fn signal(sig: i32, handler: extern "C" fn(i32)) -> usize;
}

extern "C" fn on_term(_: i32) {
    QUIT.store(true, Ordering::SeqCst);
}

fn main() {
    let mut args = std::env::args_os().skip(1);
    if args.next().is_some_and(|a| a.to_str() == Some("desktop-ready")) {
        let marks: PathBuf = args.next().map(Into::into).unwrap_or_else(|| bifrostd::BOOT_MARKS.into());
        bifrostd::release_after_desktop(Path::new(&marks));
        return;
    }
    let root: PathBuf = std::env::args_os().nth(1).map(Into::into).or_else(|| std::env::var_os("BIFROST_ROOT").map(Into::into)).unwrap_or_else(|| "/var/lib/springboard".into());
    unsafe {
        signal(15, on_term);
        signal(2, on_term);
    }
    eprintln!("bifrostd {} supervising {} (services start on demand)", env!("CARGO_PKG_VERSION"), root.display());
    // the priority socket (Game Mode) is served on its own thread, so the supervisor loop never waits for a peer
    let run = bifrostd::run_dir(&root);
    let _ = std::fs::create_dir_all(&run);
    let (socket, apps) = (run.join(bifrostd::PRIORITY_SOCKET), root.join("apps"));
    std::thread::spawn(move || bifrostd::priority::serve(socket, apps));
    let mut sup = bifrostd::Supervisor::new(root);
    while !QUIT.load(Ordering::SeqCst) {
        sup.wait(bifrostd::TICK);
        sup.tick();
    }
    sup.shutdown();
}
