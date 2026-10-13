//! Mail app entry point. The window and the network layer are not wired yet; this binary only checks that the
//! core library is linked and says so instead of opening an empty window.

fn main() {
    eprintln!("bifrost-mail {}: the mail window is not wired yet; the core is in bifrost_mail", env!("CARGO_PKG_VERSION"));
    std::process::exit(2);
}
