//! `NotificationCenter` parity: named, multi-subscriber broadcasts (Foundation).
//!
//! `NotificationCenter::default()` is the process-wide center. Observers live on the main thread and run there:
//! `post` from the main thread delivers synchronously (like), from any other thread it is queued on
//! `DispatchQueue::main()`. `object` filters: an observer registered with `Some(id)` only sees posts with that
//! object; `None` sees all. Dropping the `ObserverToken` removes the observer. A closure may post or drop tokens
//! while running (the observer list is snapshotted per post).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::dispatch::{on_main_or_unset, DispatchQueue};

#[derive(Clone, Debug, PartialEq)]
pub struct Notification {
    pub name: String,
    /// The poster (any app-defined id), as `object`.
    pub object: Option<u64>,
    pub user_info: HashMap<String, String>,
}

type Handler = Rc<dyn Fn(&Notification)>;

struct Observer {
    id: u64,
    name: String,
    object: Option<u64>,
    f: Handler,
}

#[derive(Default)]
struct Registry {
    next: u64,
    observers: Vec<Observer>,
}

thread_local! {
    static REGISTRY: RefCell<Registry> = RefCell::new(Registry::default());
}

#[derive(Clone, Copy, Debug)]
pub struct NotificationCenter;

/// Removes its observer when dropped (from any thread).
#[must_use = "dropping the token removes the observer"]
pub struct ObserverToken {
    id: u64,
}

impl Default for NotificationCenter {
    /// The process-wide center (`NotificationCenter.default`).
    fn default() -> Self {
        NotificationCenter
    }
}

impl NotificationCenter {
    /// Observes `name`. Call on the main thread (observers run there).
    pub fn add_observer(&self, name: &str, object: Option<u64>, f: impl Fn(&Notification) + 'static) -> ObserverToken {
        REGISTRY.with(|r| {
            let mut r = r.borrow_mut();
            r.next += 1;
            let id = r.next;
            r.observers.push(Observer { id, name: name.to_string(), object, f: Rc::new(f) });
            ObserverToken { id }
        })
    }

    pub fn post(&self, name: &str, object: Option<u64>, user_info: HashMap<String, String>) {
        let n = Notification { name: name.to_string(), object, user_info };
        // NativeNotificationCenter delivers synchronously on the posting thread; observers live per thread here, so the
        // main thread's observers get a post from another thread through the main queue
        deliver(&n);
        if !on_main_or_unset() {
            DispatchQueue::main().async_(move || deliver(&n));
        }
    }

    pub fn remove_observer(&self, token: ObserverToken) {
        drop(token);
    }

    /// Whether any observer is registered for `name` on this thread (the runtime asks before it repaints for a post).
    pub(crate) fn has_observer(&self, name: &str) -> bool {
        REGISTRY.with(|r| r.borrow().observers.iter().any(|o| o.name == name))
    }
}

fn deliver(n: &Notification) {
    let hit: Vec<Handler> = REGISTRY.with(|r| {
        r.borrow()
            .observers
            .iter()
            .filter(|o| o.name == n.name && (o.object.is_none() || o.object == n.object))
            .map(|o| o.f.clone())
            .collect()
    });
    for f in hit {
        f(n);
    }
}

impl Drop for ObserverToken {
    fn drop(&mut self) {
        let id = self.id;
        let remove = move || REGISTRY.with(|r| r.borrow_mut().observers.retain(|o| o.id != id));
        if on_main_or_unset() {
            remove();
        } else {
            DispatchQueue::main().async_(remove);
        }
    }
}

/// `DistributedNotificationCenter` parity: cross-app broadcast relayed by the shell.
///
/// `post` is process-local (tests, single-app use). `post_with(ctx, ..)` also sends `ToShell::DistPost`, which the
/// shell forwards as `ToApp::DistNotify` to every other running app; the runtime delivers it to observers on the
/// main thread. The sender's own observers are notified synchronously. Names live in their own namespace, apart from
/// [`NotificationCenter`]. Gaps: `user_info` is string to string, capped at 32 entries and 1024 characters a value;
/// no `suspensionBehavior` (an app that is not running misses the post, like `.drop`).
#[derive(Clone, Copy, Debug, Default)]
pub struct DistributedNotificationCenter;

impl DistributedNotificationCenter {
    pub fn add_observer(&self, name: &str, f: impl Fn(&Notification) + 'static) -> ObserverToken {
        NotificationCenter.add_observer(&format!("dist:{name}"), None, f)
    }
    /// Process-local post (no shell).
    pub fn post(&self, name: &str, user_info: HashMap<String, String>) {
        NotificationCenter.post(&format!("dist:{name}"), None, user_info)
    }
    /// Posts to this app's observers and, through the shell, to every other app.
    pub fn post_with(&self, ctx: &crate::Ctx, name: &str, user_info: HashMap<String, String>) {
        let mut info: Vec<(String, String)> = user_info.clone().into_iter().collect();
        info.sort();
        ctx.send_to_shell(crate::proto::ToShell::DistPost { name: name.to_string(), user_info: info });
        self.post(name, user_info);
    }
}

/// A post relayed by the shell from another app.
/// Returns whether an observer heard it (no observer: nothing can have changed, so the app need not repaint).
pub(crate) fn distributed_incoming(name: &str, user_info: HashMap<String, String>) -> bool {
    let heard = NotificationCenter.has_observer(&format!("dist:{name}"));
    DistributedNotificationCenter.post(name, user_info);
    heard
}
