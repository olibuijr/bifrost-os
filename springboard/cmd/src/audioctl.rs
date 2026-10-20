//! `bifrost-audio`: looks at and drives the system audio service from a shell. `status` prints the player's state,
//! `watch` prints state changes and output-route reports as they arrive, `volume`, `pan` and `fade` set the player's
//! volume, stereo pan and a timed volume fade. The parsing and the formatting live here so they can be unit tested; the
//! binary connects (`bifrost_app::system_audio`) and prints.

use bifrost_app::system_audio::{AudioError, Command, Snapshot};
use std::io::ErrorKind;
use std::path::Path;

/// The low-level audio server's control socket (`bifrost-audiod`, `docs/audio-architecture.md`).
pub const SERVER_SOCKET: &str = "/run/bifrost/audio/control";
/// The media player service's id; its socket is `<root>/run/<id>.sock` (see `bifrost_app::system_audio::socket_path`).
pub const SERVICE_ID: &str = "is.olibuijr.audio";
/// Where the OS package keeps the system services that `bifrostd` supervises.
pub const SERVICE_PACKAGE_DIR: &str = "/usr/lib/bifrost/services";

pub const USAGE: &str = "usage: bifrost-audio status\n       bifrost-audio watch [SECONDS]\n       bifrost-audio volume LEVEL\n       bifrost-audio pan PAN\n       bifrost-audio fade LEVEL MILLISECONDS\n\nLEVEL is 0 to 1, PAN is -1 (left) to 1 (right). watch prints each state change and each output-route report for SECONDS (default 30).";

#[derive(Clone, Debug, PartialEq)]
pub enum Plan {
    Help,
    Status,
    Watch { seconds: u64 },
    Send(Request),
}

/// A command to send, kept as plain values until the binary builds the checked `Command`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Request {
    Volume(f32),
    Pan(f32),
    Fade { target: f32, ms: u32 },
}

impl Request {
    /// The checked command, or the reason the service's own client refuses it.
    pub fn command(self) -> Result<Command, AudioError> {
        match self {
            Request::Volume(v) => Command::volume(v),
            Request::Pan(p) => Command::pan(p),
            Request::Fade { target, ms } => Command::fade_volume(target, ms),
        }
    }
}

fn number(text: &str, what: &str) -> Result<f32, String> {
    text.parse::<f32>().ok().filter(|v| v.is_finite()).ok_or_else(|| format!("`{text}` is not a {what}"))
}

pub fn parse(args: &[String]) -> Result<Plan, String> {
    let Some(cmd) = args.first() else { return Err(USAGE.to_string()) };
    let rest = &args[1..];
    match (cmd.as_str(), rest) {
        ("-h" | "--help" | "help", _) => Ok(Plan::Help),
        ("status", []) => Ok(Plan::Status),
        ("watch", []) => Ok(Plan::Watch { seconds: 30 }),
        ("watch", [s]) => Ok(Plan::Watch { seconds: s.parse::<u64>().map_err(|_| format!("`{s}` is not a number of seconds"))? }),
        ("volume", [v]) => Ok(Plan::Send(Request::Volume(number(v, "level from 0 to 1")?))),
        ("pan", [p]) => Ok(Plan::Send(Request::Pan(number(p, "pan from -1 to 1")?))),
        ("fade", [v, ms]) => Ok(Plan::Send(Request::Fade {
            target: number(v, "level from 0 to 1")?,
            ms: ms.parse::<u32>().map_err(|_| format!("`{ms}` is not a number of milliseconds"))?,
        })),
        _ => Err(USAGE.to_string()),
    }
}

/// One line for the player's state.
pub fn format_state(s: &Snapshot) -> String {
    let what = match &s.track {
        Some(t) if !t.path.is_empty() => t.path.clone(),
        Some(t) => t.url.clone().unwrap_or_default(),
        None => "nothing queued".to_string(),
    };
    format!(
        "{} {} at {:.1} s, volume {:.2}{}, rate {:.2}",
        if s.playing { "playing" } else { "paused" },
        what,
        s.position_ms / 1000.0,
        s.volume,
        if s.muted { " (muted)" } else { "" },
        s.rate
    )
}

fn reason_text(code: u8) -> &'static str {
    match code {
        1 => "a device appeared",
        2 => "a device went away",
        3 => "category change",
        4 => "chosen by the user",
        _ => "unknown reason",
    }
}

/// One line for an output-route report.
pub fn format_route(headphones: bool, reason: u8) -> String {
    format!("route {} ({})", if headphones { "headphones" } else { "speaker" }, reason_text(reason))
}

/// The line for the audio server: its control socket is a SEQPACKET socket, so presence is what can be told here.
pub fn server_line(socket_present: bool) -> String {
    if socket_present {
        format!("audio server (bifrost-audiod): running, control socket {SERVER_SOCKET}")
    } else {
        format!("audio server (bifrost-audiod): not running, no control socket at {SERVER_SOCKET} (systemctl status bifrost-audiod)")
    }
}

/// Why connecting to the media service failed, in words.
pub fn unreachable_reason(kind: ErrorKind) -> &'static str {
    match kind {
        ErrorKind::NotFound => "no socket: the service is not installed, or bifrostd is not running",
        ErrorKind::ConnectionRefused => "the socket exists but nothing answers: the service is starting or keeps crashing",
        ErrorKind::PermissionDenied => "permission denied on the socket",
        _ => "cannot connect",
    }
}

