//! Mail as an installed BifrOSt desktop app: hosts `window::MailWindow` through the SDK. The main window shows the
//! mailboxes and messages; viewer windows (`viewer`) and compose windows (`compose_windows`) are windows of their own.

use bifrost_app::app_windows::WindowId;
use bifrost_app::kit::{Damage, Surface};
use bifrost_app::background_tasks::{self, TaskEvent};
use bifrost_app::{passwords, usernotif};
use bifrost_app::pointer_events::PointerEvent;
use bifrost_app::proto::WindowInput;
use bifrost_app::{App, AppMenu, Command, Commands, Ctx, DocumentEvent, Input, TouchKind};
use bifrost_mail::background::{self, Check};
use bifrost_mail::compose_windows::Composes;
use bifrost_mail::index;
use bifrost_mail::viewer::Viewer;
use bifrost_mail::window::MailWindow;

/// How often the app compares the mailbox on screen with what it indexed (ms).
const INDEX_EVERY_MS: f64 = 3000.0;

struct MailApp {
    window: Option<MailWindow>,
    /// Viewer windows: a message in a window of its own (double-click, File > Open Message).
    viewers: Vec<Viewer>,
    /// Compose windows, and the one whose compose is in the main window's sheet slot.
    composes: Composes,
    /// A `mailto:` link that arrived before the window existed; it opens once the window is made.
    pending_url: Option<String>,
    /// The messages Core Search holds, by id, with the subject and sender each was indexed with (see `index.rs`).
    indexed: std::collections::HashMap<String, String>,
    /// The time of the next comparison with the index.
    next_index: f64,
    /// The mail check a background task started (see `background.rs`), while it runs.
    check: Option<Check>,
    /// The next refresh task has been asked for in this run.
    scheduled: bool,
    /// The connections the background check opens.
    transport: std::sync::Arc<dyn springboard::deildu::http::Transport>,
    /// The main window's title as last told to the shell (`draw::main_title`).
    title: String,
}

impl MailApp {
    /// Opens a viewer window for each message the main window asked for. True when a window was opened.
    fn open_viewers(&mut self, ctx: &Ctx) -> bool {
        let Some(mail) = &mut self.window else { return false };
        let snapshots = mail.take_viewers();
        let mut opened = false;
        for snapshot in snapshots {
            match ctx.open_window(snapshot.spec()) {
                Ok(id) => {
                    self.viewers.push(Viewer::new(id, snapshot));
                    opened = true;
                }
                Err(e) => mail.mail.status = e.to_string(),
            }
        }
        opened
    }

    /// Copies a body into the viewers that were opened before it was fetched. True when one changed.
    fn refresh_viewers(&mut self) -> bool {
        let Some(mail) = &self.window else { return false };
        let mut changed = false;
        for viewer in self.viewers.iter_mut().filter(|v| !v.snapshot.loaded) {
            let snapshot = mail.viewer_snapshot(viewer.snapshot.uid);
            if snapshot.loaded {
                viewer.snapshot = snapshot;
                changed = true;
            }
        }
        changed
    }

    /// Runs one call on the main window, then moves compose windows into line with its sheet. True when a window
    /// changed and the app needs a frame.
    fn with_window(&mut self, ctx: &Ctx, f: impl FnOnce(&mut MailWindow) -> bool) -> bool {
        let changed = match &mut self.window {
            Some(mail) => f(mail),
            None => false,
        };
        self.sync(ctx) | changed
    }

    fn sync(&mut self, ctx: &Ctx) -> bool {
        match &mut self.window {
            Some(mail) => {
                // the main window is titled after the mailbox it shows, so the shell's Window menu lists it by that name
                let title = bifrost_mail::draw::main_title(&mail.mail);
                if self.title != title {
                    self.title = title.to_string();
                    ctx.set_document_state(&self.title, false, "");
                }
                self.composes.sync(ctx, mail)
            }
            None => false,
        }
    }
}

impl App for MailApp {
    fn id(&self) -> &str {
        "is.olibuijr.mail.desktop"
    }

    fn configure(&mut self, ctx: &Ctx) {
        self.schedule(ctx);
        let (w, h) = (ctx.cfg.w, ctx.cfg.h);
        self.composes.main_size = (w, h);
        if self.window.is_none() {
            let mut mail = MailWindow::new(ctx);
            mail.set_size(w, h);
            self.window = Some(mail);
            ctx.wants_keyboard(true);
            if let (Some(url), Some(mail)) = (self.pending_url.take(), &mut self.window) {
                mail.open_handoff(&url);
            }
            self.sync(ctx);
        } else if self.composes.active.is_none() {
            // while a compose window is active, the main window's size is the compose window's
            if let Some(mail) = &mut self.window {
                mail.set_size(w, h);
            }
        }
    }

    fn commands(&self, _ctx: &Ctx) -> Commands {
        // the fields of a sheet take the standard editing commands; the keyboard sends them as commands when the app says it takes them
        // each one enabled only while it can act (HIG: disable what cannot)
        let (select_all, copy, cut, paste) = self.window.as_ref().map_or((false, false, false, false), |mail| mail.edit_enabled());
        Commands::default().with(Command::Paste, paste).with(Command::SelectAll, select_all).with(Command::Copy, copy).with(Command::Cut, cut)
    }

    fn main_menu(&self, _ctx: &Ctx) -> Vec<AppMenu> {
        self.window.as_ref().map_or_else(Vec::new, |mail| mail.menu_items())
    }

