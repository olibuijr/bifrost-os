//! The workspace (`NativeWorkspace`): open files and links with the user's default app, reveal in Files, move to the
//! Trash, ask about apps, and hear about apps launching, quitting and becoming active, and about sleep and wake.
//!
//! Differences from the native API that follow from the sandbox (docs/bifrost-ui.md "Workspace"):
//! - A file is a document grant ([`Item`]) the app already holds (open or save panel, drag, share), never a path. The
//!   shell resolves the grant and refuses one the app does not own; `recycle` needs a write grant.
//! - Nothing here launches a binary. `open_with` names an installed app by id; links open only as web URLs or in an app
//!   that declares the scheme.
//! - Answers arrive in a completion closure on the main thread, like `NativeWorkspace.open(_:configuration:completionHandler:)`.
//!
//! Notifications are posted on the process-wide [`NotificationCenter`] under the `DID_LAUNCH_APPLICATION` family of names
//! after [`observe`] subscribed; `user_info` holds `id` (the app id) and `name` for the three application events.

use crate::notification::{Notification, NotificationCenter, ObserverToken};
use crate::proto::{GrantId, ToShell, WorkspaceEventKind, WorkspaceOp};
use crate::Ctx;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;

pub use crate::proto::WorkspaceStatus as Status;

/// A file the app holds a document grant for.
pub type Item = GrantId;

pub const DID_LAUNCH_APPLICATION: &str = "workspace.didLaunchApplication";
pub const DID_TERMINATE_APPLICATION: &str = "workspace.didTerminateApplication";
pub const DID_ACTIVATE_APPLICATION: &str = "workspace.didActivateApplication";
pub const WILL_SLEEP: &str = "workspace.willSleep";
pub const DID_WAKE: &str = "workspace.didWake";
pub const SCREENS_DID_SLEEP: &str = "workspace.screensDidSleep";
pub const SCREENS_DID_WAKE: &str = "workspace.screensDidWake";

/// The notification name of an event kind.
pub fn notification_name(kind: WorkspaceEventKind) -> &'static str {
    match kind {
        WorkspaceEventKind::DidLaunchApplication => DID_LAUNCH_APPLICATION,
        WorkspaceEventKind::DidTerminateApplication => DID_TERMINATE_APPLICATION,
        WorkspaceEventKind::DidActivateApplication => DID_ACTIVATE_APPLICATION,
        WorkspaceEventKind::WillSleep => WILL_SLEEP,
        WorkspaceEventKind::DidWake => DID_WAKE,
        WorkspaceEventKind::ScreensDidSleep => SCREENS_DID_SLEEP,
        WorkspaceEventKind::ScreensDidWake => SCREENS_DID_WAKE,
    }
}

type Done = Box<dyn FnOnce(Status, Vec<String>)>;

thread_local! {
    static PENDING: RefCell<HashMap<u32, Done>> = RefCell::new(HashMap::new());
    static MASK: Cell<u32> = const { Cell::new(0) };
}

fn ask(ctx: &Ctx, op: WorkspaceOp, grants: &[Item], arg: &str, done: impl FnOnce(Status, Vec<String>) + 'static) -> u32 {
    let request = ctx.next_request();
    PENDING.with(|p| p.borrow_mut().insert(request, Box::new(done)));
    ctx.send_to_shell(ToShell::WorkspaceRequest { request, op, grants: grants.to_vec(), arg: arg.to_string() });
    request
}

fn ok(status: Status) -> bool {
    status == Status::Ok
}

/// `open(_ url:)` for a link: a web URL opens in the default browser, another scheme in the app that declares it.
/// `done(true)` when something opened it.
pub fn open(ctx: &Ctx, url: &str, done: impl FnOnce(bool) + 'static) -> u32 {
    ask(ctx, WorkspaceOp::OpenUrl, &[], url, move |s, _| done(ok(s)))
}

/// `open(_ url:)` for a file: opens it with the default app for its type (Default Apps in Settings). `done(true)` when
/// an app took it.
pub fn open_file(ctx: &Ctx, item: Item, done: impl FnOnce(bool) + 'static) -> u32 {
    ask(ctx, WorkspaceOp::OpenFile, &[item], "", move |s, _| done(ok(s)))
}

