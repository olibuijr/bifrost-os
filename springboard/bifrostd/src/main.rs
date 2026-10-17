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

/// `bifrostd zram-setup`: the boot step of `bifrost-zram.service`.
fn args_is_zram_setup() -> bool {
    std::env::args_os().nth(1).is_some_and(|a| a.to_str() == Some("zram-setup"))
}

fn main() {
    let mut args = std::env::args_os().skip(1);
    if args.next().is_some_and(|a| a.to_str() == Some("desktop-ready")) {
        let marks: PathBuf = args.next().map(Into::into).unwrap_or_else(|| bifrostd::BOOT_MARKS.into());
        bifrostd::release_after_desktop(Path::new(&marks));
        return;
    }
    if args_is_zram_setup() {
        let ram = bifrostd::pager::Meminfo::from_proc().map(|m| m.total).unwrap_or(0);
        match bifrostd::pager::zram_setup(ram) {
            Ok(bytes) => eprintln!("bifrostd: zram swap {bytes} bytes"),
            Err(e) => {
                eprintln!("bifrostd: zram swap: {e}");
                std::process::exit(1);
            }
        }
        return;
    }
    let root: PathBuf = std::env::args_os().nth(1).map(Into::into).or_else(|| std::env::var_os("BIFROST_ROOT").map(Into::into)).unwrap_or_else(|| "/var/lib/springboard".into());
    unsafe {
        signal(15, on_term);
        signal(2, on_term);
    }
    eprintln!("bifrostd {} supervising {} (services start on demand)", env!("CARGO_PKG_VERSION"), root.display());
    let mut sup = bifrostd::Supervisor::new(root);
    let mut pager = bifrostd::pager::Pager::new();
    let mut purger = bifrostd::purge::Purger::default();
    while !QUIT.load(Ordering::SeqCst) {
        sup.wait(bifrostd::TICK);
        sup.tick();
        let now = bifrostd::purge::now_secs();
        pager.tick(now);
        purger.tick(now);
    }
    sup.shutdown();
}
