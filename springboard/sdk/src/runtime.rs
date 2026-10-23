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
    /// A press, drag or release (`t` in ms), with the modifier keys held at the time (`NSEvent.modifierFlags`): a Shift-click
    /// extends a selection and a Cmd-click toggles one item. Plain touches and synthesized presses carry none.
    Touch { kind: TouchKind, id: u32, x: f32, y: f32, t: f64, mods: crate::proto::ModifierFlags },
    Key(KeyInput),
    Open(String),
    Command(crate::Command),
    Activate(u64),
    Context {x:f32,y:f32},
    ContextAction(u32),
    /// The user chose a Home Screen quick action (`quick_actions`): its type and user info.
    QuickAction(crate::quick_actions::QuickActionPick),
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
    /// the app's windows (`app_windows`), shared by every `Ctx` of this run like `docs`
    pub(crate) windows: std::rc::Rc<RefCell<crate::app_windows::AppWindows>>,
    /// the key window's id (`ToApp::WindowFocus`), shared like `windows`; 0 is the main window
    pub(crate) key_window_id: std::rc::Rc<std::cell::Cell<u32>>,
}

thread_local! {
    static SUDDEN: RefCell<crate::process_info::SuddenTermination> = RefCell::new(Default::default());
    static ACTIVITIES: RefCell<crate::process_info::ActivityTracker> = RefCell::new(Default::default());
}

impl Ctx {
    /// A context with no shell behind it, for tests (the `XCTest` host-less app): memory-only defaults, no data
    /// directory. What the app sends to the shell is collected; read it with `take_sent`.
    pub fn testing(ui: Ui, cfg: Configure) -> Self {
        Ctx { ui, cfg, data_dir: PathBuf::new(), out: RefCell::new(Vec::new()), defaults: RefCell::new(crate::defaults::UserDefaults::memory()), docs: Default::default(), windows: Default::default(), key_window_id: Default::default() }
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
    /// Opens a context menu at the pointer: `x` and `y` are the window-local pixels `Input::Context` carried. The shell
    /// draws it like its own menus, with its top-left corner at the pointer (flipped left or up when it would leave the
    /// screen), and sends the choice back as `Input::ContextAction(id)`. Build the rows with `ContextItem::new(id, title)`,
    /// `.key("cmd+i")` (shown right-aligned; the app still handles the chord), `.enabled(false)` (secondary ink) and
    /// `ContextItem::separator()` between groups. At most 32 rows, separators included.
    pub fn context_menu(&self,x:f32,y:f32,items:Vec<crate::proto::ContextItem>){self.send(ToShell::ContextMenu{x,y,items});}
    /// Ask the authenticated system broker to change Remote sharing.
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
    /// Takes the app's window full screen (`on`) or back to its size: the window gets its own space, with the menu bar
    /// hidden until the pointer reaches the top edge, as the green button does (`ToShell::SetFullScreen`). A window with
    /// a fixed size does not go full screen. The shell answers with `ToApp::FullScreen`.
    pub fn set_full_screen(&self, on: bool) {
        self.send(ToShell::SetFullScreen { on });
    }
    /// The app's key window: the window that has keyboard focus, or the main window (`WindowId::MAIN`) when the shell has
    /// not made an extra window key or that window is closed. Keys, commands and text edits go to it.
    pub fn key_window(&self) -> crate::app_windows::WindowId {
        let id = crate::app_windows::WindowId(self.key_window_id.get());
        if self.windows.borrow().route(id).is_some() { id } else { crate::app_windows::WindowId::MAIN }
    }
    /// The app's windows: the main window and the ones it opened (`app_windows`). Do not hold the borrow across a call
    /// back into the app.
    pub fn windows(&self) -> std::cell::Ref<'_, crate::app_windows::AppWindows> {
        self.windows.borrow()
    }

    /// Opens a window of the app's own (`AppWindows::open`) and asks the shell to show it. The id is the app's; the shell
    /// draws the window under it.
    pub fn open_window(&self, spec: crate::app_windows::WindowSpec) -> Result<crate::app_windows::WindowId, crate::app_windows::WindowError> {
        let flags = if spec.title_double_click { crate::proto::window_flags::TITLE_DOUBLE_CLICK } else { 0 };
        let position = spec.position;
        let id = self.windows.borrow_mut().open(spec.clone())?;
        self.send(ToShell::WindowOpen { id: id.0, title: spec.title, w: spec.width, h: spec.height, position, flags });
        Ok(id)
    }

    /// Closes one of the app's windows and asks the shell to drop it. The main window cannot be closed.
    pub fn close_window(&self, id: crate::app_windows::WindowId) -> Result<crate::app_windows::AppWindow, crate::app_windows::WindowError> {
        let closed = self.windows.borrow_mut().close(id)?;
        self.send(ToShell::WindowClose { id: id.0 });
        Ok(closed)
    }

    /// Moves one of the app's windows to (x, y) points relative to the visible desktop area. The shell clamps it to the
    /// screen; the shell reports the position it settles on as `ToApp::WindowMoved` when the person drags it.
    pub fn move_window(&self, id: crate::app_windows::WindowId, x: f32, y: f32) -> Result<(), crate::app_windows::WindowError> {
        self.windows.borrow_mut().set_position(id, x, y)?;
        self.send(ToShell::WindowMove { window: id.0, x, y });
        Ok(())
    }

    /// Collapses one of the app's windows to its title bar, or expands it back (the shell keeps the height it had).
    pub fn set_window_collapsed(&self, id: crate::app_windows::WindowId, collapsed: bool) -> Result<(), crate::app_windows::WindowError> {
        self.windows.borrow_mut().set_collapsed(id, collapsed)?;
        self.send(ToShell::WindowCollapse { window: id.0, collapsed });
        Ok(())
    }

