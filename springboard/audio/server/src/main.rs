//! `bifrost-audiod`: parse args, lock memory, drop to the `audio` user when started as root (keeping CAP_SYS_NICE),
//! open the control socket and run the server loop until SIGTERM or SIGINT. Owner: task 05; drivers: task 24.
//!
//! Drivers: `--drivers LIST` (or `BIFROST_AUDIO_DRIVERS`), a comma list of `alsa`, `loopback` and `null`, default
//! `alsa,loopback`. Devices are objects `1000 + index` in that order (each driver's enumeration order), so the
//! hardware comes first. `null` adds one stereo device on a clock that never moves, for tests.

use bifrost_audio_alsa::AlsaDriver;
use bifrost_audio_driver::loopback::LoopbackDriver;
use bifrost_audio_driver::null::NullDriver;
use bifrost_audio_driver::Driver;
use bifrost_audiod::control::{default_socket, listen, serve, CoreConfig, NoObjects, ServerCore};
use bifrost_audiod::io::log;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::sync::atomic::AtomicBool;

const CAP_SYS_NICE: u32 = 23;

/// The drivers when neither `--drivers` nor `BIFROST_AUDIO_DRIVERS` names any.
const DEFAULT_DRIVERS: &str = "alsa,loopback";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DriverKind {
    Alsa,
    Loopback,
    Null,
}

/// Parses a driver list ("alsa,loopback"): order kept, blanks and repeats dropped, unknown names refused.
fn parse_drivers(list: &str) -> Result<Vec<DriverKind>, String> {
    let mut out = Vec::new();
    for name in list.split(',').map(str::trim).filter(|n| !n.is_empty()) {
        let k = match name {
            "alsa" => DriverKind::Alsa,
            "loopback" => DriverKind::Loopback,
            "null" => DriverKind::Null,
            other => return Err(format!("unknown driver {other} (alsa, loopback, null)")),
        };
        if !out.contains(&k) {
            out.push(k);
        }
    }
    if out.is_empty() {
        return Err("no driver selected".into());
    }
    Ok(out)
}

fn make_driver(k: DriverKind) -> Box<dyn Driver> {
    match k {
        DriverKind::Alsa => Box::new(AlsaDriver::new()),
        DriverKind::Loopback => Box::new(LoopbackDriver::realtime()),
        DriverKind::Null => Box::new(NullDriver::new(vec![NullDriver::stereo("null:0", 48_000.0)])),
    }
}

#[repr(C)]
struct CapHeader {
    version: u32,
    pid: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CapData {
    effective: u32,
    permitted: u32,
    inheritable: u32,
}

/// Root to the `audio` user and group, keeping only CAP_SYS_NICE. The systemd unit starts the daemon as `audio`
/// already (with the ambient capability), so this is for manual starts as root.
fn drop_privileges() -> Result<(), String> {
    // SAFETY: the libc calls below take valid NUL-terminated strings and plain integers; results are checked.
    unsafe {
        if libc::geteuid() != 0 {
            return Ok(());
        }
        let pw = libc::getpwnam(b"audio\0".as_ptr() as *const libc::c_char);
        if pw.is_null() {
            return Err("no `audio` user".into());
        }
        let (uid, gid) = ((*pw).pw_uid, (*pw).pw_gid);
        if libc::initgroups(b"audio\0".as_ptr() as *const libc::c_char, gid) != 0
            || libc::prctl(libc::PR_SET_KEEPCAPS, 1, 0, 0, 0) != 0
            || libc::setgid(gid) != 0
            || libc::setuid(uid) != 0
        {
            return Err(format!("cannot drop to audio: {}", std::io::Error::last_os_error()));
        }
        let hdr = CapHeader { version: 0x2008_0522, pid: 0 };
        let bit = 1u32 << CAP_SYS_NICE;
        let data = [
            CapData { effective: bit, permitted: bit, inheritable: 0 },
            CapData { effective: 0, permitted: 0, inheritable: 0 },
        ];
        if libc::syscall(libc::SYS_capset, &hdr as *const CapHeader, data.as_ptr()) != 0 {
            return Err(format!("capset: {}", std::io::Error::last_os_error()));
        }
    }
    Ok(())
}

/// A signalfd for SIGTERM and SIGINT (blocked for the whole process first, so the poll cannot miss one).
fn signal_fd() -> Option<OwnedFd> {
    // SAFETY: the sigset is initialised by sigemptyset/sigaddset before use; the descriptor is checked and owned.
    unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        libc::sigaddset(&mut set, libc::SIGTERM);
        libc::sigaddset(&mut set, libc::SIGINT);
        libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
        let fd = libc::signalfd(-1, &set, libc::SFD_CLOEXEC);
        (fd >= 0).then(|| OwnedFd::from_raw_fd(fd))
    }
}