/// `open(_ urls:withApplicationAt:configuration:completionHandler:)`: opens the files with the installed app `app_id`.
pub fn open_with(ctx: &Ctx, items: &[Item], app_id: &str, done: impl FnOnce(Status) + 'static) -> u32 {
    ask(ctx, WorkspaceOp::OpenWith, items, app_id, move |s, _| done(s))
}

/// `activateFileViewerSelecting(_:)`: shows the files in Files.
pub fn activate_file_viewer_selecting(ctx: &Ctx, items: &[Item], done: impl FnOnce(Status) + 'static) -> u32 {
    ask(ctx, WorkspaceOp::Reveal, items, "", move |s, _| done(s))
}

/// `recycle(_:completionHandler:)`: moves the files to Recently Deleted, the folder Files uses (Put Back works). The
/// grants need write access. `done` gets the status and how many items were moved.
pub fn recycle(ctx: &Ctx, items: &[Item], done: impl FnOnce(Status, usize) + 'static) -> u32 {
    ask(ctx, WorkspaceOp::Recycle, items, "", move |s, moved| done(s, moved.len()))
}

/// `urlForApplication(withBundleIdentifier:)`: the id of the installed app, `None` when it is not installed.
pub fn url_for_application(ctx: &Ctx, app_id: &str, done: impl FnOnce(Option<String>) + 'static) -> u32 {
    ask(ctx, WorkspaceOp::UrlForApp, &[], app_id, move |s, items| done(if ok(s) { items.into_iter().next() } else { None }))
}

/// `frontmostApplication`: the id of the app in front, `None` when the desktop is.
pub fn frontmost_application(ctx: &Ctx, done: impl FnOnce(Option<String>) + 'static) -> u32 {
    ask(ctx, WorkspaceOp::Frontmost, &[], "", move |s, items| done(if ok(s) { items.into_iter().next() } else { None }))
}

/// `runningApplications`: the ids of the running apps.
pub fn running_applications(ctx: &Ctx, done: impl FnOnce(Vec<String>) + 'static) -> u32 {
    ask(ctx, WorkspaceOp::Running, &[], "", move |s, items| done(if ok(s) { items } else { Vec::new() }))
}

/// `icon(for:)` by content type: the name of the glyph that stands for the type (`photo`, `film`, `music.note`,
/// `doc.richtext`, `doc.text`, `archivebox`, `folder`, else `doc`). Gap: names, not images; the app draws them with its kit.
pub fn icon_for_content_type(uttype: &str) -> &'static str {
    use bifrost_ui::uttype::conforms;
    const TABLE: &[(&str, &str)] = &[
        ("public.folder", "folder"),
        ("public.image", "photo"),
        ("public.movie", "film"),
        ("public.audio", "music.note"),
        ("com.adobe.pdf", "doc.richtext"),
        ("public.archive", "archivebox"),
        ("public.text", "doc.text"),
    ];
    TABLE.iter().find(|(t, _)| conforms(uttype, t)).map_or("doc", |(_, g)| g)
}

/// `icon(forFile:)` for a file name (the app knows the name of a granted file from the panel); a name without a known
/// extension gets the plain document glyph.
pub fn icon_for_file(name: &str) -> &'static str {
    bifrost_ui::uttype::type_for_extension(name).map_or("doc", icon_for_content_type)
}

/// `notificationCenter.addObserver(forName:)` for a workspace event. Subscribes the app with the shell (once per kind);
/// the subscription lasts as long as the process, the token only removes this observer.
pub fn observe(ctx: &Ctx, kind: WorkspaceEventKind, f: impl Fn(&Notification) + 'static) -> ObserverToken {
    let now = MASK.with(|m| {
        let next = m.get() | kind.bit();
        m.set(next);
        next
    });
    SENT.with(|s| {
        if s.get() != now {
            s.set(now);
            ctx.send_to_shell(ToShell::WorkspaceSubscribe { mask: now });
        }
    });
    NotificationCenter.add_observer(notification_name(kind), None, f)
}

thread_local! {
    static SENT: Cell<u32> = const { Cell::new(0) };
}

/// The shell answered a request (the runtime calls this on the main thread).
pub(crate) fn reply_incoming(request: u32, status: Status, items: Vec<String>) {
    if let Some(done) = PENDING.with(|p| p.borrow_mut().remove(&request)) {
        done(status, items);
    }
}