    /// Changes the title of one of the app's windows.
    pub fn set_window_title(&self, id: crate::app_windows::WindowId, title: &str) -> Result<(), crate::app_windows::WindowError> {
        let (w, h) = self.windows.borrow().get(id).map(|x| (x.width, x.height)).ok_or(crate::app_windows::WindowError::Unknown(id))?;
        self.windows.borrow_mut().set_frame(id, title, w, h)?;
        self.send(ToShell::WindowTitle { id: id.0, title: title.to_string() });
        Ok(())
    }

    /// Changes the size, in points, of one of the app's windows.
    pub fn resize_window(&self, id: crate::app_windows::WindowId, w: f32, h: f32) -> Result<(), crate::app_windows::WindowError> {
        let title = self.windows.borrow().get(id).map(|x| x.title.clone()).ok_or(crate::app_windows::WindowError::Unknown(id))?;
        self.windows.borrow_mut().set_frame(id, &title, w, h)?;
        self.send(ToShell::WindowResize { id: id.0, w, h });
        Ok(())
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
    // the same theme definition file as the shell's: an app's colours follow it
    bifrost_render::theme::load_at_startup();
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
    // the extra windows' pixels, by window id (see `draw_windows`)
    let mut window_pixels: std::collections::BTreeMap<u32, WindowPixels> = std::collections::BTreeMap::new();
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
    let mut clip_rev = kit::clipboard::revision();
    let mut open = true;
    let mut commands_seen = None;
    let mut pointer_interest_sent = 0u32;
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
    let windows = std::rc::Rc::new(RefCell::new(crate::app_windows::AppWindows::default()));
    let key_window_id: std::rc::Rc<std::cell::Cell<u32>> = Default::default();

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
        let mut held_move: Option<(u32, f32, f32, u32, crate::proto::ModifierFlags)> = None;
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
            // pointer motion and touch frames (hundreds a second) publish nothing by themselves: they count as a busy
            // turn only when the app says it changed (it asked for a frame); every other packet may change what is published
            let motion = matches!(msg, ToApp::Touch { kind: TouchKind::Move, .. } | ToApp::Touches(_));
            busy |= !motion;
            if let ToApp::Touch { kind: TouchKind::Move, id, x, y, t_ms, mods } = msg {
                if let Some(h) = held_move.replace((id, x, y, t_ms, mods)) {
                    if h.0 != id {
                        let changed = deliver_move(&mut app, ctx.as_ref(), &mut touch_router, h, now);
                        need_frame |= changed;
                        busy |= changed;
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
                        let changed = app.touches(c, &ev);
                        need_frame |= changed;
                        busy |= changed;
                    }
                }
                continue;
            }
            if let Some(h) = held_move.take() {
                let changed = deliver_move(&mut app, ctx.as_ref(), &mut touch_router, h, now);
                need_frame |= changed;
                busy |= changed;
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
                            ctx = Some(Ctx { ui, cfg, defaults: RefCell::new(crate::defaults::UserDefaults::open(&data_dir)), data_dir: data_dir.clone(), out: RefCell::new(Vec::new()), docs: docs.clone(), windows: windows.clone(), key_window_id: key_window_id.clone() });
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
                        generation = next_generation(generation);
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
                ToApp::QuickAction { kind, user_info } => {
                    if let Some(c) = &ctx {
                        need_frame |= app.event(c, Input::QuickAction(crate::quick_actions::QuickActionPick { kind, user_info }));
                    }
                }
                ToApp::Touch { kind, id, x, y, t_ms, mods } => {
                    if let Some(c) = &ctx {
                        need_frame |= deliver_touch(&mut app, c, &mut touch_router, kind, id, x, y, event_time(now, t_ms), mods);
                    }
                }
                ToApp::Touches(_) => {}
                // wheel, hover and gestures reach an app that asked for them; the shell only sends what was asked
                ToApp::Pointer(ev) => {
                    if let Some(c) = &ctx {
                        if app.pointer_interest() & ev.interest() != 0 {
                            need_frame |= app.pointer(c, &ev);
                        }
                    }
                }
                // the shell's full-screen state; the app's layout follows the window size it is configured with
                ToApp::FullScreen { .. } => {}
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
                m @ (ToApp::WindowClosed { .. }
                | ToApp::WindowConfigure { .. }
                | ToApp::WindowTouch { .. }
                | ToApp::WindowMoved { .. }
                | ToApp::WindowTitleDoubleClick { .. }
                | ToApp::WindowFocus { .. }
                | ToApp::WindowInput { .. }) => {
                    if let Some(c) = &ctx {
                        need_frame |= crate::app_windows::deliver_window(&mut app, c, &m, now_ms()).unwrap_or(false);
                    }
                }
                // the shell is done with one of a window's buffers
                ToApp::WindowRelease { id, buf, generation: g } => {
                    release_window_buffer(&mut window_pixels, id, buf, g);
                }
                ToApp::Resume => {
                    active = true;
                    if let Some(c) = &ctx {
                        app.resume(c);
                    }
                    need_frame = true;
                    // A board that changed in the background arrives with the shell's own push to the app in front, sent in the
                    // same tick as `Resume`: asking for it here would transfer a large copy twice on every app switch. An app that
                    // needs it sooner calls `NativePasteboard::request_contents`.
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
                    // the user's accent colour: switches, checkmarks and the default button follow it
                    if let Some(accent) = kit::appearance::Accent::from_wire((flags & ACCENT_MASK) >> ACCENT_SHIFT) {
                        kit::appearance::set_accent(accent);
                    }
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
                // the board changed and this app is not in front: only the count is known, the contents follow when it is
                ToApp::PasteboardChanged { change_count } => crate::pasteboard::note_changed(change_count),
                // items too large for a packet arrive in a descriptor; the text fields read the plain-text item
                ToApp::PasteboardBlob { change_count, types } => {
                    if let Some(items) = fds.pop().and_then(|fd| crate::proto::pbblob::read(&fd, &types).ok()) {
                        let text = springboard::shell::pasteboard::text_of(&items).unwrap_or_default();
                        crate::pasteboard::mirror(change_count, items);
                        kit::clipboard::set_text(&text);
                        clip_seen = kit::clipboard::text();
                    }
                }
                // the shell's input source: the kana (Romaji) flag the text input reads (`compose_method`)
                ToApp::InputSource { japanese } => bifrost_ui::compose_method::set_japanese_source(japanese),
                ToApp::Clipboard { text } => {
                    kit::clipboard::set_text(&text);
                    clip_seen = kit::clipboard::text();
                    clip_rev = kit::clipboard::revision();
                }
                ToApp::NotificationAction { id, action, text, thread, category, user_info } => {
                    if let Some(c) = &ctx {
                        need_frame |= app.notification(c, crate::usernotif::Event::response(id, action, text, thread, category, user_info));
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
                ToApp::NotificationAuthorized { request, granted } => {
                    if let Some(c) = &ctx {
                        need_frame |= app.notification(c, crate::usernotif::Event::Authorization { request, granted });
                    }
                }
                ToApp::NotificationWillPresent { token, id, thread, category, title, body } => {
                    if let Some(c) = &ctx {
                        need_frame |= app.notification(c, crate::usernotif::Event::WillPresent { token, id, thread, category, title, body });
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
                ToApp::ReindexAll => {
                    if let Some(c) = &ctx {
                        need_frame |= app.index_request(c, crate::quicksearch::IndexRequest::ReindexAll);
                    }
                }
                ToApp::ReindexIdentifiers { identifiers } => {
                    if let Some(c) = &ctx {
                        need_frame |= app.index_request(c, crate::quicksearch::IndexRequest::ReindexIdentifiers(identifiers));
                    }
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
            let changed = deliver_move(&mut app, ctx.as_ref(), &mut touch_router, h, now_ms());
            need_frame |= changed;
            busy |= changed;
        }
        if !open {
            break;
        }
        let Some(c) = &ctx else { continue };
        // the pointer events the app takes, told to the shell when they change (the shell sends nothing before)
        let interest = app.pointer_interest() & bifrost_ui::pointer_event::ALL;
        if interest != pointer_interest_sent {
            pointer_interest_sent = interest;
            let _ = chan.send(&ToShell::PointerInterest { mask: interest }.encode(), &[]);
        }
        for activity in pending_activity.drain(..) {
            need_frame |= app.continue_activity(c, activity);
        }
        let t = now_ms();
        if active && app.tick(c, t) {
            need_frame = true;
            busy = true;
        }
        flush_restorable(&app, c, false);
        let frame_due = (active || repaint_suspended) && need_frame;
        if frame_due {
            if let Some(slots) = slots.as_mut() {
                let target = pick_free([slots[0].held, slots[1].held], last);
                if let Some(i) = target {
                    need_frame = false;
                    bifrost_ui::ax::begin_frame();
                    let _ = crate::widgets::take_frame_materials();
                    let full = first_frame || app.full_redraw();
                    let sent = draw_into(&chan, &mut app, c, slots, i, t, first_frame, &mut last_all, last);
                    // a full redraw declares every region; a partial one only those inside what it redrew, so the regions
                    // outside that damage are kept (the toolbar band stays blurred after only the sidebar redrew)
                    let drawn = crate::widgets::take_frame_materials();
                    if sent {
                        if full {
                            materials = drawn;
                        } else {
                            let scale = c.cfg.scale.max(0.01);
                            let damage: Vec<(f32, f32, f32, f32)> = take_reported_damage()
                                .iter()
                                .map(|r| (r[0] as f32 / scale, r[1] as f32 / scale, (r[0] + r[2]) as f32 / scale, (r[1] + r[3]) as f32 / scale))
                                .collect();
                            merge_materials(&mut materials, drawn, &damage, 1.0 / scale);
                        }
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
        // the extra windows: each is drawn when a frame is due, when it changed, and once after it opened or resized
        if active || repaint_suspended {
            draw_windows(&chan, &mut app, c, &mut window_pixels, t, frame_due);
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
        // the notifications posted since the last frame (`ax::post`) reach the shell, which turns them into AT-SPI signals
        let notes=bifrost_ui::ax::take_notifications();
        for chunk in notes.chunks(crate::proto::axtext::MAX_NOTES) {let _=chan.send(&ToShell::AxNotify(chunk.to_vec()).encode(),&[]);}
        let commands=app.commands(c);
        let menus=app.menus_for(c,c.key_window());
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
        flush_out(&chan, c);
        if let Some(text) = clipboard_changed(&mut clip_rev, &mut clip_seen) {
            send_message(&chan, &ToShell::SetClipboard { text });
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
pub(crate) fn event_time(now: f64, t_ms: u32) -> f64 {
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

/// Sends the messages the app queued in `ctx` (`Ctx::send`), oldest first. Anything the app sends straight on the channel
/// (frames and their buffers) must come after the requests that name it, so callers flush before they send those.
fn flush_out(chan: &Channel, ctx: &Ctx) {
    for m in ctx.take_sent() {
        send_message(chan, &m);
    }
}

/// Sends `m` to the shell. A pasteboard write or clipboard text that does not fit a packet goes through a descriptor
/// (`ToShell::PasteboardBlob`) instead of being cut or dropped; one over the blob limits is not sent at all, so the
/// shell's clipboard keeps what it had rather than getting a shortened or empty copy.
fn send_message(chan: &Channel, m: &ToShell) {
    match m {
        ToShell::PasteboardSet { items } if !crate::proto::pbblob::fits_inline(items) => send_blob(chan, items),
        ToShell::SetClipboard { text } if text.len() > crate::proto::MAX_INLINE_TEXT => send_blob(chan, &[(crate::pasteboard::STRING.to_string(), text.clone().into_bytes())]),
        _ => {
            let _ = chan.send(&m.encode(), &[]);
        }
    }
}

fn send_blob(chan: &Channel, items: &[(String, Vec<u8>)]) {
    let Ok((fd, types)) = crate::proto::pbblob::write(items) else { return };
    let _ = chan.send(&ToShell::PasteboardBlob { types }.encode(), &[fd.0]);
}

fn alloc(chan: &Channel, w: u32, h: u32, generation: u32) -> Option<[Slot; 2]> {
    alloc_pair(chan, "bifrost-frame", w, h, |w, h| ToShell::Buffers { w, h, generation })
}

/// Makes two shared-memory buffers of `w` by `h` pixels named `name`-0 and `name`-1, and sends the shell their
/// descriptors in the message `buffers` builds. The main window and each extra window get their own pair.
fn alloc_pair(chan: &Channel, name: &str, w: u32, h: u32, buffers: impl FnOnce(u32, u32) -> ToShell) -> Option<[Slot; 2]> {
    let bytes = (w as usize * h as usize * 4).max(4);
    let mk = |n: &str| -> Option<(Owned, Slot)> {
        let fd = shm_create(n, bytes).ok()?;
        let map = Map::new(&fd, bytes).ok()?;
        Some((fd, Slot { map, held: false, stale: Vec::new(), stale_all: true }))
    };
    let (fa, a) = mk(&format!("{name}-0"))?;
    let (fb, b) = mk(&format!("{name}-1"))?;
    chan.send(&buffers(w, h).encode(), &[fa.0, fb.0]).ok()?;
    Some([a, b])
}

/// The next buffer-set generation (wrapping, never 0: 0 is the value of a window with no buffers yet).
fn next_generation(g: u32) -> u32 {
    g.wrapping_add(1).max(1)
}

/// The slot to draw into: a free one other than the last frame's when there is one, else the last frame's if it is free.
fn pick_free(held: [bool; 2], last: Option<usize>) -> Option<usize> {
    (0..2).find(|&i| !held[i] && Some(i) != last).or_else(|| (0..2).find(|&i| !held[i]))
}

/// The pixel size of a window of `w` by `h` points at `scale` pixels per point, kept inside the shell's limit.
fn window_px(w: f32, h: f32, scale: f32) -> (u32, u32) {
    let px = |v: f32| if v.is_finite() { (v * scale).round().clamp(1.0, 8192.0) as u32 } else { 1 };
    (px(w), px(h))
}

/// The pixels of one extra window: its own slot pair, the size they were made for and what the shell has of them.
struct WindowPixels {
    size: (u32, u32),
    generation: u32,
    slots: [Slot; 2],
    last: Option<usize>,
    first: bool,
    last_all: bool,
    /// Something changed since the window last drew (an event, a resize, a new buffer set): draw it at the next turn.
    dirty: bool,
}

/// Records the shell's `Release` of one of a window's buffers. A release for another generation is stale and ignored.
fn release_window_buffer(pixels: &mut std::collections::BTreeMap<u32, WindowPixels>, id: u32, buf: u32, generation: u32) -> bool {
    match pixels.get_mut(&id) {
        Some(p) if p.generation == generation => match p.slots.get_mut(buf as usize) {
            Some(b) => {
                b.held = false;
                true
            }
            None => false,
        },
        _ => false,
    }
}

/// Gives every extra window of the app its own slot pair (`WindowBuffers`, again when its pixel size changed), and draws
/// each window that is dirty (or all of them when `due`, a frame is due for the app): `App::draw_window` into a free
/// slot, then `WindowFrame`. A window the app or the shell closed loses its pixels here.
fn draw_windows<A: App>(chan: &Channel, app: &mut A, ctx: &Ctx, pixels: &mut std::collections::BTreeMap<u32, WindowPixels>, t: f64, due: bool) {
    let scale = ctx.cfg.scale;
    // `WindowOpen` is queued in the context and sent at the end of the turn, but `WindowBuffers` and `WindowFrame` go
    // straight to the channel. A window opened in this turn would reach the shell as buffers for a window it does not
    // know yet: the shell drops them, the app never sends them again, and the window stays unmapped (never drawn).
    // The queued requests go first, so the shell has the window before its pixels.
    flush_out(chan, ctx);
    // the registry is borrowed only for this listing: the app may open or close windows from its draw
    let open: Vec<(u32, (u32, u32))> = ctx.windows.borrow().list().iter().filter(|w| !w.id.is_main()).map(|w| (w.id.0, window_px(w.width, w.height, scale))).collect();
    pixels.retain(|id, _| open.iter().any(|(o, _)| o == id));
    for (id, size) in open {
        if pixels.get(&id).map(|p| p.size) != Some(size) {
            let generation = next_generation(pixels.get(&id).map_or(0, |p| p.generation));
            let Some(slots) = alloc_pair(chan, &format!("bifrost-frame-win{id}"), size.0, size.1, |w, h| ToShell::WindowBuffers { id, w, h, generation }) else {
                eprintln!("bifrost-app: cannot allocate frame buffers for window {id}");
                continue;
            };
            pixels.insert(id, WindowPixels { size, generation, slots, last: None, first: true, last_all: false, dirty: true });
        }
        let Some(p) = pixels.get_mut(&id) else { continue };
        if due {
            p.dirty = true;
        }
        if !p.dirty {
            continue;
        }
        let Some(i) = pick_free([p.slots[0].held, p.slots[1].held], p.last) else { continue };
        let first = p.first;
        let narrow = !first && p.last == Some(1 - i);
        let sent = paint_slot(
            chan,
            &mut p.slots,
            i,
            (size.0 as usize, size.1 as usize),
            t,
            first,
            false,
            narrow,
            &mut p.last_all,
            |s| app.draw_window(ctx, crate::app_windows::WindowId(id), s, t),
            |buf, damage| ToShell::WindowFrame { id, buf, damage },
        );
        // a draw that changed nothing is done too: the window waits for the next event, not for a free slot
        p.first = false;
        p.dirty = false;
        if sent {
            p.last = Some(i);
        }
    }
}

/// Delivers the last of a run of moves of one touch (`t_ms` is the shell's stamp of that last sample).
fn deliver_move<A: App>(app: &mut A, ctx: Option<&Ctx>, router: &mut TouchRouter, m: (u32, f32, f32, u32, crate::proto::ModifierFlags), now: f64) -> bool {
    let Some(c) = ctx else { return false };
    deliver_touch(app, c, router, TouchKind::Move, m.0, m.1, m.2, event_time(now, m.3), m.4)
}

/// Delivers a plain touch: as `Input::Touch`, or as a touch event for an app that takes them (`App::wants_touch_events`).
#[allow(clippy::too_many_arguments)]
fn deliver_touch<A: App>(app: &mut A, c: &Ctx, router: &mut TouchRouter, kind: TouchKind, id: u32, x: f32, y: f32, t: f64, mods: crate::proto::ModifierFlags) -> bool {
    match router.plain(app.wants_touch_events(), kind, id, x, y, t) {
        Plain::Deliver => app.event(c, Input::Touch { kind, id, x, y, t, mods }),
        Plain::Frame(ev) => app.touches(c, &ev),
        Plain::Skip => false,
    }
}

/// Draws into slot `i`, brings it up to date with the other buffer, and sends the frame. Returns whether a frame went
/// out. An app whose last frame repainted everything (`last_all`) is drawn first and only then is the part of the
/// stale area its damage did not cover copied up, so a full repaint costs no copy at all.
fn draw_into<A: App>(chan: &Channel, app: &mut A, ctx: &Ctx, slots: &mut [Slot; 2], i: usize, t: f64, first: bool, last_all: &mut bool, last: Option<usize>) -> bool {
    let full = app.full_redraw();
    let canvas = app.draws_canvas();
    let (w, h) = (ctx.cfg.w as usize, ctx.cfg.h as usize);
    // the other buffer holds the frame the shell shows now: a repaint can be compared with it
    let narrow = !first && last == Some(1 - i);
    paint_slot(
        chan,
        slots,
        i,
        (w, h),
        t,
        first,
        full,
        narrow,
        last_all,
        |s| if canvas { app.draw_canvas(ctx, s, t) } else { app.draw(ctx, s, t) },
        |buf, damage| ToShell::Frame { buf, damage },
    )
}

/// The drawing shared by the main window and the extra windows: `draw` paints slot `i`, the other buffer is brought up
/// to date, and `frame` builds the message that carries the damage. Returns whether a frame went out.
#[allow(clippy::too_many_arguments)]
fn paint_slot(chan: &Channel, slots: &mut [Slot; 2], i: usize, (w, h): (usize, usize), _t: f64, first: bool, full: bool, narrow: bool, last_all: &mut bool, draw: impl FnOnce(&mut Surface) -> Damage, frame: impl FnOnce(u32, Vec<Rect>) -> ToShell) -> bool {
    let (lo, hi) = slots.split_at_mut(1);
    let (cur, other) = if i == 0 { (&mut lo[0], &mut hi[0]) } else { (&mut hi[0], &mut lo[0]) };
    let t0 = now_ms();
    let defer = !full && *last_all && !first;
    if full {
        cur.stale.clear();
    } else if !defer {
        copy_up(cur, other, w, h, &[]);
    }
    let t1 = now_ms();
    let mut surface = Surface { w, h, stride: w, px: cur.map.pixels_mut(), y0: 0 };
    let mut damage = draw(&mut surface);
    let t2 = now_ms();
    if first {
        damage = Damage::new(w, h);
        damage.all();
    }
    // `last_all` follows what the app reported, not what changed
    let reported_all = damage.is_all();
    let rect_of = |a: &bifrost_render::gfx::Area| -> Rect { [a.x0 as u32, a.y0 as u32, (a.x1 - a.x0) as u32, (a.y1 - a.y0) as u32] };
    // what the app reported: a deferred copy-up leaves exactly this alone (a full repaint copies nothing)
    let reported: Vec<Rect> = damage.areas().iter().map(rect_of).collect();
    // An app that repaints everything (it keeps no damage of its own) often paints what is already on screen: compare
    // with the buffer the shell shows and send only what differs, nothing at all when nothing does. The shell then
    // draws no frame for a repaint that changed no pixel.
    if narrow && reported_all && cur.map.pixels().len() == w * h && other.map.pixels().len() == w * h {
        damage = Damage::between(other.map.pixels(), cur.map.pixels(), w, h);
    }
    let rects: Vec<Rect> = damage.areas().iter().map(rect_of).collect();
    if defer {
        copy_up(cur, other, w, h, &reported);
    }
    cur.stale.clear();
    cur.stale_all = false;
    STATS.with(|st| st.borrow_mut().frame((t1 - t0) + (now_ms() - t2), t2 - t1));
    if damage.is_empty() {
        if narrow && reported_all {
            STATS.with(|st| st.borrow_mut().identical());
        }
        return false;
    }
    *last_all = reported_all;
    if first {
        other.stale_all = true;
    } else {
        other.stale.extend_from_slice(&rects);
        if other.stale.len() > 48 {
            other.stale_all = true;
        }
    }
    cur.held = true;
    REPORTED_DAMAGE.with(|r| *r.borrow_mut() = reported);
    let _ = chan.send(&frame(i as u32, rects).encode(), &[]);
    true
}

thread_local! {
    /// What the app redrew in the last frame `paint_slot` sent (px, x y w h), for the region-wise material merge.
    static REPORTED_DAMAGE: RefCell<Vec<Rect>> = const { RefCell::new(Vec::new()) };
}

fn take_reported_damage() -> Vec<Rect> {
    REPORTED_DAMAGE.with(|r| std::mem::take(&mut *r.borrow_mut()))
}

/// Folds the material regions a partial redraw declared (`fresh`) into the stored list. `damage` is what the frame
/// redrew in points (x0, y0, x1, y1): a stored region wholly inside it was drawn again (or went away), so it is
/// replaced by `fresh`; regions outside it were not redrawn and stay. `slack` (points) absorbs pixel rounding.
fn merge_materials(stored: &mut Vec<crate::proto::Material>, fresh: Vec<crate::proto::Material>, damage: &[(f32, f32, f32, f32)], slack: f32) {
    stored.retain(|m| {
        let r = m.rect;
        !damage.iter().any(|&(x0, y0, x1, y1)| r.x >= x0 - slack && r.y >= y0 - slack && r.x + r.w <= x1 + slack && r.y + r.h <= y1 + slack)
    });
    for m in fresh {
        if !stored.contains(&m) && stored.len() < crate::proto::MAX_MATERIALS {
            stored.push(m);
        }
    }
}

/// The clipboard text when it changed since the last call, else `None`. The revision is compared first, so a turn
/// where nothing was copied clones no text.
fn clipboard_changed(rev_seen: &mut u64, text_seen: &mut Option<String>) -> Option<String> {
    let rev = kit::clipboard::revision();
    if rev == *rev_seen {
        return None;
    }
    *rev_seen = rev;
    let now = kit::clipboard::text();
    if now == *text_seen {
        return None;
    }
    *text_seen = now.clone();
    Some(now.unwrap_or_default())
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
    /// repaints that came out pixel for pixel like the frame on screen and were not sent
    same: u32,
}

impl Stats {
    fn identical(&mut self) {
        self.same += 1;
    }

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
            eprintln!("app frames: {:.0} fps, copy-up {:.1} ms, draw {:.1} ms avg {:.1} max, {} repaints identical to the screen (not sent)", n * 1000.0 / (now - self.since), self.copy / n, self.draw / n, self.draw_max, self.same);
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
    let explicit = std::env::var_os("BIFROST_CONTROL").map(PathBuf::from);
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from);
    let path = crate::script_target::socket_path(app_id, explicit.as_deref(), runtime.as_deref(), data_dir);
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
    fn full_screen_requests_go_to_the_shell() {
        let data: &'static [u8] = Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/SFUI.ttf")).unwrap().into_boxed_slice());
        let ui = Ui::new(Font::parse(data).unwrap(), 1.0);
        let cfg = Configure { w: 100, h: 100, scale: 1.0, safe_top: 0.0, safe_bottom: 0.0, dark: true, text_scale: 1.0, reduce_motion: false };
        let ctx = Ctx::testing(ui, cfg);
        ctx.set_full_screen(true);
        ctx.set_full_screen(false);
        assert_eq!(ctx.out.borrow().as_slice(), [ToShell::SetFullScreen { on: true }, ToShell::SetFullScreen { on: false }]);
    }

    #[test]
    fn document_requests_are_numbered_and_a_save_is_answered_twice() {
        use crate::proto::{DocErrorCode, DocMode};
        let data: &'static [u8] = Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/SFUI.ttf")).unwrap().into_boxed_slice());
        let cfg = Configure { w: 100, h: 100, scale: 1.0, safe_top: 0.0, safe_bottom: 0.0, dark: true, text_scale: 1.0, reduce_motion: false };
        let ctx = Ctx { ui: Ui::new(Font::parse(data).unwrap(), 1.0), cfg, data_dir: PathBuf::new(), out: RefCell::new(Vec::new()), defaults: RefCell::new(crate::defaults::UserDefaults::memory()), docs: Default::default(), windows: Default::default(), key_window_id: Default::default() };
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
        assert!(draw_into(&a, &mut app, &ctx, &mut slots, 0, 0.0, true, &mut last_all, None));
        assert!(last_all, "the first frame was whole");
        // slot 1 is wholly stale, yet a full repaint leaves nothing to copy
        let (lo, hi) = slots.split_at_mut(1);
        assert_eq!(copy_up(&mut hi[0], &lo[0], 100, 100, &[[0, 0, 100, 100]]), 0);
        assert!(draw_into(&a, &mut app, &ctx, &mut slots, 1, 1.0, false, &mut last_all, None));
        assert!(!slots[1].stale_all && slots[1].stale.is_empty());
    }

    /// Paints the whole surface one colour, plus one pixel when asked, and reports the whole surface as damage.
    struct Repaint {
        colour: u32,
        dot: Option<(usize, usize)>,
    }

    impl App for Repaint {
        fn id(&self) -> &str {
            "is.test.repaint"
        }
        fn draw(&mut self, _ctx: &Ctx, s: &mut Surface, _t: f64) -> Damage {
            let (w, h, stride) = (s.w, s.h, s.stride);
            for y in 0..h {
                s.px[y * stride..y * stride + w].fill(self.colour);
            }
            if let Some((x, y)) = self.dot {
                s.px[y * stride + x] = !self.colour;
            }
            all()
        }
    }

    /// The `ToShell::Frame` messages waiting on the shell's end, as (buffer, damage).
    fn frames_sent(shell_end: &Channel) -> Vec<(u32, Vec<Rect>)> {
        let mut out = Vec::new();
        while let Ok(Recv::Packet(p, _)) = shell_end.recv(false) {
            if let Ok(ToShell::Frame { buf, damage }) = ToShell::decode(&p) {
                out.push((buf, damage));
            }
        }
        out
    }

    #[test]
    fn a_repaint_that_changes_no_pixel_is_not_sent_and_a_small_change_is_sent_small() {
        let (a, shell_end) = crate::proto::sys::socketpair_pair().unwrap();
        let ctx = test_ctx();
        let mut slots = alloc(&a, 100, 100, 1).expect("buffers");
        let mut app = Repaint { colour: 0xff20_2020, dot: None };
        let mut last_all = false;
        // the first frame goes whole
        assert!(draw_into(&a, &mut app, &ctx, &mut slots, 0, 0.0, true, &mut last_all, None));
        assert_eq!(frames_sent(&shell_end), vec![(0, vec![[0, 0, 100, 100]])]);
        // the same picture again, into the other buffer: nothing goes out and the buffer is not held
        assert!(!draw_into(&a, &mut app, &ctx, &mut slots, 1, 1.0, false, &mut last_all, Some(0)));
        assert!(frames_sent(&shell_end).is_empty(), "the shell hears nothing of a repaint that changed nothing");
        assert!(!slots[1].held && last_all, "the unsent buffer is free and the last frame still stands");
        // one pixel differs: that pixel is the damage
        app.dot = Some((7, 9));
        assert!(draw_into(&a, &mut app, &ctx, &mut slots, 1, 2.0, false, &mut last_all, Some(0)));
        assert_eq!(frames_sent(&shell_end), vec![(1, vec![[7, 9, 1, 1]])]);
        assert!(slots[1].held);
        // without knowing which buffer the shell shows (the last frame's slot is the only free one) the app's damage is sent as it is
        slots[0].held = false;
        assert!(draw_into(&a, &mut app, &ctx, &mut slots, 0, 3.0, false, &mut last_all, None));
        assert_eq!(frames_sent(&shell_end), vec![(0, vec![[0, 0, 100, 100]])]);
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
mod window_pixel_tests {
    use super::*;
    use crate::app_windows::{WindowId, WindowSpec};
    use std::collections::BTreeMap;

    /// Paints the main window and every extra window fully, and records which windows were drawn.
    #[derive(Default)]
    struct Two {
        drawn: Vec<u32>,
        main_drawn: usize,
    }

    impl App for Two {
        fn id(&self) -> &str {
            "is.test.two"
        }
        fn draw(&mut self, _ctx: &Ctx, s: &mut Surface, _t: f64) -> Damage {
            self.main_drawn += 1;
            let mut d = Damage::new(s.w, s.h);
            d.all();
            d
        }
        fn draw_window(&mut self, _ctx: &Ctx, window: WindowId, s: &mut Surface, _t: f64) -> Damage {
            self.drawn.push(window.0);
            let mut d = Damage::new(s.w, s.h);
            d.all();
            d
        }
    }

    #[test]
    fn each_window_gets_its_own_slot_pair_and_the_shell_is_told_each_pair() {
        let (a, b) = crate::proto::sys::socketpair_pair().unwrap();
        let one = alloc_pair(&a, "bifrost-frame-win1", 10, 10, |w, h| ToShell::WindowBuffers { id: 1, w, h, generation: 1 }).expect("window 1");
        let two = alloc_pair(&a, "bifrost-frame-win2", 10, 10, |w, h| ToShell::WindowBuffers { id: 2, w, h, generation: 1 }).expect("window 2");
        assert_ne!(one[0].map.pixels().as_ptr(), two[0].map.pixels().as_ptr());
        assert_ne!(one[0].map.pixels().as_ptr(), one[1].map.pixels().as_ptr());
        for want in [1, 2] {
            let Ok(Recv::Packet(p, fds)) = b.recv(false) else { panic!("the shell gets the buffers") };
            let ToShell::WindowBuffers { id, w, h, generation } = ToShell::decode(&p).unwrap() else { panic!("WindowBuffers") };
            assert_eq!((id, w, h, generation), (want, 10, 10, 1));
            assert_eq!(fds.len(), 2);
        }
    }

    #[test]
    fn the_slot_choice_skips_the_last_frame_and_the_held_slots() {
        assert_eq!(pick_free([false, false], Some(0)), Some(1));
        assert_eq!(pick_free([false, true], Some(0)), Some(0));
        assert_eq!(pick_free([true, false], None), Some(1));
        assert_eq!(pick_free([true, true], None), None);
    }

    #[test]
    fn generations_count_up_and_never_reach_zero() {
        assert_eq!(next_generation(0), 1);
        assert_eq!(next_generation(4), 5);
        assert_eq!(next_generation(u32::MAX), 1);
    }

    #[test]
    fn a_window_is_sized_in_pixels_at_the_scale_and_kept_in_range() {
        assert_eq!(window_px(480.0, 560.0, 1.5), (720, 840));
        assert_eq!(window_px(f32::NAN, 10.0, 1.0), (1, 10));
        assert_eq!(window_px(1.0e9, 0.0, 1.0), (8192, 1));
    }

    #[test]
    fn an_extra_window_is_drawn_when_new_or_due_and_its_pixels_go_when_it_closes() {
        let (a, _b) = crate::proto::sys::socketpair_pair().unwrap();
        let ctx = test_ctx();
        let id = ctx.windows.borrow_mut().open(WindowSpec::new("Card", 40.0, 30.0)).unwrap();
        let mut app = Two::default();
        let mut pixels = BTreeMap::new();
        draw_windows(&a, &mut app, &ctx, &mut pixels, 0.0, false);
        assert_eq!(app.drawn, vec![id.0]);
        assert_eq!(pixels[&id.0].size, (40, 30));
        // nothing changed and no frame is due: the window is not drawn again
        draw_windows(&a, &mut app, &ctx, &mut pixels, 1.0, false);
        assert_eq!(app.drawn.len(), 1);
        // a frame is due for the app: the window is drawn again
        draw_windows(&a, &mut app, &ctx, &mut pixels, 2.0, true);
        assert_eq!(app.drawn.len(), 2);
        // closed: its pixels go, and the main window was never drawn through the window path
        ctx.windows.borrow_mut().close(id).unwrap();
        draw_windows(&a, &mut app, &ctx, &mut pixels, 3.0, true);
        assert!(pixels.is_empty());
        assert_eq!(app.drawn.len(), 2);
        assert_eq!(app.main_drawn, 0);
    }

    #[test]
    fn a_window_opened_this_turn_is_known_to_the_shell_before_its_buffers_arrive() {
        let (a, b) = crate::proto::sys::socketpair_pair().unwrap();
        let ctx = test_ctx();
        let id = ctx.open_window(WindowSpec::new("New Message", 40.0, 30.0)).unwrap();
        let mut app = Two::default();
        let mut pixels = BTreeMap::new();
        draw_windows(&a, &mut app, &ctx, &mut pixels, 0.0, false);
        let mut seen = Vec::new();
        while let Ok(Recv::Packet(p, _)) = b.recv(false) {
            seen.push(ToShell::decode(&p).unwrap());
        }
        assert!(matches!(seen.first(), Some(ToShell::WindowOpen { id: o, .. }) if *o == id.0), "the open request is first: {seen:?}");
        let buffers = seen.iter().position(|m| matches!(m, ToShell::WindowBuffers { id: o, .. } if *o == id.0)).expect("the buffers follow");
        let frame = seen.iter().position(|m| matches!(m, ToShell::WindowFrame { id: o, .. } if *o == id.0)).expect("and the first frame");
        assert!(buffers > 0 && frame > buffers, "open, then buffers, then the frame: {seen:?}");
    }

    #[test]
    fn a_release_frees_only_the_buffer_and_generation_it_names() {
        let (a, _b) = crate::proto::sys::socketpair_pair().unwrap();
        let ctx = test_ctx();
        let id = ctx.windows.borrow_mut().open(WindowSpec::new("Card", 8.0, 8.0)).unwrap().0;
        let mut app = Two::default();
        let mut pixels = BTreeMap::new();
        draw_windows(&a, &mut app, &ctx, &mut pixels, 0.0, false);
        let g = pixels[&id].generation;
        assert!(pixels[&id].slots[0].held);
        assert!(!release_window_buffer(&mut pixels, id, 0, g + 1), "a stale generation is ignored");
        assert!(pixels[&id].slots[0].held);
        assert!(release_window_buffer(&mut pixels, id, 0, g));
        assert!(!pixels[&id].slots[0].held);
        assert!(!release_window_buffer(&mut pixels, 99, 0, g), "an unknown window is ignored");
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

#[cfg(test)]
mod publish_tests {
    use super::{clipboard_changed, merge_materials};
    use crate::proto::{Material, MaterialKind};
    use bifrost_ui::Rect as UiRect;

    fn mat(kind: MaterialKind, x: f32, y: f32, w: f32, h: f32) -> Material {
        Material::new(kind, UiRect { x, y, w, h }, 0.0)
    }

    #[test]
    fn a_partial_redraw_keeps_the_regions_it_did_not_redraw() {
        let sidebar = mat(MaterialKind::Sidebar, 0.0, 0.0, 200.0, 600.0);
        let toolbar = mat(MaterialKind::Titlebar, 200.0, 0.0, 600.0, 52.0);
        let mut stored = vec![sidebar, toolbar];
        // only the sidebar redrew, and it declared its material again
        merge_materials(&mut stored, vec![sidebar], &[(0.0, 0.0, 200.0, 600.0)], 0.0);
        assert_eq!(stored, vec![toolbar, sidebar], "the toolbar band survives, the sidebar is not duplicated");
        // the sidebar redrew without a material (it was removed): its region goes, the toolbar stays
        merge_materials(&mut stored, vec![], &[(0.0, 0.0, 200.0, 600.0)], 0.0);
        assert_eq!(stored, vec![toolbar]);
        // a region only partly inside the damage was not redrawn as a whole: it stays
        merge_materials(&mut stored, vec![], &[(300.0, 0.0, 400.0, 52.0)], 0.0);
        assert_eq!(stored, vec![toolbar]);
        // the same region declared with a new size replaces the old one
        let wider = mat(MaterialKind::Sidebar, 0.0, 0.0, 240.0, 600.0);
        merge_materials(&mut stored, vec![wider], &[(0.0, 0.0, 240.0, 600.0)], 0.0);
        assert_eq!(stored, vec![toolbar, wider]);
    }

    #[test]
    fn pixel_rounding_does_not_keep_a_redrawn_region() {
        let sidebar = mat(MaterialKind::Sidebar, 0.0, 0.0, 200.0, 600.0);
        let mut stored = vec![sidebar];
        // the damage is a hair smaller than the region once converted from px
        merge_materials(&mut stored, vec![], &[(0.0, 0.0, 199.7, 600.0)], 0.64);
        assert!(stored.is_empty());
    }

    #[test]
    fn the_stored_list_never_exceeds_the_wire_limit() {
        let mut stored = Vec::new();
        let fresh: Vec<Material> = (0..40).map(|i| mat(MaterialKind::Sidebar, i as f32, 0.0, 1.0, 1.0)).collect();
        merge_materials(&mut stored, fresh, &[], 0.0);
        assert_eq!(stored.len(), crate::proto::MAX_MATERIALS);
    }

    #[test]
    fn the_clipboard_is_cloned_only_when_its_revision_moved() {
        let _turn = bifrost_render::clipboard::tests::exclusive();
        let (mut rev, mut seen) = (crate::kit::clipboard::revision(), crate::kit::clipboard::text());
        assert_eq!(clipboard_changed(&mut rev, &mut seen), None, "nothing copied");
        crate::kit::clipboard::set_text("copied");
        assert_eq!(clipboard_changed(&mut rev, &mut seen).as_deref(), Some("copied"));
        assert_eq!(clipboard_changed(&mut rev, &mut seen), None, "reported once");
        // the shell's own copy (`ToApp::Clipboard`) is recorded by the loop and is not echoed back
        crate::kit::clipboard::set_text("from the shell");
        seen = crate::kit::clipboard::text();
        assert_eq!(clipboard_changed(&mut rev, &mut seen), None, "same text as already known");
        crate::kit::clipboard::set_text("");
        assert_eq!(clipboard_changed(&mut rev, &mut seen).as_deref(), Some(""), "a cleared clipboard is reported as empty text");
    }
}
