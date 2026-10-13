//! `softwareupdate`: lists and installs updates through the update agent. See `bifrost_cmd::softwareupdate`.

use bifrost_cmd::die;
use bifrost_cmd::softwareupdate::{self, Mode};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mode = match softwareupdate::parse(&args) {
        Ok(mode) => mode,
        Err(e) => die("softwareupdate", &e, 2),
    };
    let sock = softwareupdate::socket_path(
        std::env::var("BIFROST_SERVICE_SOCK").ok().as_deref(),
        std::fs::metadata("/proc/self").is_ok_and(|m| std::os::unix::fs::MetadataExt::uid(&m) == 0),
        std::env::var("XDG_DATA_HOME").ok().as_deref(),
        std::env::var("HOME").ok().as_deref(),
    );
    match mode {
        Mode::Help => println!("{}", softwareupdate::USAGE),
        Mode::List => match softwareupdate::check(&sock) {
            Ok(state) => print!("{}", softwareupdate::list_text(&state)),
            Err(e) => die("softwareupdate", &e, 1),
        },
        Mode::Install { labels, all } => match softwareupdate::install(&sock, &labels, all) {
            Ok(text) => print!("{text}"),
            Err(e) => die("softwareupdate", &e, 1),
        },
    }
}