/// The shell reported a workspace event.
pub(crate) fn event_incoming(kind: WorkspaceEventKind, app_id: String, name: String) {
    let mut info = HashMap::new();
    if kind.is_application() {
        info.insert("id".to_string(), app_id);
        info.insert("name".to_string(), name);
    }
    NotificationCenter.post(notification_name(kind), None, info);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::rc::Rc;

    fn reset() {
        PENDING.with(|p| p.borrow_mut().clear());
        MASK.with(|m| m.set(0));
        SENT.with(|m| m.set(0));
    }

    #[test]
    fn open_uses_the_default_app_for_the_type() {
        reset();
        let ctx = crate::runtime::test_ctx();
        let seen = Rc::new(Cell::new(None));
        let s = seen.clone();
        let g: Item = [5; 16];
        let r = open_file(&ctx, g, move |opened| s.set(Some(opened)));
        // the SDK names the file by its grant and leaves the choice of app to the shell (Default Apps)
        assert_eq!(ctx.take_sent(), [ToShell::WorkspaceRequest { request: r, op: WorkspaceOp::OpenFile, grants: vec![g], arg: String::new() }]);
        reply_incoming(r, Status::Ok, vec![]);
        assert_eq!(seen.get(), Some(true));
    }

    #[test]
    fn a_refused_open_reports_false_and_a_reply_runs_once() {
        reset();
        let ctx = crate::runtime::test_ctx();
        let seen = Rc::new(Cell::new(0));
        let s = seen.clone();
        let r = open(&ctx, "https://example.org", move |opened| {
            assert!(!opened);
            s.set(s.get() + 1)
        });
        reply_incoming(r, Status::Denied, vec![]);
        reply_incoming(r, Status::Denied, vec![]);
        assert_eq!(seen.get(), 1);
    }

    #[test]
    fn recycle_moves_items_to_the_trash() {
        reset();
        let ctx = crate::runtime::test_ctx();
        let moved = Rc::new(Cell::new(usize::MAX));
        let m = moved.clone();
        let r = recycle(&ctx, &[[1; 16], [2; 16]], move |s, n| {
            assert_eq!(s, Status::Ok);
            m.set(n)
        });
        assert!(matches!(&ctx.take_sent()[..], [ToShell::WorkspaceRequest { op: WorkspaceOp::Recycle, grants, .. }] if grants.len() == 2));
        reply_incoming(r, Status::Ok, vec!["a".into(), "b".into()]);
        assert_eq!(moved.get(), 2);
    }

    #[test]
    fn apps_hear_did_launch_and_did_terminate() {
        reset();
        let ctx = crate::runtime::test_ctx();
        let log = Rc::new(RefCell::new(Vec::new()));
        let (a, b) = (log.clone(), log.clone());
        let _t1 = observe(&ctx, WorkspaceEventKind::DidLaunchApplication, move |n| a.borrow_mut().push(("launch", n.user_info["id"].clone())));
        let _t2 = observe(&ctx, WorkspaceEventKind::DidTerminateApplication, move |n| b.borrow_mut().push(("terminate", n.user_info["id"].clone())));
        // one subscription per change of the set
        let sent = ctx.take_sent();
        assert_eq!(sent.last(), Some(&ToShell::WorkspaceSubscribe { mask: WorkspaceEventKind::DidLaunchApplication.bit() | WorkspaceEventKind::DidTerminateApplication.bit() }));
        event_incoming(WorkspaceEventKind::DidLaunchApplication, "is.test.app".into(), "Test".into());
        event_incoming(WorkspaceEventKind::DidTerminateApplication, "is.test.app".into(), "Test".into());
        event_incoming(WorkspaceEventKind::DidWake, String::new(), String::new());
        assert_eq!(*log.borrow(), [("launch", "is.test.app".to_string()), ("terminate", "is.test.app".to_string())]);
    }

    #[test]
    fn icons_follow_the_type_tree() {
        assert_eq!(icon_for_file("photo.PNG"), "photo");
        assert_eq!(icon_for_file("notes.txt"), "doc.text");
        assert_eq!(icon_for_file("noextension"), "doc");
        assert_eq!(icon_for_content_type("public.folder"), "folder");
    }
}
