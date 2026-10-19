//! `bifrost-midid` (owner: task 15): the MIDI server. ALSA rawmidi and seq over the kernel uapi (no libasound), BLE
//! MIDI ingress, a virtual source and destination per app, UMP inside, timestamps on the audio host clock.
//!
//! Usage: `bifrost-midid --list` prints the rawmidi devices. `bifrost-midid --dump [SECONDS]` opens every rawmidi
//! input and the sequencer port and prints each event with its host timestamp (the Midget keyboard check). With no
//! argument it runs as the daemon loop.

mod ble_link;
mod rawmidi;
mod seq;
mod server;

use bifrost_midi::broker::Broker;
use bifrost_midi::endpoints::{Direction, ObjectId};
use bifrost_midi::hub::Hub;
use bifrost_midi::ump::{Midi1ToUmp, Ump, UmpToMidi1};
use bifrost_midi::wire::SOCKET_NAME;
use std::collections::BTreeMap;

const LOG: &str = "is.olibuijr.audio/midi";

fn log(msg: &str) {
    eprintln!("{LOG}: {msg}");
}

/// `CLOCK_MONOTONIC_RAW` in nanoseconds: the audio host clock.
fn host_ns() -> u64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: `ts` is a live timespec; clock_gettime only writes to it.
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC_RAW, &mut ts) };
    ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64
}

/// One attached rawmidi node. `conv` lives as long as the open port, so running status and an open system
/// exclusive carry across reads; `out_conv` does the same for what is written to it.
struct HwPort {
    port: rawmidi::RawPort,
    device: ObjectId,
    /// The source endpoint of a port that can be read, the destination endpoint of one that can be written.
    source: Option<ObjectId>,
    destination: Option<ObjectId>,
    conv: Midi1ToUmp,
    out_conv: UmpToMidi1,
    /// Bytes the device did not take yet (its buffer was full), written first on the next round.
    pending: Vec<u8>,
    name: String,
}

impl HwPort {
    /// Writes what the device takes of the pending bytes. A device that fails is reported once and its output is
    /// dropped from then on.
    fn flush(&mut self) {
        while !self.pending.is_empty() {
            match self.port.write(&self.pending) {
                Ok(0) => break,
                Ok(n) => {
                    self.pending.drain(..n.min(self.pending.len()));
                }
                Err(e) => {
                    log(&format!("{} output failed: {e}", self.name));
                    self.pending.clear();
                    self.destination = None;
                    break;
                }
            }
        }
    }
}

/// The socket apps connect to: `BIFROST_MIDI_SOCK`, else the audio run directory.
fn socket_path() -> std::path::PathBuf {
    match std::env::var_os("BIFROST_MIDI_SOCK") {
        Some(p) => p.into(),
        None => std::path::Path::new("/run/bifrost/audio").join(SOCKET_NAME),
    }
}

/// How long the poll may sleep: until the next queued packet is due, at most 200 ms, and a short while when a
/// device or client still has bytes to take.
fn poll_timeout_ms(broker: &Broker, now: u64, unsent: bool) -> i32 {
    let due = match broker.hub.next_due_ns() {
        Some(t) if t <= now => 0,
        Some(t) => ((t - now) / 1_000_000 + 1).min(200) as i32,
        None => 200,
    };
    if unsent {
        due.min(2)
    } else {
        due
    }
}

fn open_hardware(broker: &mut Broker, ports: &mut BTreeMap<String, HwPort>) {
    let hub = &mut broker.hub;
    let mut changed = false;
    for dev in rawmidi::scan("/dev/snd") {
        if ports.contains_key(&dev.path) {
            continue;
        }
        let port = match rawmidi::RawPort::open(&dev.path) {
            Ok(p) => p,
            Err(e) => {
                log(&format!("cannot open {}: {e}", dev.path));
                continue;
            }
        };
        let name = port.info().map(|i| rawmidi::c_text(&i.name)).unwrap_or_else(|_| dev.path.clone());
        let d = hub.registry.add_device(&name, true);
        let Some(ent) = hub.registry.add_entity(d, &name) else { continue };
        // an input-only device has no destination, an output-only one no source
        let source = if port.readable() { hub.registry.add_endpoint(ent, Direction::Source, &name) } else { None };
        let destination = if port.writable() { hub.registry.add_endpoint(ent, Direction::Destination, &name) } else { None };
        if let Some(dst) = destination {
            hub.track_destination(dst);
        }
        log(&format!("rawmidi {} ({name}) attached", dev.path));
        ports.insert(
            dev.path.clone(),
            HwPort {
                port,
                device: d,
                source,
                destination,
                conv: Midi1ToUmp::new(0),
                out_conv: UmpToMidi1::new(),
                pending: Vec::new(),
                name,
            },
        );
        changed = true;
    }
    if changed {
        broker.setup_changed();
    }
}

