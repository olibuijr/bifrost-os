//! `bifrost-audiod`: parse args, lock memory, drop to the `audio` user when started as root (keeping CAP_SYS_NICE),
//! open the control socket and run the server loop until SIGTERM or SIGINT. Owner: task 05.

use bifrost_audio_driver::loopback::LoopbackDriver;
use bifrost_audio_driver::Driver;
use bifrost_audiod::control::{default_socket, listen, serve, CoreConfig, NoObjects, ServerCore};
use bifrost_audiod::io::log;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::sync::atomic::AtomicBool;

const CAP_SYS_NICE: u32 = 23;

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
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--socket" => socket = args.next().unwrap_or(socket),
            "-h" | "--help" => {
                println!("bifrost-audiod [--socket PATH]");
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

    // Drivers: the loopback runs on every machine. The ALSA driver (bifrost-audio-alsa, task 04) is added here once
    // its `AlsaDriver` lands: `drivers.push(Box::new(AlsaDriver::new()))`.
    let drivers: Vec<Box<dyn Driver>> = vec![Box::new(LoopbackDriver::realtime())];

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
