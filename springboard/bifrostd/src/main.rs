//! `bifrostd [root]`: root defaults to `BIFROST_ROOT` or `/var/lib/springboard`.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

static QUIT: AtomicBool = AtomicBool::new(false);

extern "C" {
    fn signal(sig: i32, handler: extern "C" fn(i32)) -> usize;
}

extern "C" fn on_term(_: i32) {
    QUIT.store(true, Ordering::SeqCst);
}

fn main() {
    let root: PathBuf = std::env::args_os().nth(1).map(Into::into).or_else(|| std::env::var_os("BIFROST_ROOT").map(Into::into)).unwrap_or_else(|| "/var/lib/springboard".into());
    unsafe {
        signal(15, on_term);
        signal(2, on_term);
    }
    eprintln!("bifrostd {} supervising {}", env!("CARGO_PKG_VERSION"), root.display());
    let mut sup = bifrostd::Supervisor::new(root);
    while !QUIT.load(Ordering::SeqCst) {
        sup.tick();
        std::thread::sleep(Duration::from_secs(2));
    }
    sup.shutdown();
}
