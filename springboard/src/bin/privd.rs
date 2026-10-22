//! `bifrost-privd`: the root helper daemon (see `springboard::privd`). Runs as root before the desktop session,
//! listens on /run/bifrost/privd.sock (group bifrost-session) and /run/bifrost/privd-root.sock (root only).

#[cfg(target_os = "linux")]
fn main() -> std::process::ExitCode {
    use springboard::privd::{self, methods::RootExec, server};
    use std::sync::Arc;

    if server::euid() != 0 {
        eprintln!("bifrost-privd: must run as root");
        return std::process::ExitCode::from(2);
    }
    if std::env::args().nth(1).as_deref() == Some("--version") {
        println!("bifrost-privd {}", env!("CARGO_PKG_VERSION"));
        return std::process::ExitCode::SUCCESS;
    }
    // faillock's tally outlives a reboot (pam-bifrost-login dir=); the default /run/faillock is tmpfs
    let _ = std::fs::create_dir_all("/var/lib/faillock");
    // the wireless regulatory domain follows the Language & Region country (saved, else the page's default). It is set
    // here because this daemon runs as root from boot, while netd only starts on the first Wi-Fi request.
    springboard::net::regdomain::follow_region(std::path::PathBuf::from(format!("{}/settings.ini", privd::STATE_DIR)));
    // Stage 1 (docs/privd-protocol.md): the shell runs as root, so no process of the account needs the socket. The
    // /proc/<pid>/exe check cannot tell the shell from any other process of the account (security review
    // 2026-10-11), so the account is refused outright until stage 2 hands the user shell a launch-time credential.
    let cfg = server::Config::system(None, None);
    match server::serve(cfg, privd::default_gate(), Arc::new(RootExec)) {
        Ok(handles) => {
            eprintln!("bifrost-privd {} listening on {}", env!("CARGO_PKG_VERSION"), privd::SOCK);
            for h in handles {
                let _ = h.join();
            }
            std::process::ExitCode::FAILURE
        }
        Err(e) => {
            eprintln!("bifrost-privd: cannot listen: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn main() {}