    fn menu_action(&mut self, ctx: &Ctx, id: u32) -> bool {
        self.route_focus(ctx);
        self.with_window(ctx, |mail| mail.menu(ctx, id))
    }


    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        if matches!(ev, Input::Key(_) | Input::Command(_)) {
            self.route_focus(ctx);
        }
        let changed = match &mut self.window {
            None => {
                // a mailto: link before the window exists waits for configure
                if let Input::Open(url) = ev {
                    self.pending_url = Some(url);
                }
                false
            }
            Some(mail) => match ev {
                Input::Touch { kind, x, y, t, mods, .. } => {
                    // a touch on the main window hands the sheet slot back to the main window
                    if matches!(kind, TouchKind::Down) && !self.composes.deactivate(mail) {
                        return false;
                    }
                    mail.touch(ctx, kind, x, y, t, mods)
                }
                Input::Key(_) | Input::Command(_) => key_or_command(mail, ctx, ev),
                // a right-click on the main window, and the item chosen from its context menu: both act on the main
                // window's message, so a compose window gives the sheet slot back first
                Input::Context { x, y } => self.composes.deactivate(mail) && mail.context(ctx, x, y),
                Input::ContextAction(id) => self.composes.deactivate(mail) && mail.menu(ctx, id),
                Input::Open(url) => {
                    mail.open_handoff(&url);
                    true
                }
                _ => false,
            },
        };
        self.sync(ctx) | changed
    }

    fn window_event(&mut self, ctx: &Ctx, window: WindowId, ev: Input) -> bool {
        if window.is_main() {
            return self.event(ctx, ev);
        }
        // an extra window's touches arrive in window points (`WindowTouch`); the viewer and the compose sheet are laid
        // out and hit-tested in the pixels they are drawn in
        let k = ctx.ui.pt(1.0);
        if let Some(viewer) = self.viewers.iter_mut().find(|v| v.id == window) {
            return match ev {
                Input::Touch { kind, y, t, .. } => viewer.touch(kind, y, k, t),
                _ => false,
            };
        }
        let Some(mail) = &mut self.window else { return false };
        if !self.composes.owns(window) {
            return false;
        }
        let changed = match ev {
            Input::Touch { kind, x, y, t, mods, .. } => {
                if matches!(kind, TouchKind::Down) && !self.composes.activate(mail, window) {
                    return false;
                }
                if self.composes.active != Some(window) {
                    return false;
                }
                mail.touch(ctx, kind, x * k, y * k, t, mods)
            }
            // the compose window is the key window: its keys and editing commands go to its message (the shell sends
            // them tagged with the window, not to the main window)
            Input::Key(_) | Input::Command(_) => {
                if !self.composes.activate(mail, window) {
                    return false;
                }
                key_or_command(mail, ctx, ev)
            }
            _ => false,
        };
        self.sync(ctx) | changed
    }

    fn window_input(&mut self, ctx: &Ctx, window: WindowId, input: WindowInput) -> bool {
        match input {
            // a menu item chosen while a compose window is key (Send, Format, Attach) acts on that window's message
            WindowInput::MenuAction(id) if self.composes.owns(window) => {
                if let Some(mail) = &mut self.window {
                    self.composes.activate(mail, window);
                }
                self.with_window(ctx, |mail| mail.menu(ctx, id))
            }
            other => bifrost_app::app_windows::default_window_input(self, ctx, window, other),
        }
    }

    /// The scroll wheel, in place of the drag the shell would emulate for it in every window of the app.
    fn pointer_interest(&self) -> u32 {
        bifrost_app::pointer_events::WHEEL
    }

    /// The main window's wheel (surface pixels): the list or the reader under the pointer scrolls. A compose window
    /// that holds the sheet slot gives it back first, so the main window's own panes are the ones that move.
    fn pointer(&mut self, ctx: &Ctx, ev: &PointerEvent) -> bool {
        let PointerEvent::Wheel { x, y, dy, .. } = *ev else { return false };
        let Some(mail) = &mut self.window else { return false };
        if !self.composes.deactivate(mail) {
            return false;
        }
        mail.wheel(&ctx.ui, x, y, dy)
    }

    /// An extra window's wheel (window points): a viewer scrolls its text, a compose window its message.
    fn window_pointer(&mut self, ctx: &Ctx, window: WindowId, ev: &PointerEvent) -> bool {
        if window.is_main() {
            return self.pointer(ctx, ev);
        }
        let PointerEvent::Wheel { dy, .. } = *ev else { return false };
        let k = ctx.ui.pt(1.0);
        if let Some(viewer) = self.viewers.iter_mut().find(|v| v.id == window) {
            return viewer.wheel(dy, k);
        }
        let Some(mail) = &mut self.window else { return false };
        self.composes.wheel(&ctx.ui, mail, window, dy * k)
    }

    fn service_reply(&mut self, ctx: &Ctx, request: u32, result: &Result<bifrost_core::json::Json, String>) -> bool {
        if self.check.as_mut().is_some_and(|c| c.password(request, result)) {
            return false;
        }
        self.with_window(ctx, |mail| mail.service_reply(ctx, request, result))
    }

    fn service_event(&mut self, ctx: &Ctx, service: &str, event: &str, body: &bifrost_core::json::Json) -> bool {
        match background_tasks::task_event(service, event, body) {
            Some(TaskEvent::Started { identifier, .. }) if identifier == background::TASK => {
                self.start_check(ctx, &identifier, background::accounts_to_check());
                false
            }
            Some(TaskEvent::Expiring { identifier }) if identifier == background::TASK => {
                self.finish_check(ctx);
                false
            }
            _ => false,
        }
    }

    fn document(&mut self, ctx: &Ctx, ev: DocumentEvent) -> bool {
        self.with_window(ctx, |mail| mail.document(ctx, ev))
    }

    fn keychain(&mut self, ctx: &Ctx, reply: bifrost_app::keychain::Reply) -> bool {
        self.with_window(ctx, |mail| mail.keychain(ctx, reply))
    }

    fn tick(&mut self, ctx: &Ctx, t: f64) -> bool {
        self.check_tick(ctx, t);
        let mut changed = self.open_viewers(ctx) | self.refresh_viewers();
        if let Some(mail) = &mut self.window {
            self.composes.autosave(mail, t);
        }
        changed |= self.with_window(ctx, |mail| mail.tick(ctx, t));
        self.index_tick(ctx, t);
        changed
    }

    fn continue_activity(&mut self, _ctx: &Ctx, activity: bifrost_app::quicksearch::UserActivity) -> bool {
        if activity.activity_type != bifrost_app::quicksearch::ACTION_TYPE {
            return false;
        }
        let Some((account, mailbox, uid)) = index::parse_id(&activity.unique_identifier) else { return false };
        let Some(mail) = self.window.as_mut().map(|w| &mut w.mail) else { return false };
        let Some(i) = mail.accounts.iter().position(|a| a.id == account) else { return false };
        if mail.account != i {
            mail.show_account(i);
        }
        if mail.mailbox != mailbox {
            mail.show_mailbox(mailbox);
        }
        mail.open_message(uid);
        true
    }


    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, _t: f64) -> Damage {
        if let Some(mail) = &mut self.window {
            mail.draw_hiding_compose(s, &ctx.ui, self.composes.active.is_some());
        }
        let mut d = Damage::new(s.w, s.h);
        d.all();
        d
    }

    fn draw_window(&mut self, ctx: &Ctx, window: WindowId, s: &mut Surface, t: f64) -> Damage {
        if window.is_main() {
            return self.draw(ctx, s, t);
        }
        if let Some(viewer) = self.viewers.iter_mut().find(|v| v.id == window) {
            viewer.draw(s, &ctx.ui);
        } else if let Some(mail) = &mut self.window {
            if let Some(compose) = self.composes.compose_of(window) {
                self.composes.resized(mail, window, s.w as f32, s.h as f32);
                // the window that asks the question shows it; the others show the message
                let asking = self.composes.active == Some(window) && matches!(mail.mail.sheet, Some(bifrost_mail::state::Sheet::Question(_)));
                let sheet = if asking { bifrost_mail::state::Sheet::Question(compose) } else { bifrost_mail::state::Sheet::Compose(compose) };
                mail.draw_compose(s, &ctx.ui, &sheet);
            }
        }
        let mut d = Damage::new(s.w, s.h);
        d.all();
        d
    }

    fn window_closed(&mut self, ctx: &Ctx, window: WindowId) -> bool {
        let before = self.viewers.len();
        self.viewers.retain(|v| v.id != window);
        let mut changed = before != self.viewers.len();
        if let Some(mail) = &mut self.window {
            changed |= self.composes.closed(ctx, mail, window);
        }
        changed
    }

    fn next_wake(&self, t: f64) -> Option<f64> {
        let window = self.window.as_ref().and_then(|mail| mail.next_wake(t));
        let check = self.check.as_ref().map(|_| t + background::POLL_MS);
        match (window, check) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    fn wake_fd(&self) -> Option<i32> {
        self.window.as_ref().and_then(|mail| mail.wake_fd())
    }
}

