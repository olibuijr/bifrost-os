//! Command line parsing by hand (clap is not available offline).

pub const USAGE: &str = "usage: bifrost-audio-bench <midi-latency|audio-roundtrip|xrun-sweep|midi-throughput> [flags]
  --driver null|loopback|alsa:<card>   (default null; null runs on a manual clock, the others in real time)
  --rate <Hz>          sample rate (default 48000)
  --quantum <frames>   frames per cycle (default 64)
  --count <n>          notes, impulses or events (default 100)
  --seconds <s>        run time per quantum for xrun-sweep (default 2)
  --stress <n>         busy threads during xrun-sweep (default 0)
  --device <uid>       exact device uid for the alsa driver (default: the first full-duplex device of the card)
  --interval-ms <ms>   time between notes or impulses (default 20)
  --lead <cycles>      send each note this many cycles before its stamp (default 0: stamp the note when it is sent)
  --live               run against the running bifrost-audiod through the HAL client (midi-latency, audio-roundtrip,
                       xrun-sweep); --driver is ignored, --device picks the device uid (default: the default output)
  --json               print the JSON object (the default prints a one-line summary)";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cmd {
    MidiLatency,
    AudioRoundtrip,
    XrunSweep,
    MidiThroughput,
}

impl Cmd {
    pub fn name(self) -> &'static str {
        match self {
            Cmd::MidiLatency => "midi-latency",
            Cmd::AudioRoundtrip => "audio-roundtrip",
            Cmd::XrunSweep => "xrun-sweep",
            Cmd::MidiThroughput => "midi-throughput",
        }
    }

    fn parse(s: &str) -> Option<Cmd> {
        Some(match s {
            "midi-latency" => Cmd::MidiLatency,
            "audio-roundtrip" => Cmd::AudioRoundtrip,
            "xrun-sweep" => Cmd::XrunSweep,
            "midi-throughput" => Cmd::MidiThroughput,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriverSpec {
    Null,
    Loopback,
    Alsa(String),
}

impl DriverSpec {
    pub fn parse(s: &str) -> Result<DriverSpec, String> {
        match s {
            "null" => Ok(DriverSpec::Null),
            "loopback" => Ok(DriverSpec::Loopback),
            _ => match s.strip_prefix("alsa:") {
                Some(card) if !card.is_empty() => Ok(DriverSpec::Alsa(card.to_string())),
                _ => Err(format!("unknown driver '{s}' (null, loopback or alsa:<card>)")),
            },
        }
    }

    /// The `driver` field of the JSON object.
    pub fn label(&self) -> String {
        match self {
            DriverSpec::Null => "null".to_string(),
            DriverSpec::Loopback => "loopback".to_string(),
            DriverSpec::Alsa(card) => format!("alsa:{card}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Args {
    pub cmd: Cmd,
    pub driver: DriverSpec,
    pub rate: u32,
    pub quantum: u32,
    pub count: usize,
    pub seconds: f64,
    pub stress: u32,
    pub device: Option<String>,
    pub interval_ms: f64,
    pub lead: u32,
    pub json: bool,
    /// Measure the running server through the HAL instead of the in-process model.
    pub live: bool,
}

fn num<T: std::str::FromStr>(flag: &str, v: &str) -> Result<T, String> {
    v.parse::<T>().map_err(|_| format!("{flag}: '{v}' is not a valid number"))
}

/// Parses the arguments after the program name.
pub fn parse(argv: &[String]) -> Result<Args, String> {
    let (first, rest) = argv.split_first().ok_or_else(|| "missing subcommand".to_string())?;
    let cmd = Cmd::parse(first).ok_or_else(|| format!("unknown subcommand '{first}'"))?;
    let mut a = Args {
        cmd,
        driver: DriverSpec::Null,
        rate: 48_000,
        quantum: 64,
        count: 100,
        seconds: 2.0,
        stress: 0,
        device: None,
        interval_ms: 20.0,
        lead: 0,
        json: false,
        live: false,
    };
    let mut i = 0;
    while i < rest.len() {
        let arg = &rest[i];
        i += 1;
        let (flag, inline) = match arg.split_once('=') {
            Some((f, v)) => (f.to_string(), Some(v.to_string())),
            None => (arg.clone(), None),
        };
        if flag == "--json" {
            a.json = inline.map_or(true, |v| v != "false");
            continue;
        }
        if flag == "--live" {
            a.live = inline.map_or(true, |v| v != "false");
            continue;
        }
        let value = match inline {
            Some(v) => v,
            None => {
                let v = rest.get(i).ok_or_else(|| format!("{flag} needs a value"))?.clone();
                i += 1;
                v
            }
        };
        match flag.as_str() {
            "--driver" => a.driver = DriverSpec::parse(&value)?,
            "--rate" => a.rate = num(&flag, &value)?,
            "--quantum" => a.quantum = num(&flag, &value)?,
            "--count" => a.count = num(&flag, &value)?,
            "--seconds" => a.seconds = num(&flag, &value)?,
            "--stress" => a.stress = num(&flag, &value)?,
            "--device" => a.device = Some(value),
            "--interval-ms" => a.interval_ms = num(&flag, &value)?,
            "--lead" => a.lead = num(&flag, &value)?,
            _ => return Err(format!("unknown flag '{flag}'")),
        }
    }
    if !(8_000..=768_000).contains(&a.rate) {
        return Err(format!("--rate {} is outside 8000..768000", a.rate));
    }
    if !(16..=65_536).contains(&a.quantum) {
        return Err(format!("--quantum {} is outside 16..65536", a.quantum));
    }
    if a.count == 0 {
        return Err("--count must be at least 1".to_string());
    }
    if !(a.seconds.is_finite() && a.seconds > 0.0) {
        return Err("--seconds must be positive".to_string());
    }
    if !(a.interval_ms.is_finite() && a.interval_ms > 0.0) {
        return Err("--interval-ms must be positive".to_string());
    }
    if a.live && a.cmd == Cmd::MidiThroughput {
        return Err("midi-throughput has no --live mode (it measures the scheduler, not a device)".to_string());
    }
    Ok(a)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn defaults() {
        let a = parse(&v(&["midi-latency"])).unwrap();
        assert_eq!(a.cmd, Cmd::MidiLatency);
        assert_eq!((a.driver.clone(), a.rate, a.quantum, a.count), (DriverSpec::Null, 48_000, 64, 100));
        assert_eq!((a.seconds, a.stress, a.lead, a.json), (2.0, 0, 0, false));
        assert_eq!(a.device, None);
        assert!(!a.live);
    }

    #[test]
    fn live_flag() {
        assert!(parse(&v(&["audio-roundtrip", "--live"])).unwrap().live);
        assert!(parse(&v(&["xrun-sweep", "--live=true", "--json"])).unwrap().live);
        assert!(!parse(&v(&["midi-latency", "--live=false"])).unwrap().live);
        // --live takes no value: the next word is still a flag
        let a = parse(&v(&["midi-latency", "--live", "--quantum", "32"])).unwrap();
        assert_eq!((a.live, a.quantum), (true, 32));
        assert!(parse(&v(&["midi-throughput", "--live"])).is_err());
    }

    #[test]
    fn flags_with_space_and_equals() {
        let a = parse(&v(&[
            "xrun-sweep", "--driver", "alsa:PCH", "--rate=96000", "--quantum", "128", "--seconds=0.5", "--stress", "3", "--json",
            "--device", "alsa:PCH:0", "--count=7", "--interval-ms", "10", "--lead", "2",
        ]))
        .unwrap();
        assert_eq!(a.cmd, Cmd::XrunSweep);
        assert_eq!(a.driver, DriverSpec::Alsa("PCH".into()));
        assert_eq!((a.rate, a.quantum, a.count, a.stress, a.lead), (96_000, 128, 7, 3, 2));
        assert_eq!((a.seconds, a.interval_ms, a.json), (0.5, 10.0, true));
        assert_eq!(a.device.as_deref(), Some("alsa:PCH:0"));
    }

    #[test]
    fn drivers() {
        assert_eq!(DriverSpec::parse("null"), Ok(DriverSpec::Null));
        assert_eq!(DriverSpec::parse("loopback"), Ok(DriverSpec::Loopback));
        assert!(DriverSpec::parse("alsa:").is_err());
        assert!(DriverSpec::parse("jack").is_err());
        assert_eq!(DriverSpec::Alsa("PCH".into()).label(), "alsa:PCH");
        assert_eq!(DriverSpec::Loopback.label(), "loopback");
    }

    #[test]
    fn errors() {
        assert!(parse(&[]).is_err());
        assert!(parse(&v(&["bogus"])).is_err());
        assert!(parse(&v(&["midi-latency", "--rate"])).is_err());
        assert!(parse(&v(&["midi-latency", "--rate", "fast"])).is_err());
        assert!(parse(&v(&["midi-latency", "--nope", "1"])).is_err());
        assert!(parse(&v(&["midi-latency", "--quantum", "8"])).is_err());
        assert!(parse(&v(&["midi-latency", "--rate", "100"])).is_err());
        assert!(parse(&v(&["midi-latency", "--count", "0"])).is_err());
        assert!(parse(&v(&["midi-latency", "--seconds", "0"])).is_err());
        assert!(parse(&v(&["midi-latency", "--interval-ms", "-1"])).is_err());
    }

    #[test]
    fn subcommand_names_round_trip() {
        for c in [Cmd::MidiLatency, Cmd::AudioRoundtrip, Cmd::XrunSweep, Cmd::MidiThroughput] {
            assert_eq!(Cmd::parse(c.name()), Some(c));
        }
    }
}
