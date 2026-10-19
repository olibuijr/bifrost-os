//! `bifrost-midi`: looks at and drives the system MIDI server from a shell. `list` prints the devices and their
//! sources and destinations, `monitor` prints what the connected sources send (with the host-clock stamp), `send`
//! writes MIDI bytes to a destination, `note` plays one note. The parsing and the formatting live here so they can be
//! unit tested; the binary only connects (`bifrost_app::midi_client`) and prints.

use bifrost_app::midi::ump::Ump;
use bifrost_app::midi_client::{EndpointInfo, Setup};

pub const USAGE: &str = "usage: bifrost-midi list\n       bifrost-midi monitor [SECONDS]\n       bifrost-midi send DESTINATION BYTE...\n       bifrost-midi note DESTINATION KEY [VELOCITY [MILLISECONDS]]\n       bifrost-midi thru SOURCE DESTINATION [SECONDS [TRANSPOSE]]\n       bifrost-midi ci DESTINATION SOURCE [SECONDS]\n\nDESTINATION and SOURCE are ids from `list` or part of a name. BYTEs are hexadecimal MIDI bytes (90 3C 64). monitor listens to every source for SECONDS (default 10). thru plays SOURCE through to DESTINATION inside the server for SECONDS (default 30), notes moved by TRANSPOSE semitones. ci sends a MIDI-CI discovery inquiry to DESTINATION and prints the devices that answer on SOURCE within SECONDS (default 5).";

/// What the command line asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum Plan {
    Help,
    List,
    Monitor { seconds: u64 },
    Send { destination: String, bytes: Vec<u8> },
    Note { destination: String, key: u8, velocity: u8, millis: u64 },
    Thru { source: String, destination: String, seconds: u64, transpose: i32 },
    Ci { destination: String, source: String, seconds: u64 },
}

/// One hexadecimal MIDI byte: one or two digits, optionally with a `0x` prefix.
pub fn parse_byte(text: &str) -> Result<u8, String> {
    let t = text.trim_start_matches("0x").trim_start_matches("0X");
    if t.is_empty() || t.len() > 2 {
        return Err(format!("`{text}` is not a hexadecimal byte"));
    }
    u8::from_str_radix(t, 16).map_err(|_| format!("`{text}` is not a hexadecimal byte"))
}

pub fn parse(args: &[String]) -> Result<Plan, String> {
    let Some(cmd) = args.first() else { return Err(USAGE.to_string()) };
    let rest = &args[1..];
    match cmd.as_str() {
        "-h" | "--help" | "help" => Ok(Plan::Help),
        "list" if rest.is_empty() => Ok(Plan::List),
        "monitor" if rest.len() <= 1 => {
            let seconds = match rest.first() {
                Some(s) => s.parse::<u64>().map_err(|_| format!("`{s}` is not a number of seconds"))?,
                None => 10,
            };
            Ok(Plan::Monitor { seconds })
        }
        "send" if rest.len() >= 2 => {
            let bytes = rest[1..].iter().map(|b| parse_byte(b)).collect::<Result<Vec<u8>, String>>()?;
            if bytes.iter().any(|b| *b == 0xF0) && bytes.last() != Some(&0xF7) {
                return Err("a system exclusive message must end with F7".to_string());
            }
            Ok(Plan::Send { destination: rest[0].clone(), bytes })
        }
        "note" if (2..=4).contains(&rest.len()) => {
            let key = rest[1].parse::<u8>().ok().filter(|k| *k < 128).ok_or_else(|| format!("`{}` is not a key number 0 to 127", rest[1]))?;
            let velocity = match rest.get(2) {
                Some(v) => v.parse::<u8>().ok().filter(|v| (1..128).contains(v)).ok_or_else(|| format!("`{v}` is not a velocity 1 to 127"))?,
                None => 100,
            };
            let millis = match rest.get(3) {
                Some(m) => m.parse::<u64>().map_err(|_| format!("`{m}` is not a number of milliseconds"))?,
                None => 500,
            };
            Ok(Plan::Note { destination: rest[0].clone(), key, velocity, millis })
        }
        "thru" if (2..=4).contains(&rest.len()) => {
            let seconds = match rest.get(2) {
                Some(s) => s.parse::<u64>().map_err(|_| format!("`{s}` is not a number of seconds"))?,
                None => 30,
            };
            let transpose = match rest.get(3) {
                Some(t) => t.parse::<i32>().ok().filter(|t| t.abs() <= 127).ok_or_else(|| format!("`{t}` is not a number of semitones"))?,
                None => 0,
            };
            Ok(Plan::Thru { source: rest[0].clone(), destination: rest[1].clone(), seconds, transpose })
        }
        "ci" if (2..=3).contains(&rest.len()) => {
            let seconds = match rest.get(2) {
                Some(s) => s.parse::<u64>().map_err(|_| format!("`{s}` is not a number of seconds"))?,
                None => 5,
            };
            Ok(Plan::Ci { destination: rest[0].clone(), source: rest[1].clone(), seconds })
        }
        _ => Err(USAGE.to_string()),
    }
}

