use crate::proto::sys::{now_ms, shm_create, Channel, Map, Owned, Recv};
use crate::proto::{Configure, KeyInput, Rect, ToApp, ToShell, TouchKind, SDK_VERSION};
use crate::touch_events::{localize, Plain, TouchRouter};
use crate::{kit, App};
use bifrost_render::font::Font;
use bifrost_render::gfx::{Damage, Surface};
use bifrost_render::ui::Ui;
use std::cell::RefCell;
use std::path::PathBuf;

/// The app's own controls plus every kit-drawn node it did not publish itself (same role and label), like the native toolkit
/// where standard controls stay accessible whatever the app adds. App nodes come first and win.
pub(crate) fn merge_controls(mut app: Vec<crate::controls::Node>, drawn: &[crate::controls::Node]) -> Vec<crate::controls::Node> {
    let extra: Vec<_> = drawn.iter().filter(|d| !app.iter().any(|a| a.role == d.role && a.label == d.label)).cloned().collect();
    app.extend(extra);
    app
}

/// The app's own `controls_tree` plus the parents the kit's draw helpers recorded (`ax::begin_group`). An entry the app sent
/// wins; a drawn node it did not mention gets one with its recorded parent. Capped at the packet limit.
pub(crate) fn merge_tree(mut app: Vec<crate::proto::NodeExt>, links: &[bifrost_ui::ax::AxLink]) -> Vec<crate::proto::NodeExt> {
    for l in links {
        if app.len() >= crate::proto::MAX_NODE_EXT {
            break;
        }
        if !app.iter().any(|e| e.id == l.id) {
            app.push(crate::proto::NodeExt { id: l.id, parent: Some(l.parent), focused: false, checked: None });
        }
    }
    app
}

/// What the shell sends the app besides configuration.
#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    Touch { kind: TouchKind, id: u32, x: f32, y: f32, t: f64 },
    Key(KeyInput),
    Open(String),
    Command(crate::Command),
    Activate(u64),
    Context {x:f32,y:f32},
    ContextAction(u32),
}

/// A file the user chose in an open or save panel (`NativeOpenPanel` / `NativeSavePanel`). The app gets the open file, never
/// its path; `grant` reopens it later (`Ctx::reopen_document`), like a security-scoped bookmark.
#[derive(Debug)]
pub struct Document {
    pub request: u32,
    pub grant: crate::proto::GrantId,
    /// the file's name, for the window title (not a path)
    pub name: String,
    pub size: u64,
    pub mode: crate::proto::DocMode,
    pub file: std::fs::File,
}

/// The answer to a document request, delivered to `App::document`.
#[derive(Debug)]
pub enum DocumentEvent {
    /// the file chosen in an open panel, or a reopened grant
    Opened(Document),
    /// the files chosen in an open panel that allows several (`OpenOptions::multiple`), in the panel's order
    /// (`NativeOpenPanel.urls`). With `multiple` the answer is always `OpenedMany`, even for one file.
    OpenedMany { request: u32, docs: Vec<Document> },
    /// a save panel's file: write it, then call `ctx.commit_document(request)` (or `abort_document`). Nothing at the
    /// chosen place is replaced until the commit.
    SaveTo(Document),
    /// the commit is done: `file` is the saved file and `grant` reopens it
    Saved(Document),
    /// the user cancelled (`DocErrorCode::Cancelled`) or the request was refused
    Failed { request: u32, code: crate::proto::DocErrorCode },
}

/// Request ids and which requests are saves (a save is answered twice: the temporary file, then the saved one).
#[derive(Default)]
struct DocState {
    next: u32,
    /// save request -> its temporary file has arrived
    saves: std::collections::HashMap<u32, bool>,
    /// multi-selection request -> (files still expected, files received)
    batches: std::collections::HashMap<u32, (u32, Vec<Document>)>,
}

/// `NativeOpenPanel` options for `Ctx::open_panel`. The default is the plain panel `open_document` shows: one file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenOptions {
    pub mode: crate::proto::DocMode,
    pub title: String,
    /// `allowedContentTypes`: UTIs or extensions
    pub filters: Vec<String>,
    /// `allowsMultipleSelection`
    pub multiple: bool,
    /// `canChooseFiles`
    pub files: bool,
    /// `canChooseDirectories`
    pub directories: bool,
    /// `canCreateDirectories`
    pub create_directories: bool,
}

impl Default for OpenOptions {
    fn default() -> Self {
        OpenOptions { mode: crate::proto::DocMode::Read, title: String::new(), filters: Vec::new(), multiple: false, files: true, directories: false, create_directories: false }
    }
}

impl OpenOptions {
    /// The wire bits (`proto::docopt`).
    pub fn bits(&self) -> u32 {
        use crate::proto::docopt::*;
        [(self.multiple, MULTIPLE), (self.directories, DIRECTORIES), (self.files, FILES), (self.create_directories, CREATE_DIRS)]
            .iter()
            .filter(|(on, _)| *on)
            .fold(0, |a, (_, b)| a | b)
    }
}

impl DocState {
    fn next(&mut self) -> u32 {
        self.next = self.next.wrapping_add(1).max(1);
        self.next
    }

    /// Turns a shell reply and its descriptor into the app's event.
    fn event(&mut self, msg: ToApp, file: Option<std::fs::File>) -> Option<DocumentEvent> {
        use crate::proto::DocErrorCode;
        match msg {
            ToApp::DocGranted { request, grant, mode, name, size } => {
                let Some(file) = file else {
                    self.saves.remove(&request);
                    return Some(DocumentEvent::Failed { request, code: DocErrorCode::Denied });
                };
                let doc = Document { request, grant, name, size, mode, file };
                if let Some((want, got)) = self.batches.get_mut(&request) {
                    got.push(doc);
                    if got.len() as u32 >= *want {
                        let (_, docs) = self.batches.remove(&request).unwrap();
                        return Some(DocumentEvent::OpenedMany { request, docs });
                    }
                    return None;
                }
                Some(match self.saves.get_mut(&request) {
                    Some(got) if !*got => {
                        *got = true;
                        DocumentEvent::SaveTo(doc)
                    }
                    Some(_) => {
                        self.saves.remove(&request);
                        DocumentEvent::Saved(doc)
                    }
                    None => DocumentEvent::Opened(doc),
                })
            }
            ToApp::DocBatch { request, count } => {
                self.batches.insert(request, (count, Vec::new()));
                // an empty batch has nothing to wait for
                (count == 0).then(|| {
                    self.batches.remove(&request);
                    DocumentEvent::OpenedMany { request, docs: Vec::new() }
                })
            }
            ToApp::DocError { request, code } => {
                self.saves.remove(&request);
                self.batches.remove(&request);
                Some(DocumentEvent::Failed { request, code })
            }
            _ => None,
        }
    }
}

/// Everything an app may read or ask for; handed to every `App` method.
pub struct Ctx {
    pub ui: Ui,
    pub cfg: Configure,
    /// `BIFROST_DATA`: the app's private storage directory
    pub data_dir: PathBuf,
    out: RefCell<Vec<ToShell>>,
    defaults: RefCell<crate::defaults::UserDefaults>,
    /// shared by every `Ctx` of this run (a new one is made on each Configure)
    docs: std::rc::Rc<RefCell<DocState>>,
}

thread_local! {
    static SUDDEN: RefCell<crate::process_info::SuddenTermination> = RefCell::new(Default::default());
    static ACTIVITIES: RefCell<crate::process_info::ActivityTracker> = RefCell::new(Default::default());
}

