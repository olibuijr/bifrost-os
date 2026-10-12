//! `bifrost-midid` (owner: task 15): the MIDI server. ALSA rawmidi and seq over the kernel uapi (no libasound), BLE
//! MIDI ingress, a virtual source and destination per app, UMP inside, timestamps on the audio host clock.
//!
//! Usage: `bifrost-midid --list` prints the rawmidi devices. `bifrost-midid --dump [SECONDS]` opens every rawmidi
//! input and the sequencer port and prints each event with its host timestamp (the Midget keyboard check). With no
//! argument it runs as the daemon loop.

mod ble_link;
mod rawmidi;
mod seq;

use bifrost_midi::endpoints::{Direction, ObjectId};
use bifrost_midi::hub::Hub;
use bifrost_midi::ump::{from_midi1, Midi1ToUmp, Ump};
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

struct HwPort {
    port: rawmidi::RawPort,
    source: ObjectId,
    conv: Midi1ToUmp,
    name: String,
}

fn open_hardware(hub: &mut Hub, ports: &mut BTreeMap<String, HwPort>) {
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
        let Some(src) = hub.registry.add_endpoint(ent, Direction::Source, &name) else { continue };
        hub.registry.add_endpoint(ent, Direction::Destination, &name);
        log(&format!("rawmidi {} ({name}) attached", dev.path));
        ports.insert(dev.path.clone(), HwPort { port, source: src, conv: Midi1ToUmp::new(0), name });
    }
}

fn describe(p: &Ump) -> String {
    let w: Vec<String> = p.as_words().iter().map(|w| format!("{w:08X}")).collect();
    w.join(" ")
}

fn run(dump_seconds: Option<u64>) {
    let mut hub = Hub::new();
    let mut ports: BTreeMap<String, HwPort> = BTreeMap::new();
    let (_client, _vsrc, _vdst) = hub.create_app("bifrost-midid");
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
        let d = hub.registry.add_device("Sequencer", true);
        hub.registry.add_entity(d, "Sequencer").and_then(|e| hub.registry.add_endpoint(e, Direction::Source, "Sequencer"))
    };
    let start = host_ns();
    let mut last_scan = 0u64;
    let mut buf = [0u8; 4096];
    loop {
        let now = host_ns();
        if dump_seconds.is_some_and(|s| now - start > s * 1_000_000_000) {
            return;
        }
        if now - last_scan > 2_000_000_000 || last_scan == 0 {
            open_hardware(&mut hub, &mut ports);
            last_scan = now;
        }
        let mut fds: Vec<libc::pollfd> =
            ports.values().map(|p| libc::pollfd { fd: p.port.fd(), events: libc::POLLIN, revents: 0 }).collect();
        if let Some(s) = &seq {
            fds.push(libc::pollfd { fd: s.fd(), events: libc::POLLIN, revents: 0 });
        }
        // SAFETY: `fds` is a valid array of `fds.len()` pollfd structures.
        unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, 200) };
        let stamp = host_ns();
        let mut dead = Vec::new();
        for p in ports.values_mut() {
            match p.port.read(&mut buf) {
                Ok(0) => {}
                Ok(n) => {
                    let mut pk = Vec::new();
                    for &b in &buf[..n] {
                        p.conv.push(b, &mut pk);
                    }
                    for u in pk {
                        hub.send_from_source(p.source, stamp, u);
                        if dump_seconds.is_some() {
                            println!("{stamp} {} {}", p.name, describe(&u));
                        }
                    }
                }
                Err(e) => {
                    log(&format!("{} went away: {e}", p.name));
                    dead.push(p.name.clone());
                }
            }
        }
        ports.retain(|_, p| !dead.contains(&p.name));
        if let (Some(s), Some(src)) = (&seq, seq_source) {
            if let Ok(evs) = s.read_events(&mut buf) {
                for e in evs {
                    if let Some(m) = e.to_midi1() {
                        let u = from_midi1(0, m);
                        hub.send_from_source(src, stamp, u);
                        if dump_seconds.is_some() {
                            println!("{stamp} seq {}:{} {}", e.source().0, e.source().1, describe(&u));
                        }
                    }
                }
            }
        }
        // Everything routed so far has been queued for its listeners; drain what no client collects.
        let _ = hub.pending(_client);
        let _ = hub.drain_inbox(_client, stamp);
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
