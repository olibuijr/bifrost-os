//! Background work for the window: the sign-in, the sync loop and the sends. Each job runs on its own thread and sends
//! one [`Event`] back; a byte on the wake pipe tells the window's loop to read the channel, so nothing waits on a network
//! call. The sync loop runs for as long as the app does.

use crate::api::Api;
use crate::protocol::Sync;
use springboard::deildu::http::Transport;
use std::io::Write;
use std::os::unix::net::UnixStream;
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Duration;

/// How long the sync loop waits after a failed call before it tries again.
const RETRY: Duration = Duration::from_secs(5);

/// The answer of one job.
pub enum Event {
    /// A sign-in finished. `typed` is true when the user typed the password (so it is saved on success), false when it
    /// came from Passwords.
    SignedIn { homeserver: String, user: String, password: String, typed: bool, result: Result<Arc<Api>, String> },
    /// One sync answer, or the error of a failed one.
    Synced(Result<Sync, String>),
    /// A message was sent to `room`.
    Sent { room: String, result: Result<(), String> },
    /// A read receipt was sent.
    Read { result: Result<(), String> },
    /// A reaction, edit or removal finished. `what` names it for the error line ("React", "Edit", "Undo Send").
    Changed { what: &'static str, result: Result<(), String> },
    /// A new conversation was made: the room id, or why not.
    Created { result: Result<String, String> },
    /// The account left `room`.
    Left { room: String, result: Result<(), String> },
    /// The room was renamed to `name`.
    Renamed { room: String, name: String, result: Result<(), String> },
    /// The session ended on the homeserver (the local sign-out has already happened).
    LoggedOut { result: Result<(), String> },
}

/// Runs `work` on a new thread and sends its event, then writes the wake pipe.
pub fn spawn(tx: Sender<Event>, wake: Option<UnixStream>, work: impl FnOnce() -> Event + Send + 'static) {
    std::thread::spawn(move || {
        let event = work();
        let mut wake = wake;
        notify(&tx, &mut wake, event);
    });
}

/// Sends `event` and wakes the window. False when the window has gone and nobody reads the channel.
fn notify(tx: &Sender<Event>, wake: &mut Option<UnixStream>, event: Event) -> bool {
    if tx.send(event).is_err() {
        return false;
    }
    if let Some(w) = wake.as_mut() {
        let _ = w.write(&[1]);
    }
    true
}

/// Signs in to `homeserver` as `user`. The result carries the session, or the error the server gave.
pub fn sign_in(transport: Arc<dyn Transport>, homeserver: String, user: String, password: String, typed: bool) -> Event {
    let result = Api::login(transport, &homeserver, &user, &password).map(Arc::new);
    Event::SignedIn { homeserver, user, password, typed, result }
}

/// Starts the sync loop for `api` on its own thread. Every answer is sent to the window. A failed call is sent too, and
/// the loop waits before it tries again. An expired or revoked token ends the loop: the user must sign in again.
pub fn run_sync(api: Arc<Api>, tx: Sender<Event>, wake: Option<UnixStream>) {
    std::thread::spawn(move || {
        let mut wake = wake;
        let mut since: Option<String> = None;
        loop {
            match api.sync(since.as_deref()) {
                Ok(sync) => {
                    since = Some(sync.next_batch.clone());
                    if !notify(&tx, &mut wake, Event::Synced(Ok(sync))) {
                        return;
                    }
                }
                Err(e) => {
                    let revoked = e.starts_with("M_UNKNOWN_TOKEN") || e.starts_with("M_FORBIDDEN");
                    if !notify(&tx, &mut wake, Event::Synced(Err(e))) || revoked {
                        return;
                    }
                    std::thread::sleep(RETRY);
                }
            }
        }
    });
}

/// Sends a text message to `room`.
pub fn send_text(api: Arc<Api>, room: String, text: String) -> Event {
    let result = api.send_text(&room, &text);
    Event::Sent { room, result }
}

/// Uploads a picture and sends it to `room`.
pub fn send_image(api: Arc<Api>, room: String, name: String, mime: &'static str, bytes: Vec<u8>) -> Event {
    let result = api.send_image(&room, &name, mime, &bytes);
    Event::Sent { room, result }
}

/// Marks `event` as read in `room`.
pub fn read(api: Arc<Api>, room: String, event: String) -> Event {
    Event::Read { result: api.read_receipt(&room, &event) }
}

/// Uploads any file and sends it to `room`.
pub fn send_file(api: Arc<Api>, room: String, name: String, mime: &'static str, bytes: Vec<u8>) -> Event {
    let result = api.send_file(&room, &name, mime, &bytes);
    Event::Sent { room, result }
}

/// Sends a text that answers `reply_to`.
pub fn send_reply(api: Arc<Api>, room: String, text: String, reply_to: String) -> Event {
    let result = api.send_reply(&room, &text, &reply_to);
    Event::Sent { room, result }
}

/// Replaces the words of `target`.
pub fn send_edit(api: Arc<Api>, room: String, target: String, text: String) -> Event {
    Event::Changed { what: "Edit", result: api.send_edit(&room, &target, &text) }
}

/// Reacts to `target` with `key`.
pub fn send_reaction(api: Arc<Api>, room: String, target: String, key: String) -> Event {
    Event::Changed { what: "React", result: api.send_reaction(&room, &target, &key) }
}

/// Removes `event` from `room`.
pub fn redact(api: Arc<Api>, room: String, event: String, what: &'static str) -> Event {
    Event::Changed { what, result: api.redact(&room, &event) }
}

/// Makes a direct conversation with `user_id`.
pub fn create_direct(api: Arc<Api>, user_id: String) -> Event {
    Event::Created { result: api.create_direct(&user_id) }
}

/// Leaves and forgets `room`.
pub fn leave(api: Arc<Api>, room: String) -> Event {
    let result = api.leave(&room);
    Event::Left { room, result }
}

/// Renames `room`.
pub fn rename(api: Arc<Api>, room: String, name: String) -> Event {
    let result = api.rename_room(&room, &name);
    Event::Renamed { room, name, result }
}

/// Ends the session on the homeserver.
pub fn logout(api: Arc<Api>) -> Event {
    Event::LoggedOut { result: api.logout() }
}
