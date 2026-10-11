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
    let uid = privd::account_ids().map(|(u, _, _)| u);
    let gid = std::fs::read_to_string("/etc/group").ok().and_then(|g| privd::parse_group(&g, "bifrost-session"));
    if uid.is_none() {
        eprintln!("bifrost-privd: no valid installed account; only root may connect");
    }
    if gid.is_none() {
        eprintln!("bifrost-privd: group bifrost-session is missing (systemd-sysusers); the socket stays root-only");
    }
    let cfg = server::Config::system(uid, gid);
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
