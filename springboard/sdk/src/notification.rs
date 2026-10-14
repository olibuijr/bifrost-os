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
    /// A suspended observer hears no posts until it is resumed.
    suspended: bool,
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

/// Removes its observer when dropped (from any thread). [`ObserverToken::suspend`] and [`ObserverToken::resume`]
/// pause and restart delivery without removing it.
#[must_use = "dropping the token removes the observer"]
pub struct ObserverToken {
    id: u64,
}

impl ObserverToken {
    /// Stops delivery to the observer until [`ObserverToken::resume`]. Posts made meanwhile are not kept.
    pub fn suspend(&self) {
        self.set_suspended(true);
    }

    /// Restarts delivery to a suspended observer.
    pub fn resume(&self) {
        self.set_suspended(false);
    }

    fn set_suspended(&self, on: bool) {
        let id = self.id;
        REGISTRY.with(|r| {
            if let Some(o) = r.borrow_mut().observers.iter_mut().find(|o| o.id == id) {
                o.suspended = on;
            }
        });
    }
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
            r.observers.push(Observer { id, name: name.to_string(), object, suspended: false, f: Rc::new(f) });
            ObserverToken { id }
        })
    }

    pub fn post(&self, name: &str, object: Option<u64>, user_info: HashMap<String, String>) {
        let n = Notification { name: name.to_string(), object, user_info };
        // NativeNotificationCenter delivers synchronously on the posting thread; observers live per thread here, so the
        // main thread's observers get a post from another thread through the main queue
        deliver(&n);
        if !on_main_or_unset() {
            // only the main thread's observers: a main queue drained elsewhere (tests) must not deliver twice here
            DispatchQueue::main().async_(move || {
                if crate::dispatch::on_main_thread() {
                    deliver(&n);
                }
            });
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
            .filter(|o| !o.suspended && o.name == n.name && (o.object.is_none() || o.object == n.object))
            .map(|o| o.f.clone())
            .collect()
    });
    for f in hit {
        f(n);
    }
}

impl Drop for ObserverToken {
    fn drop(&mut self) {
        // observers live in the registry of the thread that added them, which is the thread the token belongs to:
        // removing through the main queue would touch the main thread's registry and leave this one observing
        let id = self.id;
        REGISTRY.with(|r| r.borrow_mut().observers.retain(|o| o.id != id));
    }
}

/// `DistributedNotificationCenter` parity: cross-app broadcast relayed by the shell.
///
/// `post` is process-local (tests, single-app use). `post_with(ctx, ..)` also sends `ToShell::DistPost`, which the
/// shell forwards as `ToApp::DistNotify` to every other running app; the runtime delivers it to observers on the
/// main thread. The sender's own observers are notified synchronously, with the same payload the other apps get (see
/// [`distributed_user_info`]). Names live in their own namespace, apart from [`NotificationCenter`]. Gaps: no
/// `suspensionBehavior` (an app that is not running misses the post, like `.drop`).
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
        let info = distributed_user_info(&user_info);
        ctx.send_to_shell(crate::proto::ToShell::DistPost { name: name.to_string(), user_info: info.clone() });
        self.post(name, info.into_iter().collect());
    }
}

/// Most entries a distributed post carries in its `user_info`.
pub const MAX_USER_INFO_ENTRIES: usize = 32;
/// Longest value, in characters, a distributed post carries in its `user_info`. A longer value is cut to this length.
pub const MAX_USER_INFO_VALUE_CHARS: usize = 1024;

/// The `user_info` a distributed post carries, sorted by key. Values are cut to [`MAX_USER_INFO_VALUE_CHARS`] characters
/// and only the first [`MAX_USER_INFO_ENTRIES`] keys (in sorted order) are kept, so every app sees the same payload.
pub fn distributed_user_info(user_info: &HashMap<String, String>) -> Vec<(String, String)> {
    let mut info: Vec<(String, String)> =
        user_info.iter().map(|(k, v)| (k.clone(), v.chars().take(MAX_USER_INFO_VALUE_CHARS).collect())).collect();
    info.sort();
    info.truncate(MAX_USER_INFO_ENTRIES);
    info
}