impl Ctx {
    /// A context with no shell behind it, for tests (the `XCTest` host-less app): memory-only defaults, no data
    /// directory. What the app sends to the shell is collected; read it with `take_sent`.
    pub fn testing(ui: Ui, cfg: Configure) -> Self {
        Ctx { ui, cfg, data_dir: PathBuf::new(), out: RefCell::new(Vec::new()), defaults: RefCell::new(crate::defaults::UserDefaults::memory()), docs: Default::default() }
    }
    /// The messages queued for the shell since the last call, oldest first, and empties the queue.
    pub fn take_sent(&self) -> Vec<ToShell> {
        std::mem::take(&mut *self.out.borrow_mut())
    }
    fn send(&self, m: ToShell) {
        self.out.borrow_mut().push(m);
    }
    /// `ProcessInfo.enableSuddenTermination`: the shell may kill the app at once on quit, logout or shutdown.
    /// Counted; the app starts disabled, so call it once at startup when nothing is ever left unsaved.
    pub fn enable_sudden_termination(&self) {
        if let Some(enabled) = SUDDEN.with(|s| s.borrow_mut().enable()) {
            self.send(ToShell::SuddenTermination { enabled });
        }
    }
    /// `ProcessInfo.disableSuddenTermination`: pair with `enable_sudden_termination` around work that must finish.
    pub fn disable_sudden_termination(&self) {
        if let Some(enabled) = SUDDEN.with(|s| s.borrow_mut().disable()) {
            self.send(ToShell::SuddenTermination { enabled });
        }
    }
    /// `NativeApplication.invalidateRestorableState()`: the state `App::encode_restorable_state` would save has changed. Calls
    /// are coalesced: the runtime writes once, `restoration::DEBOUNCE_MS` after the first call of a burst, never per
    /// frame. Until the write the app counts as unable to be killed at once (sudden termination is held).
    pub fn invalidate_restorable_state(&self) {
        if crate::restoration::invalidate(now_ms()) {
            self.disable_sudden_termination();
        }
    }
    /// Balances the hold `invalidate_restorable_state` took, once the state is written.
    pub(crate) fn finish_restorable_write(&self) {
        self.enable_sudden_termination();
    }
    /// `ProcessInfo.beginActivity(options:reason:)`: while a user-initiated or latency-critical activity is live the
    /// shell does not suspend the app when it leaves the foreground. Pair with `end_activity`.
    pub fn begin_activity(&self, options: crate::process_info::ActivityOptions, reason: &str) -> crate::process_info::Activity {
        let (token, changed) = ACTIVITIES.with(|a| a.borrow_mut().begin(options, reason));
        if let Some(options) = changed {
            self.send(ToShell::Activity { options });
        }
        token
    }
    /// `ProcessInfo.endActivity`.
    pub fn end_activity(&self, activity: crate::process_info::Activity) {
        if let Some(options) = ACTIVITIES.with(|a| a.borrow_mut().end(activity)) {
            self.send(ToShell::Activity { options });
        }
    }
    /// A fresh request id (shared with the document requests), for `keychain`.
    pub(crate) fn next_request(&self) -> u32 {
        self.docs.borrow_mut().next()
    }
    pub(crate) fn send_to_shell(&self, m: ToShell) {
        self.send(m);
    }
    /// The app's preferences (`NativeUserDefaults.standard`): read with `ctx.defaults().string(key)`.
    pub fn defaults(&self) -> std::cell::Ref<'_, crate::defaults::UserDefaults> {
        self.defaults.borrow()
    }
    /// Changes the preferences; each change is saved at once: `ctx.defaults_mut().set_bool(key, true)`.
    pub fn defaults_mut(&self) -> std::cell::RefMut<'_, crate::defaults::UserDefaults> {
        self.defaults.borrow_mut()
    }
    /// The text on the general pasteboard (`NativePasteboard.general.string(forType: .string)`), or `None`.
    pub fn pasteboard_string(&self) -> Option<String> {
        kit::clipboard::text()
    }
    /// Puts text on the general pasteboard; the shell learns of it when the app's loop turns.
    pub fn set_pasteboard_string(&self, text: &str) {
        kit::clipboard::set_text(text);
    }
    /// The system appearance settings (Reduce Motion, Increase Contrast, Reduce Transparency, text size).
    pub fn appearance(&self) -> kit::appearance::Appearance {
        kit::appearance::current()
    }
    /// Ask the authenticated system broker to change Remote sharing.
    pub fn context_menu(&self,x:f32,y:f32,items:Vec<crate::proto::ContextItem>){self.send(ToShell::ContextMenu{x,y,items});}
    pub fn remote_sharing(&self,on:bool) {self.send(ToShell::RemoteSharing{on});}
    pub fn haptic(&self) {
        self.send(ToShell::Haptic);
    }
    /// The announcement-requested accessibility notification: a screen reader speaks `text` now, without moving
    /// focus. `Priority::High` interrupts (assertive); low and medium wait their turn (polite). The shell turns it into an
    /// AT-SPI `Object:Announcement` event. Use it for results the user cannot see change, such as "3 files copied".
    pub fn announce(&self, text: &str, priority: crate::proto::AnnouncePriority) {
        if !text.is_empty() {
            self.send(ToShell::Announce { text: text.into(), priority });
        }
    }
    /// Report the focused field's surrounding text, selection and marked range (`NativeTextInput`), for predictive text.
    pub fn text_state(&self, state: crate::proto::TextState) {
        self.send(ToShell::TextState(state));
    }
    /// Report the caret rectangle in window points (`firstRectForCharacterRange:`) so the candidate window or the
    /// keyboard can position itself.
    pub fn caret_rect(&self, x: f32, y: f32, w: f32, h: f32) {
        self.send(ToShell::CaretRect { x, y, w, h });
    }
    /// Report the focused field's `NativeTextInputTraits` (return key type, content kind, flags `tiflags::*`).
    pub fn input_traits(&self, traits: crate::proto::InputTraits) {
        self.send(ToShell::InputTraits(traits));
    }
    /// Set the `inputAccessoryView` buttons; an empty list removes the row. Pressing one calls `App::text_input`
    /// with `ToApp::AccessoryAction(id)`.
    pub fn input_accessory(&self, items: Vec<crate::proto::AccessoryItem>) {
        self.send(ToShell::InputAccessory(items));
    }
    pub fn wants_keyboard(&self, on: bool) {
        self.send(ToShell::WantsKeyboard { on });
    }
    /// Keyboard system app only: send what the user typed to the focused field (`NativeKeyInput`:
    /// `insertText`, `deleteBackward`, the return key) or report the keyboard's height. The shell ignores this
    /// from any other app.
    pub fn keyboard_out(&self, out: crate::proto::KeyboardOut) {
        self.send(ToShell::KeyboardOut(out));
    }
    pub fn keyboard_insert_text(&self, text: &str) {
        self.keyboard_out(crate::proto::KeyboardOut::Text(text.into()));
    }
    pub fn keyboard_delete_backward(&self) {
        self.keyboard_out(crate::proto::KeyboardOut::Backspace);
    }
    pub fn keyboard_height(&self, px: u32) {
        self.keyboard_out(crate::proto::KeyboardOut::Height(px));
    }
    pub fn open_url(&self, url: &str) {
        self.send(ToShell::OpenUrl { url: url.into() });
    }
    pub fn notify(&self, title: &str, body: &str) {
        self.send(ToShell::Notify { title: title.into(), body: body.into() });
    }
    /// Sets the app's dock badge (`NativeDockTile.badgeLabel`), such as an unread count. The shell shows at most
    /// four characters; an empty label clears it. Needs the `notifications` permission.
    pub fn set_badge(&self, label: &str) {
        self.send(ToShell::SetBadge { label: label.into() });
    }
    pub fn log(&self, text: &str) {
        self.send(ToShell::Log { text: text.into() });
    }

    /// Shows the open panel (`NativeOpenPanel.begin`). `filters` are UTIs (`"public.text"`) or extensions (`"md"`); other
    /// files are dimmed. The answer arrives in `App::document` with the returned request id. Needs the `documents`
    /// permission; without it the answer is `Failed { code: Unsupported }`.
    pub fn open_document(&self, mode: crate::proto::DocMode, title: &str, filters: &[&str]) -> u32 {
        let request = self.docs.borrow_mut().next();
        self.send(ToShell::DocOpen { request, mode, title: title.into(), filters: filters.iter().map(|f| f.to_string()).collect() });
        request
    }

    /// Shows the open panel with `NativeOpenPanel` options: several files, folders, New Folder. A panel with
    /// `multiple` answers `OpenedMany`; otherwise `Opened`. The grant of a chosen folder reopens the folder.
    pub fn open_panel(&self, options: &OpenOptions) -> u32 {
        let request = self.docs.borrow_mut().next();
        self.send(ToShell::DocOpenEx { request, mode: options.mode, title: options.title.clone(), filters: options.filters.clone(), options: options.bits() });
        request
    }

    /// Shows the save panel (`NativeSavePanel.begin`) with `suggested_name` in its field. The answer is `SaveTo` with a
    /// file to write; then `commit_document` puts it in place.
    pub fn save_document(&self, suggested_name: &str, filters: &[&str]) -> u32 {
        let request = {
            let mut d = self.docs.borrow_mut();
            let r = d.next();
            d.saves.insert(r, false);
            r
        };
        self.send(ToShell::DocSave { request, suggested_name: suggested_name.into(), filters: filters.iter().map(|f| f.to_string()).collect() });
        request
    }

    /// Saves over a file already granted, without a panel (`NativeDocument` save in place, autosave). The answer is
    /// `SaveTo` with a temporary file next to the original; write it, then `commit_document` renames it over the
    /// file (crash-safe) and answers `Saved` with the same grant. Needs a `Write` grant.
    pub fn save_document_in_place(&self, grant: crate::proto::GrantId) -> u32 {
        let request = {
            let mut d = self.docs.borrow_mut();
            let r = d.next();
            d.saves.insert(r, false);
            r
        };
        self.send(ToShell::DocSaveGrant { request, grant });
        request
    }

    /// Tells the shell the window's document state (`NativeDocument`): the title bar shows `title` (with "- Edited"), the
    /// close light shows the dirty dot while `edited`, and `represented` is the file name. Sent by `document::DocumentController`.
    pub fn set_document_state(&self, title: &str, edited: bool, represented: &str) {
        self.send(ToShell::DocumentState { title: title.into(), edited, represented: represented.into() });
    }

    /// Puts a written save in place (atomic replace). Answered with `Saved` or `Failed`.
    pub fn commit_document(&self, request: u32) {
        self.send(ToShell::DocCommit { request });
    }

    /// Throws a save away; the file at the chosen place is untouched.
    pub fn abort_document(&self, request: u32) {
        self.docs.borrow_mut().saves.remove(&request);
        self.send(ToShell::DocAbort { request });
    }

    /// Opens a file chosen earlier again, by its grant (a recent document). `Failed { code: Stale }` when the file
    /// was replaced since.
    pub fn reopen_document(&self, grant: crate::proto::GrantId) -> u32 {
        let request = self.docs.borrow_mut().next();
        self.send(ToShell::DocReopen { request, grant });
        request
    }

    /// The app no longer needs the grant (the document left the recent list).
    pub fn release_document(&self, grant: crate::proto::GrantId) {
        self.send(ToShell::DocRelease { grant });
    }

    /// A read grant for the file or folder at `path` (absolute, under the account's home), for the file items of a drag
    /// (`dnd::begin`). Answered with `Opened` (the grant and the descriptor) or `Failed`. Needs the `documents` permission.
    pub fn grant_path(&self, path: &str) -> u32 {
        let request = self.docs.borrow_mut().next();
        self.send(ToShell::DocGrantPath { request, path: path.into() });
        request
    }
}

