//! Clients of the supervised services (docs/services.md). The shell and the app SDK
//! (`bifrost_app::audio`) share these.

pub mod audio {
    //! The audio service client: connects to `<root>/run/is.olibuijr.audio.sock`, subscribes,
    //! keeps the latest `Snapshot`, and reconnects (and re-subscribes) when the service restarts.
    //! Drive it from the event loop: wait on `fd()`, call `pump()` when readable and `tick(now)`
    //! every turn (`next_wake(now)` says when it needs one with nothing else going on).

    pub use bifrost_proto::svc::audio::{Cmd, Evt, Live, Repeat, Snapshot, Track};
    use bifrost_proto::svc::{read_frame, MAX_FRAME};
    use std::ffi::c_int;
    use std::io::{self, Read, Write};
    use std::os::fd::AsRawFd;
    use std::os::unix::net::UnixStream;
    use std::path::{Path, PathBuf};

    pub const SERVICE_ID: &str = "is.olibuijr.audio";
    const RETRY_MIN_MS: f64 = 1000.0;
    const RETRY_MAX_MS: f64 = 15_000.0;

    /// The microphone meter, from the last `Record` event.
    #[derive(Clone, Debug, PartialEq)]
    pub struct Recording {
        pub active: bool,
        pub elapsed_ms: u64,
        pub level: f32,
        pub error: Option<String>,
    }

    pub fn socket_path(root: &Path) -> PathBuf {
        root.join("run").join(format!("{SERVICE_ID}.sock"))
    }

    /// A snapshot of nothing playing, what `Client::snapshot` reports while disconnected.
    pub fn idle() -> Snapshot {
        Snapshot { track: None, index: None, playing: false, position_ms: 0.0, duration_ms: None, shuffle: false, repeat: Repeat::Off, queue_len: 0, volume: 1.0, muted: false, error: None, live: None, rate: 1.0 }
    }

    pub struct Client {
        path: PathBuf,
        stream: Option<UnixStream>,
        rx: Vec<u8>,
        tx: Vec<u8>,
        /// the last `State` as received
        received: Snapshot,
        received_at: f64,
        clock: f64,
        /// `received` with the position advanced to the last `tick`
        view: Snapshot,
        recording: Option<Recording>,
        done: Vec<(String, u64)>,
        version: Option<String>,
        retry_at: f64,
        backoff: f64,
        /// bumped on every (re)connect, so a caller can resend what it set (volume, mute)
        connects: u32,
        /// false until `arm`: a lazy client does not connect, and so does not start the service (socket activation)
        armed: bool,
    }

    impl Client {
        pub fn new(path: PathBuf) -> Self {
            Client { path, stream: None, rx: Vec::new(), tx: Vec::new(), received: idle(), received_at: 0.0, clock: 0.0, view: idle(), recording: None, done: Vec::new(), version: None, retry_at: 0.0, backoff: RETRY_MIN_MS, connects: 0, armed: true }
        }

        /// The client of the service under the shell data dir `root`. It connects lazily: nothing is sent to the
        /// service (so bifrostd does not start it) until `arm`.
        pub fn for_root(root: &Path) -> Self {
            Client { armed: false, ..Self::new(socket_path(root)) }
        }

        /// Allows the connection. The shell arms its client once the first desktop frame is drawn, so the audio
        /// service starts after the desktop is on screen and not on the boot path.
        pub fn arm(&mut self) {
            self.armed = true;
        }

        pub fn armed(&self) -> bool {
            self.armed
        }

        /// The fd to wait for readability on, -1 while disconnected.
        pub fn fd(&self) -> c_int {
            self.stream.as_ref().map_or(-1, |s| s.as_raw_fd())
        }

        pub fn connected(&self) -> bool {
            self.stream.is_some()
        }

        /// How many times it has connected; changes when the service restarted.
        pub fn connects(&self) -> u32 {
            self.connects
        }

        /// The service's version from its last `Pong`, if it has answered.
        pub fn version(&self) -> Option<&str> {
            self.version.as_deref()
        }

        /// The cached state; the position keeps moving between the service's once-a-second updates.
        pub fn snapshot(&self) -> &Snapshot {
            &self.view
        }

        pub fn recording(&self) -> Option<&Recording> {
            self.recording.as_ref()
        }

        /// Recordings finished since the last call: (path, duration in ms).
        pub fn take_recordings(&mut self) -> Vec<(String, u64)> {
            std::mem::take(&mut self.done)
        }

        /// Reads whatever the service sent; true when the snapshot or recording changed.
        pub fn pump(&mut self) -> bool {
            let mut changed = self.flush().is_err();
            if self.stream.is_none() {
                if changed {
                    self.drop_connection();
                }
                return changed;
            }
            let mut buf = [0u8; 8192];
            loop {
                let Some(s) = self.stream.as_mut() else { break };
                match s.read(&mut buf) {
                    Ok(0) => {
                        self.drop_connection();
                        changed = true;
                    }
                    Ok(n) => self.rx.extend_from_slice(&buf[..n]),
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(_) => {
                        self.drop_connection();
                        changed = true;
                    }
                }
            }
            changed | self.decode()
        }

