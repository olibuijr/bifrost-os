//! `bifrost-audio`: looks at and drives the system audio service. See `bifrost_cmd::audioctl`.

use bifrost_app::system_audio::{socket_path, AudioClient, AudioError, Evt, SOCKET_ENV};
use bifrost_cmd::audioctl::{self, Plan};
use bifrost_cmd::die;
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

const TOOL: &str = "bifrost-audio";

fn fail<E: std::fmt::Debug>(e: E) -> ! {
    die(TOOL, &format!("{e:?}"), 1)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let plan = audioctl::parse(&args).unwrap_or_else(|e| die(TOOL, &e, 1));
    if plan == Plan::Help {
        println!("{}", audioctl::USAGE);
        return;
    }
    let path = socket_path(std::env::var(SOCKET_ENV).ok().as_deref());
    let stream = UnixStream::connect(&path).unwrap_or_else(|e| die(TOOL, &format!("cannot reach the audio service at {}: {e}", path.display()), 1));
    stream.set_read_timeout(Some(Duration::from_millis(500))).unwrap_or_else(|e| fail(e));
    let mut client = AudioClient::new(stream);
    match plan {
        Plan::Help => {}
        Plan::Status => {
            let state = client.subscribe().unwrap_or_else(|e| fail(e));
            println!("{}", audioctl::format_state(&state));
        }
        Plan::Send(request) => {
            let command = request.command().unwrap_or_else(|e| fail(e));
            client.send(&command).unwrap_or_else(|e| fail(e));
            // the service answers a subscription with the state, so the change can be seen from here
            std::thread::sleep(Duration::from_millis(200));
            let state = client.subscribe().unwrap_or_else(|e| fail(e));
            println!("{}", audioctl::format_state(&state));
        }
        Plan::Watch { seconds } => {
            let state = client.subscribe().unwrap_or_else(|e| fail(e));
            println!("{}", audioctl::format_state(&state));
            let end = Instant::now() + Duration::from_secs(seconds);
            let mut last = audioctl::format_state(&state);
            while Instant::now() < end {
                match client.next_event() {
                    Ok(Evt::State(s)) => {
                        let line = audioctl::format_state(&s);
                        if line != last {
                            println!("{line}");
                            last = line;
                        }
                    }
                    Ok(Evt::Route { headphones, reason }) => println!("{}", audioctl::format_route(headphones, reason)),
                    Ok(_) => {}
                    // a read that timed out: the service had nothing to say, ask again
                    Err(AudioError::Io(kind)) if matches!(kind, std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
                    Err(e) => fail(e),
                }
            }
        }
    }
}
