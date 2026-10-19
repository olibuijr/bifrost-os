//! A client of the system MIDI server (`bifrost-midid`): an app sees the machine's MIDI devices as sources and
//! destinations, connects to the sources it wants to hear, sends packets to destinations at a host time, and creates
//! virtual sources and destinations other apps can use.
//!
//! ```ignore
//! let mut midi = MidiClient::connect("My Synth")?;
//! let keyboard = midi.sources().into_iter().find(|s| s.name.contains("Keys")).unwrap();
//! midi.connect_source(keyboard.id)?;
//! let out = midi.create_destination("My Synth In")?;     // other apps can send here
//! for event in midi.poll()? { /* MidiEvent::Packet { packet, host_ns, .. } */ }
//! ```
//!
//! Timestamps are nanoseconds on the audio host clock (`CLOCK_MONOTONIC_RAW`); 0 means now, and a time in the past
//! is also played at once. Packets are Universal MIDI Packets (`bifrost_midi::ump`); `send_bytes` takes MIDI 1.0
//! bytes. One client has one input port: it hears every source it connected, in time order.
//!
//! The wire and the server's logic live in `bifrost_midi::wire` and `bifrost_midi::broker`, so the protocol is tested
//! without a device. Devices appear and go away while the app runs: `MidiEvent::SetupChanged` says the lists are
//! stale, and `sources()` and `destinations()` re-read them on the next call.

pub use bifrost_midi::wire::{DeviceInfo, EndpointInfo, EntityInfo, Refusal, Setup};
use bifrost_midi::ump::{Midi1ToUmp, Ump};
use bifrost_midi::wire::{frame, FrameReader, Message, Request, WireError, SOCKET_NAME, VERSION};
use std::collections::VecDeque;
use std::fmt;
use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// How long a request waits for the server's answer.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(2);

/// What can go wrong talking to the MIDI server.
#[derive(Debug)]
pub enum MidiError {
    /// The socket could not be opened or used.
    Io(io::Error),
    /// The server refused the request.
    Refused(Refusal),
    /// The server sent something this client does not understand.
    Protocol(WireError),
    /// The server closed the connection.
    Closed,
    /// No answer in time.
    TimedOut,
}

impl fmt::Display for MidiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MidiError::Io(e) => write!(f, "MIDI server: {e}"),
            MidiError::Refused(r) => write!(f, "MIDI server refused: {r}"),
            MidiError::Protocol(e) => write!(f, "MIDI server protocol: {e}"),
            MidiError::Closed => f.write_str("MIDI server closed the connection"),
            MidiError::TimedOut => f.write_str("MIDI server did not answer"),
        }
    }
}

impl std::error::Error for MidiError {}

impl From<io::Error> for MidiError {
    fn from(e: io::Error) -> MidiError {
        MidiError::Io(e)
    }
}

impl From<WireError> for MidiError {
    fn from(e: WireError) -> MidiError {
        MidiError::Protocol(e)
    }
}

/// What the server pushes.
#[derive(Clone, Debug, PartialEq)]
pub enum MidiEvent {
    /// A packet from a source this client connected.
    Packet { source: u32, host_ns: u64, packet: Ump },
    /// A packet that reached one of this client's virtual destinations.
    Delivered { destination: u32, from: u32, host_ns: u64, packet: Ump },
    /// Devices or endpoints appeared or went away.
    SetupChanged,
    /// A `send` or `inject` was refused (they have no reply).
    SendRefused(Refusal),
}

/// The socket the MIDI server listens on: `BIFROST_MIDI_SOCK`, else the audio run directory.
pub fn socket_path() -> PathBuf {
    match std::env::var_os("BIFROST_MIDI_SOCK") {
        Some(p) => PathBuf::from(p),
        None => Path::new("/run/bifrost/audio").join(SOCKET_NAME),
    }
}

pub struct MidiClient {
    stream: UnixStream,
    reader: FrameReader,
    client: u32,
    events: VecDeque<MidiEvent>,
    setup: Setup,
    stale: bool,
    /// The converter for `send_bytes`: running status and system exclusive carry across calls.
    bytes: Midi1ToUmp,
    timeout: Duration,
}

impl MidiClient {
    /// Connects to the system server and says hello as `name`.
    pub fn connect(name: &str) -> Result<MidiClient, MidiError> {
        MidiClient::connect_at(&socket_path(), name)
    }