fn describe(p: &Ump) -> String {
    let w: Vec<String> = p.as_words().iter().map(|w| format!("{w:08X}")).collect();
    w.join(" ")
}

fn run(dump_seconds: Option<u64>) {
    let mut broker = Broker::new(Hub::new());
    let mut ports: BTreeMap<String, HwPort> = BTreeMap::new();
    // The client socket belongs to the daemon, not to the `--dump` check, which may run next to it.
    let mut server = if dump_seconds.is_none() {
        let path = socket_path();
        match server::Server::bind(&path) {
            Ok(s) => {
                log(&format!("clients connect at {}", path.display()));
                Some(s)
            }
            Err(e) => {
                log(&format!("cannot listen at {}: {e}", path.display()));
                None
            }
        }
    } else {
        None
    };
    let seq = match seq::Seq::open("/dev/snd/seq") {
        Ok(s) => {
            let _ = s.set_name("BifrOSt MIDI");
            match s.create_port("BifrOSt MIDI") {
                Ok(p) => log(&format!("sequencer client {} port {p}", s.client)),
                Err(e) => log(&format!("sequencer port: {e}")),
            }
            Some(s)
        }
        Err(e) => {
            log(&format!("no sequencer: {e}"));
            None
        }
    };
    let seq_source = {
        let reg = &mut broker.hub.registry;
        let d = reg.add_device("Sequencer", true);
        reg.add_entity(d, "Sequencer").and_then(|e| reg.add_endpoint(e, Direction::Source, "Sequencer"))
    };
    let start = host_ns();
    let mut last_scan = 0u64;
    let mut buf = [0u8; 4096];
    // The sequencer buffer grows when one event's payload does not fit (see `Seq::read_events`).
    let mut seq_buf: Vec<u8> = vec![0; 4096];
    // System exclusive arrives from the sequencer as raw byte chunks per sender; one converter per sender keeps a
    // message split over several events whole.
    let mut seq_sysex: BTreeMap<(u8, u8), Midi1ToUmp> = BTreeMap::new();
    let mut seq_error: Option<String> = None;
    loop {
        let now = host_ns();
        if dump_seconds.is_some_and(|s| now.saturating_sub(start) > s * 1_000_000_000) {
            return;
        }
        if now.saturating_sub(last_scan) > 2_000_000_000 || last_scan == 0 {
            open_hardware(&mut broker, &mut ports);
            last_scan = now;
        }
        // Output-only ports are never read (EBADF), so they are not polled either.
        let mut fds: Vec<libc::pollfd> = ports
            .values()
            .filter(|p| p.port.readable())
            .map(|p| libc::pollfd { fd: p.port.fd(), events: libc::POLLIN, revents: 0 })
            .collect();
        if let Some(s) = &seq {
            fds.push(libc::pollfd { fd: s.fd(), events: libc::POLLIN, revents: 0 });
        }
        if let Some(srv) = &server {
            srv.fds(&mut fds);
        }
        let unsent = ports.values().any(|p| !p.pending.is_empty()) || server.as_ref().is_some_and(|s| s.has_unsent());
        let timeout = poll_timeout_ms(&broker, now, unsent);
        // SAFETY: `fds` is a valid array of `fds.len()` pollfd structures; every fd in it belongs to a port, to
        // `seq` or to `server`, which outlive this call.
        unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, timeout) };
        let mut dead = Vec::new();
        for (path, p) in ports.iter_mut().filter(|(_, p)| p.port.readable()) {
            let read = p.port.read(&mut buf);
            // Stamp as close to the read as the kernel allows: right after it, per port, so the ports read before
            // this one do not add their time to its events.
            let stamp = host_ns();
            match read {
                Ok(0) => {}
                Ok(n) => {
                    let mut pk = Vec::new();
                    for &b in &buf[..n] {
                        p.conv.push(b, &mut pk);
                    }
                    for u in pk {
                        if let Some(src) = p.source {
                            broker.hub.send_from_source(src, stamp, u);
                        }
                        if dump_seconds.is_some() {
                            println!("{stamp} {} {}", p.name, describe(&u));
                        }
                    }
                }
                Err(e) => {
                    log(&format!("{} went away: {e}", p.name));
                    dead.push(path.clone());
                }
            }
        }
        // Keyed by node path: two devices with the same name stay apart. The registry device goes with the port
        // (closing its fd), so a replug attaches a fresh one instead of piling up stale devices.
        let removed = !dead.is_empty();
        for path in dead {
            if let Some(p) = ports.remove(&path) {
                if let Some(dst) = p.destination {
                    broker.hub.untrack_destination(dst);
                }
                broker.hub.registry.remove_external_device(p.device);
            }
        }
        if removed {
            broker.setup_changed();
        }
        if let (Some(s), Some(src)) = (&seq, seq_source) {
            let read = s.read_events(&mut seq_buf);
            let stamp = host_ns();
            match read {
                Ok(evs) => {
                    seq_error = None;
                    for r in evs {
                        let from = r.event.source();
                        let mut pk = Vec::new();
                        if r.event.kind() == seq::EVENT_SYSEX {
                            let conv = seq_sysex.entry(from).or_insert_with(|| Midi1ToUmp::new(0));
                            for &b in &r.payload {
                                conv.push(b, &mut pk);
                            }
                        } else if let Some(u) = r.event.to_ump(0) {
                            pk.push(u);
                        }
                        for u in pk {
                            broker.hub.send_from_source(src, stamp, u);
                            if dump_seconds.is_some() {
                                println!("{stamp} seq {}:{} {}", from.0, from.1, describe(&u));
                            }
                        }
                    }
                }
                Err(e) => {
                    // ENOSPC is the kernel saying the input queue overflowed and was cleared; say each new error once.
                    let text = e.to_string();
                    if seq_error.as_deref() != Some(text.as_str()) {
                        log(&format!("sequencer read: {text}"));
                        seq_error = Some(text);
                    }
                }
            }
        }
        // App connections: requests in, events out. Then what apps sent to hardware goes to the devices, each
        // packet at its timestamp (a packet stamped in the future waits in the hub).
        let now = host_ns();
        if let Some(srv) = server.as_mut() {
            srv.service(&mut broker, now);
        } else {
            // no socket: nothing collects the inboxes, so keep them from growing
            let _ = broker.pump(now);
        }
        for (dst, t) in broker.hardware_output(now) {
            if let Some(p) = ports.values_mut().find(|p| p.destination == Some(dst)) {
                p.out_conv.push(&t.packet, &mut p.pending);
            }
        }
        for p in ports.values_mut() {
            p.flush();
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--list") => {
            for d in rawmidi::scan("/dev/snd") {
                let name = rawmidi::RawPort::open(&d.path)
                    .and_then(|p| p.info())
                    .map(|i| rawmidi::c_text(&i.name))
                    .unwrap_or_else(|e| format!("(cannot open: {e})"));
                println!("card {} device {} {} {name}", d.card, d.device, d.path);
            }
        }
        Some("--dump") => run(Some(args.get(1).and_then(|s| s.parse().ok()).unwrap_or(10))),
        _ => run(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_clock_is_monotonic() {
        let a = host_ns();
        let b = host_ns();
        assert!(b >= a && a > 0);
    }

    #[test]
    fn describe_prints_words() {
        assert_eq!(describe(&Ump::one(0x2090_3C64)), "20903C64");
        assert_eq!(describe(&Ump::two(1, 0xFFFF_0000)), "00000001 FFFF0000");
    }
}