/// A key or a standard editing command for the main window's message list or its sheet (the active compose's message
/// while a compose window is key).
fn key_or_command(mail: &mut MailWindow, ctx: &Ctx, ev: Input) -> bool {
    // typing shows the end of the message again, where the caret is, after the wheel scrolled it up
    if let Some(bifrost_mail::state::Sheet::Compose(compose)) = &mut mail.mail.sheet {
        compose.scroll_back = 0;
    }
    match ev {
        Input::Key(k) => mail.key(ctx, k),
        Input::Command(Command::Paste) => mail.paste(),
        Input::Command(Command::SelectAll) => mail.select_all(),
        Input::Command(Command::Copy) => mail.copy(),
        Input::Command(Command::Cut) => mail.cut(),
        _ => false,
    }
}

fn now_unix() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

impl MailApp {
    /// Asks the shell to start the app for a mail check when one is due, also when Mail is closed. Once per run.
    fn schedule(&mut self, ctx: &Ctx) {
        if !self.scheduled {
            self.scheduled = true;
            background_tasks::submit(ctx, &background::next_request(now_unix()));
        }
    }

    /// A refresh task started: asks Passwords for the password of each of `accounts`; the listings follow the answers.
    fn start_check(&mut self, ctx: &Ctx, identifier: &str, accounts: Vec<bifrost_app::controls::internet_accounts::Account>) {
        if self.check.is_some() {
            return;
        }
        let baseline = background::Baseline::load(&ctx.data_dir.join(background::FILE));
        let muted = bifrost_mail::mute::load(&ctx.data_dir.join(bifrost_mail::mute::FILE));
        let mut check = Check::new(identifier, accounts, baseline, muted, std::sync::Arc::clone(&self.transport));
        for (index, site, username) in check.logins() {
            let request = passwords::fill(ctx, &site, &username);
            check.expect(request, index);
        }
        self.check = Some(check);
    }

    /// Posts the notifications for what finished listings found, and ends the check when it is done.
    fn check_tick(&mut self, ctx: &Ctx, t: f64) {
        let Some(check) = &mut self.check else { return };
        for notice in check.poll() {
            let b = background::notification(&notice);
            let _ = usernotif::add(ctx, usernotif::Request::new(&b.id, &b.title).subtitle(&b.subtitle).body(&b.body).thread(&b.thread));
        }
        if check.finished(t) {
            self.finish_check(ctx);
        }
    }

    /// Saves the baseline, asks for the next refresh, and tells the shell the task is done (it then ends a hidden app).
    fn finish_check(&mut self, ctx: &Ctx) {
        let Some(check) = self.check.take() else { return };
        if let Err(e) = check.baseline().save(&ctx.data_dir.join(background::FILE)) {
            eprintln!("bifrost-mail: mail baseline: {e}");
        }
        for problem in &check.problems {
            eprintln!("bifrost-mail: background check: {problem}");
        }
        background_tasks::submit(ctx, &background::next_request(now_unix()));
        background_tasks::completed(ctx, check.identifier());
    }

    /// The window with focus decides which compose a key or a command acts on: the focused compose window's message.
    fn route_focus(&mut self, ctx: &Ctx) {
        // the key window the shell reported (`WindowFocus`), not the registry's front window: the registry keeps the
        // order windows were opened in, so a compose window opened once would stay "focused" after a click on the main
        // window and take its keys
        let focused = ctx.key_window();
        if let Some(mail) = &mut self.window {
            if self.composes.owns(focused) {
                self.composes.activate(mail, focused);
            } else if focused.is_main() {
                self.composes.deactivate(mail);
            }
        }
    }

    /// Keeps Core Search in step with the newest messages of the mailbox on screen: the fields of `index::SCHEMA`, no bodies.
    fn index_tick(&mut self, ctx: &Ctx, t: f64) {
        if t < self.next_index {
            return;
        }
        self.next_index = t + INDEX_EVERY_MS;
        let Some(w) = &self.window else { return };
        let m = &w.mail;
        let account = m.accounts.get(m.account).map(|a| a.id.clone());
        let p = index::plan(&mut self.indexed, m.prefs.show_in_search, account.as_deref(), &m.mailbox, &m.rows);
        let spot = bifrost_app::quicksearch::CSSearchableIndex::default_index();
        if !p.changed.is_empty() {
            spot.index_searchable_items(ctx, &p.changed);
        }
        if !p.gone.is_empty() {
            let refs: Vec<&str> = p.gone.iter().map(String::as_str).collect();
            spot.delete_searchable_items(ctx, &refs);
        }
    }
}