/// Takes ownership of a received descriptor as a `File`.
fn into_file(o: Owned) -> std::fs::File {
    use std::os::fd::FromRawFd;
    let o = std::mem::ManuallyDrop::new(o);
    // SAFETY: the descriptor arrived with the packet and only `o` owned it; ManuallyDrop keeps it from closing twice.
    unsafe { std::fs::File::from_raw_fd(o.0) }
}

struct Slot {
    map: Map,
    held: bool,
    /// what the other buffer drew since this one was last drawn into
    stale: Vec<Rect>,
    stale_all: bool,
}

/// Whether the SDK can bind a layer context on its Wayland connection (`wl::connection`). The direct path needs the
/// connection to dispatch the `bifrost_layer_*` objects, which it does not yet (todo_left in
/// `docs/graphics/g2-sdk-direct-runtime.md`). Until it does, the direct path is never entered.
pub(crate) const LAYER_CONTEXT_WIRED: bool = false;

/// Why the direct path is not taken, in the order the checks run. None means the app may run as a Wayland client.
/// Connecting is the first side effect (it creates the app's toplevel), so every check that can be made before it is
/// made here: the switch, the Vulkan device, the compositor's `bifrost_layer_manager_v1`, and the layer context.
pub(crate) fn direct_fallback_reason(requested: bool, vulkan: bool, layer_manager: bool, context_wired: bool) -> Option<&'static str> {
    if !requested {
        Some("BIFROST_DIRECT is not 1")
    } else if !vulkan {
        Some("no Vulkan device")
    } else if !layer_manager {
        Some("the compositor does not offer bifrost_layer_manager_v1")
    } else if !context_wired {
        Some("the connection cannot bind layer contexts yet")
    } else {
        None
    }
}

/// Runs `app` on the socket the shell passed in `BIFROST_SOCK` until it quits.
pub fn run<A: App>(app: A) {
    let fd: i32 = std::env::var("BIFROST_SOCK").ok().and_then(|v| v.parse().ok()).unwrap_or_else(|| {
        eprintln!("bifrost-app: BIFROST_SOCK is not set; apps are started by SpringBoard");
        std::process::exit(2)
    });
    let data = PathBuf::from(std::env::var("BIFROST_DATA").unwrap_or_else(|_| ".".into()));
    let shell_dir = std::env::var("BIFROST_SHELL_DIR").unwrap_or_else(|_| "/var/lib/springboard".into());
    let _ = std::fs::create_dir_all(&data);
    run_on(Channel::from_fd(fd), app, data, &shell_dir);
}