/// The report when the media service cannot be reached: it names the service, its socket, the likely cause and the
/// state of the audio server underneath it, so one missing part is not mistaken for the other.
pub fn unreachable_report(socket: &Path, kind: ErrorKind, server_socket_present: bool) -> String {
    let mut out = format!("media service ({SERVICE_ID}): not reachable at {}: {}\n", socket.display(), unreachable_reason(kind));
    out.push_str(&server_line(server_socket_present));
    out.push('\n');
    if kind == ErrorKind::NotFound {
        out.push_str(&format!("The OS package provides the service in {SERVICE_PACKAGE_DIR}/{SERVICE_ID}; bifrostd binds its socket (systemctl status bifrostd)."));
    }
    out.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_app::system_audio::{Repeat, Snapshot};

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_command_line_is_parsed() {
        assert_eq!(parse(&args(&["status"])), Ok(Plan::Status));
        assert_eq!(parse(&args(&["watch"])), Ok(Plan::Watch { seconds: 30 }));
        assert_eq!(parse(&args(&["watch", "5"])), Ok(Plan::Watch { seconds: 5 }));
        assert_eq!(parse(&args(&["volume", "0.5"])), Ok(Plan::Send(Request::Volume(0.5))));
        assert_eq!(parse(&args(&["pan", "-1"])), Ok(Plan::Send(Request::Pan(-1.0))));
        assert_eq!(parse(&args(&["fade", "0", "3000"])), Ok(Plan::Send(Request::Fade { target: 0.0, ms: 3000 })));
        assert_eq!(parse(&args(&["--help"])), Ok(Plan::Help));
    }

    #[test]
    fn bad_command_lines_are_refused() {
        assert!(parse(&[]).is_err());
        assert!(parse(&args(&["status", "x"])).is_err());
        assert!(parse(&args(&["volume"])).is_err());
        assert!(parse(&args(&["pan", "left"])).unwrap_err().contains("pan"));
        assert!(parse(&args(&["pan", "NaN"])).is_err());
        assert!(parse(&args(&["fade", "1"])).is_err());
        assert!(parse(&args(&["fade", "1", "-5"])).unwrap_err().contains("milliseconds"));
        assert!(parse(&args(&["watch", "soon"])).is_err());
    }

    #[test]
    fn requests_become_checked_commands() {
        assert!(Request::Volume(0.5).command().is_ok());
        assert_eq!(Request::Volume(1.5).command(), Err(AudioError::InvalidVolume));
        assert!(Request::Pan(1.0).command().is_ok());
        assert_eq!(Request::Pan(2.0).command(), Err(AudioError::InvalidPan));
        assert!(Request::Fade { target: 0.0, ms: 100 }.command().is_ok());
        assert_eq!(Request::Fade { target: 3.0, ms: 100 }.command(), Err(AudioError::InvalidVolume));
    }

    #[test]
    fn state_and_route_lines() {
        let s = Snapshot {
            track: Some(bifrost_proto_track("/m/a.mp3")),
            index: Some(0),
            playing: true,
            position_ms: 12_340.0,
            duration_ms: Some(200_000.0),
            shuffle: false,
            repeat: Repeat::Off,
            queue_len: 1,
            volume: 0.6,
            muted: false,
            error: None,
            live: None,
            rate: 1.0,
        };
        assert_eq!(format_state(&s), "playing /m/a.mp3 at 12.3 s, volume 0.60, rate 1.00");
        let paused = Snapshot { playing: false, muted: true, track: None, ..s };
        assert_eq!(format_state(&paused), "paused nothing queued at 12.3 s, volume 0.60 (muted), rate 1.00");
        assert_eq!(format_route(true, 1), "route headphones (a device appeared)");
        assert_eq!(format_route(false, 2), "route speaker (a device went away)");
        assert_eq!(format_route(false, 0), "route speaker (unknown reason)");
    }

    #[test]
    fn a_missing_service_is_reported_apart_from_the_audio_server() {
        let sock = Path::new("/var/lib/springboard/run/is.olibuijr.audio.sock");
        let missing = unreachable_report(sock, ErrorKind::NotFound, true);
        assert!(missing.contains("media service (is.olibuijr.audio): not reachable at /var/lib/springboard/run/is.olibuijr.audio.sock"), "{missing}");
        assert!(missing.contains("not installed, or bifrostd is not running"));
        assert!(missing.contains("audio server (bifrost-audiod): running"), "the server is fine and says so: {missing}");
        assert!(missing.contains("/usr/lib/bifrost/services/is.olibuijr.audio"));
        let both = unreachable_report(sock, ErrorKind::ConnectionRefused, false);
        assert!(both.contains("nothing answers"));
        assert!(both.contains("audio server (bifrost-audiod): not running"), "{both}");
        assert!(!both.contains("OS package provides"), "the install hint is for a missing socket only");
    }

    #[test]
    fn the_server_line_names_the_control_socket() {
        assert!(server_line(true).contains("running"));
        assert!(server_line(false).contains("not running"));
        assert!(server_line(false).contains(SERVER_SOCKET));
    }

    fn bifrost_proto_track(path: &str) -> bifrost_app::proto::svc::audio::Track {
        bifrost_app::proto::svc::audio::Track { id: 1, path: path.to_string(), url: None }
    }
}
