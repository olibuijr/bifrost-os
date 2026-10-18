//! `bifrost-audiod`: parse args, lock memory, drop to the `audio` user when started as root (keeping CAP_SYS_NICE),
//! open the control socket and run the server loop until SIGTERM or SIGINT. Owner: task 05; drivers: task 24.
//!
//! Drivers: `--drivers LIST` (or `BIFROST_AUDIO_DRIVERS`), a comma list of `alsa`, `loopback`, `plugin` and `null`,
//! default `alsa,loopback` (`plugin` is opt-in until it is verified on hardware). Devices are objects `1000 + index` in that order (each driver's enumeration order), so the
//! hardware comes first (the drivers are the providers of one `CompositeDriver`, which sends each device to the provider
//! that published its uid and refuses a uid two providers publish). `plugin` publishes the virtual devices described by the `*.vdev` files of
//! `BIFROST_AUDIO_PLUGIN_DIR` (default `/etc/bifrost/audio/plugins`). `null` adds one stereo device on a clock that never moves, for tests.

use bifrost_audio_alsa::AlsaDriver;
use bifrost_audio_driver::loopback::LoopbackDriver;
use bifrost_audio_driver::null::NullDriver;
use bifrost_audio_driver::plugin::{PluginDriver, PLUGIN_DIR};
use bifrost_audio_driver::transport::CompositeDriver;
use bifrost_audio_driver::Driver;
use bifrost_audio_policy::dsp::DspManager;
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
    Plugin,
    Null,
}

/// Parses a driver list ("alsa,loopback"): order kept, blanks and repeats dropped, unknown names refused.
fn parse_drivers(list: &str) -> Result<Vec<DriverKind>, String> {
    let mut out = Vec::new();
    for name in list.split(',').map(str::trim).filter(|n| !n.is_empty()) {
        let k = match name {
            "alsa" => DriverKind::Alsa,
            "loopback" => DriverKind::Loopback,
            "plugin" => DriverKind::Plugin,
            "null" => DriverKind::Null,
            other => return Err(format!("unknown driver {other} (alsa, loopback, plugin, null)")),
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

impl DriverKind {
    fn name(self) -> &'static str {
        match self {
            DriverKind::Alsa => "alsa",
            DriverKind::Loopback => "loopback",
            DriverKind::Plugin => "plugin",
            DriverKind::Null => "null",
        }
    }
}

/// The selected drivers as the providers of one composite driver, in the order given.
fn build_composite(kinds: &[DriverKind]) -> CompositeDriver {
    let mut composite = CompositeDriver::new();
    for k in kinds {
        composite.add(k.name(), make_driver(*k));
    }
    composite
}

/// Where the DSP configuration (which stages run for each device) is read from: `--dsp`, `BIFROST_AUDIO_DSP`, or this.
const DSP_FILE: &str = "/etc/bifrost/audio/dsp.conf";

/// Reads a DSP configuration. A missing file is no configuration (`Ok(None)`); a file that does not parse refuses the
/// whole configuration, so a half-read chain never runs.
fn load_dsp(path: &str) -> Result<Option<DspManager>, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{path}: {e}")),
    };
    DspManager::load(&text).map(Some).map_err(|e| format!("{path}: {e:?}"))
}

