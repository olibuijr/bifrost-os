//! `bifrost-midi`: looks at and drives the system MIDI server. See `bifrost_cmd::midictl`.

use bifrost_app::midi_ci::{DeviceInfo, DiscoveryManager, Muid};
use bifrost_app::midi_client::{CiOut, MidiCiLink, MidiClient, MidiEvent, Refusal, ThruSpec};
use bifrost_cmd::die;
use bifrost_cmd::midictl::{self, Plan};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

const TOOL: &str = "bifrost-midi";

fn fail<E: std::fmt::Display>(e: E) -> ! {
    die(TOOL, &e.to_string(), 1)
}

/// Waits for the server to have handled what was sent, and reports a refused send.
fn settle(client: &mut MidiClient) {
    if let Err(e) = client.ping() {
        fail(e);
    }
    match client.poll() {
        Ok(events) => {
            for e in events {
                if let MidiEvent::SendRefused(why) = e {
                    fail(match why {
                        Refusal::BadObject => "the destination does not exist (any more)".to_string(),
                        other => other.to_string(),
                    });
                }
            }
        }
        Err(e) => fail(e),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let plan = midictl::parse(&args).unwrap_or_else(|e| fail(e));
    if plan == Plan::Help {
        println!("{}", midictl::USAGE);
        return;
    }
    let mut client = MidiClient::connect(TOOL).unwrap_or_else(|e| fail(format!("cannot reach the MIDI server: {e}")));
    match plan {
        Plan::Help => {}
        Plan::List => {
            let setup = client.setup().unwrap_or_else(|e| fail(e)).clone();
            print!("{}", midictl::format_list(&setup));
        }
        Plan::Monitor { seconds } => {
            let mut names: BTreeMap<u32, String> = BTreeMap::new();
            let end = Instant::now() + Duration::from_secs(seconds);
            while Instant::now() < end {
                // hardware plugged in while watching is picked up too
                for s in client.sources() {
                    if !names.contains_key(&s.id) {
                        names.insert(s.id, s.name.clone());
                        if !s.connected {
                            let _ = client.connect_source(s.id);
                        }
                    }
                }
                let events = client.wait(Duration::from_millis(100)).unwrap_or_else(|e| fail(e));
                for e in events {
                    if let MidiEvent::Packet { source, host_ns, packet } = e {
                        let name = names.get(&source).map(String::as_str).unwrap_or("?");
                        println!("{}", midictl::monitor_line(host_ns, name, &packet));
                    }
                }
            }
        }
        Plan::Send { destination, bytes } => {
            let setup = client.setup().unwrap_or_else(|e| fail(e)).clone();
            let dest = midictl::find_endpoint(&setup, &destination, false).unwrap_or_else(|e| fail(e));
            client.send_bytes(dest, 0, &bytes).unwrap_or_else(|e| fail(e));
            settle(&mut client);
        }
        Plan::Thru { source, destination, seconds, transpose } => {
            let setup = client.setup().unwrap_or_else(|e| fail(e)).clone();
            let src = midictl::find_endpoint(&setup, &source, true).unwrap_or_else(|e| fail(e));
            let dst = midictl::find_endpoint(&setup, &destination, false).unwrap_or_else(|e| fail(e));
            let spec = ThruSpec { channel: None, note: midictl::transpose_table(transpose), velocity: Vec::new(), controls: Vec::new() };
            let id = client.create_thru(src, dst, spec).unwrap_or_else(|e| fail(e));
            println!("thru {id}: {source} -> {destination}, notes moved by {transpose} semitones, for {seconds} s");
            // the connection lives in the server for as long as this client does
            std::thread::sleep(Duration::from_secs(seconds));
            let _ = client.dispose_thru(id);
        }
        Plan::Ci { destination, source, seconds } => {
            let setup = client.setup().unwrap_or_else(|e| fail(e)).clone();
            let dst = midictl::find_endpoint(&setup, &destination, false).unwrap_or_else(|e| fail(e));
            let src = midictl::find_endpoint(&setup, &source, true).unwrap_or_else(|e| fail(e));
            client.connect_source(src).unwrap_or_else(|e| fail(e));
            let info = DeviceInfo {
                manufacturer: [0x7D, 0, 0],
                family: 0,
                model: 0,
                software_revision: [0, 0, 0, 1],
                capability_category: 0x01,
                max_sysex_size: 512,
            };
            let mut link = MidiCiLink::new(DiscoveryManager::new(Muid(midictl::local_muid()), info), CiOut::Destination(dst));
            link.discover(&mut client).unwrap_or_else(|e| fail(e));
            let end = Instant::now() + Duration::from_secs(seconds);
            while Instant::now() < end {
                let events = client.wait(Duration::from_millis(100)).unwrap_or_else(|e| fail(e));
                link.process(&mut client, &events).unwrap_or_else(|e| fail(e));
            }
            let found = link.manager().discovered_nodes();
            if found.is_empty() {
                println!("no MIDI-CI device answered");
            }
            for d in found {
                println!(
                    "device {:07X}: manufacturer {:02X}{:02X}{:02X}, family {}, model {}, {} profile{}",
                    d.muid.0,
                    d.info.manufacturer[0],
                    d.info.manufacturer[1],
                    d.info.manufacturer[2],
                    d.info.family,
                    d.info.model,
                    d.profiles.len(),
                    if d.profiles.len() == 1 { "" } else { "s" }
                );
            }
        }
        Plan::Note { destination, key, velocity, millis } => {
            let setup = client.setup().unwrap_or_else(|e| fail(e)).clone();
            let dest = midictl::find_endpoint(&setup, &destination, false).unwrap_or_else(|e| fail(e));
            client.send_bytes(dest, 0, &[0x90, key, velocity]).unwrap_or_else(|e| fail(e));
            settle(&mut client);
            std::thread::sleep(Duration::from_millis(millis));
            client.send_bytes(dest, 0, &[0x80, key, 0]).unwrap_or_else(|e| fail(e));
            settle(&mut client);
        }
    }
}
