//! Bounded app-command delivery and Servo event-loop scheduling.
//! Servo wakes the owning thread; callbacks never block waiting for it.
use super::*;
use std::io::{self, BufRead};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, sync_channel};
use std::sync::Arc;

const COMMAND_BUDGET: usize = 128;
const ANIMATION_INTERVAL: Duration = Duration::from_millis(16);

#[derive(Debug)]
enum Msg {
    Line(String),
    Wake,
    Eof,
}

#[derive(Clone)]
struct Waker {
    sender: SyncSender<Msg>,
    pending: Arc<AtomicBool>,
}
impl EventLoopWaker for Waker {
    fn clone_box(&self) -> Box<dyn EventLoopWaker> {
        Box::new(self.clone())
    }
    fn wake(&self) {
        if !self.pending.swap(true, Ordering::AcqRel) {
            // A full queue already wakes the loop. Never block a Servo thread on its consumer.
            let _ = self.sender.try_send(Msg::Wake);
        }
    }
}

pub(super) struct EventLoop {
    receiver: Receiver<Msg>,
    waker: Waker,
}

impl EventLoop {
    pub(super) fn new() -> Self {
        let (sender, receiver) = sync_channel(COMMAND_BUDGET);
        let reader = sender.clone();
        std::thread::spawn(move || {
            let mut input = io::stdin().lock();
            loop {
                match read_command(&mut input) {
                    Ok(Command::Line(line)) => {
                        if reader.send(Msg::Line(line)).is_err() { return; }
                    }
                    Ok(Command::Oversized) => continue,
                    Ok(Command::Eof) | Err(_) => break,
                }
            }
            let _ = reader.send(Msg::Eof);
        });
        Self { receiver, waker: Waker { sender, pending: Arc::new(AtomicBool::new(false)) } }
    }

    pub(super) fn waker(&self) -> Box<dyn EventLoopWaker> {
        Box::new(self.waker.clone())
    }

    pub(super) fn run(self, engine: &mut Engine, graphics: &mut graphics::Graphics) {
        let mut animating = false;
        'running: loop {
            // Idle pages have no polling timer. Only animations need a frame deadline.
            let mut first = if animating {
                match self.receiver.recv_timeout(ANIMATION_INTERVAL) {
                    Ok(message) => Some(message),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            } else {
                match self.receiver.recv() {
                    Ok(message) => Some(message),
                    Err(_) => break,
                }
            };
            for _ in 0..COMMAND_BUDGET {
                let Some(message) = first.take().or_else(|| self.receiver.try_recv().ok()) else { break };
                match message {
                    Msg::Line(line) if !engine.command(&line) => break 'running,
                    Msg::Line(_) | Msg::Wake => {},
                    Msg::Eof => break 'running,
                }
            }
            // Rearm AFTER draining and BEFORE spinning: otherwise a drained Wake can leave
            // pending set, suppress the next Servo wake, and strand work when we block again.
            self.waker.pending.store(false, Ordering::Release);
            if let Err(error) = graphics.pump() {
                say(&format!("fatal {}", escape_line(&error)));
                break;
            }
            engine.apply_size();
            engine.servo.spin_event_loop();
            engine.notes();
            engine.present();
            animating = !engine.suspended && engine.visible_view().is_some_and(|view| view.animating());
        }
        // Native controls and queued notes hold WebView references and responder tokens.
        // Drop those before releasing the views themselves; do not present UI during shutdown.
        blocker::cancel_all();
        engine.retire_prompts(None);
        let notes = std::mem::take(&mut *engine.delegate.notes.borrow_mut());
        for note in notes {
            // NavigationRequest::drop allows by default in Servo 0.7; shutdown must deny.
            if let Note::Nav(_, request) = note { request.deny(); }
        }
        engine.delegate.hidden.borrow_mut().clear();
        engine.visible = None;
        engine.views.clear();
    }
}

enum Command {
    Line(String),
    Oversized,
    Eof,
}

/// Read through a complete line while allocating at most MAX_LINE + 1 bytes.
/// Oversized commands are discarded whole, never executed as a truncated JS/program prefix.
fn read_command(input: &mut impl BufRead) -> io::Result<Command> {
    let mut bytes = Vec::new();
    let mut oversized = false;
    loop {
        let available = input.fill_buf()?;
        if available.is_empty() {
            if bytes.is_empty() && !oversized { return Ok(Command::Eof); }
            break;
        }
        let end = available.iter().position(|byte| *byte == b'\n');
        let count = end.unwrap_or(available.len());
        if !oversized {
            let remaining = (guard::MAX_LINE + 1).saturating_sub(bytes.len());
            bytes.reserve_exact(count.min(remaining));
            bytes.extend_from_slice(&available[..count.min(remaining)]);
            oversized = count > remaining;
        }
        input.consume(count + usize::from(end.is_some()));
        if end.is_some() { break; }
    }
    if bytes.last() == Some(&b'\r') { bytes.pop(); }
    if oversized || bytes.len() > guard::MAX_LINE { return Ok(Command::Oversized); }
    String::from_utf8(bytes).map(Command::Line).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wake_burst_is_coalesced_without_blocking_a_full_queue() {
        let (sender, receiver) = sync_channel(1);
        let pending = Arc::new(AtomicBool::new(false));
        let waker = Waker { sender, pending: pending.clone() };
        for _ in 0..1000 { waker.wake(); }
        assert!(matches!(receiver.try_recv(), Ok(Msg::Wake)));
        assert!(receiver.try_recv().is_err());
        pending.store(false, Ordering::Release);
        waker.sender.send(Msg::Line("ping 1".into())).unwrap();
        waker.wake();
        assert!(matches!(receiver.try_recv(), Ok(Msg::Line(_))));
        pending.store(false, Ordering::Release);
        waker.wake();
        assert!(matches!(receiver.try_recv(), Ok(Msg::Wake)));
    }
}