/// `run` on an explicit channel (tests, embedding).
pub fn run_on<A: App>(chan: Channel, mut app: A, data_dir: PathBuf, shell_dir: &str) {
    let _ = chan.send(&ToShell::Hello { sdk_version: SDK_VERSION, app_id: app.id().into() }.encode(), &[]);
    if crate::wl::requested() {
        // the Vulkan probe and the layer manager are only known after connecting, which this build never does yet
        if let Some(reason) = direct_fallback_reason(true, false, false, LAYER_CONTEXT_WIRED) {
            eprintln!("bifrost-app: BIFROST_DIRECT=1, shared-memory path: {reason}");
        }
    }
    // an app has no keyboard of its own: its `Keyboard` asks the shell for the system keyboard
    springboard::shell::keyboard::use_app_backend();
    // mapped, not read: every app shares the shell's page-cache copy of the 8 MB font
    // the system font the shell uses, with the Settings > Fonts override
    let font_data: &'static [u8] = match bifrost_render::fontprefs::map_font(shell_dir, bifrost_render::fontprefs::FontRole::System) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("bifrost-app: system font in {shell_dir}: {e}");
            return;
        }
    };
    let mut symbols = Some(bifrost_render::symbols::Symbols::load(shell_dir));
    let mut ctx: Option<Ctx> = None;
    let mut pending_activity: Vec<crate::quicksearch::UserActivity> = Vec::new();
    let mut slots: Option<[Slot; 2]> = None;
    let mut last: Option<usize> = None;
    let mut first_frame = true;
    let mut restore_tried = false;
    let mut repaint_suspended = false;
    let mut need_frame = false;
    let mut active = true;
    // plain touches or touch events, whichever the app takes (touch_events.rs)
    let mut touch_router = TouchRouter::new(1.0);
    let mut size = (0u32, 0u32);
    // C1: counts the sets of buffers sent (wrapping, from 1); a `Release` for another set is stale
    let mut generation = 0u32;
    // the last frame repainted everything: the next one is likely to as well, so its copy-up waits for the damage
    let mut last_all = false;
    let mut first_turn_flag = true;
    let mut clip_seen = kit::clipboard::text();
    let mut open = true;
    let mut commands_seen = None;
    let mut undo_titles_seen: (String, String) = Default::default();
    let mut menus_seen: Vec<crate::proto::AppMenu> = Vec::new();
    let mut controls_seen = Vec::new();
    let mut tree_seen: Vec<crate::proto::NodeExt> = Vec::new();
    let mut ax_text_seen: Vec<bifrost_ui::ax::AxText> = Vec::new();
    let mut ax_meta_seen: Vec<bifrost_ui::ax::AxMeta> = Vec::new();
    // what the kit's draw helpers recorded in the last frame: the element map of an app that declares no controls itself
    let mut drawn_nodes: Vec<bifrost_ui::Node> = Vec::new();
    let mut drawn_links: Vec<bifrost_ui::ax::AxLink> = Vec::new();
    // the behind-window materials the shell last got (`ToShell::Materials`)
    let mut materials_seen: Vec<crate::proto::Material> = Vec::new();
    let mut materials: Vec<crate::proto::Material> = Vec::new();
    let mut control = control_endpoint(&app.id(), &data_dir);
    let docs = std::rc::Rc::new(RefCell::new(DocState::default()));

    // Blocks for the first Configure; later ones arrive in the loop.
    while open {
        let t = now_ms();
        let first_turn = std::mem::take(&mut first_turn_flag);
        let have_ctx = ctx.is_some();
        // wait: forever when idle, 0 when a frame is due, else until the app's next wake
        let free = slots.as_ref().is_some_and(|s| s.iter().any(|b| !b.held));
        let timeout = if !have_ctx || !active {
            None
        } else if need_frame && free {
            Some(0)
        } else if need_frame {
            None
        } else {
            app.next_wake(t).map(|w| (w - t).clamp(0.0, i32::MAX as f64).ceil() as i32)
        };
        // async control calls (control::RpcError::pending) are completed and timed out from `service`: poll them
        let timeout = if control.as_ref().is_some_and(|c| c.has_pending()) { Some(timeout.map_or(50, |t| t.min(50))) } else { timeout };
        // a coalesced restorable-state write is due at its deadline
        let timeout = match crate::restoration::deadline() {
            Some(d) if have_ctx => {
                let ms = (d - t).max(1.0).ceil() as i32;
                Some(timeout.map_or(ms, |w| w.min(ms)))
            }
            _ => timeout,
        };
        // the app's own descriptor (`App::wake_fd`, a web view engine for example) wakes the loop for `tick`
        let app_fd = if active { app.wake_fd() } else { None };
        let (chan_ready, ready) = chan.wait_many(&[control.as_ref().map_or(-1, |c| c.fd()), app_fd.unwrap_or(-1), crate::dispatch::main_fd()], timeout);
        let control_ready = ready[0];
        // main-queue work (`DispatchQueue::main().async_`) runs here, before anything draws
        // whether anything happened this turn that could change what the app publishes (controls, menus, titles)
        let mut busy = ready[1] || ready[2] || control_ready || control.as_ref().is_some_and(|c| c.has_pending());
        if ready[2] && crate::dispatch::drain_main() {
            need_frame = true;
        }
        if control_ready || control.as_ref().is_some_and(|c| c.has_pending()) {
            if let Some(c) = control.as_mut() {
                let ctx_ref = ctx.as_ref();
                c.service(&mut |method, params| serve_rpc(&mut app, ctx_ref, method, params, &data_dir));
            }
        }
        if !chan_ready && !have_ctx {
            continue;
        }
        // drain; consecutive moves of one touch are held back and only the last is delivered
        let mut held_move: Option<(u32, f32, f32, u32)> = None;
        loop {
            let (pkt, mut fds) = match chan.recv(false) {
                Ok(Recv::Packet(p, f)) => (p, f),
                Ok(Recv::Empty) => break,
                Ok(Recv::Closed) | Err(_) => {
                    open = false;
                    break;
                }
            };
            let Ok(msg) = ToApp::decode(&pkt) else { continue };
            let now = now_ms();
            busy = true;
            if let ToApp::Touch { kind: TouchKind::Move, id, x, y, t_ms } = msg {
                if let Some(h) = held_move.replace((id, x, y, t_ms)) {
                    if h.0 != id {
                        need_frame |= deliver_move(&mut app, ctx.as_ref(), &mut touch_router, h, now);
                    }
                }
                continue;
            }
            // a touch frame leaves the held plain move alone: the frame of every sample would otherwise defeat the
            // coalescing for apps that take plain touches
            if let ToApp::Touches(mut ev) = msg {
                if let Some(c) = &ctx {
                    localize(&mut ev, now);
                    if let Some(ev) = touch_router.frame(app.wants_touch_events(), ev) {
                        need_frame |= app.touches(c, &ev);
                    }
                }
                continue;
            }
            if let Some(h) = held_move.take() {
                need_frame |= deliver_move(&mut app, ctx.as_ref(), &mut touch_router, h, now);
            }
            match msg {
                ToApp::Configure(cfg) => {
                    springboard::shell::keyboard::set_app_inset(cfg.safe_bottom);
                    // the scheme the shell is showing (Configure::dark = dark): every Pair colour the app draws follows it
                    kit::appearance::jump_to(!cfg.dark);
                    // the first Configure builds the context (the font is parsed once); later ones update it in place,
                    // so the glyph cache, Bold Text, the defaults and the queued messages all survive a resize
                    match ctx.as_mut() {
                        Some(c) => reconfigure(c, cfg),
                        None => {
                            let mut ui = Ui::new(Font::parse(font_data).expect("SFUI.ttf"), cfg.scale);
                            ui.text_scale = cfg.text_scale;
                            ui.bold = kit::appearance::current().bold;
                            ui.symbols = symbols.take().unwrap_or_default();
                            ctx = Some(Ctx { ui, cfg, defaults: RefCell::new(crate::defaults::UserDefaults::open(&data_dir)), data_dir: data_dir.clone(), out: RefCell::new(Vec::new()), docs: docs.clone() });
                        }
                    }
                    crate::layout::set_from(&cfg);
                    touch_router.set_scale(cfg.scale);
                    let c = ctx.as_ref().unwrap();
                    app.configure(c);
                    // `restoreState(with:)`: once, when the shell asked this launch to restore
                    if !restore_tried {
                        restore_tried = true;
                        if crate::restoration::restore_requested() {
                            // the Configure below sets `need_frame` anyway, so the restore's "changed" flag is not needed here
                            crate::restoration::restore_app(&mut app, c);
                        }
                    }
                    if size != (cfg.w, cfg.h) || slots.is_none() {
                        size = (cfg.w, cfg.h);
                        generation = generation.wrapping_add(1).max(1);
                        match alloc(&chan, cfg.w, cfg.h, generation) {
                            Some(s) => slots = Some(s),
                            None => {
                                eprintln!("bifrost-app: cannot allocate frame buffers");
                                return;
                            }
                        }
                        last = None;
                        first_frame = true;
                    }
                    need_frame = true;
                }
                ToApp::Context{x,y}=>{if let Some(ctx)=&ctx {need_frame|=app.event(ctx,Input::Context{x,y});}},
                ToApp::MenuAction(id)=>{if let Some(ctx)=&ctx {need_frame|=app.menu_action(ctx,id);}},
                ToApp::ContextAction(id)=>{if let Some(ctx)=&ctx {need_frame|=app.event(ctx,Input::ContextAction(id));}},
                ToApp::Touch { kind, id, x, y, t_ms } => {
                    if let Some(c) = &ctx {
                        need_frame |= deliver_touch(&mut app, c, &mut touch_router, kind, id, x, y, event_time(now, t_ms));
                    }
                }
                ToApp::Touches(_) => {}
                ToApp::GuidedAccessState { active } => {
                    if let Some(c) = &ctx {
                        need_frame |= app.guided_access(c, crate::guided_access::GuidedAccessChange::Session(active));
                    }
                }
                ToApp::GuidedRestriction { id, allowed } => {
                    if let Some(c) = &ctx {
                        need_frame |= app.guided_access(c, crate::guided_access::GuidedAccessChange::Restriction { id, allowed });
                    }
                }
                ToApp::Key(k) => {
                    if let Some(c) = &ctx {
                        need_frame |= app.event(c, Input::Key(k));
                    }
                }
                ToApp::AxSet { id, action } => {if let Some(c)=&ctx {need_frame |= app.ax_set(c,id,action);}}
                ToApp::Activate(id) => {if let Some(c)=&ctx {need_frame |= app.event(c,Input::Activate(id));}}
                ToApp::Command(command) => {if let Some(c)=&ctx {need_frame |= app.event(c,Input::Command(command));}}
                ToApp::Open { url } => {
                    if let Some(c) = &ctx {
                        need_frame |= app.event(c, Input::Open(url));
                    }
                }
                ToApp::Release { buf, generation: g } => {
                    if g == generation {
                        if let Some(b) = slots.as_mut().and_then(|s| s.get_mut(buf as usize)) {
                            b.held = false;
                        }
                    }
                }
                ToApp::Resume => {
                    active = true;
                    if let Some(c) = &ctx {
                        app.resume(c);
                    }
                    need_frame = true;
                }
                ToApp::Suspend => {
                    active = false;
                    if let Some(c) = &ctx {
                        // touches still down on the plain stream end here (`touchesCancelled`)
                        if let Some(ev) = touch_router.cancel_all(app.wants_touch_events(), now) {
                            need_frame |= app.touches(c, &ev);
                        }
                        app.suspend(c);
                        flush_restorable(&app, c, true);
                    }
                }
                ToApp::Quit => {
                    if let Some(c) = &ctx {
                        if active {
                            app.suspend(c);
                        }
                        flush_restorable(&app, c, true);
                    }
                    open = false;
                }
                ToApp::Appearance { flags } => {
                    use crate::proto::appearance::*;
                    // Publish to this process's appearance hooks so every drawing helper (contrast, opaque materials,
                    // borders) follows the system setting, and repaint.
                    let mut a = kit::appearance::current();
                    a.increase_contrast = flags & INCREASE_CONTRAST != 0;
                    a.reduce_transparency = flags & REDUCE_TRANSPARENCY != 0;
                    a.button_shapes = flags & SHOW_BORDERS != 0;
                    a.bold = flags & BOLD_TEXT != 0;
                    kit::appearance::publish(a);
                    kit::appearance::jump_to(flags & LIGHT != 0);
                    let vibrant = flags & VIBRANCY != 0 && flags & REDUCE_TRANSPARENCY == 0;
                    crate::widgets::set_vibrancy(vibrant, flags & MATERIALS != 0);
                    if !vibrant {
                        materials.clear();
                    }
                    if let Some(c) = ctx.as_mut() {
                        c.ui.bold = a.bold;
                        c.cfg.dark = flags & LIGHT == 0;
                    }
                    // every pixel changes colour: `configure` is the app's "appearance changed" hook (it drops
                    // its damage tracking), and the frame is sent whole so no region keeps the old scheme
                    if let Some(c) = &ctx {
                        app.configure(c);
                    }
                    first_frame = true;
                    need_frame = true;
                    // a background window is still on screen on the desktop: repaint it once even while suspended
                    repaint_suspended = true;
                }
                m @ (ToApp::SetMarkedText { .. } | ToApp::UnmarkText { .. } | ToApp::DeleteSurrounding { .. } | ToApp::AccentHold { .. } | ToApp::AccessoryAction(_)) => {
                    if let Some(c) = &ctx {
                        need_frame |= app.text_input(c, m);
                    }
                }
                ToApp::KeyboardConfig(config) => {
                    if let Some(c) = &ctx {
                        need_frame |= app.keyboard_config(c, config);
                    }
                }
                ToApp::Pasteboard { change_count, items } => crate::pasteboard::mirror(change_count, items),
                // the shell's input source: the kana (Romaji) flag the text input reads (`compose_method`)
                ToApp::InputSource { japanese } => bifrost_ui::compose_method::set_japanese_source(japanese),
                ToApp::Clipboard { text } => {
                    kit::clipboard::set_text(&text);
                    clip_seen = kit::clipboard::text();
                }
                ToApp::NotificationAction { id, action, text, thread, category } => {
                    if let Some(c) = &ctx {
                        need_frame |= app.notification(c, crate::usernotif::Event::response(id, action, text, thread, category));
                    }
                }
                ToApp::NotificationStatus { request, status } => {
                    if let Some(c) = &ctx {
                        need_frame |= app.notification(c, crate::usernotif::Event::Status { request, status });
                    }
                }
                ToApp::NotificationList { request, delivered, ids } => {
                    if let Some(c) = &ctx {
                        need_frame |= app.notification(c, crate::usernotif::Event::List { request, delivered, ids });
                    }
                }
                ToApp::NotificationSettings { request, settings } => {
                    if let Some(c) = &ctx {
                        need_frame |= app.notification(c, crate::usernotif::Event::Settings { request, settings });
                    }
                }
                ToApp::LoginItemStatus { request, status } => {
                    crate::service_management::mirror(status);
                    crate::service_management::complete(request, status);
                    if let Some(c) = &ctx {
                        need_frame |= app.login_item_status(c, request, status);
                    }
                }
                ToApp::StatusItemAction { item, id } => {
                    if let Some(c) = &ctx {
                        need_frame |= app.status_item_action(c, item, id);
                    }
                }
                ToApp::DistNotify { name, user_info } => {
                    need_frame |= crate::notification::distributed_incoming(&name, user_info.into_iter().collect());
                }
                ToApp::ServiceReply { request, ok, body } => {
                    let result = crate::service::decode_reply(ok, &body);
                    let taken = ctx.as_ref().is_some_and(|c| app.service_reply(c, request, &result));
                    if taken {
                        need_frame = true;
                    } else {
                        crate::service::keep(request, result);
                    }
                }
                ToApp::ServiceEvent { service, event, body } => {
                    let body = bifrost_core::json::parse(&body).unwrap_or(bifrost_core::json::Json::Null);
                    if let Some(c) = &ctx {
                        need_frame |= app.service_event(c, &service, &event, &body);
                    }
                }
                ToApp::WorkspaceReply { request, status, items } => {
                    need_frame |= crate::workspace::reply_incoming(request, status, items);
                }
                ToApp::WorkspaceEvent { kind, app_id, name } => {
                    need_frame |= crate::workspace::event_incoming(kind, app_id, name);
                }
                ToApp::ShareDone { request, target } => {
                    if let Some(c) = &ctx {
                        need_frame |= app.share_done(c, request, (!target.is_empty()).then_some(target));
                    }
                }
                ToApp::DragDest { session, phase, x, y, allowed, items } => {
                    if let Some(c) = &ctx {
                        need_frame |= app.drag(c, crate::dnd::Event::Dest { session, phase, x, y, allowed, items });
                    }
                }
                ToApp::DragEnded { session, op } => {
                    if let Some(c) = &ctx {
                        need_frame |= app.drag(c, crate::dnd::Event::Ended { session, op });
                    }
                }
                ToApp::ContinueActivity { activity_type, unique_id, domain } => {
                    // the shell sends it right after starting the app, possibly before the first Configure
                    pending_activity.push(crate::quicksearch::UserActivity { activity_type, unique_identifier: unique_id, domain_identifier: domain });
                }
                ToApp::KeychainReply { request, status, value } => {
                    if let Some(c) = &ctx {
                        need_frame |= app.keychain(c, crate::keychain::Reply::from_wire(request, status, value));
                    }
                }
                m @ (ToApp::DocGranted { .. } | ToApp::DocError { .. } | ToApp::DocBatch { .. }) => {
                    let file = if matches!(m, ToApp::DocBatch { .. }) { None } else { fds.pop().map(into_file) };
                    let ev = docs.borrow_mut().event(m, file);
                    if let (Some(c), Some(ev)) = (&ctx, ev) {
                        need_frame |= app.document(c, ev);
                    }
                }
            }
        }
        if let Some(h) = held_move.take() {
            need_frame |= deliver_move(&mut app, ctx.as_ref(), &mut touch_router, h, now_ms());
        }
        if !open {
            break;
        }
        let Some(c) = &ctx else { continue };
        for activity in pending_activity.drain(..) {
            need_frame |= app.continue_activity(c, activity);
        }
        let t = now_ms();
        if active && app.tick(c, t) {
            need_frame = true;
            busy = true;
        }
        flush_restorable(&app, c, false);
        if (active || repaint_suspended) && need_frame {
            if let Some(slots) = slots.as_mut() {
                let target = (0..2).find(|&i| !slots[i].held && Some(i) != last).or_else(|| (0..2).find(|&i| !slots[i].held));
                if let Some(i) = target {
                    need_frame = false;
                    bifrost_ui::ax::begin_frame();
                    let _ = crate::widgets::take_frame_materials();
                    let full = first_frame || app.full_redraw();
                    let sent = draw_into(&chan, &mut app, c, slots, i, t, first_frame, &mut last_all);
                    // a partial redraw may skip the sidebar: keep the regions it declared until a frame draws some
                    // (or a full redraw draws none)
                    let drawn = crate::widgets::take_frame_materials();
                    if sent && (full || !drawn.is_empty()) {
                        materials = drawn;
                    }
                    if sent {
                        busy = true;
                        drawn_links = bifrost_ui::ax::take_links();
                        drawn_nodes = bifrost_ui::ax::take_frame();
                        first_frame = false;
                        repaint_suspended = false;
                        last = Some(i);
                    }
                }
            }
        }
        if materials!=materials_seen {let _=chan.send(&ToShell::Materials(materials.clone()).encode(),&[]);materials_seen=materials.clone();}
        if busy || first_turn {
        let controls=merge_controls(app.controls(c),&drawn_nodes);
        if controls!=controls_seen {let _=chan.send(&ToShell::Controls(controls.clone()).encode(),&[]);controls_seen=controls;}
        let tree=merge_tree(app.controls_tree(c),&drawn_links);
        if tree!=tree_seen {let _=chan.send(&ToShell::ControlsTree(tree.clone()).encode(),&[]);tree_seen=tree;}
        let ax_text=app.ax_text(c);
        if ax_text!=ax_text_seen {let _=chan.send(&ToShell::AxText(ax_text.clone()).encode(),&[]);ax_text_seen=ax_text;}
        let ax_meta=app.ax_meta(c);
        if ax_meta!=ax_meta_seen {let _=chan.send(&ToShell::AxMeta(ax_meta.clone()).encode(),&[]);ax_meta_seen=ax_meta;}
        let commands=app.commands(c);
        let menus=app.main_menu(c);
        if menus!=menus_seen {let _=chan.send(&ToShell::AppMenus(menus.clone()).encode(),&[]);menus_seen=menus;}
        if commands_seen != Some(commands) {commands_seen=Some(commands);let _=chan.send(&ToShell::Commands(commands).encode(),&[]);}
        let titles=app.undo_titles(c);
        if titles!=undo_titles_seen {let _=chan.send(&ToShell::UndoTitles{undo:titles.0.clone(),redo:titles.1.clone()}.encode(),&[]);undo_titles_seen=titles;}
        }
        // the system keyboard was asked for (or dismissed) through a `Keyboard`
        if let Some(on) = springboard::shell::keyboard::take_app_request() {
            c.wants_keyboard(on);
        }
        // flush requests and clipboard changes
        for m in c.out.borrow_mut().drain(..) {
            let _ = chan.send(&m.encode(), &[]);
        }
        let now_clip = kit::clipboard::text();
        if now_clip != clip_seen {
            clip_seen = now_clip.clone();
            let _ = chan.send(&ToShell::SetClipboard { text: now_clip.unwrap_or_default() }.encode(), &[]);
        }
    }
}