        /// Reconnects when due and advances the position; true when the snapshot changed on screen.
        pub fn tick(&mut self, now: f64) -> bool {
            let mut changed = false;
            if self.armed && self.stream.is_none() && now >= self.retry_at {
                changed |= self.connect(now);
            }
            if self.flush().is_err() {
                self.drop_connection();
                changed = true;
            }
            self.clock = now;
            let mut v = self.received.clone();
            if v.playing && v.live.is_none() {
                let p = v.position_ms + (now - self.received_at).max(0.0);
                v.position_ms = v.duration_ms.map_or(p, |d| p.min(d));
            }
            if v != self.view {
                let shown = |s: &Snapshot| Snapshot { position_ms: (s.position_ms / 1000.0).floor(), ..s.clone() };
                changed |= shown(&v) != shown(&self.view);
                self.view = v;
            }
            changed
        }

        /// When `tick` next has to run on its own: the next reconnect attempt, only while the
        /// service socket exists (a missing service is not polled for).
        pub fn next_wake(&self, now: f64) -> Option<f64> {
            (self.armed && self.stream.is_none() && self.path.exists()).then_some(self.retry_at.max(now))
        }

        fn connect(&mut self, now: f64) -> bool {
            let schedule = |c: &mut Self| {
                c.retry_at = now + c.backoff;
                c.backoff = (c.backoff * 2.0).min(RETRY_MAX_MS);
            };
            if !self.path.exists() {
                self.retry_at = now + RETRY_MIN_MS * 3.0;
                return false;
            }
            let s = match UnixStream::connect(&self.path).and_then(|s| s.set_nonblocking(true).map(|_| s)) {
                Ok(s) => s,
                Err(_) => {
                    schedule(self);
                    return false;
                }
            };
            self.stream = Some(s);
            self.rx.clear();
            self.tx.clear();
            self.backoff = RETRY_MIN_MS;
            self.connects = self.connects.wrapping_add(1);
            self.queue(&Cmd::Ping);
            self.queue(&Cmd::Subscribe);
            true
        }

        fn drop_connection(&mut self) {
            self.stream = None;
            self.rx.clear();
            self.tx.clear();
            self.received = idle();
            self.view = idle();
            self.recording = None;
            self.version = None;
            self.retry_at = 0.0;
        }

        fn queue(&mut self, cmd: &Cmd) {
            let payload = cmd.encode();
            self.tx.extend_from_slice(&(payload.len() as u32).to_le_bytes());
            self.tx.extend_from_slice(&payload);
        }

