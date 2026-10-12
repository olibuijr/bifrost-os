//! `bifrost-audio-pulse` (owner: task 16): the PulseAudio native protocol (version 35+) on
//! `$XDG_RUNTIME_DIR/pulse/native`, each stream a HAL client.
//!
//! Started by the systemd user socket `bifrost-audio-pulse.socket` (the listening socket arrives as fd 3), or on its
//! own, in which case it binds the socket itself. See docs/audio/16-pulse-compat.md.

mod backend;
mod proto;
mod ring;
mod server;
mod session;
mod tagstruct;

use std::process::ExitCode;

fn main() -> ExitCode {
    let listener = match server::inherited_listener() {
        Some(l) => l,
        None => {
            let path = server::default_socket_path();
            match server::bind(&path) {
                Ok(l) => {
                    eprintln!("is.olibuijr.audio/client: listening on {}", path.display());
                    l
                }
                Err(e) => {
                    eprintln!("is.olibuijr.audio/client: cannot bind {}: {}", path.display(), e);
                    return ExitCode::FAILURE;
                }
            }
        }
    };
    let backend = match backend::HalBackend::connect() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("is.olibuijr.audio/client: the audio server is not reachable: {:?}", e);
            return ExitCode::FAILURE;
        }
    };
    let mut srv = match server::Server::new(listener, Box::new(backend)) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("is.olibuijr.audio/client: {}", e);
            return ExitCode::FAILURE;
        }
    };
    loop {
        if let Err(e) = srv.step(-1) {
            eprintln!("is.olibuijr.audio/client: poll failed: {}", e);
            return ExitCode::FAILURE;
        }
    }
}