/// Writes the restorable state when a coalesced write is due (`force`: at once, for suspend and quit), then releases
/// the sudden-termination hold `invalidate_restorable_state` took.
fn flush_restorable<A: App>(app: &A, ctx: &Ctx, force: bool) {
    let due = if force { crate::restoration::take_now() } else { crate::restoration::take_due(now_ms()) };
    if due {
        crate::restoration::save_app(app, ctx);
        ctx.finish_restorable_write();
    }
}

/// When a touch happened, from the shell's stamp (the low 32 bits of the same monotonic ms clock).
/// Moves arrive in batches, so stamping them with the time they are read would give several the
/// same time and break scroll velocity; a stamp that is far off (another clock) falls back to `now`.
fn event_time(now: f64, t_ms: u32) -> f64 {
    let ago = (now as u64 as u32).wrapping_sub(t_ms) as f64;
    if ago < 1000.0 { now - ago } else { now }
}

/// Applies a new `Configure` to the live context: size, scale and Dynamic Type change; Bold Text, the glyph cache,
/// the defaults and the messages still queued in `out` stay.
fn reconfigure(c: &mut Ctx, cfg: Configure) {
    c.ui.s = cfg.scale;
    c.ui.text_scale = cfg.text_scale;
    c.cfg = cfg;
}