        /// Writes queued commands; a full socket buffer leaves the rest for the next turn.
        fn flush(&mut self) -> io::Result<()> {
            let Some(s) = self.stream.as_mut() else { return Ok(()) };
            if self.tx.len() > 4 * MAX_FRAME {
                return Err(io::ErrorKind::OutOfMemory.into());
            }
            while !self.tx.is_empty() {
                match s.write(&self.tx) {
                    Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                    Ok(n) => {
                        self.tx.drain(..n);
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(e) => return Err(e),
                }
            }
            Ok(())
        }

        /// Sends a command; false (nothing sent) while disconnected.
        pub fn send(&mut self, cmd: Cmd) -> bool {
            if self.stream.is_none() {
                return false;
            }
            self.queue(&cmd);
            if self.flush().is_err() {
                self.drop_connection();
                return false;
            }
            true
        }

        pub fn play(&mut self) -> bool {
            self.send(Cmd::Play)
        }
        pub fn pause(&mut self) -> bool {
            self.send(Cmd::Pause)
        }
        pub fn toggle(&mut self) -> bool {
            self.send(Cmd::Toggle)
        }
        pub fn next(&mut self) -> bool {
            self.send(Cmd::Next)
        }
        pub fn prev(&mut self) -> bool {
            self.send(Cmd::Prev)
        }
        pub fn stop(&mut self) -> bool {
            self.send(Cmd::Stop)
        }
        pub fn seek_ms(&mut self, ms: f64) -> bool {
            self.send(Cmd::SeekMs(ms))
        }
        pub fn set_volume(&mut self, v: f32) -> bool {
            self.send(Cmd::SetVolume(v.clamp(0.0, 1.0)))
        }
        /// Sets the playback rate (clamped to 0.5 to 2.0); audiod time-stretches, so the pitch does not change.
        pub fn set_rate(&mut self, rate: f32) -> bool {
            self.send(Cmd::SetRate(crate::media::timestretch::clamp_rate(rate)))
        }
        pub fn set_muted(&mut self, m: bool) -> bool {
            self.send(Cmd::SetMuted(m))
        }
        pub fn set_repeat(&mut self, r: Repeat) -> bool {
            self.send(Cmd::SetRepeat(r))
        }
        pub fn set_shuffle(&mut self, on: bool, seed: u64) -> bool {
            self.send(Cmd::SetShuffle { on, seed })
        }
        pub fn play_queue(&mut self, tracks: Vec<Track>, start: u32, shuffle: bool, seed: u64) -> bool {
            self.send(Cmd::PlayQueue { tracks, start, shuffle, seed })
        }
        pub fn record_start(&mut self, path: &str) -> bool {
            self.send(Cmd::RecordStart { path: path.into() })
        }
        pub fn record_stop(&mut self) -> bool {
            self.send(Cmd::RecordStop)
        }

        /// Applies the complete frames in `rx`; true when something visible changed.
        fn decode(&mut self) -> bool {
            let mut changed = false;
            let mut used = 0;
            loop {
                let mut rest = &self.rx[used..];
                if rest.len() < 4 {
                    break;
                }
                let len = u32::from_le_bytes(rest[..4].try_into().unwrap()) as usize;
                if len > MAX_FRAME {
                    self.drop_connection();
                    return true;
                }
                if rest.len() < 4 + len {
                    break;
                }
                let Ok(frame) = read_frame(&mut rest) else { break };
                used += 4 + len;
                match Evt::decode(&frame) {
                    Ok(Evt::State(s)) => {
                        changed |= s != self.received;
                        self.received = s;
                        self.received_at = self.clock;
                    }
                    Ok(Evt::Pong { version, .. }) => self.version = Some(version),
                    Ok(Evt::Record { active, elapsed_ms, level, error }) => {
                        let r = Recording { active, elapsed_ms, level, error };
                        changed |= self.recording.as_ref() != Some(&r);
                        self.recording = Some(r);
                    }
                    Ok(Evt::RecordDone { path, duration_ms }) => {
                        self.done.push((path, duration_ms));
                        self.recording = None;
                        changed = true;
                    }
                    Err(_) => {}
                }
            }
            self.rx.drain(..used);
            if changed {
                self.view = self.received.clone();
            }
            changed
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use bifrost_proto::svc::write_frame;
        use std::os::unix::net::UnixListener;

        fn pump_until(c: &mut Client) -> bool {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while std::time::Instant::now() < deadline {
                if c.pump() {
                    return true;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            false
        }

        #[test]
        fn a_lazy_client_connects_only_once_armed() {
            let dir = std::env::temp_dir().join(format!("svc-audio-lazy-{}", std::process::id()));
            std::fs::create_dir_all(dir.join("run")).unwrap();
            let sock = socket_path(&dir);
            let _ = std::fs::remove_file(&sock);
            let l = UnixListener::bind(&sock).unwrap();
            let mut c = Client::for_root(&dir);
            // a waiting socket does not make the lazy client connect: nothing is sent, so the service is not started
            assert!(!c.armed() && !c.tick(0.0) && !c.connected());
            assert!(c.next_wake(0.0).is_none(), "an unarmed client schedules no reconnects");
            l.set_nonblocking(true).unwrap();
            assert!(l.accept().is_err(), "no connection was made");
            c.arm();
            assert!(c.tick(0.0) && c.connected());
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        fn subscribes_caches_state_sends_commands_and_reconnects() {
            let dir = std::env::temp_dir().join(format!("svc-audio-{}", std::process::id()));
            std::fs::create_dir_all(dir.join("run")).unwrap();
            let sock = socket_path(&dir);
            let _ = std::fs::remove_file(&sock);
            let l = UnixListener::bind(&sock).unwrap();
            let mut c = Client::for_root(&dir);
            c.arm();
            assert!(c.tick(0.0) && c.connected());
            let (mut srv, _) = l.accept().unwrap();
            let first = Cmd::decode(&read_frame(&mut srv).unwrap()).unwrap();
            let second = Cmd::decode(&read_frame(&mut srv).unwrap()).unwrap();
            assert_eq!((first, second), (Cmd::Ping, Cmd::Subscribe));
            let mut s = idle();
            s.playing = true;
            s.track = Some(Track { id: 1, path: "/a.mp3".into(), url: None });
            s.duration_ms = Some(10_000.0);
            write_frame(&mut srv, &Evt::State(s).encode()).unwrap();
            assert!(pump_until(&mut c));
            assert!(c.snapshot().playing);
            c.tick(1500.0);
            assert!(c.snapshot().position_ms >= 1500.0 - 1.0);
            assert!(c.toggle());
            assert_eq!(Cmd::decode(&read_frame(&mut srv).unwrap()), Ok(Cmd::Toggle));
            drop(srv);
            assert!(pump_until(&mut c) && !c.connected() && c.snapshot().track.is_none());
            assert!(!c.send(Cmd::Play));
            assert!(c.tick(2000.0) && c.connected() && c.connects() == 2);
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}
