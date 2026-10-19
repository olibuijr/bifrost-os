//! `bifrost-midi`: looks at and drives the system MIDI server. See `bifrost_cmd::midictl`.

use bifrost_app::midi_client::{MidiClient, MidiEvent, Refusal};
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