fn main() {
    let mut socket = default_socket().to_string();
    let mut driver_list = std::env::var("BIFROST_AUDIO_DRIVERS").unwrap_or_else(|_| DEFAULT_DRIVERS.into());
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--socket" => socket = args.next().unwrap_or(socket),
            "--drivers" => driver_list = args.next().unwrap_or(driver_list),
            "-h" | "--help" => {
                println!("bifrost-audiod [--socket PATH] [--drivers alsa,loopback,null]");
                return;
            }
            other => {
                eprintln!("bifrost-audiod: unknown argument {other}");
                std::process::exit(2);
            }
        }
    }

    let sigs = signal_fd();
    if let Err(e) = drop_privileges() {
        log("io", &e);
        std::process::exit(1);
    }

    // Drivers: the hardware (ALSA) first, so it takes the first device ids, then the loopback.
    let kinds = match parse_drivers(&driver_list) {
        Ok(k) => k,
        Err(e) => {
            eprintln!("bifrost-audiod: {e}");
            std::process::exit(2);
        }
    };
    log("driver", &format!("drivers: {kinds:?}"));
    let drivers: Vec<Box<dyn Driver>> = kinds.into_iter().map(make_driver).collect();

    let listener = match listen(&socket) {
        Ok(l) => l,
        Err(e) => {
            log("io", &format!("cannot listen on {socket}: {e}"));
            std::process::exit(1);
        }
    };
    log("io", &format!("listening on {socket}"));
    let mut core = ServerCore::new(drivers, Box::new(NoObjects), CoreConfig::default());
    // Lock memory after setup so everything setup allocated is resident too (MCL_CURRENT | MCL_FUTURE).
    bifrost_audiod::rt::lock_memory();
    let stop = AtomicBool::new(false);
    serve(&mut core, &listener, &stop, sigs.as_ref().map(|f| f.as_raw_fd()));
    let _ = std::fs::remove_file(&socket);
    log("io", "stopped");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_puts_the_hardware_first() {
        assert_eq!(parse_drivers(DEFAULT_DRIVERS), Ok(vec![DriverKind::Alsa, DriverKind::Loopback]));
    }

    #[test]
    fn driver_lists_keep_their_order_and_drop_blanks_and_repeats() {
        assert_eq!(parse_drivers("null"), Ok(vec![DriverKind::Null]));
        assert_eq!(parse_drivers(" loopback, ,null,loopback "), Ok(vec![DriverKind::Loopback, DriverKind::Null]));
    }

    #[test]
    fn unknown_or_empty_driver_lists_are_refused() {
        assert!(parse_drivers("alsa,oss").unwrap_err().contains("oss"));
        assert_eq!(parse_drivers(" , "), Err("no driver selected".to_string()));
    }

    #[test]
    fn the_test_drivers_enumerate_their_devices() {
        let mut out = Vec::new();
        make_driver(DriverKind::Null).enumerate(&mut out);
        make_driver(DriverKind::Loopback).enumerate(&mut out);
        let uids: Vec<&str> = out.iter().map(|d| d.uid.as_str()).collect();
        assert_eq!(uids, vec!["null:0", "loopback:out", "loopback:in"]);
    }
}