/// A note-number table that moves every note by `by` semitones, held at the ends of the range (what a thru
/// connection's note map takes).
pub fn transpose_table(by: i32) -> Vec<u8> {
    (0..128i32).map(|i| (i + by).clamp(0, 127) as u8).collect()
}

/// A MIDI-CI unique id for this process: 28 bits from the clock and the process id.
pub fn local_muid() -> u32 {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos());
    (t ^ std::process::id().wrapping_mul(2_654_435_761)) & 0x0FFF_FFFF
}

/// The packet's words in hexadecimal, one group per word (`20903C64`).
pub fn describe_packet(p: &Ump) -> String {
    p.as_words().iter().map(|w| format!("{w:08X}")).collect::<Vec<_>>().join(" ")
}

/// A line of `monitor` output: the host-clock nanoseconds, the source's name and the packet.
pub fn monitor_line(host_ns: u64, source: &str, p: &Ump) -> String {
    format!("{host_ns} {source} {}", describe_packet(p))
}

fn kind(e: &EndpointInfo) -> &'static str {
    match (e.source, e.owner != 0) {
        (true, false) => "source",
        (false, false) => "destination",
        (true, true) => "virtual source",
        (false, true) => "virtual destination",
    }
}

/// The text `list` prints: each device with its entities and endpoints, then the virtual endpoints.
pub fn format_list(setup: &Setup) -> String {
    let mut out = String::new();
    let line = |e: &EndpointInfo| format!("    {:>4}  {:<19} {}{}\n", e.id, kind(e), e.name, if e.connected { "  (connected)" } else { "" });
    for d in &setup.devices {
        out.push_str(&format!("{} ({})\n", d.name, if d.external { "external" } else { "built in" }));
        for ent in setup.entities.iter().filter(|e| e.device == d.id) {
            out.push_str(&format!("  {}\n", ent.name));
            for id in ent.sources.iter().chain(ent.destinations.iter()) {
                if let Some(e) = setup.endpoints.iter().find(|e| e.id == *id) {
                    out.push_str(&line(e));
                }
            }
        }
    }
    let virtuals: Vec<&EndpointInfo> = setup.endpoints.iter().filter(|e| e.owner != 0).collect();
    if !virtuals.is_empty() {
        out.push_str("virtual endpoints\n");
        for e in virtuals {
            out.push_str(&line(e));
        }
    }
    if out.is_empty() {
        out.push_str("no MIDI devices\n");
    }
    out
}