    pub fn connect_at(path: &Path, name: &str) -> Result<MidiClient, MidiError> {
        MidiClient::from_stream(UnixStream::connect(path)?, name)
    }

    /// Uses an open stream (a test double, or a socket the caller connected).
    pub fn from_stream(stream: UnixStream, name: &str) -> Result<MidiClient, MidiError> {
        let mut c = MidiClient {
            stream,
            reader: FrameReader::new(),
            client: 0,
            events: VecDeque::new(),
            setup: Setup::default(),
            stale: true,
            bytes: Midi1ToUmp::new(0),
            timeout: REQUEST_TIMEOUT,
        };
        match c.request(Request::Hello { name: name.to_string(), version: VERSION })? {
            Message::Welcome { client, .. } => c.client = client,
            _ => return Err(MidiError::Protocol(WireError::BadPacket)),
        }
        Ok(c)
    }

    /// The server's id for this client.
    pub fn client_id(&self) -> u32 {
        self.client
    }

    /// Changes how long a request waits for its answer.
    pub fn set_timeout(&mut self, timeout: Duration) {
        self.timeout = timeout;
    }

    fn write(&mut self, req: &Request) -> Result<(), MidiError> {
        self.stream.write_all(&frame(&req.encode()))?;
        Ok(())
    }

    fn absorb(&mut self, m: Message) -> Option<Message> {
        let ev = match m {
            Message::Received { source, host_ns, packet } => MidiEvent::Packet { source, host_ns, packet },
            Message::Delivered { destination, from, host_ns, packet } => {
                MidiEvent::Delivered { destination, from, host_ns, packet }
            }
            Message::SetupChanged { .. } => {
                self.stale = true;
                MidiEvent::SetupChanged
            }
            Message::SendRefused(r) => MidiEvent::SendRefused(r),
            other => return Some(other),
        };
        self.events.push_back(ev);
        None
    }