/// A post relayed by the shell from another app.
/// Returns whether an observer heard it (no observer: nothing can have changed, so the app need not repaint).
pub(crate) fn distributed_incoming(name: &str, user_info: HashMap<String, String>) -> bool {
    let heard = NotificationCenter.has_observer(&format!("dist:{name}"));
    DistributedNotificationCenter.post(name, user_info);
    heard
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn info(k: &str, v: &str) -> HashMap<String, String> {
        HashMap::from([(k.to_string(), v.to_string())])
    }

    #[test]
    fn a_post_reaches_every_observer_of_the_name_with_its_payload() {
        let seen: Rc<RefCell<Vec<Notification>>> = Rc::default();
        let (a, b) = (seen.clone(), seen.clone());
        let _t1 = NotificationCenter.add_observer("nc.test.payload", None, move |n| a.borrow_mut().push(n.clone()));
        let _t2 = NotificationCenter.add_observer("nc.test.payload", None, move |n| b.borrow_mut().push(n.clone()));
        NotificationCenter.post("nc.test.payload", Some(7), info("k", "v"));
        let got = seen.borrow();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0], Notification { name: "nc.test.payload".into(), object: Some(7), user_info: info("k", "v") });
    }

    #[test]
    fn observers_only_hear_their_own_name() {
        let hits = Rc::new(Cell::new(0));
        let h = hits.clone();
        let _t = NotificationCenter.add_observer("nc.test.name.a", None, move |_| h.set(h.get() + 1));
        NotificationCenter.post("nc.test.name.b", None, HashMap::new());
        assert_eq!(hits.get(), 0);
        NotificationCenter.post("nc.test.name.a", None, HashMap::new());
        assert_eq!(hits.get(), 1);
    }

    #[test]
    fn the_object_filter_matches_only_that_poster_and_none_matches_all() {
        let filtered = Rc::new(Cell::new(0));
        let any = Rc::new(Cell::new(0));
        let (f, a) = (filtered.clone(), any.clone());
        let _t1 = NotificationCenter.add_observer("nc.test.obj", Some(1), move |_| f.set(f.get() + 1));
        let _t2 = NotificationCenter.add_observer("nc.test.obj", None, move |_| a.set(a.get() + 1));
        NotificationCenter.post("nc.test.obj", Some(1), HashMap::new());
        NotificationCenter.post("nc.test.obj", Some(2), HashMap::new());
        NotificationCenter.post("nc.test.obj", None, HashMap::new());
        assert_eq!(filtered.get(), 1);
        assert_eq!(any.get(), 3);
    }

    #[test]
    fn dropping_or_removing_a_token_stops_delivery() {
        let hits = Rc::new(Cell::new(0));
        let h = hits.clone();
        let token = NotificationCenter.add_observer("nc.test.remove", None, move |_| h.set(h.get() + 1));
        assert!(NotificationCenter.has_observer("nc.test.remove"));
        NotificationCenter.post("nc.test.remove", None, HashMap::new());
        NotificationCenter.remove_observer(token);
        NotificationCenter.post("nc.test.remove", None, HashMap::new());
        assert_eq!(hits.get(), 1);
        assert!(!NotificationCenter.has_observer("nc.test.remove"));
    }

    #[test]
    fn an_observer_may_post_another_notification_while_running() {
        let hits = Rc::new(Cell::new(0));
        let h = hits.clone();
        let _inner = NotificationCenter.add_observer("nc.test.inner", None, move |_| h.set(h.get() + 1));
        let _outer = NotificationCenter.add_observer("nc.test.outer", None, |_| {
            NotificationCenter.post("nc.test.inner", None, HashMap::new());
        });
        NotificationCenter.post("nc.test.outer", None, HashMap::new());
        assert_eq!(hits.get(), 1);
    }

    #[test]
    fn distributed_names_are_a_separate_namespace() {
        let local = Rc::new(Cell::new(0));
        let dist = Rc::new(Cell::new(0));
        let (l, d) = (local.clone(), dist.clone());
        let _t1 = NotificationCenter.add_observer("nc.test.shared", None, move |_| l.set(l.get() + 1));
        let _t2 = DistributedNotificationCenter.add_observer("nc.test.shared", move |_| d.set(d.get() + 1));
        DistributedNotificationCenter.post("nc.test.shared", info("a", "b"));
        assert_eq!((local.get(), dist.get()), (0, 1));
        NotificationCenter.post("nc.test.shared", None, HashMap::new());
        assert_eq!((local.get(), dist.get()), (1, 1));
    }

    #[test]
    fn an_incoming_distributed_post_reports_whether_anyone_heard_it() {
        assert!(!distributed_incoming("nc.test.incoming", HashMap::new()));
        let got = Rc::new(RefCell::new(None));
        let g = got.clone();
        let _t = DistributedNotificationCenter.add_observer("nc.test.incoming", move |n| *g.borrow_mut() = Some(n.user_info.clone()));
        assert!(distributed_incoming("nc.test.incoming", info("x", "1")));
        assert_eq!(got.borrow().clone(), Some(info("x", "1")));
    }
}