fn alloc(chan: &Channel, w: u32, h: u32, generation: u32) -> Option<[Slot; 2]> {
    let bytes = (w as usize * h as usize * 4).max(4);
    let mk = |n: &str| -> Option<(Owned, Slot)> {
        let fd = shm_create(n, bytes).ok()?;
        let map = Map::new(&fd, bytes).ok()?;
        Some((fd, Slot { map, held: false, stale: Vec::new(), stale_all: true }))
    };
    let (fa, a) = mk("bifrost-frame-0")?;
    let (fb, b) = mk("bifrost-frame-1")?;
    chan.send(&ToShell::Buffers { w, h, generation }.encode(), &[fa.0, fb.0]).ok()?;
    Some([a, b])
}

/// Delivers the last of a run of moves of one touch (`t_ms` is the shell's stamp of that last sample).
fn deliver_move<A: App>(app: &mut A, ctx: Option<&Ctx>, router: &mut TouchRouter, m: (u32, f32, f32, u32), now: f64) -> bool {
    let Some(c) = ctx else { return false };
    deliver_touch(app, c, router, TouchKind::Move, m.0, m.1, m.2, event_time(now, m.3))
}

/// Delivers a plain touch: as `Input::Touch`, or as a touch event for an app that takes them (`App::wants_touch_events`).
#[allow(clippy::too_many_arguments)]
fn deliver_touch<A: App>(app: &mut A, c: &Ctx, router: &mut TouchRouter, kind: TouchKind, id: u32, x: f32, y: f32, t: f64) -> bool {
    match router.plain(app.wants_touch_events(), kind, id, x, y, t) {
        Plain::Deliver => app.event(c, Input::Touch { kind, id, x, y, t }),
        Plain::Frame(ev) => app.touches(c, &ev),
        Plain::Skip => false,
    }
}

/// Draws into slot `i`, brings it up to date with the other buffer, and sends the frame. Returns whether a frame went
/// out. An app whose last frame repainted everything (`last_all`) is drawn first and only then is the part of the
/// stale area its damage did not cover copied up, so a full repaint costs no copy at all.
fn draw_into<A: App>(chan: &Channel, app: &mut A, ctx: &Ctx, slots: &mut [Slot; 2], i: usize, t: f64, first: bool, last_all: &mut bool) -> bool {
    let (w, h) = (ctx.cfg.w as usize, ctx.cfg.h as usize);
    let (lo, hi) = slots.split_at_mut(1);
    let (cur, other) = if i == 0 { (&mut lo[0], &mut hi[0]) } else { (&mut hi[0], &mut lo[0]) };
    let t0 = now_ms();
    let full = app.full_redraw();
    let defer = !full && *last_all && !first;
    if full {
        cur.stale.clear();
    } else if !defer {
        copy_up(cur, other, w, h, &[]);
    }
    let t1 = now_ms();
    let mut surface = Surface { w, h, stride: w, px: cur.map.pixels_mut(), y0: 0 };
    let mut damage = if app.draws_canvas() { app.draw_canvas(ctx, &mut surface, t) } else { app.draw(ctx, &mut surface, t) };
    let t2 = now_ms();
    if first {
        damage = Damage::new(w, h);
        damage.all();
    }
    let rects: Vec<Rect> = damage.areas().iter().map(|a| [a.x0 as u32, a.y0 as u32, (a.x1 - a.x0) as u32, (a.y1 - a.y0) as u32]).collect();
    if defer {
        copy_up(cur, other, w, h, &rects);
    }
    cur.stale.clear();
    cur.stale_all = false;
    STATS.with(|st| st.borrow_mut().frame((t1 - t0) + (now_ms() - t2), t2 - t1));
    if damage.is_empty() {
        return false;
    }
    *last_all = damage.is_all();
    if first {
        other.stale_all = true;
    } else {
        other.stale.extend_from_slice(&rects);
        if other.stale.len() > 48 {
            other.stale_all = true;
        }
    }
    cur.held = true;
    let _ = chan.send(&ToShell::Frame { buf: i as u32, damage: rects }.encode(), &[]);
    true
}

/// Brings `cur` up to date with `other`: everything when `cur` is wholly stale, else the rectangles `other` drew since;
/// pixels inside `except` (what the app just drew) are left alone. Returns the pixels copied.
fn copy_up(cur: &mut Slot, other: &Slot, w: usize, h: usize, except: &[Rect]) -> usize {
    let mut n = 0;
    if cur.stale_all {
        if except.is_empty() {
            cur.map.pixels_mut().copy_from_slice(other.map.pixels());
            n = w * h;
        } else {
            n = copy_rect_except(other.map.pixels(), cur.map.pixels_mut(), w, h, [0, 0, w as u32, h as u32], except);
        }
    } else {
        for r in cur.stale.clone() {
            n += copy_rect_except(other.map.pixels(), cur.map.pixels_mut(), w, h, r, except);
        }
    }
    n
}

/// `copy_rect` minus the rectangles in `except`; returns the pixels copied.
fn copy_rect_except(src: &[u32], dst: &mut [u32], w: usize, h: usize, r: Rect, except: &[Rect]) -> usize {
    let (x0, y0) = (r[0] as usize, r[1] as usize);
    let (x1, y1) = ((x0 + r[2] as usize).min(w), (y0 + r[3] as usize).min(h));
    let mut n = 0;
    for row in y0..y1 {
        // the covered spans on this row, left to right
        let mut cut: Vec<(usize, usize)> = except
            .iter()
            .filter(|e| (e[1] as usize) <= row && row < (e[1] as usize + e[3] as usize))
            .map(|e| (e[0] as usize, e[0] as usize + e[2] as usize))
            .collect();
        cut.sort_unstable();
        let mut x = x0;
        for (a, b) in cut {
            if a > x {
                let end = a.min(x1);
                if end > x {
                    dst[row * w + x..row * w + end].copy_from_slice(&src[row * w + x..row * w + end]);
                    n += end - x;
                }
            }
            x = x.max(b);
        }
        if x < x1 {
            dst[row * w + x..row * w + x1].copy_from_slice(&src[row * w + x..row * w + x1]);
            n += x1 - x;
        }
    }
    n
}

/// Frame timing, logged once a second while the app draws (`app: N fps, copy-up ..., draw ...`
/// in the shell log): what an app costs per frame on the device.
#[derive(Default)]
struct Stats {
    since: f64,
    n: u32,
    copy: f64,
    draw: f64,
    draw_max: f64,
}

impl Stats {
    fn frame(&mut self, copy: f64, draw: f64) {
        let now = now_ms();
        if self.n == 0 {
            self.since = now;
        }
        self.n += 1;
        self.copy += copy;
        self.draw += draw;
        self.draw_max = self.draw_max.max(draw);
        if now - self.since >= 1000.0 {
            let n = self.n as f64;
            if std::env::var_os("BIFROST_FRAME_STATS").is_some_and(|v| v == "1") {
            eprintln!("app frames: {:.0} fps, copy-up {:.1} ms, draw {:.1} ms avg {:.1} max", n * 1000.0 / (now - self.since), self.copy / n, self.draw / n, self.draw_max);
            }
            *self = Stats::default();
        }
    }
}

thread_local! {
    static STATS: std::cell::RefCell<Stats> = std::cell::RefCell::new(Stats::default());
}

/// Where the app's control socket lives: `BIFROST_CONTROL`, else `$XDG_RUNTIME_DIR/bifrost/apps/<id>.sock`,
/// else `control.sock` in the app's data directory (no XDG runtime directory is assumed).
fn control_endpoint(app_id: &str, data_dir: &std::path::Path) -> Option<crate::control::Control> {
    let path = std::env::var_os("BIFROST_CONTROL")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("XDG_RUNTIME_DIR").map(|d| PathBuf::from(d).join("bifrost/apps").join(format!("{app_id}.sock"))))
        .unwrap_or_else(|| data_dir.join("control.sock"));
    match crate::control::Control::bind(&path) {
        Ok(c) => Some(c),
        Err(e) => {
            eprintln!("bifrost-app: control socket {}: {e}", path.display());
            None
        }
    }
}