/// The endpoint a user means: an id, else the single endpoint of that direction whose name contains the text
/// (case does not matter). Several matches are an error that lists them.
pub fn find_endpoint(setup: &Setup, spec: &str, source: bool) -> Result<u32, String> {
    let candidates: Vec<&EndpointInfo> = setup.endpoints.iter().filter(|e| e.source == source).collect();
    if let Ok(id) = spec.parse::<u32>() {
        return candidates.iter().find(|e| e.id == id).map(|e| e.id).ok_or_else(|| format!("no {} with id {id}", if source { "source" } else { "destination" }));
    }
    let want = spec.to_lowercase();
    let hits: Vec<&&EndpointInfo> = candidates.iter().filter(|e| e.name.to_lowercase().contains(&want)).collect();
    match hits.as_slice() {
        [] => Err(format!("no {} named like `{spec}`", if source { "source" } else { "destination" })),
        [one] => Ok(one.id),
        many => Err(format!("`{spec}` matches {}: {}", many.len(), many.iter().map(|e| format!("{} ({})", e.name, e.id)).collect::<Vec<_>>().join(", "))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_app::midi_client::{DeviceInfo, EntityInfo};

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn setup() -> Setup {
        Setup {
            generation: 1,
            devices: vec![DeviceInfo { id: 2, name: "USB Keys".into(), external: true, entities: vec![3] }],
            entities: vec![EntityInfo { id: 3, name: "Port 1".into(), device: 2, sources: vec![4], destinations: vec![5] }],
            endpoints: vec![
                EndpointInfo { id: 4, name: "USB Keys out".into(), source: true, entity: 3, owner: 0, ref_con: 0, connected: true },
                EndpointInfo { id: 5, name: "USB Keys in".into(), source: false, entity: 3, owner: 0, ref_con: 0, connected: false },
                EndpointInfo { id: 6, name: "Synth in".into(), source: false, entity: 0, owner: 9, ref_con: 0, connected: false },
            ],
        }
    }

    #[test]
    fn the_command_line_is_parsed() {
        assert_eq!(parse(&args(&["list"])), Ok(Plan::List));
        assert_eq!(parse(&args(&["monitor"])), Ok(Plan::Monitor { seconds: 10 }));
        assert_eq!(parse(&args(&["monitor", "3"])), Ok(Plan::Monitor { seconds: 3 }));
        assert_eq!(parse(&args(&["send", "5", "90", "3C", "0x64"])), Ok(Plan::Send { destination: "5".into(), bytes: vec![0x90, 0x3C, 0x64] }));
        assert_eq!(parse(&args(&["note", "keys", "60"])), Ok(Plan::Note { destination: "keys".into(), key: 60, velocity: 100, millis: 500 }));
        assert_eq!(parse(&args(&["note", "keys", "60", "80", "250"])), Ok(Plan::Note { destination: "keys".into(), key: 60, velocity: 80, millis: 250 }));
        assert_eq!(parse(&args(&["--help"])), Ok(Plan::Help));
    }

    #[test]
    fn bad_command_lines_are_refused_with_a_reason() {
        assert!(parse(&[]).is_err());
        assert!(parse(&args(&["list", "x"])).is_err());
        assert!(parse(&args(&["monitor", "soon"])).unwrap_err().contains("seconds"));
        assert!(parse(&args(&["send", "5"])).is_err());
        assert!(parse(&args(&["send", "5", "ZZ"])).unwrap_err().contains("hexadecimal"));
        assert!(parse(&args(&["send", "5", "F0", "7E"])).unwrap_err().contains("F7"));
        assert!(parse(&args(&["send", "5", "F0", "7E", "F7"])).is_ok());
        assert!(parse(&args(&["note", "5", "200"])).unwrap_err().contains("key"));
        assert!(parse(&args(&["note", "5", "60", "0"])).unwrap_err().contains("velocity"));
        assert!(parse(&args(&["fly"])).is_err());
    }

    #[test]
    fn thru_ci_and_transpose_are_parsed() {
        assert_eq!(
            parse(&args(&["thru", "keys", "synth"])),
            Ok(Plan::Thru { source: "keys".into(), destination: "synth".into(), seconds: 30, transpose: 0 })
        );
        assert_eq!(
            parse(&args(&["thru", "4", "6", "10", "-12"])),
            Ok(Plan::Thru { source: "4".into(), destination: "6".into(), seconds: 10, transpose: -12 })
        );
        assert!(parse(&args(&["thru", "4"])).is_err());
        assert!(parse(&args(&["thru", "4", "6", "soon"])).is_err());
        assert!(parse(&args(&["thru", "4", "6", "5", "999"])).unwrap_err().contains("semitones"));
        assert_eq!(parse(&args(&["ci", "5", "4"])), Ok(Plan::Ci { destination: "5".into(), source: "4".into(), seconds: 5 }));
        assert_eq!(parse(&args(&["ci", "5", "4", "9"])), Ok(Plan::Ci { destination: "5".into(), source: "4".into(), seconds: 9 }));
        assert!(parse(&args(&["ci", "5"])).is_err());
    }

    #[test]
    fn the_transpose_table_moves_notes_and_holds_the_ends() {
        let t = transpose_table(12);
        assert_eq!(t.len(), 128);
        assert_eq!((t[0], t[60], t[115], t[116], t[127]), (12, 72, 127, 127, 127));
        let d = transpose_table(-12);
        assert_eq!((d[0], d[11], d[12], d[60]), (0, 0, 0, 48));
        assert_eq!(transpose_table(0), (0..128u8).collect::<Vec<u8>>());
    }

    #[test]
    fn the_local_muid_fits_28_bits() {
        for _ in 0..5 {
            assert!(local_muid() <= 0x0FFF_FFFF);
        }
    }

    #[test]
    fn bytes_take_one_or_two_hex_digits() {
        assert_eq!(parse_byte("9"), Ok(9));
        assert_eq!(parse_byte("ff"), Ok(255));
        assert_eq!(parse_byte("0x7F"), Ok(127));
        assert!(parse_byte("").is_err());
        assert!(parse_byte("100").is_err());
    }

    #[test]
    fn packets_print_as_words() {
        assert_eq!(describe_packet(&Ump::one(0x2090_3C64)), "20903C64");
        assert_eq!(describe_packet(&Ump::two(1, 0xFFFF_0000)), "00000001 FFFF0000");
        assert_eq!(monitor_line(55, "Keys", &Ump::one(0x2080_3C00)), "55 Keys 20803C00");
    }

    #[test]
    fn the_list_shows_devices_endpoints_and_virtual_ones() {
        let text = format_list(&setup());
        assert!(text.contains("USB Keys (external)"), "{text}");
        assert!(text.contains("source              USB Keys out  (connected)") || text.contains("USB Keys out  (connected)"), "{text}");
        assert!(text.contains("destination         USB Keys in") || text.contains("USB Keys in"), "{text}");
        assert!(text.contains("virtual endpoints"));
        assert!(text.contains("virtual destination"));
        assert_eq!(format_list(&Setup::default()), "no MIDI devices\n");
    }

    #[test]
    fn endpoints_are_found_by_id_or_name() {
        let s = setup();
        assert_eq!(find_endpoint(&s, "5", false), Ok(5));
        assert_eq!(find_endpoint(&s, "KEYS", false), Ok(5));
        assert_eq!(find_endpoint(&s, "keys", true), Ok(4));
        assert!(find_endpoint(&s, "4", false).is_err(), "4 is a source");
        assert!(find_endpoint(&s, "piano", false).unwrap_err().contains("no destination"));
        // "in" matches both destinations
        assert!(find_endpoint(&s, "in", false).unwrap_err().contains("matches 2"));
    }
}