    /// The next message, waiting until `deadline`. `Ok(None)` when it passes with nothing whole received.
    fn next_message(&mut self, deadline: Instant) -> Result<Option<Message>, MidiError> {
        loop {
            if let Some(payload) = self.reader.next_frame()? {
                return Ok(Some(Message::decode(&payload)?));
            }
            let now = Instant::now();
            if now >= deadline {
                return Ok(None);
            }
            self.stream.set_read_timeout(Some((deadline - now).max(Duration::from_millis(1))))?;
            let mut buf = [0u8; 16 * 1024];
            match self.stream.read(&mut buf) {
                Ok(0) => return Err(MidiError::Closed),
                Ok(n) => self.reader.push(&buf[..n]),
                Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => return Ok(None),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e.into()),
            }
        }
    }

    /// Sends a request and waits for its reply; events that arrive meanwhile are queued for `poll`.
    fn request(&mut self, req: Request) -> Result<Message, MidiError> {
        self.write(&req)?;
        let deadline = Instant::now() + self.timeout;
        loop {
            match self.next_message(deadline)? {
                None => return Err(MidiError::TimedOut),
                Some(m) => {
                    if let Some(reply) = self.absorb(m) {
                        return match reply {
                            Message::Failed(r) => Err(MidiError::Refused(r)),
                            other => Ok(other),
                        };
                    }
                }
            }
        }
    }

    fn expect_done(&mut self, req: Request) -> Result<(), MidiError> {
        match self.request(req)? {
            Message::Done => Ok(()),
            _ => Err(MidiError::Protocol(WireError::BadPacket)),
        }
    }

    fn expect_created(&mut self, req: Request) -> Result<u32, MidiError> {
        match self.request(req)? {
            Message::Created { id } => Ok(id),
            _ => Err(MidiError::Protocol(WireError::BadPacket)),
        }
    }

    /// Reads the device, entity and endpoint lists again.
    pub fn refresh(&mut self) -> Result<&Setup, MidiError> {
        match self.request(Request::List)? {
            Message::Setup(s) => {
                self.setup = s;
                self.stale = false;
                Ok(&self.setup)
            }
            _ => Err(MidiError::Protocol(WireError::BadPacket)),
        }
    }

    /// The lists, read again when the server said they changed (or never were read).
    pub fn setup(&mut self) -> Result<&Setup, MidiError> {
        if self.stale {
            self.refresh()?;
        }
        Ok(&self.setup)
    }

    /// Every source (hardware and virtual), the ones this client connected marked `connected`. Errors read as none.
    pub fn sources(&mut self) -> Vec<EndpointInfo> {
        self.setup().map(|s| s.endpoints.iter().filter(|e| e.source).cloned().collect()).unwrap_or_default()
    }

    /// Every destination (hardware and virtual).
    pub fn destinations(&mut self) -> Vec<EndpointInfo> {
        self.setup().map(|s| s.endpoints.iter().filter(|e| !e.source).cloned().collect()).unwrap_or_default()
    }

    /// The devices (built in and external).
    pub fn devices(&mut self) -> Vec<DeviceInfo> {
        self.setup().map(|s| s.devices.clone()).unwrap_or_default()
    }

    /// The device that owns an endpoint, through its entity. `None` for a virtual endpoint.
    pub fn device_of(&mut self, endpoint: u32) -> Option<DeviceInfo> {
        let setup = self.setup().ok()?;
        let entity = setup.endpoints.iter().find(|e| e.id == endpoint)?.entity;
        let device = setup.entities.iter().find(|e| e.id == entity)?.device;
        setup.devices.iter().find(|d| d.id == device).cloned()
    }

    /// A virtual source: the app injects packets into it (`inject`) and any client may connect to it.
    pub fn create_source(&mut self, name: &str) -> Result<u32, MidiError> {
        self.expect_created(Request::CreateSource { name: name.to_string() })
    }

    /// A virtual destination: packets other clients send to it arrive as `MidiEvent::Delivered`.
    pub fn create_destination(&mut self, name: &str) -> Result<u32, MidiError> {
        self.expect_created(Request::CreateDestination { name: name.to_string() })
    }

    /// Disposes a virtual endpoint this client created.
    pub fn dispose(&mut self, endpoint: u32) -> Result<(), MidiError> {
        self.expect_done(Request::Dispose { id: endpoint })
    }

    /// Listens to a source (hardware or virtual).
    pub fn connect_source(&mut self, source: u32) -> Result<(), MidiError> {
        self.expect_done(Request::Connect { source })
    }

    pub fn disconnect_source(&mut self, source: u32) -> Result<(), MidiError> {
        self.expect_done(Request::Disconnect { source })
    }

    /// Stores a value on an endpoint this client created, returned in its setup entry.
    pub fn set_ref_con(&mut self, endpoint: u32, value: u64) -> Result<(), MidiError> {
        self.expect_done(Request::SetRefCon { id: endpoint, value })
    }

    /// Sends packets to a destination at `host_ns` (0: now). A refusal comes back as `MidiEvent::SendRefused`.
    pub fn send(&mut self, destination: u32, host_ns: u64, packets: &[Ump]) -> Result<(), MidiError> {
        for chunk in packets.chunks(bifrost_midi::wire::MAX_PACKETS) {
            self.write(&Request::Send { destination, host_ns, packets: chunk.to_vec() })?;
        }
        Ok(())
    }

    /// Sends MIDI 1.0 bytes (channel messages, system exclusive...) to a destination as UMP packets of group 0.
    pub fn send_bytes(&mut self, destination: u32, host_ns: u64, bytes: &[u8]) -> Result<(), MidiError> {
        let mut packets = Vec::new();
        for b in bytes {
            self.bytes.push(*b, &mut packets);
        }
        if packets.is_empty() {
            return Ok(());
        }
        self.send(destination, host_ns, &packets)
    }

    /// Injects packets from a virtual source this client created, to every client connected to it.
    pub fn inject(&mut self, source: u32, host_ns: u64, packets: &[Ump]) -> Result<(), MidiError> {
        for chunk in packets.chunks(bifrost_midi::wire::MAX_PACKETS) {
            self.write(&Request::Inject { source, host_ns, packets: chunk.to_vec() })?;
        }
        Ok(())
    }

    /// Round trip to the server; the time it took.
    pub fn ping(&mut self) -> Result<Duration, MidiError> {
        let t = Instant::now();
        match self.request(Request::Ping)? {
            Message::Pong => Ok(t.elapsed()),
            _ => Err(MidiError::Protocol(WireError::BadPacket)),
        }
    }

    /// The events that are waiting, without blocking.
    pub fn poll(&mut self) -> Result<Vec<MidiEvent>, MidiError> {
        self.stream.set_nonblocking(true)?;
        let read = self.fill_nonblocking();
        self.stream.set_nonblocking(false)?;
        read?;
        Ok(self.events.drain(..).collect())
    }

    fn fill_nonblocking(&mut self) -> Result<(), MidiError> {
        let mut buf = [0u8; 16 * 1024];
        loop {
            match self.stream.read(&mut buf) {
                Ok(0) => return Err(MidiError::Closed),
                Ok(n) => self.reader.push(&buf[..n]),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e.into()),
            }
        }
        while let Some(payload) = self.reader.next_frame()? {
            let m = Message::decode(&payload)?;
            // a reply with no request waiting is stale (the request timed out): drop it
            let _ = self.absorb(m);
        }
        Ok(())
    }

    /// Waits up to `timeout` for at least one event, then returns everything waiting. Empty on timeout.
    pub fn wait(&mut self, timeout: Duration) -> Result<Vec<MidiEvent>, MidiError> {
        let deadline = Instant::now() + timeout;
        while self.events.is_empty() {
            match self.next_message(deadline)? {
                None => break,
                Some(m) => {
                    let _ = self.absorb(m);
                }
            }
        }
        self.poll()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_midi::broker::{Broker, ConnId};
    use bifrost_midi::endpoints::Direction;
    use bifrost_midi::events::MidiEvent as Midi1;
    use bifrost_midi::hub::Hub;
    use bifrost_midi::ump::from_midi1;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{channel, Sender};
    use std::sync::Arc;
    use std::thread::JoinHandle;

    /// A MIDI server on socket pairs: one thread, one broker, the same request handling as the daemon.
    struct TestServer {
        connect: Sender<UnixStream>,
        stop: Arc<AtomicBool>,
        thread: Option<JoinHandle<()>>,
        /// Hardware destinations tracked by the broker, so a test can see what a send to a device became.
        out: Arc<std::sync::Mutex<Vec<(u32, u64, Ump)>>>,
    }

    impl TestServer {
        fn start(hub: Hub) -> TestServer {
            let (tx, rx) = channel::<UnixStream>();
            let stop = Arc::new(AtomicBool::new(false));
            let out = Arc::new(std::sync::Mutex::new(Vec::new()));
            let (stop2, out2) = (stop.clone(), out.clone());
            let thread = std::thread::spawn(move || {
                let mut broker = Broker::new(hub);
                let mut links: Vec<(ConnId, UnixStream, FrameReader)> = Vec::new();
                let t0 = Instant::now();
                while !stop2.load(Ordering::Relaxed) {
                    while let Ok(s) = rx.try_recv() {
                        s.set_nonblocking(true).unwrap();
                        let conn = broker.open();
                        links.push((conn, s, FrameReader::new()));
                    }
                    let now = t0.elapsed().as_nanos() as u64 + 1;
                    let mut gone = Vec::new();
                    for (conn, s, reader) in links.iter_mut() {
                        let mut buf = [0u8; 8192];
                        match s.read(&mut buf) {
                            Ok(0) => gone.push(*conn),
                            Ok(n) => reader.push(&buf[..n]),
                            Err(_) => {}
                        }
                        while let Ok(Some(p)) = reader.next_frame() {
                            let req = Request::decode(&p).unwrap();
                            for m in broker.handle(*conn, req, now) {
                                write_blocking(s, &frame(&m.encode()));
                            }
                        }
                    }
                    for (conn, m) in broker.pump(now) {
                        if let Some((_, s, _)) = links.iter_mut().find(|(c, _, _)| *c == conn) {
                            write_blocking(s, &frame(&m.encode()));
                        }
                    }
                    for (dst, t) in broker.hardware_output(now) {
                        out2.lock().unwrap().push((dst.0, t.host_ns, t.packet));
                    }
                    for c in gone {
                        broker.close(c);
                        links.retain(|(x, _, _)| *x != c);
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
            });
            TestServer { connect: tx, stop, thread: Some(thread), out }
        }

        fn client(&self, name: &str) -> MidiClient {
            let (a, b) = UnixStream::pair().unwrap();
            self.connect.send(b).unwrap();
            MidiClient::from_stream(a, name).unwrap()
        }
    }

    impl Drop for TestServer {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
        }
    }

    fn write_blocking(s: &mut UnixStream, mut bytes: &[u8]) {
        while !bytes.is_empty() {
            match s.write(bytes) {
                Ok(n) => bytes = &bytes[n..],
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(1)),
                Err(_) => return,
            }
        }
    }

    fn note(key: u8) -> Ump {
        from_midi1(0, Midi1::NoteOn { channel: 0, key, velocity: 100 })
    }

    fn keyboard_hub() -> (Hub, u32, u32) {
        let mut hub = Hub::new();
        let dev = hub.registry.add_device("USB Keys", true);
        let ent = hub.registry.add_entity(dev, "Port 1").unwrap();
        let src = hub.registry.add_endpoint(ent, Direction::Source, "USB Keys out").unwrap();
        let dst = hub.registry.add_endpoint(ent, Direction::Destination, "USB Keys in").unwrap();
        hub.track_destination(dst);
        (hub, src.0, dst.0)
    }

    #[test]
    fn the_client_lists_hardware_devices_and_their_endpoints() {
        let (hub, src, dst) = keyboard_hub();
        let server = TestServer::start(hub);
        let mut c = server.client("Lister");
        assert!(c.client_id() > 0);
        let sources = c.sources();
        assert_eq!(sources.len(), 1);
        assert_eq!((sources[0].id, sources[0].name.as_str(), sources[0].connected), (src, "USB Keys out", false));
        assert_eq!(c.destinations().len(), 1);
        assert_eq!(c.destinations()[0].id, dst);
        assert_eq!(c.devices()[0].name, "USB Keys");
        assert_eq!(c.device_of(src).map(|d| d.name), Some("USB Keys".to_string()));
        assert_eq!(c.device_of(9999), None);
        assert!(c.ping().unwrap() < Duration::from_secs(1));
    }

    #[test]
    fn a_virtual_source_reaches_a_client_that_connected_to_it() {
        let server = TestServer::start(Hub::new());
        let mut synth = server.client("Synth");
        let mut daw = server.client("DAW");
        let out = synth.create_source("Synth out").unwrap();
        daw.connect_source(out).unwrap();
        // the connection shows in the DAW's list
        assert!(daw.refresh().unwrap().endpoints.iter().any(|e| e.id == out && e.connected));
        synth.inject(out, 0, &[note(60), note(64)]).unwrap();
        let mut got = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(2);
        while got.iter().filter(|e| matches!(e, MidiEvent::Packet { .. })).count() < 2 && Instant::now() < deadline {
            got.extend(daw.wait(Duration::from_millis(50)).unwrap());
        }
        let packets: Vec<Ump> = got
            .iter()
            .filter_map(|e| match e {
                MidiEvent::Packet { source, packet, .. } if *source == out => Some(*packet),
                _ => None,
            })
            .collect();
        assert_eq!(packets, vec![note(60), note(64)]);
        daw.disconnect_source(out).unwrap();
        assert!(matches!(daw.disconnect_source(out), Err(MidiError::Refused(Refusal::NotApplicable))));
    }

    #[test]
    fn packets_sent_to_a_virtual_destination_are_delivered_to_its_owner() {
        let server = TestServer::start(Hub::new());
        let mut synth = server.client("Synth");
        let mut host = server.client("Host");
        let inbox = synth.create_destination("Synth in").unwrap();
        host.send(inbox, 0, &[note(72)]).unwrap();
        let mut got = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !got.iter().any(|e| matches!(e, MidiEvent::Delivered { .. })) && Instant::now() < deadline {
            got.extend(synth.wait(Duration::from_millis(50)).unwrap());
        }
        assert!(
            got.iter().any(|e| matches!(e, MidiEvent::Delivered { destination, packet, .. } if *destination == inbox && *packet == note(72))),
            "{got:?}"
        );
    }

    #[test]
    fn sending_to_hardware_queues_it_at_the_timestamp() {
        let (hub, _src, dst) = keyboard_hub();
        let server = TestServer::start(hub);
        let mut c = server.client("Player");
        c.send(dst, 0, &[note(60)]).unwrap();
        // far in the future: it stays queued
        c.send(dst, u64::MAX / 2, &[note(61)]).unwrap();
        c.ping().unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while server.out.lock().unwrap().is_empty() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(2));
        }
        let out = server.out.lock().unwrap().clone();
        assert_eq!(out.len(), 1, "{out:?}");
        assert_eq!((out[0].0, out[0].2), (dst, note(60)));
    }

    #[test]
    fn send_bytes_turns_midi_one_bytes_into_packets() {
        let server = TestServer::start(Hub::new());
        let mut synth = server.client("Synth");
        let mut host = server.client("Host");
        let inbox = synth.create_destination("in").unwrap();
        // note on with running status for a second note, then a program change
        host.send_bytes(inbox, 0, &[0x90, 60, 100, 62, 90, 0xC0, 5]).unwrap();
        let mut got = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(2);
        while got.iter().filter(|e| matches!(e, MidiEvent::Delivered { .. })).count() < 3 && Instant::now() < deadline {
            got.extend(synth.wait(Duration::from_millis(50)).unwrap());
        }
        let words: Vec<u32> = got
            .iter()
            .filter_map(|e| match e {
                MidiEvent::Delivered { packet, .. } => Some(packet.words[0]),
                _ => None,
            })
            .collect();
        assert_eq!(words, vec![0x2090_3C64, 0x2090_3E5A, 0x20C0_0500]);
    }

    #[test]
    fn refusals_come_back_as_errors_or_events() {
        let server = TestServer::start(Hub::new());
        let mut c = server.client("Refused");
        assert!(matches!(c.connect_source(4242), Err(MidiError::Refused(Refusal::BadObject))));
        assert!(matches!(c.dispose(4242), Err(MidiError::Refused(Refusal::BadObject))));
        let dst = c.create_destination("in").unwrap();
        assert!(matches!(c.connect_source(dst), Err(MidiError::Refused(Refusal::WrongDirection))));
        // a send to a missing destination has no reply: it arrives as an event
        c.send(777, 0, &[note(1)]).unwrap();
        c.ping().unwrap();
        let events = c.poll().unwrap();
        assert!(events.contains(&MidiEvent::SendRefused(Refusal::BadObject)), "{events:?}");
    }

    #[test]
    fn setup_changes_mark_the_lists_stale_and_they_are_read_again() {
        let server = TestServer::start(Hub::new());
        let mut a = server.client("A");
        let mut b = server.client("B");
        assert!(b.destinations().is_empty());
        let made = a.create_destination("New").unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut changed = false;
        while !changed && Instant::now() < deadline {
            changed = b.wait(Duration::from_millis(50)).unwrap().contains(&MidiEvent::SetupChanged);
        }
        assert!(changed);
        let found: Vec<u32> = b.destinations().iter().map(|d| d.id).collect();
        assert_eq!(found, vec![made]);
        // disposing it is announced the same way
        a.dispose(made).unwrap();
        let mut changed = false;
        let deadline = Instant::now() + Duration::from_secs(2);
        while !changed && Instant::now() < deadline {
            changed = b.wait(Duration::from_millis(50)).unwrap().contains(&MidiEvent::SetupChanged);
        }
        assert!(changed);
        assert!(b.destinations().is_empty());
    }

    #[test]
    fn a_dead_server_is_reported_not_hung_on() {
        let (a, b) = UnixStream::pair().unwrap();
        drop(b);
        let r = MidiClient::from_stream(a, "x");
        assert!(matches!(r, Err(MidiError::Closed) | Err(MidiError::Io(_))), "{:?}", r.err());
    }

    #[test]
    fn a_silent_server_times_out() {
        let (a, _b) = UnixStream::pair().unwrap();
        // the peer is open but never answers the hello
        let started = Instant::now();
        let mut c = MidiClient {
            stream: a,
            reader: FrameReader::new(),
            client: 0,
            events: VecDeque::new(),
            setup: Setup::default(),
            stale: true,
            bytes: Midi1ToUmp::new(0),
            timeout: Duration::from_millis(60),
        };
        assert!(matches!(c.ping(), Err(MidiError::TimedOut)));
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn socket_path_follows_the_environment_default() {
        let p = socket_path();
        // either the override or the audio run directory, always ending in the socket name or an override
        assert!(p.ends_with(SOCKET_NAME) || std::env::var_os("BIFROST_MIDI_SOCK").is_some());
    }
}