fn make_driver(k: DriverKind) -> Box<dyn Driver> {
    match k {
        DriverKind::Alsa => Box::new(AlsaDriver::new()),
        DriverKind::Loopback => Box::new(LoopbackDriver::realtime()),
        DriverKind::Plugin => {
            let dir = std::env::var("BIFROST_AUDIO_PLUGIN_DIR").unwrap_or_else(|_| PLUGIN_DIR.into());
            let mut driver = PluginDriver::realtime();
            let published = driver.load_dir(std::path::Path::new(&dir));
            log("driver", &format!("plugin: {published} virtual devices from {dir}"));
            for (file, why) in driver.refused() {
                log("driver", &format!("plugin: {file} refused: {why}"));
            }
            Box::new(driver)
        }
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

/// Backstop for SIGTERM: the serve loop ends on the signalfd by itself, but if it is stuck in a driver call the
/// signal would go unanswered and systemd would wait out its stop timeout. This thread sees the same readable
/// descriptor; when the process is still here `grace` later, it removes the socket and exits.
fn spawn_stop_watchdog(sig_fd: std::os::fd::RawFd, socket: String, grace: std::time::Duration) {
    // SAFETY: dup of a valid descriptor; the copy is owned by the thread and only polled.
    let fd = unsafe { libc::dup(sig_fd) };
    if fd < 0 {
        return;
    }
    let _ = std::thread::Builder::new().name("stop-watchdog".into()).spawn(move || {
        let mut p = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
        loop {
            // SAFETY: one valid pollfd.
            let n = unsafe { libc::poll(&mut p, 1, -1) };
            if n > 0 {
                break;
            }
            if n < 0 && std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
                return;
            }
        }
        std::thread::sleep(grace);
        log("io", "the serve loop did not stop after the termination signal; exiting");
        let _ = std::fs::remove_file(&socket);
        // SAFETY: _exit ends the process without running destructors, which is the point.
        unsafe { libc::_exit(0) };
    });
}

fn main() {
    let mut socket = default_socket().to_string();
    let mut driver_list = std::env::var("BIFROST_AUDIO_DRIVERS").unwrap_or_else(|_| DEFAULT_DRIVERS.into());
    let mut dsp_file = std::env::var("BIFROST_AUDIO_DSP").unwrap_or_else(|_| DSP_FILE.into());
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--socket" => socket = args.next().unwrap_or(socket),
            "--drivers" => driver_list = args.next().unwrap_or(driver_list),
            "--dsp" => dsp_file = args.next().unwrap_or(dsp_file),
            "-h" | "--help" => {
                println!("bifrost-audiod [--socket PATH] [--drivers alsa,loopback,plugin,null] [--dsp FILE]");
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
    let mut composite = build_composite(&kinds);
    // A uid two providers publish belongs to the first; say so, so a second device is not silently missing.
    composite.enumerate(&mut Vec::new());
    for lost in composite.conflicts() {
        log("driver", &format!("device published by two providers, the later one is not offered: {lost}"));
    }
    let drivers: Vec<Box<dyn Driver>> = vec![Box::new(composite)];

    let listener = match listen(&socket) {
        Ok(l) => l,
        Err(e) => {
            log("io", &format!("cannot listen on {socket}: {e}"));
            std::process::exit(1);
        }
    };
    log("io", &format!("listening on {socket}"));
    let mut core = ServerCore::new(drivers, Box::new(NoObjects), CoreConfig::default());
    match load_dsp(&dsp_file) {
        Ok(Some(manager)) => {
            log("driver", &format!("dsp configuration from {dsp_file}"));
            core.set_dsp(manager);
        }
        Ok(None) => {}
        Err(e) => log("driver", &format!("dsp configuration refused, no stages run: {e}")),
    }
    // Lock memory after setup so everything setup allocated is resident too (MCL_CURRENT | MCL_FUTURE).
    bifrost_audiod::rt::lock_memory();
    let stop = AtomicBool::new(false);
    if let Some(f) = sigs.as_ref() {
        spawn_stop_watchdog(f.as_raw_fd(), socket.clone(), std::time::Duration::from_secs(2));
    }
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
    fn the_selected_drivers_are_the_providers_of_one_composite_in_order() {
        let mut c = build_composite(&[DriverKind::Null, DriverKind::Loopback]);
        assert_eq!(c.provider_names(), vec!["null", "loopback"]);
        let mut all = Vec::new();
        c.enumerate(&mut all);
        let uids: Vec<&str> = all.iter().map(|d| d.uid.as_str()).collect();
        assert_eq!(uids, vec!["null:0", "loopback:out", "loopback:in"]);
        assert_eq!(c.provider_of("loopback:in"), Some("loopback"));
        assert_eq!(c.provider_of("null:0"), Some("null"));
        assert!(c.conflicts().is_empty());
    }

    #[test]
    fn the_server_core_starts_a_device_through_the_provider_that_owns_it() {
        use bifrost_audiod::io::WaitPolicy;
        use bifrost_audio_types::control::{IoProcRequest, Reply, Request, PROTOCOL_VERSION};
        use bifrost_audio_types::ObjectId;
        let cfg = CoreConfig { spawn_threads: false, wait: WaitPolicy::None, idle_stop: std::time::Duration::from_secs(2) };
        let composite = build_composite(&[DriverKind::Null, DriverKind::Loopback]);
        let mut core = ServerCore::new(vec![Box::new(composite)], Box::new(NoObjects), cfg);
        assert_eq!(core.device_count(), 3, "every provider's devices are served by the one composite");
        core.add_client(1, std::process::id() as i32, 0);
        core.handle(1, Request::Hello { version: PROTOCOL_VERSION, client: "t".into(), policy: false });
        // the loopback output is the second device; opening it goes to the loopback provider, not to the null one
        let req = IoProcRequest { device: ObjectId(1001), input_channels: 0, output_channels: 2, buffer_frames: 256 };
        let Reply::IoProcAdded { id, .. } = core.handle(1, Request::AddIoProc(req)).reply else { panic!("no io proc") };
        assert_eq!(core.handle(1, Request::StartIoProc { id }).reply, Reply::Done);
        assert!(core.device_running(1) && !core.device_running(0));
        core.shutdown();
    }

    #[test]
    fn a_dsp_file_is_read_whole_or_not_at_all() {
        let dir = std::env::temp_dir().join(format!("bifrost-audiod-dsp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let good = dir.join("good.conf");
        std::fs::write(&good, "device null:0 out\nstage trim gain on db=-6\n").unwrap();
        let m = load_dsp(good.to_str().unwrap()).unwrap().expect("a configuration");
        assert_eq!(m.graph().len(), 1);
        let bad = dir.join("bad.conf");
        std::fs::write(&bad, "device null:0 out\nstage lim limiter on\nstage trim gain on\n").unwrap();
        assert!(load_dsp(bad.to_str().unwrap()).unwrap_err().contains("LimiterNotLast"), "a limiter must come last");
        assert!(load_dsp(dir.join("missing.conf").to_str().unwrap()).unwrap().is_none(), "no file is no configuration");
        std::fs::remove_dir_all(&dir).unwrap();
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