impl MailApp {
    fn new(transport: std::sync::Arc<dyn springboard::deildu::http::Transport>) -> MailApp {
        MailApp { window: None, viewers: Vec::new(), composes: Composes::default(), pending_url: None, indexed: Default::default(), next_index: 0.0, check: None, scheduled: false, transport, title: String::new() }
    }
}

fn main() {
    bifrost_app::run(MailApp::new(bifrost_mail::transport::shared()));
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_app::controls::internet_accounts::{Account, Kind, Security, Server};
    use bifrost_app::kit::{Font, Ui};
    use bifrost_app::proto::{Configure, ToApp, ToShell};
    use bifrost_app::{KeyInput, ModifierFlags};
    use bifrost_mail::compose_windows::{COMPOSE_H, COMPOSE_W};
    use bifrost_mail::draw::{choice_count, sheet_layout, Hit};
    use bifrost_mail::state::{Sheet, INBOX};
    use bifrost_core::json::Json;
    use bifrost_mail::flags::{Flag, Flags};
    use bifrost_mail::session::Listed;
    use springboard::deildu::http::{Stream, Transport};
    use std::io::{Cursor, Read, Write};

    /// A mail server that greets, accepts the login, and lists an inbox with uid 3 (read) and uid 9 (unread).
    struct Canned;
    struct Wire(Cursor<Vec<u8>>);
    impl Read for Wire {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.0.read(buf)
        }
    }
    impl Write for Wire {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    impl Transport for Canned {
        fn connect(&self, _: &str, _: u16, _: bool) -> std::io::Result<Box<dyn Stream>> {
            let (old, new) = ("Subject: Old\r\nFrom: A <a@x.is>\r\n\r\n", "Subject: Fundur\r\nFrom: Gunna <gunna@x.is>\r\n\r\n");
            let script = format!(
                "* OK ready\r\nA001 OK done\r\n* LIST (\\HasNoChildren) \"/\" \"INBOX\"\r\nA002 OK done\r\nA003 OK done\r\n* SEARCH 3 9\r\nA004 OK done\r\n\
                 * 1 FETCH (UID 3 FLAGS (\\Seen) BODY[HEADER] {{{}}}\r\n{old})\r\n* 2 FETCH (UID 9 FLAGS () BODY[HEADER] {{{}}}\r\n{new})\r\nA005 OK done\r\nA006 OK done\r\n",
                old.len(),
                new.len()
            );
            Ok(Box::new(Wire(Cursor::new(script.into_bytes()))))
        }
    }

    fn ctx(dir: &std::path::Path) -> Ctx {
        ctx_at(dir, 1.0)
    }

    /// A test context on a screen of `scale` pixels a point.
    fn ctx_at(dir: &std::path::Path, scale: f32) -> Ctx {
        let data: &'static [u8] = Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/SFUI.ttf")).unwrap().into_boxed_slice());
        let cfg = Configure { w: 800, h: 600, scale, safe_top: 0.0, safe_bottom: 0.0, dark: false, text_scale: 1.0, reduce_motion: false };
        let mut ctx = Ctx::testing(Ui::new(Font::parse(data).unwrap(), scale), cfg);
        ctx.data_dir = dir.to_path_buf();
        ctx
    }

    fn work() -> Account {
        Account {
            email: "me@x.is".into(),
            imap: Some(Server { host: "imap.x.is".into(), port: 993, security: Security::Tls }),
            smtp: Some(Server { host: "smtp.x.is".into(), port: 587, security: Security::StartTls }),
            ..Account::new(Kind::Mail, "work", "Work", "me")
        }
    }

    fn calls(sent: &[ToShell]) -> Vec<(u32, String, String, String)> {
        sent.iter()
            .filter_map(|m| match m {
                ToShell::ServiceCall { request, service, method, body } => Some((*request, service.clone(), method.clone(), body.clone())),
                _ => None,
            })
            .collect()
    }

    /// A Mail app with its main window and one account, and no server behind it.
    fn app_with_account(ctx: &Ctx) -> MailApp {
        let mut app = MailApp::new(std::sync::Arc::new(Canned));
        let mut mail = MailWindow::new(ctx);
        mail.mail.accounts = vec![work()];
        mail.mail.account = 0;
        mail.set_size(800, 600);
        app.window = Some(mail);
        app.composes.main_size = (800, 600);
        app
    }

    /// The value of field `label` in the first compose window's message (the sheet's while it is active).
    fn compose_field(app: &MailApp, label: &str) -> String {
        let compose = match &app.window.as_ref().unwrap().mail.sheet {
            Some(Sheet::Compose(c)) if app.composes.active.is_some() => c.clone(),
            _ => app.composes.slots[0].compose.clone(),
        };
        compose.form.fields.iter().find(|f| f.label == label).map(|f| f.value.clone()).unwrap_or_default()
    }

    /// The centre of field `label` of the active compose, as a compose window of `w` x `h` pixels draws it.
    fn field_centre(app: &MailApp, ui: &Ui, label: &str, w: f32, h: f32) -> (f32, f32) {
        let mail = app.window.as_ref().unwrap();
        let sheet = mail.mail.sheet.clone().expect("the compose is in the sheet slot");
        let at = sheet.form().fields.iter().position(|f| f.label == label).expect("the field");
        let r = sheet_layout(ui, w, h, &sheet, choice_count(&mail.mail), true).fields[at];
        (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0)
    }

    /// The test app with its inbox listed: message 9 ("Fundur") above message 3 ("Old").
    fn app_with_inbox(ctx: &Ctx) -> MailApp {
        let mut app = app_with_account(ctx);
        let listed = |uid: u32, subject: &str| Listed { uid, flags: Flags::empty(), header: format!("Subject: {subject}\r\nFrom: Ana <ana@x.is>\r\n\r\n") };
        app.window.as_mut().unwrap().mail.apply_listing("work", INBOX, vec![], vec![listed(3, "Old"), listed(9, "Fundur")]);
        app
    }

    /// A point of the main window on message `uid`'s row.
    fn row_point(app: &MailApp, ui: &Ui, uid: u32) -> (f32, f32) {
        let mail = app.window.as_ref().unwrap();
        for y in (0..600).step_by(4) {
            for x in (0..800).step_by(8) {
                if matches!(mail.hit(ui, x as f32, y as f32), Hit::Message(u) if u == uid) {
                    return (x as f32, y as f32);
                }
            }
        }
        panic!("message {uid} is not in the list");
    }

    /// A click (press and release) on the main window at `t` milliseconds.
    fn click(app: &mut MailApp, ctx: &Ctx, at: (f32, f32), t: f64) {
        click_with(app, ctx, at, t, ModifierFlags::EMPTY);
    }

    /// A click with modifier keys held (a Shift-click, a Cmd-click), as the shell sends it.
    fn click_with(app: &mut MailApp, ctx: &Ctx, (x, y): (f32, f32), t: f64, mods: ModifierFlags) {
        for kind in [TouchKind::Down, TouchKind::Up] {
            app.event(ctx, Input::Touch { kind, id: 0, x, y, t, mods });
        }
    }

    fn selected(app: &MailApp) -> Vec<u32> {
        app.window.as_ref().unwrap().mail.selected()
    }

    /// E2E-3: Cmd-click toggles a message into the selection and Shift-click selects the range from the anchor; the
    /// context menu keeps the selection and its actions take every selected message.
    #[test]
    fn cmd_click_and_shift_click_select_several_messages_and_archive_takes_them_all() {
        let dir = std::env::temp_dir().join(format!("bifrost-mail-multi-{}", std::process::id()));
        let ctx = ctx(&dir);
        let mut app = app_with_account(&ctx);
        let listed = |uid: u32| Listed { uid, flags: Flags::empty(), header: format!("Subject: m{uid}\r\nFrom: Ana <ana@x.is>\r\n\r\n") };
        let mailboxes = vec![INBOX.to_string(), "Archive".to_string()];
        app.window.as_mut().unwrap().mail.apply_listing("work", INBOX, mailboxes, (1..=4).map(listed).collect());
        // the list is newest first: 4, 3, 2, 1
        let at = row_point(&app, &ctx.ui, 4);
        click(&mut app, &ctx, at, 1000.0);
        assert_eq!(selected(&app), vec![4]);
        let at = row_point(&app, &ctx.ui, 2);
        click_with(&mut app, &ctx, at, 3000.0, ModifierFlags::COMMAND);
        assert_eq!(selected(&app), vec![4, 2], "Cmd-click adds a message");
        assert_eq!(app.window.as_ref().unwrap().mail.open, Some(2), "the reader follows the last click");
        let at = row_point(&app, &ctx.ui, 4);
        click_with(&mut app, &ctx, at, 5000.0, ModifierFlags::COMMAND);
        assert_eq!(selected(&app), vec![2], "Cmd-click takes a selected message out");
        let at = row_point(&app, &ctx.ui, 4);
        click_with(&mut app, &ctx, at, 7000.0, ModifierFlags::SHIFT);
        assert_eq!(selected(&app), vec![4, 3, 2], "Shift-click selects from the anchor to the click");
        assert_eq!(app.window.as_ref().unwrap().mail.selected_count(), 3);
        assert!(app.window.as_mut().unwrap().take_viewers().is_empty(), "modifier clicks never count as a double-click");
        // a right-click inside the selection keeps it, and Archive from its menu moves all three
        let (x, y) = row_point(&app, &ctx.ui, 3);
        assert!(app.event(&ctx, Input::Context { x, y }));
        assert_eq!(selected(&app), vec![4, 3, 2]);
        app.event(&ctx, Input::ContextAction(bifrost_mail::window::MENU_ARCHIVE));
        let rows: Vec<u32> = app.window.as_ref().unwrap().mail.rows.iter().map(|r| r.uid).collect();
        assert_eq!(rows, vec![1], "every selected message left the list");
        assert!(selected(&app).is_empty());
        // a plain click is a single selection again
        let at = row_point(&app, &ctx.ui, 1);
        click(&mut app, &ctx, at, 9000.0);
        assert_eq!(selected(&app), vec![1]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn shift_arrows_extend_the_selection_and_select_all_takes_the_list() {
        let dir = std::env::temp_dir().join(format!("bifrost-mail-shift-arrows-{}", std::process::id()));
        let ctx = ctx(&dir);
        let mut app = app_with_inbox(&ctx);
        let (down, shift_down, shift_up) = (
            Input::Key(KeyInput::Code(40)),
            Input::Key(KeyInput::chord(108, bifrost_app::proto::CHORD_SHIFT)),
            Input::Key(KeyInput::chord(103, bifrost_app::proto::CHORD_SHIFT)),
        );
        app.event(&ctx, down.clone());
        assert_eq!(selected(&app), vec![9]);
        assert!(app.event(&ctx, shift_down.clone()), "Shift-Down extends the selection");
        assert_eq!(selected(&app), vec![9, 3]);
        assert!(!app.event(&ctx, shift_down), "the end of the list");
        app.event(&ctx, shift_up);
        assert_eq!(selected(&app), vec![9]);
        // Edit > Select All with no sheet selects every message the list shows
        assert!(app.event(&ctx, Input::Command(Command::SelectAll)));
        assert_eq!(selected(&app), vec![9, 3]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_double_click_on_a_message_opens_a_viewer_window() {
        let dir = std::env::temp_dir().join(format!("bifrost-mail-double-{}", std::process::id()));
        let ctx = ctx(&dir);
        let mut app = app_with_inbox(&ctx);
        let at = row_point(&app, &ctx.ui, 9);
        click(&mut app, &ctx, at, 1000.0);
        assert_eq!(app.window.as_ref().unwrap().mail.open, Some(9), "one click selects");
        assert!(app.window.as_mut().unwrap().take_viewers().is_empty());
        // the second click 100 ms later (the event clock is in milliseconds) is a double-click
        click(&mut app, &ctx, at, 1100.0);
        let viewers: Vec<u32> = app.window.as_mut().unwrap().take_viewers().iter().map(|v| v.uid).collect();
        assert_eq!(viewers, vec![9]);
        // clicks a second apart are single clicks
        click(&mut app, &ctx, at, 5000.0);
        click(&mut app, &ctx, at, 6000.0);
        assert!(app.window.as_mut().unwrap().take_viewers().is_empty());
        // a double-click needs both clicks on the same message
        let other = row_point(&app, &ctx.ui, 3);
        click(&mut app, &ctx, at, 9000.0);
        click(&mut app, &ctx, other, 9100.0);
        assert!(app.window.as_mut().unwrap().take_viewers().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_right_click_on_a_message_selects_it_and_offers_its_actions() {
        let dir = std::env::temp_dir().join(format!("bifrost-mail-context-{}", std::process::id()));
        let ctx = ctx(&dir);
        let mut app = app_with_inbox(&ctx);
        let _ = ctx.take_sent();
        let (x, y) = row_point(&app, &ctx.ui, 3);
        assert!(app.event(&ctx, Input::Context { x, y }));
        assert_eq!(app.window.as_ref().unwrap().mail.open, Some(3), "the clicked message is the one the menu acts on");
        let menu = ctx.take_sent().into_iter().find_map(|m| match m {
            ToShell::ContextMenu { items, .. } => Some(items),
            _ => None,
        });
        let items = menu.expect("a context menu");
        assert!(items.iter().any(|i| i.id == bifrost_mail::window::MENU_REPLY && i.enabled));
        // the choice runs the menu action: Open in New Window
        app.event(&ctx, Input::ContextAction(bifrost_mail::window::MENU_NEW_VIEWER));
        let viewers: Vec<u32> = app.window.as_mut().unwrap().take_viewers().iter().map(|v| v.uid).collect();
        assert_eq!(viewers, vec![3]);
        // a right-click on no message opens nothing
        let _ = ctx.take_sent();
        assert!(!app.event(&ctx, Input::Context { x: 1.0, y: 599.0 }));
        assert!(!ctx.take_sent().iter().any(|m| matches!(m, ToShell::ContextMenu { .. })));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_arrow_keys_move_the_selection_in_the_list() {
        let dir = std::env::temp_dir().join(format!("bifrost-mail-arrows-{}", std::process::id()));
        let ctx = ctx(&dir);
        let mut app = app_with_inbox(&ctx);
        let open = |app: &MailApp| app.window.as_ref().unwrap().mail.open;
        let (up, down) = (Input::Key(KeyInput::Code(38)), Input::Key(KeyInput::Code(40)));
        assert!(app.event(&ctx, down.clone()), "Down selects the first message");
        assert_eq!(open(&app), Some(9));
        app.event(&ctx, down.clone());
        assert_eq!(open(&app), Some(3));
        assert!(!app.event(&ctx, down), "the last message stays selected");
        assert_eq!(open(&app), Some(3));
        app.event(&ctx, up);
        assert_eq!(open(&app), Some(9));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_compose_window_takes_its_clicks_keys_and_escape() {
        let dir = std::env::temp_dir().join(format!("bifrost-mail-compose-{}", std::process::id()));
        let ctx = ctx(&dir);
        let mut app = app_with_account(&ctx);
        app.with_window(&ctx, |mail| {
            mail.mail.open_compose(None);
            true
        });
        let id = app.composes.active.expect("the new message opens in a window of its own");
        // the clicks of the compose window are hit-tested at its size, not at the main window's
        let mail = app.window.as_ref().unwrap();
        assert_eq!((mail.w, mail.h), (COMPOSE_W as u32, COMPOSE_H as u32));
        // the shell sends the compose window's keys tagged with that window: they type into its message
        app.window_event(&ctx, id, Input::Key(KeyInput::Text("ana@x.is".into())));
        assert_eq!(compose_field(&app, "To"), "ana@x.is");
        // a click on Subject moves the focus there, and typing follows it
        let (x, y) = field_centre(&app, &ctx.ui, "Subject", COMPOSE_W, COMPOSE_H);
        for kind in [TouchKind::Down, TouchKind::Up] {
            app.window_event(&ctx, id, Input::Touch { kind, id: 0, x, y, t: 10.0, mods: bifrost_app::ModifierFlags::EMPTY });
        }
        app.window_event(&ctx, id, Input::Key(KeyInput::Text("Fundur".into())));
        assert_eq!(compose_field(&app, "Subject"), "Fundur");
        // Escape closes the message, asking first because it has text
        app.window_event(&ctx, id, Input::Key(KeyInput::Code(1)));
        assert!(matches!(app.window.as_ref().unwrap().mail.sheet, Some(Sheet::Question(_))), "Save This Message, Don't Save, Cancel");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// E2E N9: a compose window that closes (here an empty message's Escape) gives the main window its own size back, so
    /// a click in the sidebar picks a mailbox instead of landing on a list row of the compose window's narrow layout.
    #[test]
    fn a_closed_compose_window_gives_the_main_window_its_own_clicks_back() {
        let dir = std::env::temp_dir().join(format!("bifrost-mail-compose-closed-{}", std::process::id()));
        let ctx = ctx(&dir);
        let mut app = app_with_inbox(&ctx);
        app.with_window(&ctx, |mail| {
            mail.mail.open_compose(None);
            true
        });
        let id = app.composes.active.expect("a compose window");
        app.window_event(&ctx, id, Input::Key(KeyInput::Code(1)));
        assert!(app.composes.slots.is_empty(), "an empty message closes without asking");
        assert_eq!(app.composes.active, None);
        let mail = app.window.as_ref().unwrap();
        assert_eq!((mail.w, mail.h), (800, 600), "the main window's size is back");
        assert!(!mail.in_compose_window);
        let side = bifrost_mail::draw::columns(&ctx.ui, 800.0, mail.mail.show_sidebar)[0];
        assert!(side > 0.0, "the sidebar is shown");
        let hits: Vec<Hit> = (0..600).step_by(4).map(|y| mail.hit(&ctx.ui, side / 2.0, y as f32)).collect();
        assert!(hits.iter().any(|h| matches!(h, Hit::Side(_))), "the sidebar takes its clicks");
        assert!(!hits.iter().any(|h| matches!(h, Hit::Message(_))), "no sidebar click lands on a message");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// E2E N15: why a Send did not go out shows on the compose sheet, in the compose window, not in the main window's
    /// list footer; the message stays in its window to be fixed.
    #[test]
    fn a_send_that_fails_says_why_in_the_compose_window() {
        let dir = std::env::temp_dir().join(format!("bifrost-mail-compose-error-{}", std::process::id()));
        let ctx = ctx(&dir);
        let mut app = app_with_account(&ctx);
        app.with_window(&ctx, |mail| {
            mail.mail.open_compose(None);
            true
        });
        let id = app.composes.active.expect("a compose window");
        app.window_event(&ctx, id, Input::Key(KeyInput::Text("e2e@".into())));
        app.window_input(&ctx, id, WindowInput::MenuAction(bifrost_mail::window::MENU_SEND));
        assert_eq!(app.composes.active, Some(id), "the message stays in its window");
        let mail = app.window.as_ref().unwrap();
        let Some(Sheet::Compose(compose)) = &mail.mail.sheet else { panic!("the compose is still open") };
        assert!(compose.error.contains("e2e@"), "the compose says why: {:?}", compose.error);
        assert!(mail.mail.status.is_empty(), "the main window's footer stays quiet: {:?}", mail.mail.status);
        // an edit clears it
        app.window_event(&ctx, id, Input::Key(KeyInput::Text("x.is".into())));
        let Some(Sheet::Compose(compose)) = &app.window.as_ref().unwrap().mail.sheet else { panic!("the compose is still open") };
        assert!(compose.error.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A wheel event turned `dy` (negative: down) at (`x`, `y`).
    fn wheel(x: f32, y: f32, dy: f32) -> PointerEvent {
        use bifrost_app::pointer_events::Phase;
        PointerEvent::Wheel { x, y, dx: 0.0, dy, precise: false, phase: Phase::None, momentum: Phase::None }
    }

    /// Puts `lines` lines of text into the message of the active compose.
    fn fill_message(app: &mut MailApp, lines: usize) {
        let text: Vec<String> = (0..lines).map(|i| format!("line {i}")).collect();
        let Some(Sheet::Compose(compose)) = &mut app.window.as_mut().unwrap().mail.sheet else { panic!("a compose in the slot") };
        let field = compose.form.fields.iter_mut().find(|f| f.label == "Message").expect("the message field");
        field.value = text.join("\n");
        compose.styles = vec![Default::default(); compose.message().chars().count()];
    }

    fn scroll_back(app: &MailApp) -> usize {
        match &app.window.as_ref().unwrap().mail.sheet {
            Some(Sheet::Compose(c)) => c.scroll_back,
            _ => app.composes.slots[0].compose.scroll_back,
        }
    }

    /// E2E: Mail takes the wheel (so the shell no longer emulates a drag for it in any window): the main window's
    /// message list scrolls under the pointer, a viewer window's text scrolls in pixels from the window points it
    /// gets, and a compose window's message scrolls back from its end until typing brings the end back.
    #[test]
    fn the_wheel_scrolls_the_list_a_viewer_and_a_compose_message() {
        let dir = std::env::temp_dir().join(format!("bifrost-mail-wheel-{}", std::process::id()));
        let ctx = ctx(&dir);
        let mut app = app_with_account(&ctx);
        assert_eq!(app.pointer_interest(), bifrost_app::pointer_events::WHEEL);
        let listed = |uid: u32| Listed { uid, flags: Flags::empty(), header: format!("Subject: m{uid}\r\nFrom: Ana <ana@x.is>\r\n\r\n") };
        app.window.as_mut().unwrap().mail.apply_listing("work", INBOX, vec![], (1..=60).map(listed).collect());
        let (x, y) = row_point(&app, &ctx.ui, 60);
        // the list's range is set when it is drawn
        app.window.as_mut().unwrap().list_scroll.set_max(1000.0);
        assert!(app.pointer(&ctx, &wheel(x, y, -40.0)), "the list under the pointer scrolls");
        assert_eq!(app.window.as_ref().unwrap().list_scroll.offset(), 40.0);
        assert!(app.window_pointer(&ctx, WindowId::MAIN, &wheel(x, y, 15.0)), "window 0 is the main window");
        assert_eq!(app.window.as_ref().unwrap().list_scroll.offset(), 25.0);

        // a viewer window
        let id = ctx.open_window(bifrost_mail::viewer::Snapshot::default().spec()).unwrap();
        app.viewers.push(Viewer::new(id, bifrost_mail::viewer::Snapshot::default()));
        app.viewers[0].scroll.set_max(300.0);
        assert!(app.window_pointer(&ctx, id, &wheel(10.0, 10.0, -30.0)));
        assert_eq!(app.viewers[0].scroll.offset(), 30.0);
        // over the wire, as the shell sends it: the SDK hands it to `window_pointer`
        let msg = ToApp::WindowInput { window: id.0, input: WindowInput::Pointer(wheel(10.0, 10.0, -30.0)) };
        let ToApp::WindowInput { input, .. } = ToApp::decode(&msg.encode()).unwrap() else { unreachable!() };
        assert!(app.window_input(&ctx, id, input));
        assert_eq!(app.viewers[0].scroll.offset(), 60.0);

        // a compose window with a long message shows its end; the wheel turned up shows earlier lines
        app.with_window(&ctx, |mail| {
            mail.mail.open_compose(None);
            true
        });
        let compose = app.composes.active.expect("a compose window");
        fill_message(&mut app, 40);
        assert!(app.window_pointer(&ctx, compose, &wheel(10.0, 10.0, 60.0)));
        let back = scroll_back(&app);
        assert!(back >= 3, "about three lines a 60-point turn: {back}");
        assert!(app.window_pointer(&ctx, compose, &wheel(10.0, 10.0, 99_999.0)));
        let top = scroll_back(&app);
        assert!(top > back && top < 40, "clamped so the top line shows at the top: {top}");
        assert!(!app.window_pointer(&ctx, compose, &wheel(10.0, 10.0, 60.0)), "already at the top");
        app.window_pointer(&ctx, compose, &wheel(10.0, 10.0, -99_999.0));
        assert_eq!(scroll_back(&app), 0, "back at the end");
        app.window_pointer(&ctx, compose, &wheel(10.0, 10.0, 40.0));
        assert!(scroll_back(&app) > 0);
        app.window_event(&ctx, compose, Input::Key(KeyInput::Text("x".into())));
        assert_eq!(scroll_back(&app), 0, "typing shows the end, where the caret is");
        // the main window's wheel gives the sheet slot back and scrolls the list, not the compose
        app.window_pointer(&ctx, compose, &wheel(10.0, 10.0, 40.0));
        let held = scroll_back(&app);
        app.pointer(&ctx, &wheel(x, y, -10.0));
        assert_eq!(app.composes.active, None);
        assert_eq!(app.window.as_ref().unwrap().list_scroll.offset(), 35.0);
        // a compose window that is not active scrolls its own message in its slot
        assert!(app.window_pointer(&ctx, compose, &wheel(10.0, 10.0, -20.0)));
        assert!(app.composes.slots[0].compose.scroll_back < held);
        assert_eq!(app.composes.active, None, "the wheel does not move the sheet slot");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The shell sends a compose window's touches in window points; the sheet is hit-tested in the pixels it is drawn
    /// in, so on a 2x screen a click lands on the field drawn under it.
    #[test]
    fn a_compose_window_click_in_points_lands_on_its_field_on_a_2x_screen() {
        let dir = std::env::temp_dir().join(format!("bifrost-mail-compose-2x-{}", std::process::id()));
        let ctx = ctx_at(&dir, 2.0);
        let mut app = app_with_account(&ctx);
        app.with_window(&ctx, |mail| {
            mail.mail.open_compose(None);
            true
        });
        let id = app.composes.active.expect("a compose window");
        let (x, y) = field_centre(&app, &ctx.ui, "Subject", 2.0 * COMPOSE_W, 2.0 * COMPOSE_H);
        for kind in [TouchKind::Down, TouchKind::Up] {
            app.window_event(&ctx, id, Input::Touch { kind, id: 0, x: x / 2.0, y: y / 2.0, t: 10.0, mods: bifrost_app::ModifierFlags::EMPTY });
        }
        app.window_event(&ctx, id, Input::Key(KeyInput::Text("Fundur".into())));
        assert_eq!(compose_field(&app, "Subject"), "Fundur");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_main_window_keeps_its_keys_once_it_is_key_again() {
        let dir = std::env::temp_dir().join(format!("bifrost-mail-key-{}", std::process::id()));
        let ctx = ctx(&dir);
        let mut app = app_with_account(&ctx);
        app.with_window(&ctx, |mail| {
            mail.mail.open_compose(None);
            true
        });
        let id = app.composes.active.expect("a compose window");
        app.window_event(&ctx, id, Input::Key(KeyInput::Text("bo@x.is".into())));
        // the main window is key (no compose window was reported focused): its keys never reach the compose, although
        // the compose window is the newest in the app's window list
        app.event(&ctx, Input::Key(KeyInput::Text("zz".into())));
        assert_eq!(app.composes.active, None, "the compose waits in its window");
        assert_eq!(compose_field(&app, "To"), "bo@x.is");
        let mail = app.window.as_ref().unwrap();
        assert_eq!((mail.w, mail.h), (800, 600), "the main window's size is back for its own clicks");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_refresh_task_checks_the_inbox_posts_new_mail_and_asks_for_the_next_check() {
        let root = std::env::temp_dir().join(format!("bifrost-mail-bg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (data, accounts_home) = (root.join("data"), root.join("home"));
        // the shared account list lives under XDG_DATA_HOME; this is the only test of this binary that sets it (the
        // compose tests put their account into the window directly)
        std::env::set_var("XDG_DATA_HOME", &accounts_home);
        bifrost_mail::accounts::add(work()).unwrap();
        // the last check saw uid 3 in the inbox
        let mut baseline = background::Baseline::default();
        let seen = Listed { uid: 3, flags: Flags::empty().with(Flag::Seen, true), header: "Subject: Old\r\n\r\n".into() };
        baseline.apply("work", &[seen], &Default::default());
        baseline.save(&data.join(background::FILE)).unwrap();

        let ctx = ctx(&data);
        let mut app = MailApp::new(std::sync::Arc::new(Canned));
        // starting up asks the shell for a check, once
        app.schedule(&ctx);
        app.schedule(&ctx);
        let sent = calls(&ctx.take_sent());
        assert_eq!(sent.len(), 1);
        assert_eq!((sent[0].1.as_str(), sent[0].2.as_str()), ("background", "submit"));
        assert!(sent[0].3.contains(background::TASK) && sent[0].3.contains("earliest"), "{}", sent[0].3);

        // another task's start is not ours
        let other = bifrost_core::json::parse(r#"{"identifier":"x.other","limit":30}"#).unwrap();
        app.service_event(&ctx, "background", "taskStarted", &other);
        assert!(app.check.is_none());
        // the shell starts the refresh task: Passwords is asked for the account's password
        let started = bifrost_core::json::parse(&format!(r#"{{"identifier":"{}","limit":30}}"#, background::TASK)).unwrap();
        app.service_event(&ctx, "background", "taskStarted", &started);
        assert!(app.check.is_some());
        let asked = calls(&ctx.take_sent());
        assert_eq!(asked.len(), 1, "{asked:?}");
        let request = asked[0].0;
        // Passwords answers; the listing runs on a thread and the tick posts what it found
        let hex: String = b"pw".iter().map(|b| format!("{b:02x}")).collect();
        let answer: Result<Json, String> = Ok(Json::Obj(vec![("status".into(), Json::Num(0.0)), ("value".into(), Json::Str(hex))]));
        app.service_reply(&ctx, request, &answer);
        let mut t = 0.0;
        let mut sent = Vec::new();
        for _ in 0..400 {
            t += 10.0;
            app.tick(&ctx, t);
            sent.extend(ctx.take_sent());
            if app.check.is_none() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(app.check.is_none(), "the check finished");
        let notes: Vec<&ToShell> = sent.iter().filter(|m| matches!(m, ToShell::NotifyRequest { .. })).collect();
        assert_eq!(notes.len(), 1, "one unread arrival: {sent:?}");
        match notes[0] {
            ToShell::NotifyRequest { req, .. } => {
                assert_eq!((req.id.as_str(), req.title.as_str(), req.subtitle.as_str(), req.body.as_str(), req.thread.as_str()), ("mail-work-9", "New Mail", "Gunna", "Fundur", "work"));
            }
            _ => unreachable!(),
        }
        // the task is reported done and the next check asked for
        let methods: Vec<String> = calls(&sent).into_iter().filter(|c| c.1 == "background").map(|c| c.2).collect();
        assert_eq!(methods, vec!["submit", "completed"], "next check first, then done");
        // the baseline moved, so the same mail is not announced again
        assert_eq!(background::Baseline::load(&data.join(background::FILE)).get("work"), Some(9));
        let _ = std::fs::remove_dir_all(&root);
    }
}