/// Built-in `app.*` methods first, then the app's own `rpc`.
fn serve_rpc<A: App>(
    app: &mut A,
    ctx: Option<&Ctx>,
    method: &str,
    params: &bifrost_core::json::Json,
    data_dir: &std::path::Path,
) -> Result<bifrost_core::json::Json, crate::control::RpcError> {
    use bifrost_core::json::Json;
    match method {
        "app.info" => Ok(Json::Obj(vec![
            ("id".into(), Json::Str(app.id().into())),
            ("sdk_version".into(), Json::Num(SDK_VERSION as f64)),
            ("pid".into(), Json::Num(std::process::id() as f64)),
            ("data".into(), Json::Str(data_dir.display().to_string())),
        ])),
        "app.methods" => {
            let mut names: Vec<Json> = ["app.info", "app.methods"].iter().map(|m| Json::Str((*m).into())).collect();
            names.extend(crate::intents::methods(app).into_iter().map(Json::Str));
            names.extend(app.rpc_methods().into_iter().map(|m| Json::Str(m.into())));
            Ok(Json::Arr(names))
        }
        _ => match crate::intents::dispatch(app, ctx, data_dir, method, params) {
            Some(r) => r,
            None => app.rpc(ctx, data_dir, method, params),
        },
    }
}

/// A context for module tests that only look at what the app sends (`Ctx::testing` with the bundled font).
#[cfg(test)]
pub(crate) fn test_ctx() -> Ctx {
    let data: &'static [u8] = Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/SFUI.ttf")).unwrap().into_boxed_slice());
    let cfg = Configure { w: 100, h: 100, scale: 1.0, safe_top: 0.0, safe_bottom: 0.0, dark: true, text_scale: 1.0, reduce_motion: false };
    Ctx::testing(Ui::new(Font::parse(data).unwrap(), 1.0), cfg)
}

#[cfg(test)]
mod badge_tests {
    use super::*;

    #[test]
    fn kit_drawn_controls_stay_published_next_to_the_apps_own() {
        use crate::controls::Node;
        use bifrost_ui::{Rect, Role};
        let r = Rect { x: 0., y: 0., w: 10., h: 10. };
        let n = |id, role, label: &str| Node { id, role, label: label.into(), value: String::new(), bounds: r, enabled: true };
        let app = vec![n(1, Role::Button, "Add")];
        let drawn = vec![n(2, Role::Row, "All Contacts"), n(3, Role::Button, "Add")];
        let merged = super::merge_controls(app, &drawn);
        assert_eq!(merged.iter().map(|m| m.id).collect::<Vec<_>>(), vec![1, 2], "the app's Add wins, the sidebar row is kept");
        assert_eq!(super::merge_controls(Vec::new(), &drawn).len(), 2, "no app controls: every drawn node");
    }

    #[test]
    fn nspasteboard_rich_types_round_trip_and_reach_the_shell() {
        use crate::pasteboard::{NativePasteboard, FILE_URL, HTML, STRING};
        let data: &'static [u8] = Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/SFUI.ttf")).unwrap().into_boxed_slice());
        let cfg = Configure { w: 100, h: 100, scale: 1.0, safe_top: 0.0, safe_bottom: 0.0, dark: true, text_scale: 1.0, reduce_motion: false };
        let ctx = Ctx::testing(Ui::new(Font::parse(data).unwrap(), 1.0), cfg);
        let pb = NativePasteboard::general();
        let before = pb.change_count();
        assert_eq!(pb.clear_contents(&ctx), before.wrapping_add(1));
        pb.set_data(&ctx, b"<b>x</b>", HTML);
        pb.set_string(&ctx, "x");
        assert_eq!(pb.data(HTML).as_deref(), Some(&b"<b>x</b>"[..]));
        assert_eq!(pb.string().as_deref(), Some("x"));
        assert_eq!(pb.types(), [HTML, STRING]);
        pb.write_objects(&ctx, &["file:///a", "file:///b"]);
        assert_eq!(pb.data(FILE_URL).as_deref(), Some(&b"file:///a\nfile:///b"[..]));
        // another app's set arrives as ToApp::Pasteboard and is readable
        let incoming = ToApp::decode(&ToApp::Pasteboard { change_count: 99, items: vec![("public.png".into(), vec![9, 8])] }.encode()).unwrap();
        if let ToApp::Pasteboard { change_count, items } = incoming { crate::pasteboard::mirror(change_count, items); }
        assert_eq!((pb.change_count(), pb.data("public.png")), (99, Some(vec![9, 8])));
        let sent = ctx.take_sent();
        assert!(matches!(sent.last(), Some(ToShell::PasteboardSet { items }) if items.len() == 1 && items[0].0 == FILE_URL));
    }

    #[test]
    fn a_testing_context_collects_what_the_app_sends_to_the_shell() {
        let data: &'static [u8] = Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/SFUI.ttf")).unwrap().into_boxed_slice());
        let cfg = Configure { w: 100, h: 100, scale: 1.0, safe_top: 0.0, safe_bottom: 0.0, dark: true, text_scale: 1.0, reduce_motion: false };
        let ctx = Ctx::testing(Ui::new(Font::parse(data).unwrap(), 1.0), cfg);
        ctx.open_url("https://example.com");
        ctx.notify("Done", "Export finished");
        assert_eq!(ctx.take_sent(), [ToShell::OpenUrl { url: "https://example.com".into() }, ToShell::Notify { title: "Done".into(), body: "Export finished".into() }]);
        assert!(ctx.take_sent().is_empty(), "taking empties the queue");
        ctx.defaults_mut().set_bool("k", true);
        assert!(ctx.defaults().bool("k") && ctx.data_dir.as_os_str().is_empty(), "memory-only defaults");
    }

    #[test]
    fn distributed_post_goes_to_the_shell_and_local_observers() {
        use std::cell::Cell;
        let data: &'static [u8] = Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/SFUI.ttf")).unwrap().into_boxed_slice());
        let cfg = Configure { w: 100, h: 100, scale: 1.0, safe_top: 0.0, safe_bottom: 0.0, dark: true, text_scale: 1.0, reduce_motion: false };
        let ctx = Ctx::testing(Ui::new(Font::parse(data).unwrap(), 1.0), cfg);
        let hits = std::rc::Rc::new(Cell::new(0));
        let h = hits.clone();
        let _t = crate::DistributedNotificationCenter.add_observer("is.test.changed", move |_| h.set(h.get() + 1));
        let info = std::collections::HashMap::from([("k".to_string(), "v".to_string())]);
        crate::DistributedNotificationCenter.post_with(&ctx, "is.test.changed", info);
        assert_eq!(ctx.take_sent(), [ToShell::DistPost { name: "is.test.changed".into(), user_info: vec![("k".into(), "v".into())] }]);
        assert_eq!(hits.get(), 1);
        crate::notification::distributed_incoming("is.test.changed", Default::default());
        assert_eq!(hits.get(), 2);
    }

    #[test]
    fn set_badge_queues_a_badge_message() {
        let data: &'static [u8] = Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/SFUI.ttf")).unwrap().into_boxed_slice());
        let ui = Ui::new(Font::parse(data).unwrap(), 1.0);
        let cfg = Configure { w: 100, h: 100, scale: 1.0, safe_top: 0.0, safe_bottom: 0.0, dark: true, text_scale: 1.0, reduce_motion: false };
        let ctx = Ctx::testing(ui, cfg);
        ctx.set_badge("3");
        ctx.set_badge("");
        assert_eq!(ctx.out.borrow().as_slice(), [ToShell::SetBadge { label: "3".into() }, ToShell::SetBadge { label: String::new() }]);
    }

    #[test]
    fn document_requests_are_numbered_and_a_save_is_answered_twice() {
        use crate::proto::{DocErrorCode, DocMode};
        let data: &'static [u8] = Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/SFUI.ttf")).unwrap().into_boxed_slice());
        let cfg = Configure { w: 100, h: 100, scale: 1.0, safe_top: 0.0, safe_bottom: 0.0, dark: true, text_scale: 1.0, reduce_motion: false };
        let ctx = Ctx { ui: Ui::new(Font::parse(data).unwrap(), 1.0), cfg, data_dir: PathBuf::new(), out: RefCell::new(Vec::new()), defaults: RefCell::new(crate::defaults::UserDefaults::memory()), docs: Default::default() };
        let open = ctx.open_document(DocMode::Read, "Import", &["public.text"]);
        let save = ctx.save_document("a.md", &[]);
        assert_eq!((open, save), (1, 2));
        assert_eq!(ctx.out.borrow()[1], ToShell::DocSave { request: 2, suggested_name: "a.md".into(), filters: vec![] });
        let file = || Some(std::fs::File::open("/dev/null").unwrap());
        let granted = |request| ToApp::DocGranted { request, grant: [1; 16], mode: DocMode::Write, name: "a.md".into(), size: 0 };
        let mut d = ctx.docs.borrow_mut();
        assert!(matches!(d.event(granted(1), file()), Some(DocumentEvent::Opened(Document { request: 1, .. }))));
        assert!(matches!(d.event(granted(2), file()), Some(DocumentEvent::SaveTo(_))), "the temporary file");
        assert!(matches!(d.event(granted(2), file()), Some(DocumentEvent::Saved(_))), "the commit");
        assert!(matches!(d.event(granted(2), file()), Some(DocumentEvent::Opened(_))), "the save is finished");
        assert!(matches!(d.event(granted(9), None), Some(DocumentEvent::Failed { request: 9, code: DocErrorCode::Denied })), "a grant without a file");
        assert!(matches!(d.event(ToApp::DocError { request: 3, code: DocErrorCode::Cancelled }, None), Some(DocumentEvent::Failed { code: DocErrorCode::Cancelled, .. })));
    }

