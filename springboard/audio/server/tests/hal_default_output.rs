//! The full output path through the real control loop and the client library, on the null and loopback drivers: a
//! client reads the default output from the system object, adds an IO procedure on that device (asking for the
//! device's own buffer size, as the speaker does), starts it and receives IO cycles.
//!
//! This is the path that failed on the lab machine with `add the IO procedure: UnknownObject`: the HAL reads the
//! device's `BUFFER_FRAMES` before it adds the IO procedure, and the device objects did not carry it.

use bifrost_audio_driver::null::NullDriver;
use bifrost_audio_driver::loopback::LoopbackDriver;
use bifrost_audio_driver::transport::CompositeDriver;
use bifrost_audio_driver::Driver;
use bifrost_audio_hal::{IoProcConfig, Role, System};
use bifrost_audio_types::{props, AudioError, ObjectId, PropertyAddress, PropertyValue, SYSTEM_OBJECT};
use bifrost_audiod::control::{listen, serve, CoreConfig, NoObjects, ServerCore};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// A server on a private socket, run on its own thread until dropped.
struct Server {
    path: String,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Server {
    /// `drivers` is built on the server thread (a driver need not be `Send`).
    fn start(tag: &str, drivers: impl FnOnce() -> Vec<Box<dyn Driver>> + Send + 'static) -> Server {
        let dir = std::env::temp_dir().join(format!("bifrost-hal-default-output-{}-{tag}", std::process::id()));
        let path = dir.join("control").to_string_lossy().into_owned();
        let listener = listen(&path).expect("listen on the private socket");
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let thread = std::thread::spawn(move || {
            let mut core = ServerCore::new(drivers(), Box::new(NoObjects), CoreConfig::default());
            serve(&mut core, &listener, &stop2, None);
        });
        Server { path, stop, thread: Some(thread) }
    }

    fn connect(&self, client: &str) -> System {
        System::connect_path(&self.path, client).expect("connect to the private server")
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        let _ = std::fs::remove_file(&self.path);
    }
}

/// A null device that is a real playback device as far as the default choice goes (the null driver's own devices are
/// "virtual", which are never a default), named like the lab machine's analog output.
fn analog_null() -> NullDriver {
    let mut d = NullDriver::stereo("alsa:PCH:0", 48_000.0);
    d.name = "ALC233 Analog".into();
    d.transport = "pci".into();
    NullDriver::new(vec![d])
}

/// Waits (at most `limit`) until `cond` holds.
fn wait_for(limit: Duration, mut cond: impl FnMut() -> bool) -> bool {
    let until = Instant::now() + limit;
    while Instant::now() < until {
        if cond() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    cond()
}

/// Adds an IO procedure with the device's own buffer size on `device`, starts it, and counts its cycles.
fn run_output(device: &bifrost_audio_hal::Device) -> (u64, bool) {
    let cycles = Arc::new(AtomicU64::new(0));
    let seen = cycles.clone();
    let id = device
        .add_io_proc(
            IoProcConfig { input_channels: 0, output_channels: 2, buffer_frames: 0 },
            Box::new(move |ctx, _now, _input, _input_ts, output, _output_ts| {
                assert!(ctx.frames > 0);
                for b in output.buffers.iter_mut() {
                    b.data.iter_mut().for_each(|v| *v = 0.0);
                }
                seen.fetch_add(1, Ordering::Relaxed);
            }),
        )
        .expect("add the IO procedure on the device");
    device.start(id).expect("start the IO procedure");
    let got = wait_for(Duration::from_secs(5), || cycles.load(Ordering::Relaxed) >= 4);
    let n = cycles.load(Ordering::Relaxed);
    device.stop(id).expect("stop the IO procedure");
    device.remove_io_proc(id).expect("remove the IO procedure");
    (n, got)
}

#[test]
fn a_client_plays_on_the_default_output_the_server_chose() {
    let server = Server::start("default", || {
        let mut composite = CompositeDriver::new();
        composite.add("null", Box::new(analog_null()));
        composite.add("loopback", Box::new(LoopbackDriver::realtime()));
        vec![Box::new(composite) as Box<dyn Driver>]
    });
    let sys = server.connect("test.output");

    // the system object names the default output, and it is a device the server lists
    let default = match sys.get(SYSTEM_OBJECT, PropertyAddress::global(props::DEFAULT_OUTPUT)) {
        Ok(PropertyValue::Object(id)) => id,
        other => panic!("default-output: {other:?}"),
    };
    assert_eq!(default, ObjectId(1000), "the analog device, not the loopback");
    let listed: Vec<ObjectId> = sys.devices().unwrap().iter().map(|d| d.id).collect();
    assert!(listed.contains(&default), "{default:?} is not in {listed:?}");
    for role in [Role::Output, Role::SoundEffects] {
        assert_eq!(sys.default_device(role).unwrap().id, default);
    }
    assert_eq!(sys.default_device(Role::Input).unwrap().id, default, "the analog device captures too");

    // what the HAL and the speaker read before adding the IO procedure
    let device = sys.default_device(Role::Output).unwrap();
    assert_eq!(device.get(PropertyAddress::global(props::BUFFER_FRAMES)), Ok(PropertyValue::U32(256)));
    assert_eq!(device.get(PropertyAddress::global(props::NOMINAL_RATE)), Ok(PropertyValue::F64(48_000.0)));
    assert_eq!(device.get(PropertyAddress::global(props::DEVICE_UID)), Ok(PropertyValue::String("alsa:PCH:0".into())));
    // a property the device does not carry is a missing property, not a missing device
    assert_eq!(device.get(PropertyAddress::global(props::VOLUME_SCALAR)), Err(AudioError::UnknownProperty));

    let (cycles, ok) = run_output(&device);
    assert!(ok, "the IO procedure got {cycles} cycles in 5 s");
}

#[test]
fn a_client_plays_on_the_loopback_device() {
    let server = Server::start("loopback", || vec![Box::new(LoopbackDriver::realtime()) as Box<dyn Driver>]);
    let sys = server.connect("test.loopback");
    let devices = sys.devices().unwrap();
    let out = devices
        .iter()
        .find(|d| d.get(PropertyAddress::global(props::DEVICE_UID)) == Ok(PropertyValue::String("loopback:out".into())))
        .expect("the loopback playback device is listed");
    let (cycles, ok) = run_output(out);
    assert!(ok, "the IO procedure got {cycles} cycles in 5 s");
}

#[test]
fn an_io_procedure_on_a_device_the_server_does_not_know_is_refused_and_a_known_one_is_not() {
    let server = Server::start("unknown", || vec![Box::new(analog_null()) as Box<dyn Driver>]);
    let sys = server.connect("test.unknown");
    let known = sys.default_device(Role::Output).unwrap();
    let ghost = bifrost_audio_hal::Device { id: ObjectId(4242), system: sys.clone() };
    let cfg = IoProcConfig { input_channels: 0, output_channels: 2, buffer_frames: 256 };
    let proc: bifrost_audio_hal::IoProc = Box::new(|_, _, _, _, _, _| {});
    assert_eq!(ghost.add_io_proc(cfg, proc).err(), Some(AudioError::UnknownObject));
    let proc: bifrost_audio_hal::IoProc = Box::new(|_, _, _, _, _, _| {});
    let id = known.add_io_proc(cfg, proc).expect("the default output accepts an IO procedure");
    known.remove_io_proc(id).unwrap();
}