    #[test]
    fn request_numbers_wrap_past_zero() {
        let mut d = DocState::default();
        d.next = u32::MAX;
        assert_eq!(d.next(), 1, "zero is never a request id");
        assert_eq!(d.next(), 2);
    }

    #[test]
    fn an_empty_batch_completes_at_once_and_unrelated_messages_are_ignored() {
        use crate::proto::DocMode;
        let mut d = DocState::default();
        assert!(matches!(d.event(ToApp::DocBatch { request: 7, count: 0 }, None), Some(DocumentEvent::OpenedMany { request: 7, ref docs }) if docs.is_empty()));
        assert!(d.batches.is_empty());
        let _ = DocMode::Read;
        assert!(d.event(ToApp::Quit, None).is_none());
    }

    #[test]
    fn default_open_options_choose_one_file() {
        let o = OpenOptions::default();
        assert!(o.files && !o.multiple && !o.directories && !o.create_directories);
        assert!(o.filters.is_empty() && o.title.is_empty());
    }

    #[test]
    fn open_panel_options_map_to_bits_and_a_batch_arrives_as_one_event() {
        use crate::proto::{docopt, DocErrorCode, DocMode};
        let o = OpenOptions::default();
        assert_eq!(o.bits(), docopt::FILES);
        let o = OpenOptions { multiple: true, directories: true, files: false, create_directories: true, ..o };
        assert_eq!(o.bits(), docopt::MULTIPLE | docopt::DIRECTORIES | docopt::CREATE_DIRS);
        let mut d = DocState::default();
        let file = || Some(std::fs::File::open("/dev/null").unwrap());
        let granted = |request| ToApp::DocGranted { request, grant: [1; 16], mode: DocMode::Read, name: "a".into(), size: 0 };
        assert!(d.event(ToApp::DocBatch { request: 4, count: 2 }, None).is_none());
        assert!(d.event(granted(4), file()).is_none(), "one of two");
        let Some(DocumentEvent::OpenedMany { request: 4, docs }) = d.event(granted(4), file()) else { panic!() };
        assert_eq!(docs.len(), 2);
        assert!(matches!(d.event(granted(4), file()), Some(DocumentEvent::Opened(_))), "the batch is finished");
        assert!(d.event(ToApp::DocBatch { request: 5, count: 3 }, None).is_none());
        assert!(matches!(d.event(ToApp::DocError { request: 5, code: DocErrorCode::Denied }, None), Some(DocumentEvent::Failed { .. })));
        assert!(d.batches.is_empty(), "an error drops the batch");
    }
}

#[cfg(test)]
mod direct_gate_tests {
    use super::{direct_fallback_reason, LAYER_CONTEXT_WIRED};

    #[test]
    fn without_the_switch_the_direct_path_is_never_entered() {
        assert_eq!(direct_fallback_reason(false, true, true, true), Some("BIFROST_DIRECT is not 1"));
    }

    #[test]
    fn the_switch_alone_is_not_enough_without_vulkan() {
        assert_eq!(direct_fallback_reason(true, false, true, true), Some("no Vulkan device"));
    }

    #[test]
    fn a_compositor_without_the_layer_manager_falls_back() {
        assert_eq!(direct_fallback_reason(true, true, false, true), Some("the compositor does not offer bifrost_layer_manager_v1"));
    }

    #[test]
    fn an_unwired_connection_falls_back_even_when_everything_else_is_present() {
        assert_eq!(direct_fallback_reason(true, true, true, false), Some("the connection cannot bind layer contexts yet"));
    }

    #[test]
    fn everything_present_takes_the_direct_path() {
        assert_eq!(direct_fallback_reason(true, true, true, true), None);
    }

    #[test]
    fn the_checks_run_in_order_so_the_first_missing_piece_is_named() {
        assert_eq!(direct_fallback_reason(false, false, false, false), Some("BIFROST_DIRECT is not 1"));
        assert_eq!(direct_fallback_reason(true, false, false, false), Some("no Vulkan device"));
    }

    #[test]
    fn the_layer_context_is_not_wired_yet() {
        assert!(!LAYER_CONTEXT_WIRED);
    }
}

#[cfg(test)]
mod audit_fix_tests {
    use super::*;
    use bifrost_render::canvas::Canvas;

    /// Draws everything each frame; with `canvas` its `draw` must never be called.
    struct Fake {
        canvas: bool,
    }

    fn all() -> Damage {
        let mut d = Damage::new(100, 100);
        d.all();
        d
    }

    impl App for Fake {
        fn id(&self) -> &str {
            "is.test.fake"
        }
        fn draws_canvas(&self) -> bool {
            self.canvas
        }
        fn draw(&mut self, _ctx: &Ctx, _s: &mut Surface, _t: f64) -> Damage {
            assert!(!self.canvas, "a canvas app is drawn through draw_canvas");
            all()
        }
        fn draw_canvas(&mut self, _ctx: &Ctx, _c: &mut dyn Canvas, _t: f64) -> Damage {
            all()
        }
    }

    #[test]
    fn copy_rect_except_skips_the_covered_span() {
        let src = vec![7u32; 8];
        let mut dst = vec![0u32; 8];
        // 4x2, everything stale, row 0 columns 1..3 covered by the new damage
        let n = copy_rect_except(&src, &mut dst, 4, 2, [0, 0, 4, 2], &[[1, 0, 2, 1]]);
        assert_eq!(n, 6);
        assert_eq!(dst, [7, 0, 0, 7, 7, 7, 7, 7]);
    }

    #[test]
    fn a_full_repaint_copies_nothing_up_and_a_canvas_app_is_drawn_through_draw_canvas() {
        let (a, _b) = crate::proto::sys::socketpair_pair().unwrap();
        let ctx = test_ctx();
        let mut slots = alloc(&a, 100, 100, 1).expect("buffers");
        let mut app = Fake { canvas: true };
        let mut last_all = false;
        assert!(draw_into(&a, &mut app, &ctx, &mut slots, 0, 0.0, true, &mut last_all));
        assert!(last_all, "the first frame was whole");
        // slot 1 is wholly stale, yet a full repaint leaves nothing to copy
        let (lo, hi) = slots.split_at_mut(1);
        assert_eq!(copy_up(&mut hi[0], &lo[0], 100, 100, &[[0, 0, 100, 100]]), 0);
        assert!(draw_into(&a, &mut app, &ctx, &mut slots, 1, 1.0, false, &mut last_all));
        assert!(!slots[1].stale_all && slots[1].stale.is_empty());
    }

    #[test]
    fn reconfigure_keeps_bold_text_and_queued_messages() {
        let mut ctx = test_ctx();
        ctx.ui.bold = true;
        ctx.open_url("https://example.com");
        let cfg = Configure { w: 200, h: 150, scale: 2.0, text_scale: 1.5, ..ctx.cfg };
        reconfigure(&mut ctx, cfg);
        assert!(ctx.ui.bold, "Bold Text survives a resize");
        assert_eq!((ctx.cfg.w, ctx.cfg.h, ctx.ui.s, ctx.ui.text_scale), (200, 150, 2.0, 1.5));
        assert_eq!(ctx.take_sent(), [ToShell::OpenUrl { url: "https://example.com".into() }], "the queue is not dropped");
    }
}

#[cfg(test)]
mod tree_merge_tests {
    use super::merge_tree;
    use crate::proto::NodeExt;
    use bifrost_ui::ax::AxLink;

    #[test]
    fn drawn_parents_fill_in_and_the_apps_own_entries_win() {
        let own = vec![NodeExt { id: 2, parent: None, focused: true, checked: None }];
        let links = [AxLink { id: 2, parent: 1 }, AxLink { id: 3, parent: 1 }];
        let t = merge_tree(own, &links);
        assert_eq!(t.len(), 2);
        assert_eq!((t[0].id, t[0].parent, t[0].focused), (2, None, true), "the app's entry is kept");
        assert_eq!((t[1].id, t[1].parent), (3, Some(1)));
    }

    #[test]
    fn the_merged_tree_stays_within_the_packet_limit() {
        let links: Vec<AxLink> = (0..300).map(|i| AxLink { id: i, parent: 999 }).collect();
        assert_eq!(merge_tree(Vec::new(), &links).len(), crate::proto::MAX_NODE_EXT);
    }
}
