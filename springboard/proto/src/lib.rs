//! Wire format between SpringBoard and an app process (docs/apps.md).
//!
//! One message per `SOCK_SEQPACKET` packet, little-endian, `u32 tag` first.
//! Strings are `u32 len` + UTF-8. Unknown tags decode to
//! `Err(DecodeError::UnknownTag)` so both sides can skip them.

/// Bumped on every breaking change of this module.
pub use bifrost_ui::{Command, Commands, Node, Role, Rect as UiRect};

pub const SDK_VERSION: u32 = 2;
/// Largest packet either side sends (damage lists and text are clamped to it).
pub mod sys;
pub mod pbblob;
pub mod svc;
pub mod rich;
pub mod extras;
pub use extras::{LoginOp, LoginStatus, StatusMenuItem};
pub mod quicksearch;
pub mod peek;
pub mod axtext;
pub mod axrotor;
pub mod guided;
pub mod touch;
pub mod pointer;
pub use bifrost_ui::touch::{Raw as RawTouch, Sample as TouchSample, Touch as TouchPoint, TouchEvent, TouchPhase, TouchTracker, TouchType};
pub use bifrost_ui::ax::{AxCustomRotor, AxMeta, AxText, Priority as AnnouncePriority};
pub use guided::GuidedRestriction;
pub mod workspace;
pub use workspace::{WorkspaceEventKind, WorkspaceOp, WorkspaceStatus};
pub use rich::{dragop, DragItem, DragPhase, DragPreview, NotifAction, NotifCategory, NotifRequest, NotifSettings, NotifStatus, Payload, TextInput, NOTIF_AUTH_ALERT, NOTIF_AUTH_BADGE, NOTIF_AUTH_CRITICAL, NOTIF_AUTH_MASK, NOTIF_AUTH_PROVISIONAL, NOTIF_AUTH_SOUND, NOTIF_DEFAULT_ACTION, NOTIF_DISMISS_ACTION, NOTIF_PRESENT_BADGE, NOTIF_PRESENT_BANNER, NOTIF_PRESENT_LIST, NOTIF_PRESENT_MASK, NOTIF_PRESENT_SOUND, NotifTrigger, MAX_NOTIF_ATTACHMENTS, MAX_NOTIF_ATTACHMENT_PATH, NOTIF_CATEGORY_CUSTOM_DISMISS, NOTIF_CATEGORY_HIDDEN_SHOW_SUBTITLE, NOTIF_CATEGORY_HIDDEN_SHOW_TITLE};

pub const MAX_PACKET: usize = 16 * 1024;
/// The most user info pairs a quick action may carry on the wire.
pub const MAX_QUICK_INFO: u32 = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodeError {
    UnknownTag(u32),
    Truncated,
    BadUtf8,
    BadValue,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TouchKind {
    Down = 0,
    Move = 1,
    Up = 2,
    Cancel = 3,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Configure {
    pub w: u32,
    pub h: u32,
    pub scale: f32,
    pub safe_top: f32,
    pub safe_bottom: f32,
    pub dark: bool,
    pub text_scale: f32,
    pub reduce_motion: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyInput {
    Text(String),
    Code(u32),
}

/// Input for one window of an app (`ToApp::WindowInput`): the keys, menu items, commands and text edits that go to the
/// window with key focus, and the pointer events (wheel, hover, gestures) that go to the window under the pointer. The
/// main window gets the same input in the messages every app already handles (`WindowInput::to_app` with window 0), so
/// an app without extra windows is unchanged.
#[derive(Clone, Debug, PartialEq)]
pub enum WindowInput {
    Key(KeyInput),
    Command(Command),
    MenuAction(u32),
    /// as `ToApp::SetMarkedText`
    SetMarkedText { text: String, sel_start: u32, sel_len: u32 },
    /// as `ToApp::UnmarkText`
    UnmarkText { commit: bool },
    /// as `ToApp::DeleteSurrounding`
    DeleteSurrounding { before: u32, after: u32 },
    /// as `ToApp::Pointer`, for the window under the pointer (not the key window), sent only when the app asked for
    /// that kind (`ToShell::PointerInterest`). An extra window's locations and wheel deltas are window points, like
    /// `ToApp::WindowTouch`; window 0 gets `ToApp::Pointer` in surface pixels as before.
    Pointer(bifrost_ui::pointer_event::PointerEvent),
}

impl WindowInput {
    /// The message that carries this input to `window`. Window 0 is the main window and keeps the messages every app
    /// already handles; an extra window gets `ToApp::WindowInput`.
    pub fn to_app(self, window: u32) -> ToApp {
        if window != 0 {
            return ToApp::WindowInput { window, input: self };
        }
        match self {
            WindowInput::Key(k) => ToApp::Key(k),
            WindowInput::Command(c) => ToApp::Command(c),
            WindowInput::MenuAction(id) => ToApp::MenuAction(id),
            WindowInput::SetMarkedText { text, sel_start, sel_len } => ToApp::SetMarkedText { text, sel_start, sel_len },
            WindowInput::UnmarkText { commit } => ToApp::UnmarkText { commit },
            WindowInput::DeleteSurrounding { before, after } => ToApp::DeleteSurrounding { before, after },
            WindowInput::Pointer(ev) => ToApp::Pointer(ev),
        }
    }
}

/// A Ctrl or Cmd chord the shell did not use, sent as `KeyInput::Code` with `CHORD_FLAG` set.
/// The SDK checks versions exactly, so chords ride on the existing message: an app built
/// before chords sees a code it does not match and ignores it.
pub const CHORD_FLAG: u32 = 0x1_0000;
pub const CHORD_CTRL: u32 = 1;
pub const CHORD_SHIFT: u32 = 2;
pub const CHORD_ALT: u32 = 4;
pub const CHORD_META: u32 = 8;

/// The modifier keys held when a pointer press or touch happened (`NSEvent.modifierFlags`). The shell sets them from its
/// live keyboard state on every `ToApp::Touch` and `ToApp::WindowTouch`, so a click can be a Shift-click (extend a
/// selection) or a Cmd-click (toggle one item). The bits are the reference's `NSEventModifierFlags` raw values; a bit
/// this crate does not name is dropped on decode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ModifierFlags(u32);

impl ModifierFlags {
    pub const EMPTY: ModifierFlags = ModifierFlags(0);
    /// `NSEventModifierFlagCapsLock`: Caps Lock is on.
    pub const CAPS_LOCK: ModifierFlags = ModifierFlags(1 << 16);
    /// `NSEventModifierFlagShift`
    pub const SHIFT: ModifierFlags = ModifierFlags(1 << 17);
    /// `NSEventModifierFlagControl`
    pub const CONTROL: ModifierFlags = ModifierFlags(1 << 18);
    /// `NSEventModifierFlagOption` (Alt)
    pub const OPTION: ModifierFlags = ModifierFlags(1 << 19);
    /// `NSEventModifierFlagCommand` (Meta, Super)
    pub const COMMAND: ModifierFlags = ModifierFlags(1 << 20);
    /// `NSEventModifierFlagDeviceIndependentFlagsMask` restricted to the flags above.
    pub const ALL: ModifierFlags = ModifierFlags(0x1f << 16);

    /// The wire value.
    pub const fn bits(self) -> u32 {
        self.0
    }
    /// The flags of a wire value; unknown bits are dropped.
    pub const fn from_bits_truncate(bits: u32) -> ModifierFlags {
        ModifierFlags(bits & Self::ALL.0)
    }
    pub const fn contains(self, other: ModifierFlags) -> bool {
        self.0 & other.0 == other.0
    }
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
    pub const fn union(self, other: ModifierFlags) -> ModifierFlags {
        ModifierFlags(self.0 | other.0)
    }
    /// The flags of a set of `CHORD_*` bits (a chord's modifiers).
    pub const fn from_chord(mods: u32) -> ModifierFlags {
        let mut f = 0;
        if mods & CHORD_SHIFT != 0 {
            f |= Self::SHIFT.0;
        }
        if mods & CHORD_CTRL != 0 {
            f |= Self::CONTROL.0;
        }
        if mods & CHORD_ALT != 0 {
            f |= Self::OPTION.0;
        }
        if mods & CHORD_META != 0 {
            f |= Self::COMMAND.0;
        }
        ModifierFlags(f)
    }
    /// The flags held as named keys: (shift, control, option, command, caps lock).
    pub fn from_held(shift: bool, control: bool, option: bool, command: bool, caps_lock: bool) -> ModifierFlags {
        [(shift, Self::SHIFT), (control, Self::CONTROL), (option, Self::OPTION), (command, Self::COMMAND), (caps_lock, Self::CAPS_LOCK)]
            .into_iter()
            .filter(|(on, _)| *on)
            .fold(Self::EMPTY, |all, (_, f)| all.union(f))
    }
    pub const fn shift(self) -> bool {
        self.contains(Self::SHIFT)
    }
    pub const fn control(self) -> bool {
        self.contains(Self::CONTROL)
    }
    pub const fn option(self) -> bool {
        self.contains(Self::OPTION)
    }
    pub const fn command(self) -> bool {
        self.contains(Self::COMMAND)
    }
    pub const fn caps_lock(self) -> bool {
        self.contains(Self::CAPS_LOCK)
    }
}

impl std::ops::BitOr for ModifierFlags {
    type Output = ModifierFlags;
    fn bitor(self, other: ModifierFlags) -> ModifierFlags {
        self.union(other)
    }
}

/// The `KeyInput::Code` the SDK delivers for Escape (the direct Wayland path and the shell's app protocol send 1).
pub const KEY_ESCAPE: u32 = 1;
/// ASCII ESC. Nothing sends it today; an app matches it next to `KEY_ESCAPE` so either reading cancels.
pub const KEY_ESCAPE_ASCII: u32 = 27;

impl KeyInput {
    /// `key` is the evdev code of a letter, digit or punctuation key; `mods` is a set of CHORD_* bits.
    pub fn chord(key: u32, mods: u32) -> Self {
        KeyInput::Code(CHORD_FLAG | (mods & 0xff) << 8 | (key & 0xff))
    }
    /// The (evdev key, CHORD_* bits) of a chord, or None for plain keys and text.
    pub fn as_chord(&self) -> Option<(u32, u32)> {
        match self {
            KeyInput::Code(c) if c & CHORD_FLAG != 0 => Some((c & 0xff, (c >> 8) & 0xff)),
            _ => None,
        }
    }
}

/// A damaged rectangle in pixels: x, y, w, h.
pub type Rect = [u32; 4];

/// One row of an app's context menu (`ToShell::ContextMenu`, `Ctx::context_menu`). The wire format is the three fields; a
/// separator and a key equivalent ride inside them, so apps that fill the fields by hand keep working:
/// - a separator is id 0 with an empty label and `enabled` false (`ContextItem::separator`);
/// - a key equivalent follows the title after a tab, in `KeyCombo::parse` notation (`ContextItem::key`), and the shell
///   shows it right-aligned. The key is only shown: the app still handles the chord itself.
#[derive(Clone,Debug,PartialEq)]
pub struct ContextItem {pub id:u32,pub label:String,pub enabled:bool}

/// Separates a context item's title from its key equivalent inside `ContextItem::label`.
pub const CONTEXT_KEY_SEPARATOR:char='\t';

impl ContextItem {
    /// An enabled row with no key equivalent.
    pub fn new(id:u32,title:&str)->Self {ContextItem{id,label:title.into(),enabled:true}}
    /// A line between two groups of rows.
    pub fn separator()->Self {ContextItem{id:0,label:String::new(),enabled:false}}
    pub fn enabled(mut self,on:bool)->Self {self.enabled=on;self}
    /// Shows `combo` ("cmd+i", "return", "shift+cmd+backspace") right-aligned in the row.
    pub fn key(mut self,combo:&str)->Self {let title=self.title().to_string();self.label=format!("{title}{CONTEXT_KEY_SEPARATOR}{combo}");self}
    /// True for the row `ContextItem::separator` makes.
    pub fn is_separator(&self)->bool {self.id==0&&self.label.is_empty()}
    /// The row's title, without its key equivalent.
    pub fn title(&self)->&str {self.label.split_once(CONTEXT_KEY_SEPARATOR).map_or(self.label.as_str(),|(title,_)|title)}
    /// The row's key equivalent in `KeyCombo::parse` notation, if it has one.
    pub fn key_combo(&self)->Option<&str> {self.label.split_once(CONTEXT_KEY_SEPARATOR).map(|(_,key)|key).filter(|key|!key.is_empty())}
}

/// Layout of the on-screen keyboard (`ToApp::KeyboardConfig`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum KeyboardLayout {
    #[default]
    Docked = 0,
    /// two thumb clusters with a gap in the middle
    Split = 1,
    /// narrow, slid sideways by dragging the dock handle
    Floating = 2,
}

/// What the focused field wants its return key to say (`NativeReturnKeyType`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ReturnKey {
    #[default]
    Return = 0,
    Search = 1,
    Go = 2,
    Done = 3,
    Next = 4,
    Send = 5,
}

/// Bits of `KeyboardConfig::flags`.
pub mod kbflags {
    pub const PREDICTIVE: u32 = 1;
    pub const AUTO_CAPS: u32 = 2;
    /// the terminal accessory row (Esc, Tab, arrows, paste)
    pub const ACCESSORY: u32 = 4;
    pub const SHOWN: u32 = 8;
    /// Settings > General > Keyboard: Enable Caps Lock, the "." Shortcut
    pub const CAPS_LOCK: u32 = 16;
    pub const PERIOD_SHORTCUT: u32 = 32;
}

/// The shell's configuration of the keyboard system app: what the focused field asks of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct KeyboardConfig {
    pub layout: KeyboardLayout,
    pub return_key: ReturnKey,
    pub flags: u32,
    /// the enabled input languages (Settings > Keyboard > Keyboards), a bit mask in the shell's order
    pub languages: u8,
    /// the active input language (an index into that order)
    pub language: u8,
}

impl KeyboardConfig {
    pub fn predictive(&self) -> bool {
        self.flags & kbflags::PREDICTIVE != 0
    }
    pub fn shown(&self) -> bool {
        self.flags & kbflags::SHOWN != 0
    }
}

/// What the keyboard system app tells the shell (`ToShell::KeyboardOut`); the shell forwards it to the focused field.
#[derive(Clone, Debug, PartialEq)]
pub enum KeyboardOut {
    Text(String),
    Backspace,
    Enter,
    /// the dismiss key
    Hide,
    /// terminal keys: "Escape", "Tab", "Paste", "Up", "Down", "Left", "Right"
    Named(String),
    /// the keyboard's current height in px, so the shell can lay the app above it
    Height(u32),
    /// a key sound for the shell to play when Keyboard Clicks is on: 0 letter, 1 delete, 2 modifier
    Click(u8),
    /// the user switched input language with the globe key; the shell stores it (Settings > Keyboard > Keyboards)
    Language(u8),
    /// composition in progress (candidate text); the shell forwards it as `ToApp::SetMarkedText`. `sel_*` select
    /// inside `text` in UTF-8 bytes. Empty `text` cancels the composition.
    Preedit { text: String, sel_start: u32, sel_len: u32 },
    /// the composition ended; the shell forwards it as `ToApp::UnmarkText { commit }`
    Unmark { commit: bool },
}

/// Access asked of a document grant (`NativeOpenPanel` / `NativeSavePanel`, docs/file-portal-design.md).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DocMode {
    Read = 0,
    Write = 1,
}

/// Why a document request produced no file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DocErrorCode {
    /// the user closed the panel
    Cancelled = 0,
    /// the user (account) may not open the file, or the grant belongs to another app
    Denied = 1,
    NotFound = 2,
    /// the file was replaced since it was granted
    Stale = 3,
    Revoked = 4,
    /// the shell or the app lacks the `documents` permission or the feature
    Unsupported = 5,
}

impl DocErrorCode {
    fn from_wire(v: u8) -> Option<Self> {
        Some(match v {
            0 => Self::Cancelled,
            1 => Self::Denied,
            2 => Self::NotFound,
            3 => Self::Stale,
            4 => Self::Revoked,
            5 => Self::Unsupported,
            _ => return None,
        })
    }
}

/// `NativeOpenPanel` options carried by `ToShell::DocOpenEx::options` (a bit set; 0 is not a valid open panel, the shell
/// then treats it as `FILES`). Save panels always allow New Folder (`NativeSavePanel.canCreateDirectories` defaults to true).
pub mod docopt {
    /// `allowsMultipleSelection`
    pub const MULTIPLE: u32 = 1;
    /// `canChooseDirectories`
    pub const DIRECTORIES: u32 = 2;
    /// `canChooseFiles`
    pub const FILES: u32 = 4;
    /// `canCreateDirectories`: the New Folder button
    pub const CREATE_DIRS: u32 = 8;
}

/// At most this many files one open panel answer carries.
pub const MAX_DOC_BATCH: u32 = 256;

/// A document grant's id: random, chosen by the shell, only valid for the app it was given to.
pub type GrantId = [u8; 16];

/// At most this many type filters per request, each at most `MAX_DOC_FILTER` bytes (a UTI or an extension).

/// `NativeReturnKeyType` in full. The keyboard system app only knows `ReturnKey`; `basic()` is what it can show today.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ReturnKeyType {
    #[default]
    Default = 0,
    Go = 1,
    Google = 2,
    Join = 3,
    Next = 4,
    Route = 5,
    Search = 6,
    Send = 7,
    Yahoo = 8,
    Done = 9,
    EmergencyCall = 10,
    Continue = 11,
}

impl ReturnKeyType {
    pub fn from_wire(v: u8) -> Option<Self> {
        use ReturnKeyType::*;
        Some(match v {
            0 => Default, 1 => Go, 2 => Google, 3 => Join, 4 => Next, 5 => Route, 6 => Search, 7 => Send,
            8 => Yahoo, 9 => Done, 10 => EmergencyCall, 11 => Continue,
            _ => return None,
        })
    }
    /// The nearest label of the keyboard app's `ReturnKey`.
    pub fn basic(self) -> ReturnKey {
        use ReturnKeyType::*;
        match self {
            Default => ReturnKey::Return,
            Go | Route | Join | EmergencyCall => ReturnKey::Go,
            Google | Yahoo | Search => ReturnKey::Search,
            Next | Continue => ReturnKey::Next,
            Send => ReturnKey::Send,
            Done => ReturnKey::Done,
        }
    }
}

/// `NativeKeyboardType` / `NativeTextContentType` in short.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TextContentKind {
    #[default]
    Default = 0,
    Ascii = 1,
    Number = 2,
    Url = 3,
    Phone = 4,
    Email = 5,
    Decimal = 6,
    Password = 7,
}

impl TextContentKind {
    pub fn from_wire(v: u8) -> Option<Self> {
        use TextContentKind::*;
        Some(match v { 0 => Default, 1 => Ascii, 2 => Number, 3 => Url, 4 => Phone, 5 => Email, 6 => Decimal, 7 => Password, _ => return None })
    }
}

/// Bits of `InputTraits::flags`.
pub mod tiflags {
    /// `autocorrectionType`
    pub const AUTOCORRECT: u32 = 1;
    /// `spellCheckingType`
    pub const SPELLCHECK: u32 = 2;
    /// `autocapitalizationType = sentences`
    pub const CAPITALIZE: u32 = 4;
    /// `enablesReturnKeyAutomatically`: Return stays disabled while the field is empty
    pub const RETURN_NEEDS_TEXT: u32 = 8;
    /// `isSecureTextEntry`: no prediction, no marked text shown to the keyboard
    pub const SECURE: u32 = 16;
    /// `smartQuotes / smartDashes / smartInsertDelete`
    pub const SMART_PUNCTUATION: u32 = 32;
}

/// The traits of the focused text field (`NativeTextInputTraits`), ToShell 0x152.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct InputTraits {
    pub return_key: ReturnKeyType,
    pub content: TextContentKind,
    pub flags: u32,
}

/// `ProcessInfo.ActivityOptions` bits (`ToShell::Activity`).
pub mod activity {
    /// `.userInitiated`: work the user asked for; do not throttle.
    pub const USER_INITIATED: u32 = 1;
    /// `.latencyCritical`: timing-sensitive work (audio, capture); do not throttle.
    pub const LATENCY_CRITICAL: u32 = 2;
    /// `.background`: may be deferred.
    pub const BACKGROUND: u32 = 4;
    /// `.idleSystemSleepDisabled`: keep the machine awake.
    pub const IDLE_SYSTEM_SLEEP_DISABLED: u32 = 8;
}

pub const MAX_ACCESSORY_ITEMS: usize = 8;

/// One button of the `inputAccessoryView` (a row above the keyboard), ToShell 0x153. Pressing it answers
/// `ToApp::AccessoryAction(id)`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessoryItem {
    pub id: u32,
    pub label: String,
    pub enabled: bool,
}

/// The text around the caret, for predictive text and IME (`NativeTextInput` `selectedTextRange`, `markedTextRange`,
/// `textInRange:`), ToShell 0x150. All offsets are UTF-8 byte offsets into `text` (the Wayland `text-input-v3`
/// convention); `text` is a window of the field around the caret, at most `MAX_SURROUNDING` bytes.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct TextState {
    pub text: String,
    pub cursor: u32,
    /// the other end of the selection; equal to `cursor` when nothing is selected
    pub anchor: u32,
    /// the marked (composing) range, if any
    pub marked: Option<(u32, u32)>,
}

pub const MAX_SURROUNDING: usize = 4000;

pub const MAX_DOC_FILTERS: usize = 16;
pub const MAX_DOC_FILTER: usize = 64;
/// Panel titles and file names are cut to this many characters.
pub const MAX_DOC_TEXT: usize = 255;
/// The longest path a `DocGrantPath` carries (bytes).
pub const MAX_DOC_PATH: usize = 4096;

/// The stacking level an app asks for its window (`ToShell::WindowLevel`). A floating window stays above the normal
/// windows of the desktop, and below menus, panels and the menu bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum WindowLevel {
    #[default]
    Normal = 0,
    Floating = 1,
}

impl WindowLevel {
    /// The wire byte; an unknown byte is a bad value.
    pub fn from_wire(v: u8) -> Option<Self> {
        match v {
            0 => Some(WindowLevel::Normal),
            1 => Some(WindowLevel::Floating),
            _ => None,
        }
    }
}

/// Flags of `ToShell::WindowOpen`.
pub mod window_flags {
    /// The app wants `ToApp::WindowTitleDoubleClick` instead of the shell's zoom when the title bar is double-clicked.
    pub const TITLE_DOUBLE_CLICK: u32 = 1;
}

/// Shell -> app.
#[derive(Clone, Debug, PartialEq)]
pub enum ToApp {
    Configure(Configure),
    /// A pointer press, drag or release (or a finger), with the modifier keys held at the time (`ModifierFlags`).
    Touch { kind: TouchKind, id: u32, x: f32, y: f32, t_ms: u32, mods: ModifierFlags },
    /// Every touch of one frame, with phases, previous locations, tap counts and optional force and radius (see the
    /// `touch` module). The shell sends it ahead of the matching `Touch`. Tag 0x1f0.
    Touches(TouchEvent),
    /// The scroll wheel, the pointer over the window and the two-finger gestures (see `pointer`), sent only to an app
    /// that announced it wants them (`ToShell::PointerInterest`). Tag 0x1f1.
    Pointer(bifrost_ui::pointer_event::PointerEvent),
    Key(KeyInput),
    /// The shell is done with buffer `buf` of the `Buffers` set numbered `generation`; an app ignores it when
    /// that is not its current set (a late release must not free a slot of the new set).
    Release { buf: u32, generation: u32 },
    /// Several windows (`app_windows`): the shell closed window `id` from its own controls. The app drops its state for it.
    WindowClosed { id: u32 },
    /// The shell's full-screen state for the app's window changed (the green button, or the app's own request).
    /// The window takes the whole screen, with the menu bar hidden until the pointer reaches the top edge.
    FullScreen { on: bool },
    /// Several windows: the shell resized window `id` to `w` x `h` points.
    WindowConfigure { id: u32, w: f32, h: f32 },
    /// Several windows: a touch on window `id`, with the same fields as `Touch` (`touch` is the touch id).
    WindowTouch { id: u32, touch: u32, kind: TouchKind, x: f32, y: f32, t_ms: u32, mods: ModifierFlags },
    /// Several windows: buffer `buf` of window `id`'s set `generation` is free again (as `Release` for the main window).
    WindowRelease { id: u32, buf: u32, generation: u32 },
    /// Several windows: the person dragged window `window` to (x, y) points relative to the visible desktop area (the
    /// shell's clamped position). Tag 0x24d.
    WindowMoved { window: u32, x: f32, y: f32 },
    /// Several windows: the person double-clicked the title bar of window `window`. Only sent to windows opened with
    /// `window_flags::TITLE_DOUBLE_CLICK`. Tag 0x24e.
    WindowTitleDoubleClick { window: u32 },
    /// Several windows: window `window` gained (`focused`) or lost key focus. The main window is key when no extra
    /// window is, and it is told as `WindowFocus { window: 0, .. }` when focus moves back to it. Tag 0x24f.
    WindowFocus { window: u32, focused: bool },
    /// Several windows: input for the key window `window` (see `WindowInput`). Window 0 never arrives here. Tag 0x250.
    WindowInput { window: u32, input: WindowInput },
    Resume,
    Suspend,
    Quit,
    Command(Command),
    Activate(u64),
    Context {x:f32,y:f32},
    ContextAction(u32),
    /// The user chose a Home Screen quick action: its type and the user info the app gave it.
    QuickAction { kind: String, user_info: Vec<(String, String)> },
    Open { url: String },
    /// the shell clipboard text, sent on Resume and when it changes
    Clipboard { text: String },
    /// Rich pasteboard contents (`NativePasteboard`): the change count and every (UTType id, bytes) item.
    Pasteboard { change_count: u32, items: Vec<(String, Vec<u8>)> },
    /// The pasteboard changed, and this app is not the one in front: only the change count is sent, never the contents
    /// (an app in the background cannot read what was copied). The contents follow when the app comes to the front, or when
    /// it asks with `ToShell::PasteboardRead`. An app built before this message ignores it. Tag 0x192.
    PasteboardChanged { change_count: u32 },
    /// The pasteboard of `Pasteboard`, for items too large for a packet (`pbblob`): one file descriptor follows as
    /// SCM_RIGHTS, holding the items' bytes one after the other; `types` gives each item's type and length. The text for the
    /// app's text fields is the plain-text item. An app built before this message ignores it. Tag 0x191.
    PasteboardBlob { change_count: u32, types: Vec<(String, u32)> },
    /// System appearance settings (`appearance::*` bits), sent on start and whenever they change. Apps built before
    /// this message ignore it; bits an app does not know are ignored too.
    Appearance { flags: u32 },
    /// Sent only to the keyboard system app (`kind = "system"`, `provides = ["keyboard"]`).
    KeyboardConfig(KeyboardConfig),
    /// The answer to `DocOpen`, `DocSave` or `DocReopen`: one file descriptor follows as SCM_RIGHTS. For a save it is
    /// a temporary file in the chosen folder; `DocCommit` puts it in place. The app never learns the path.
    DocGranted { request: u32, grant: GrantId, mode: DocMode, name: String, size: u64 },
    DocError { request: u32, code: DocErrorCode },
    /// The user chose item `id` of the app's own main menu (`ToShell::AppMenus`), from the bar or by key equivalent.
    MenuAction(u32),
    /// The answer to `ToShell::Keychain`. `value` is the secret for a successful `Get`, empty otherwise.
    KeychainReply { request: u32, status: KeychainStatus, value: Vec<u8> },
    /// Sent before the answers of a `DocOpenEx` with `docopt::MULTIPLE`: `count` `DocGranted` (each with its
    /// descriptor and the same `request`) follow. Not sent for a single file.
    DocBatch { request: u32, count: u32 },
    /// An assistive technology (the AT-SPI bridge) acts on node `id` of the latest `Controls` (`ax set`). Apps built
    /// before this message ignore it.
    AxSet { id: u64, action: AxAction },
    /// IME composition (`setMarkedText:selectedRange:replacementRange:`), from Wayland `text-input-v3` preedit or the
    /// on-screen keyboard. Replaces the marked text (or the selection when nothing is marked) with `text`, marked;
    /// `sel_start`/`sel_len` are the selection inside `text` in UTF-8 bytes. Tag 60.
    SetMarkedText { text: String, sel_start: u32, sel_len: u32 },
    /// `unmarkText`: `commit` keeps the marked text as normal text, otherwise it is removed. Tag 61.
    UnmarkText { commit: bool },
    /// The active input source changed. `japanese` is true for the kana (Romaji) source, which the app's text input
    /// reads through `bifrost_ui::compose_method::set_japanese_source`. Sent on first contact and on each change.
    /// Tag 0x162.
    InputSource { japanese: bool },
    /// A letter held down in the focused field (the key repeat's accent hold): the field takes back the letter typed at
    /// its press and composes its accents as an open list (digits pick), as its own compose method does. Tag 0x163.
    AccentHold { letter: char },
    /// `text-input-v3` `delete_surrounding_text`: delete `before` bytes before and `after` bytes after the
    /// selection. Tag 62.
    DeleteSurrounding { before: u32, after: u32 },
    /// An `inputAccessoryView` button was pressed. Tag 63.
    AccessoryAction(u32),
    /// The user acted on a notification the app posted (`userNotificationCenter(_:didReceive:)`). `action` is the action
    /// identifier, `NOTIF_DEFAULT_ACTION` for opening it or `NOTIF_DISMISS_ACTION`; `text` is the reply of a text-input
    /// action. Delivered on start if the app was not running.
    /// `userInfo` of the request is returned with the response, as `UNNotificationResponse.notification.request.content`.
    NotificationAction { id: String, action: String, text: String, thread: String, category: String, user_info: Vec<(String, String)> },
    /// The user opened an indexed item from Search (`NativeUserActivity` of `quicksearch::ACTION_TYPE`, `continue(_:)`). 0x1c2.
    ContinueActivity { activity_type: String, unique_id: String, domain: String },
    /// The shell asks an app that declares `index_extension = true` to index all its content again (the
    /// `CSIndexExtensionRequestHandler` reindex-all request). 0x1c3.
    ReindexAll,
    /// The shell asks an app to index again the items of `identifiers` (the reindex-identifiers request). 0x1c4.
    ReindexIdentifiers { identifiers: Vec<String> },
    /// The answer to `ToShell::WorkspaceRequest`. Tag 100.
    WorkspaceReply { request: u32, status: WorkspaceStatus, items: Vec<String> },
    /// A workspace notification the app subscribed to (`ToShell::WorkspaceSubscribe`). `app_id` and `name` are empty for sleep and wake. Tag 101.
    WorkspaceEvent { kind: WorkspaceEventKind, app_id: String, name: String },
    /// The answer to `ToShell::NotifyRequest`.
    NotificationStatus { request: u32, status: NotifStatus },
    /// The answer to `ToShell::NotifyQuery` (`getDeliveredNotifications`, `getPendingNotificationRequests`).
    NotificationList { request: u32, delivered: bool, ids: Vec<String> },
    /// The answer to `ToShell::NotifySettings`. Tag 56.
    NotificationSettings { request: u32, settings: NotifSettings },
    /// The answer to `ToShell::NotifyAuthorize` (`requestAuthorization(options:)`'s `granted`). Tag 57.
    NotificationAuthorized { request: u32, granted: bool },
    /// A notification of this app arrives while the app is frontmost and the app opted in with
    /// `ToShell::NotifyPresentDelegate`. The app answers `ToShell::NotifyPresentReply { token, .. }`; with no answer
    /// within the shell's timeout the notification is not shown (`willPresent` not implemented). Tag 58.
    NotificationWillPresent { token: u32, id: String, thread: String, category: String, title: String, body: String },
    /// The user picked a target of `ToShell::Share`; `target` is the app id, "copy" for Copy, empty when dismissed.
    ShareDone { request: u32, target: String },
    /// The answer to `ToShell::LoginItem` (`SMAppService.status`). Tag 0x1b0.
    LoginItemStatus { request: u32, status: LoginStatus },
    /// The user chose item `id` in the menu of status item `item` (`NativeStatusItem.menu`). Tag 0x1b1.
    StatusItemAction { item: u32, id: u32 },
    /// Guided Access started (`active`) or ended (`ToShell::GuidedAccessRequest`). Tag 0x171.
    GuidedAccessState { active: bool },
    /// The user changed one restriction of the active guided-access session. Tag 0x172.
    GuidedRestriction { id: String, allowed: bool },
    /// A `DistributedNotificationCenter` post by another app, relayed by the shell.
    DistNotify { name: String, user_info: Vec<(String, String)> },
    /// The answer to `ToShell::ServiceCall` `request`: `ok` with a JSON result, else an error message. Tag 0x1d2.
    ServiceReply { request: u32, ok: bool, body: String },
    /// An event a system service pushes to the app (JSON `body`). Tag 0x1d3.
    ServiceEvent { service: String, event: String, body: String },
    /// A dragging session moves over the app's window (`NativeDraggingDestination`); `x`, `y` are window-local points.
    /// `items` are sent with `Entered` and `Perform`. For `Perform` the files of the items arrive first as `DocBatch`
    /// and `DocGranted` with `request == session`. Answer `Entered` and `Updated` with `ToShell::DragReply`.
    DragDest { session: u32, phase: DragPhase, x: f32, y: f32, allowed: u8, items: Vec<DragItem> },
    /// The session this app started ended (`draggingSession(_:endedAt:operation:)`); `op` is 0 when nobody took it.
    DragEnded { session: u32, op: u8 },
}

/// Keychain Services parity (`SecItem*`, `kSecClassGenericPassword`): the item is `(service, account)`; the shell
/// stores it for the asking app only (the app id comes from the connection, never from the message).
#[derive(Clone, Debug, PartialEq)]
pub enum KeychainOp {
    /// `SecItemAdd`; `Duplicate` when the item exists
    Add { service: String, account: String, value: Vec<u8> },
    /// `SecItemCopyMatching` with `kSecReturnData`
    Get { service: String, account: String },
    /// `SecItemUpdate`; `NotFound` when the item does not exist
    Update { service: String, account: String, value: Vec<u8> },
    /// `SecItemDelete`; `NotFound` when the item does not exist
    Delete { service: String, account: String },
}

/// `OSStatus` of a keychain call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeychainStatus {
    /// errSecSuccess
    Ok = 0,
    /// errSecDuplicateItem
    Duplicate = 1,
    /// errSecItemNotFound
    NotFound = 2,
    /// errSecNotAvailable: the app has no `keychain` permission
    NotPermitted = 3,
    /// errSecParam: empty or oversized service, account or value, or too many items
    Param = 4,
    /// errSecIO: the store could not be read or written
    Io = 5,
}

impl KeychainStatus {
    pub fn from_wire(v: u8) -> Option<Self> {
        Some(match v {
            0 => Self::Ok,
            1 => Self::Duplicate,
            2 => Self::NotFound,
            3 => Self::NotPermitted,
            4 => Self::Param,
            5 => Self::Io,
            _ => return None,
        })
    }
}

/// Limits of keychain items (longer fields are refused with `Param`).
pub const MAX_KEYCHAIN_NAME: usize = 255;
pub const MAX_KEYCHAIN_VALUE: usize = 8 * 1024;

/// Bits of `ToApp::Appearance { flags }`.
pub mod appearance {
    pub const INCREASE_CONTRAST: u32 = 1;
    pub const REDUCE_TRANSPARENCY: u32 = 2;
    /// Show Borders / Button Shapes: controls draw a visible outline.
    pub const SHOW_BORDERS: u32 = 4;
    pub const BOLD_TEXT: u32 = 8;
    /// The light scheme is on (Appearance: Light, or Auto in daytime). Clear means dark. The shell also sends the
    /// same as `Configure::dark` (`!light`), so the first frame is right.
    pub const LIGHT: u32 = 16;
    /// The shell presents windows on the GPU and blurs the desktop behind declared materials
    /// (`ToShell::Materials`): sidebars and toolbar bands may paint a translucent tint. Clear on the CPU/SHM path
    /// and with Reduce Transparency, where they stay opaque.
    pub const VIBRANCY: u32 = 32;
    /// The shell decodes every `MaterialKind` and `MaterialState`. Without it an app sends only `Sidebar` and
    /// `Titlebar` with the default state (`MaterialKind::legacy`), which every shell with `VIBRANCY` decodes.
    pub const MATERIALS: u32 = 64;
    /// Bits 8 to 11: the user's accent colour (Settings > Appearance), as `Accent::to_wire` (the position in
    /// `Accent::ALL` plus one). Zero means the shell did not say, and the app keeps the system blue. Controls tinted
    /// by the accent (switches, checkmarks, the default button) follow it; links stay blue.
    pub const ACCENT_SHIFT: u32 = 8;
    pub const ACCENT_MASK: u32 = 0xF << ACCENT_SHIFT;
}

/// The material model is shared with the shell and the window server (`bifrost_ui::visual_effect`): one kind, state,
/// blending and corner type for built-in shell surfaces and app windows. The kind discriminants are the wire values
/// (declaration order, guarded by a test); `MaterialKind::legacy` is what a shell without `appearance::MATERIALS` decodes.
pub use bifrost_ui::visual_effect::{BlendingMode, Corners, MaterialKind, MaterialState};

/// A vibrancy region of the app's window, in window points (the `Configure` space), with its corner radius in points.
/// On the wire the state rides in the top two bits of the kind byte, then rect and the uniform radius, then a flags byte
/// (bit 0 emphasized, bit 1 within-window blending, bit 2 per-corner radii follow as four f32). `corners` is a
/// rounded-rect mask; `None` means the uniform `radius`. Protocol 2 and later (`SDK_VERSION`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Material {
    pub kind: MaterialKind,
    pub rect: UiRect,
    pub radius: f32,
    pub state: MaterialState,
    pub blending: BlendingMode,
    pub emphasized: bool,
    pub corners: Option<Corners>,
}

impl Material {
    /// A behind-window, follows-active, not emphasized region with a uniform radius.
    pub fn new(kind: MaterialKind, rect: UiRect, radius: f32) -> Material {
        Material { kind, rect, radius, state: MaterialState::FollowsWindowActiveState, blending: BlendingMode::BehindWindow, emphasized: false, corners: None }
    }
    /// The full request for this region (a rounded-rect mask from `corners` or `radius`).
    pub fn request(&self) -> bifrost_ui::visual_effect::MaterialRequest {
        use bifrost_ui::visual_effect::{Mask, MaterialRequest};
        let mut r = MaterialRequest::new(self.kind, self.rect, self.radius).blending(self.blending).state(self.state).emphasized(self.emphasized);
        if let Some(c) = self.corners {
            r = r.mask(Mask::RoundedRect(c));
        }
        r
    }
    /// The wire form of a request; an alpha mask is dropped to its bounding rectangle (rounded rect only on the wire).
    pub fn from_request(r: &bifrost_ui::visual_effect::MaterialRequest) -> Material {
        use bifrost_ui::visual_effect::Mask;
        let corners = match &r.mask { Mask::RoundedRect(c) => Some(*c), _ => None };
        Material { kind: r.kind, rect: r.rect, radius: corners.map_or(0.0, |c| c.tl), state: r.state, blending: r.blending, emphasized: r.emphasized, corners }
    }
}

/// At most this many material regions per window.
pub const MAX_MATERIALS: usize = 16;

/// Hierarchy and state for one `Node` (`NativeAccessibility` parent/child and the state attributes), sent beside
/// `ToShell::Controls` as `ToShell::ControlsTree`. A node with no entry, or `parent: None`, is a direct child of the
/// window, which is how packets from apps that never send the tree decode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NodeExt {
    /// the `Node::id` this describes
    pub id: u64,
    /// the `Node::id` of the containing node (a `List` for its `Row`s); `None` is the window
    pub parent: Option<u64>,
    /// the node holds keyboard focus (`AXFocused`)
    pub focused: bool,
    /// `AXValue` of a toggle or checkbox; `None` when the node is not checkable
    pub checked: Option<bool>,
}

/// Maximum entries in one `ControlsTree` packet (same cap as `Controls`).
pub const MAX_NODE_EXT: usize = 128;

/// What `ToApp::AxSet` asks of a node (`AXPress`, `AXValue` write, `AXFocused` write).
#[derive(Clone, Debug, PartialEq)]
pub enum AxAction {
    Press,
    SetValue(String),
    Focus,
    /// `accessibilityPerformIncrement`: one step up on a slider or stepper.
    Increment,
    /// `accessibilityPerformDecrement`: one step down.
    Decrement,
}

/// App -> shell.
#[derive(Clone, Debug, PartialEq)]
pub enum ToShell {
    Hello { sdk_version: u32, app_id: String },
    /// two buffers of `w * h * 4` bytes follow as SCM_RIGHTS fds
    /// `generation` counts the sets the app has sent (wrapping, starting at 1).
    Buffers { w: u32, h: u32, generation: u32 },
    Frame { buf: u32, damage: Vec<Rect> },
    /// Several windows: open window `id` (the app chose the id; the shell never invents one). The title is at most 512
    /// bytes; the size is in points. The window has no pixels until `WindowBuffers` and `WindowFrame` arrive.
    /// `position` (optional, points relative to the visible desktop area) asks where the window goes; the shell clamps it to
    /// the screen. `flags` is a `window_flags` set. Tag 0x240.
    WindowOpen { id: u32, title: String, w: f32, h: f32, position: Option<(f32, f32)>, flags: u32 },
    /// Several windows: close window `id`. The shell drops its frames.
    WindowClose { id: u32 },
    /// Several windows: the window's title.
    WindowTitle { id: u32, title: String },
    /// Several windows: the window's size in points, as the app asks for it.
    WindowResize { id: u32, w: f32, h: f32 },
    /// Several windows: two buffers of `w * h * 4` bytes for window `id`, as SCM_RIGHTS fds, like `Buffers`.
    WindowBuffers { id: u32, w: u32, h: u32, generation: u32 },
    /// Several windows: the newest frame of window `id` is in buffer `buf`, like `Frame`.
    WindowFrame { id: u32, buf: u32, damage: Vec<Rect> },
    WantsKeyboard { on: bool },
    /// Which pointer events the app takes (`bifrost_ui::pointer_event::{WHEEL, HOVER, GESTURES}`); sent once the app has
    /// its first window size. An app that never sends it gets the plain touch stream only (the wheel as a drag, no
    /// hover). Tag 0x1f1.
    PointerInterest { mask: u32 },
    Haptic,
    OpenUrl { url: String },
    /// Plays the built-in alert sound `name` (one of the shell's alert tones) once, on the sound-effects role. An
    /// unknown name is dropped. Tag 0x1f4.
    PlaySystemSound { name: String },
    Notify { title: String, body: String },
    Log { text: String },
    SetClipboard { text: String },
    /// `NativePasteboard.clearContents` + `setData` for every (UTType id, bytes) item, atomically.
    PasteboardSet { items: Vec<(String, Vec<u8>)> },
    /// `PasteboardSet` for items too large for a packet (`pbblob`): one file descriptor follows as SCM_RIGHTS, holding the
    /// items' bytes one after the other; `types` gives each item's type and length. Tag 0x191.
    PasteboardBlob { types: Vec<(String, u32)> },
    /// Asks for the pasteboard's contents (`ToApp::Pasteboard` or `ToApp::PasteboardBlob`). The shell answers an app in
    /// front, and ignores one in the background. Tag 0x193.
    PasteboardRead,
    Commands(Commands),
    /// The Edit menu titles of the undo manager (`undoMenuItemTitle`, `redoMenuItemTitle`): "Undo Typing", "Redo Move".
    /// An empty string means the plain "Undo" or "Redo". Tag 0x220.
    UndoTitles { undo: String, redo: String },
    Controls(Vec<Node>),
    RemoteSharing { on: bool },
    ContextMenu {x:f32,y:f32,items:Vec<ContextItem>},
    /// The app's dock badge (`NativeDockTile.badgeLabel`): a short label such as an unread count; empty clears it.
    SetBadge { label: String },
    /// Enter (`on`) or leave full screen for the app's window (`NSWindow.toggleFullScreen`). Answered by `ToApp::FullScreen`.
    SetFullScreen { on: bool },
    /// Sent only by the keyboard system app.
    KeyboardOut(KeyboardOut),
    /// Ask the user for a file (`NativeOpenPanel`). `filters` are UTIs or extensions; they only narrow the panel's list.
    DocOpen { request: u32, mode: DocMode, title: String, filters: Vec<String> },
    /// `DocOpen` with `NativeOpenPanel` options (`docopt::*`): several files, folders, New Folder.
    DocOpenEx { request: u32, mode: DocMode, title: String, filters: Vec<String>, options: u32 },
    /// Ask the user where to save (`NativeSavePanel`).
    DocSave { request: u32, suggested_name: String, filters: Vec<String> },
    /// Put the temporary file of save `request` in place (atomic rename), or throw it away.
    DocCommit { request: u32 },
    DocAbort { request: u32 },
    /// Open an earlier grant again (a recent document), checked against the file it was given for.
    DocReopen { request: u32, grant: GrantId },
    /// The app no longer needs the grant.
    DocRelease { grant: GrantId },
    /// Save over an existing granted file without a panel (`NativeDocument` save in place, autosave): the answer is a
    /// temporary file next to it (the same reply as a save panel's), then `DocCommit` renames it over the file and the
    /// grant keeps its id. Needs a `Write` grant.
    DocSaveGrant { request: u32, grant: GrantId },
    /// A read grant for the file or folder at `path` (absolute, under the account's home), to put in the file items of a
    /// drag this window begins (`dnd::begin`). Answered with `DocGranted` (the descriptor follows) or `DocError`. Needs the
    /// `documents` permission.
    DocGrantPath { request: u32, path: String },
    /// The window's vibrancy regions (`NativeVisualEffectView` .sidebar / .titlebar), sent when they change, only after
    /// the shell set `appearance::VIBRANCY`. An empty list clears them.
    Materials(Vec<Material>),
    /// The app's own main-menu items (`NativeApp.mainMenu`), shown after the app-name menu while the app is in front.
    /// Sent when they change; an empty list clears them. The app recomputes enabled and checked state, which is
    /// `validateMenuItem:`. Choosing an item answers `ToApp::MenuAction(id)`.
    AppMenus(Vec<AppMenu>),
    /// `NativeDocument` window state: the title to show (`displayName`), whether the document has unsaved changes
    /// (`isDocumentEdited`: the close light shows the dirty dot and the title gets "- Edited") and the represented
    /// file name (`representedFilename`, a name only, never a path). Empty title: the app name.
    DocumentState { title: String, edited: bool, represented: String },
    /// Parent/child relations and focus/checked state for the nodes of the latest `Controls` packet. Optional: apps
    /// that never send it are read as a flat list of window children.
    ControlsTree(Vec<NodeExt>),
    /// A keychain call (`SecItemAdd` and friends). Needs the `keychain` permission; answered with `KeychainReply`.
    Keychain { request: u32, op: KeychainOp },
    /// Surrounding text, selection and marked range of the focused field. Sent when they change. 0x150.
    TextState(TextState),
    /// `firstRectForCharacterRange:`: where the caret (or marked text) is, in window points, so the candidate
    /// window or keyboard can position itself. 0x151.
    CaretRect { x: f32, y: f32, w: f32, h: f32 },
    /// `NativeTextInputTraits` of the focused field. 0x152.
    InputTraits(InputTraits),
    /// The `inputAccessoryView` buttons; an empty list removes the row. 0x153.
    InputAccessory(Vec<AccessoryItem>),
    /// `ProcessInfo.enableSuddenTermination` / `disableSuddenTermination`: whether the shell may SIGKILL the app
    /// at once on quit, logout or shutdown, with no Quit round trip. Sent only when the state flips. 0x160.
    SuddenTermination { enabled: bool },
    /// `ProcessInfo.beginActivity` / `endActivity`: the union of the `ActivityOptions` bits of the app's live
    /// activities (0 = none). While `USER_INITIATED` or `LATENCY_CRITICAL` is set the shell does not suspend the app
    /// when it leaves the foreground. Sent only when the union changes. 0x161.
    Activity { options: u32 },
    /// `NSWindow.level` for the app's window: `Floating` keeps it above the normal windows (the Calculator's
    /// Always on Top, Stickies' Float on Top). Sent when it changes; the shell keeps it while the app lives. Tag 0x165.
    WindowLevel(WindowLevel),
    /// The level of one of the app's extra windows (`WindowOpen`'s id). Id 0 is the main window and is refused by the
    /// shell (use `WindowLevel`). Tag 0x24c.
    WindowLevelOf { id: u32, level: WindowLevel },
    /// Several windows: move window `window` to (x, y) points relative to the visible desktop area. The shell clamps it
    /// to the screen. Tag 0x246.
    WindowMove { window: u32, x: f32, y: f32 },
    /// Several windows: collapse window `window` to its title bar (`collapsed`) or expand it back. Tag 0x247.
    WindowCollapse { window: u32, collapsed: bool },
    /// `setNotificationCategories`: replaces the app's categories. Needs the `notifications` permission.
    NotifyCategories(Vec<NotifCategory>),
    /// `CSSearchableIndex.indexSearchableItems`: at most `quicksearch::MAX_ITEMS` per message. 0x1c0.
    QuickSearchIndex(Vec<quicksearch::SearchableItem>),
    /// `deleteSearchableItems(withIdentifiers:)`, `(withDomainIdentifiers:)` and `deleteAllSearchableItems`; `kind` is a
    /// `quicksearch::DELETE_*`. 0x1c1.
    QuickSearchDelete { kind: u8, values: Vec<String> },
    /// Workspace: open, reveal, recycle and queries; files are named by the app's document grants. Tag 0x230.
    WorkspaceRequest { request: u32, op: WorkspaceOp, grants: Vec<GrantId>, arg: String },
    /// Which `WorkspaceEventKind`s (bit mask) the app wants to hear. Tag 0x231.
    WorkspaceSubscribe { mask: u32 },
    /// `QLPreviewPanel` shown or updated: the ordered items and the current index (see `peek`). 0x1e0.
    Preview { items: Vec<peek::PreviewItem>, index: u32 },
    /// `QLPreviewPanel.close()`. 0x1e1.
    PreviewClose,
    /// Selection, insertion point and visible range of text elements (`AXSelectedTextRange`, `AXVisibleCharacterRange`); a
    /// secure field sends only its length. Sent when they change. 0x210.
    AxText(Vec<AxText>),
    /// `accessibilityTitle` and `accessibilityHelp` of elements (`Node::label` is `accessibilityLabel`). 0x211.
    AxMeta(Vec<AxMeta>),
    /// the announcement-requested accessibility notification: speak `text` now. Leaves as an AT-SPI `announcement` event. 0x212.
    Announce { text: String, priority: AnnouncePriority },
    /// Custom rotors the screen reader steps through beside the standard ones (`bifrost_ui::ax_rotor`). Tag 0x213.
    AxRotors(Vec<AxCustomRotor>),
    /// The accessibility notifications an app posted for its elements (`bifrost_ui::ax::post`), in posting order; the
    /// shell turns each into an AT-SPI signal on the element's object. Tag 0x214.
    AxNotify(Vec<(u64, bifrost_ui::ax::AxNotification)>),
    /// Guided Access: `on` asks for a single-app session with these restrictions, `!on` ends it. Tag 0x170.
    GuidedAccessRequest { on: bool, restrictions: Vec<GuidedRestriction> },
    /// `UNUserNotificationCenter.add`: answered with `NotificationStatus`.
    NotifyRequest { request: u32, req: NotifRequest },
    /// `removeDeliveredNotifications` / `removePendingNotificationRequests`; no ids removes all.
    NotifyRemove { delivered: bool, ids: Vec<String> },
    /// `getDeliveredNotifications` / `getPendingNotificationRequests`: answered with `NotificationList`.
    NotifyQuery { request: u32, delivered: bool },
    /// The notification settings query: answered with `NotificationSettings`, also when the app has no
    /// `notifications` permission (then `authorized` is false). Tag 0x14a.
    NotifySettings { request: u32 },
    /// `UNUserNotificationCenter.requestAuthorization(options:)`: `options` is the `NOTIF_AUTH_*` bit set. Answered with
    /// `NotificationAuthorized`: at once when the app was asked before, otherwise after the first-time alert. Tag 0x14b.
    NotifyAuthorize { request: u32, options: u8 },
    /// Opts the app in to `ToApp::NotificationWillPresent` (`enabled`) or out (`false`). Tag 0x14d.
    NotifyPresentDelegate { enabled: bool },
    /// The answer to `ToApp::NotificationWillPresent`: `options` is the `NOTIF_PRESENT_*` bit set, empty to hide it.
    /// Tag 0x14c.
    NotifyPresentReply { token: u32, options: u8 },
    /// `UNUserNotificationCenter.add` with a calendar or time-interval trigger. The trigger sets the time, so
    /// `req.delay_ms` and `req.repeats` are not used. Answered with `NotificationStatus`. Tag 0x14e.
    NotifySchedule { request: u32, req: NotifRequest, trigger: NotifTrigger },
    /// `NativeSharingServicePicker.show(relativeTo:)`: `anchor` is in window points. Answered with `ShareDone`.
    Share { request: u32, items: Vec<Payload>, anchor: UiRect },
    /// `SMAppService.mainApp` register, unregister or status; always answered with `LoginItemStatus`. Tag 0x1b0.
    LoginItem { request: u32, op: LoginOp },
    /// `NativeStatusBar.statusItem`: create or update menu-bar extra `item` of this app (title, symbol name, menu). Tag 0x1b1.
    StatusItemSet { item: u32, visible: bool, title: String, symbol: String, menu: Vec<StatusMenuItem> },
    /// `NativeStatusBar.removeStatusItem`. Tag 0x1b2.
    StatusItemRemove { item: u32 },
    /// `DistributedNotificationCenter.post`: the shell forwards it to every other running app as `ToApp::DistNotify`.
    DistPost { name: String, user_info: Vec<(String, String)> },
    /// Calls `method` on the shell's named system service with JSON `body` (`springboard/src/shell/services/`);
    /// answered with `ToApp::ServiceReply` carrying the same `request`. Tag 0x1d2.
    ServiceCall { request: u32, service: String, method: String, body: String },
    /// `beginDraggingSession`: the shell tracks the pointer until it is released. `session` should have bit 31 set.
    DragBegin { session: u32, allowed: u8, items: Vec<DragItem>, preview: DragPreview },
    /// `registerForDraggedTypes`: the UTTypes the window accepts (conformance counts); empty accepts none.
    DragRegister { types: Vec<String> },
    /// The operation (a single `dragop` bit, 0 = refuse) the destination will perform at the current point.
    DragReply { session: u32, op: u8 },
    /// The result of `performDragOperation`.
    DragPerformed { session: u32, accepted: bool },
    /// The source gives the session up.
    DragCancel { session: u32 },
}

pub const MAX_APP_MENUS: usize = 8;
pub const MAX_APP_MENU_ITEMS: usize = 48;
/// Item titles are cut to this many characters.
pub const MAX_APP_MENU_TITLE: usize = 64;
/// How deep submenus nest (items carry a `depth`, 0 to this).
pub const MAX_APP_MENU_DEPTH: u8 = 2;

/// One top-level menu of an app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppMenu {
    pub title: String,
    /// Items in order. A submenu is an item with `submenu` set, followed by its children at `depth + 1`.
    pub items: Vec<AppMenuItem>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppMenuItem {
    /// Echoed back in `ToApp::MenuAction`; 0 for separators and submenu headers.
    pub id: u32,
    pub title: String,
    /// Key equivalent in `KeyCombo::parse` notation ("cmd+n", "cmd+shift+z"); empty for none.
    pub key: String,
    pub enabled: bool,
    pub checked: bool,
    pub separator: bool,
    pub submenu: bool,
    pub depth: u8,
    /// The app's own Print (`printDocument:` / `print:` parity: the app owns File > Print… for its documents).
    /// The shell then shows this item in place of its own File > Print… (the window snapshot) instead of next to it,
    /// and Cmd+P chooses it. See `AppMenuItem::print`. Bit 4 of the item flags on the wire.
    pub prints: bool,
}

impl AppMenuItem {
    pub fn new(id: u32, title: &str) -> Self {
        AppMenuItem { id, title: title.into(), key: String::new(), enabled: true, checked: false, separator: false, submenu: false, depth: 0, prints: false }
    }
    /// The app's own File > Print… (`printDocument:`): titled "Print…" with Cmd+P, it replaces the shell's Print… row
    /// (which prints a snapshot of the window) while the app is in front. Put it in the app's "File" menu; it is shown
    /// where the shell's row was, at the end of File, whichever menu it is declared in.
    pub fn print(id: u32) -> Self {
        AppMenuItem { prints: true, ..AppMenuItem::new(id, "Print\u{2026}").key("cmd+p") }
    }
    pub fn separator() -> Self {
        AppMenuItem { separator: true, enabled: false, ..AppMenuItem::new(0, "") }
    }
    pub fn submenu(title: &str) -> Self {
        AppMenuItem { submenu: true, ..AppMenuItem::new(0, title) }
    }
    pub fn key(mut self, combo: &str) -> Self {
        self.key = combo.into();
        self
    }
    pub fn enabled(mut self, on: bool) -> Self {
        self.enabled = on;
        self
    }
    pub fn checked(mut self, on: bool) -> Self {
        self.checked = on;
        self
    }
    pub fn depth(mut self, depth: u8) -> Self {
        self.depth = depth;
        self
    }
}

mod tag {
    pub const CONFIGURE: u32 = 1;
    pub const TOUCH: u32 = 2;
    pub const KEY: u32 = 3;
    pub const RELEASE: u32 = 4;
    pub const RESUME: u32 = 5;
    pub const SUSPEND: u32 = 6;
    pub const QUIT: u32 = 7;
    pub const OPEN: u32 = 8;
    pub const CLIPBOARD: u32 = 9;
    pub const COMMAND: u32 = 10;
    pub const ACTIVATE: u32 = 11;
    pub const CONTEXT:u32=12;pub const CONTEXT_ACTION:u32=13;
    pub const APPEARANCE: u32 = 14;
    pub const KEYBOARD_CONFIG: u32 = 15;
    pub const DOC_GRANTED: u32 = 16;
    pub const DOC_ERROR: u32 = 17;
    pub const MENU_ACTION: u32 = 18;
    pub const KEYCHAIN_REPLY: u32 = 19;
    pub const DOC_BATCH: u32 = 20;
    pub const AX_SET: u32 = 21;
    pub const SET_MARKED_TEXT: u32 = 60;
    pub const UNMARK_TEXT: u32 = 61;
    pub const DELETE_SURROUNDING: u32 = 62;
    pub const ACCESSORY_ACTION: u32 = 63;
    pub const INPUT_SOURCE: u32 = 0x162;
    pub const ACCENT_HOLD: u32 = 0x163;
    pub const QUICK_ACTION: u32 = 0x164;
    // several windows, window level and full screen: 0x240..=0x24f (0x1c0..=0x1cf belongs to `quicksearch`)
    pub const WINDOW_CLOSED: u32 = 0x248;
    pub const WINDOW_CONFIGURE: u32 = 0x249;
    pub const WINDOW_TOUCH: u32 = 0x24a;
    pub const WINDOW_RELEASE: u32 = 0x24b;
    pub const WINDOW_OPEN: u32 = 0x240;
    pub const WINDOW_CLOSE: u32 = 0x241;
    pub const WINDOW_TITLE: u32 = 0x242;
    pub const WINDOW_RESIZE: u32 = 0x243;
    pub const WINDOW_BUFFERS: u32 = 0x244;
    pub const WINDOW_FRAME: u32 = 0x245;
    /// Shell -> app: window key focus changed (`ToApp::WindowFocus`).
    pub const WINDOW_FOCUS: u32 = 0x24f;
    /// Shell -> app: input for the key window (`ToApp::WindowInput`).
    pub const WINDOW_INPUT: u32 = 0x250;

    pub const HELLO: u32 = 0x101;
    pub const BUFFERS: u32 = 0x102;
    pub const FRAME: u32 = 0x103;
    pub const WANTS_KEYBOARD: u32 = 0x104;
    pub const HAPTIC: u32 = 0x105;
    pub const OPEN_URL: u32 = 0x106;
    pub const PLAY_SYSTEM_SOUND: u32 = 0x1f4;
    pub const NOTIFY: u32 = 0x107;
    pub const LOG: u32 = 0x108;
    pub const SET_CLIPBOARD: u32 = 0x109;
    pub const COMMANDS: u32 = 0x10a;
    pub const UNDO_TITLES: u32 = 0x220;
    pub const CONTROLS: u32 = 0x10b;
    pub const REMOTE_SHARING: u32 = 0x10c;
    pub const CONTEXT_MENU:u32=0x10d;
    pub const SET_BADGE: u32 = 0x10e;
    /// App -> shell: enter or leave full screen for the app's window (`ToShell::SetFullScreen`).
    pub const SET_FULL_SCREEN: u32 = 0x24d;
    /// Shell -> app: the window's full-screen state changed (`ToApp::FullScreen`).
    pub const FULL_SCREEN: u32 = 0x24c;
    pub const KEYBOARD_OUT: u32 = 0x10f;
    pub const DOC_OPEN: u32 = 0x110;
    pub const DOC_SAVE: u32 = 0x111;
    pub const DOC_COMMIT: u32 = 0x112;
    pub const DOC_ABORT: u32 = 0x113;
    pub const DOC_REOPEN: u32 = 0x114;
    pub const DOC_RELEASE: u32 = 0x115;
    pub const DOC_OPEN_EX: u32 = 0x119;
    pub const MATERIALS: u32 = 0x116;
    pub const APP_MENUS: u32 = 0x117;
    pub const KEYCHAIN: u32 = 0x118;
    pub const CONTROLS_TREE: u32 = 0x11a;
    pub const PASTEBOARD: u32 = 0x190;
    pub const PASTEBOARD_SET: u32 = 0x190;
    /// Both directions (`ToApp::PasteboardBlob`, `ToShell::PasteboardBlob`).
    pub const PASTEBOARD_BLOB: u32 = 0x191;
    pub const PASTEBOARD_CHANGED: u32 = 0x192;
    pub const PASTEBOARD_READ: u32 = 0x193;
    pub const DOCUMENT_STATE: u32 = 0x180;
    pub const DOC_SAVE_GRANT: u32 = 0x181;
    pub const DOC_GRANT_PATH: u32 = 0x11b;
    pub const TEXT_STATE: u32 = 0x150;
    pub const CARET_RECT: u32 = 0x151;
    pub const INPUT_TRAITS: u32 = 0x152;
    pub const INPUT_ACCESSORY: u32 = 0x153;
    pub const SUDDEN_TERMINATION: u32 = 0x160;
    pub const ACTIVITY: u32 = 0x161;
    pub const WINDOW_LEVEL: u32 = 0x165;
    pub const WINDOW_LEVEL_OF: u32 = 0x24c;
    pub const WINDOW_MOVE: u32 = 0x246;
    pub const WINDOW_COLLAPSE: u32 = 0x247;
    pub const WINDOW_MOVED: u32 = 0x24d;
    pub const WINDOW_TITLE_DOUBLE_CLICK: u32 = 0x24e;
}

/// The part of `text` sent as surrounding text: at most `MAX_SURROUNDING` bytes around `caret`, cut on character
/// boundaries. Returns the slice and its start offset in `text`.
fn surrounding_window(text: &str, caret: usize) -> (&str, usize) {
    if text.len() <= MAX_SURROUNDING {
        return (text, 0);
    }
    let mut start = caret.saturating_sub(MAX_SURROUNDING / 2).min(text.len() - MAX_SURROUNDING);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    let mut end = (start + MAX_SURROUNDING).min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (&text[start..end], start)
}

fn short(s: &str) -> String {
    s.chars().take(MAX_DOC_TEXT).collect()
}

fn filters_(b: &mut Vec<u8>, filters: &[String]) {
    let kept: Vec<&String> = filters.iter().filter(|f| !f.is_empty() && f.len() <= MAX_DOC_FILTER).take(MAX_DOC_FILTERS).collect();
    u32_(b, kept.len() as u32);
    for f in kept {
        str_(b, f);
    }
}

fn mode_(v: u8) -> Result<DocMode, DecodeError> {
    match v {
        0 => Ok(DocMode::Read),
        1 => Ok(DocMode::Write),
        _ => Err(DecodeError::BadValue),
    }
}

fn u32_(b: &mut Vec<u8>, v: u32) {
    b.extend_from_slice(&v.to_le_bytes());
}

fn f32_(b: &mut Vec<u8>, v: f32) {
    b.extend_from_slice(&v.to_le_bytes());
}

fn bytes_(b: &mut Vec<u8>, v: &[u8]) {
    let n = v.len().min(MAX_KEYCHAIN_VALUE + 1);
    u32_(b, n as u32);
    b.extend_from_slice(&v[..n]);
}

/// Longest pasteboard type, in bytes.
const MAX_PB_TYPE: usize = 128;

/// The pasteboard type that marks a copy as a secret (a password): the shell does not write it to disk, does not hand it to
/// background apps, offers it to other programs only with the password-manager hint, and clears it after
/// [`PB_CONCEALED_SECS`]. It carries no data; the sender adds it beside the item's real types.
pub const PB_CONCEALED: &str = "org.nspasteboard.ConcealedType";
/// The pasteboard type that marks a copy as short-lived: the shell does not write it to disk and tells other programs not to
/// keep it (the password-manager hint), but does not clear it on a timer.
pub const PB_TRANSIENT: &str = "org.nspasteboard.TransientType";
/// How long a concealed copy stays on the pasteboard, in seconds, before the shell clears it (when nothing else was copied).
pub const PB_CONCEALED_SECS: u64 = 45;

/// Whether `t` is one of the marker types (they carry no data and are not given to apps as items).
pub fn is_pb_marker(t: &str) -> bool {
    t == PB_CONCEALED || t == PB_TRANSIENT
}

/// Whether `t` is acceptable as a pasteboard type: no control characters (a NUL would end a C string in the window server
/// client). An unacceptable item is left out of a write, on both the sending and the receiving side.
pub fn valid_pb_type(t: &str) -> bool {
    !t.chars().any(char::is_control)
}

/// `t` cut to [`MAX_PB_TYPE`] bytes on a character boundary.
fn cut_type(t: &str) -> &str {
    let mut n = t.len().min(MAX_PB_TYPE);
    while !t.is_char_boundary(n) {
        n -= 1;
    }
    &t[..n]
}

/// The largest payload one of `count` items may have to ride in a packet: its share of the room left for payloads.
fn inline_item_limit(count: usize) -> usize {
    (MAX_PACKET - 4096) / count.clamp(1, 16)
}

/// Pasteboard items: at most 16. An item whose payload does not fit its share of the packet is left out whole
/// (never cut, so no receiver ever sees a shortened payload under the full type); types are cut on a char boundary.
/// A sender with a larger item uses `pbblob` instead (`ToShell::PasteboardBlob`, `ToApp::PasteboardBlob`). An item whose
/// type has a control character is left out.
fn items_(b: &mut Vec<u8>, items: &[(String, Vec<u8>)]) {
    let items = &items[..items.len().min(16)];
    let per = inline_item_limit(items.len());
    let kept: Vec<&(String, Vec<u8>)> = items.iter().filter(|(t, d)| d.len() <= per && valid_pb_type(t)).collect();
    u32_(b, kept.len() as u32);
    for (t, d) in kept {
        str_(b, cut_type(t));
        u32_(b, d.len() as u32);
        b.extend_from_slice(d);
    }
}

/// The type and length of each item of a blob message (the bytes ride in the descriptor).
fn layout_(b: &mut Vec<u8>, types: &[(String, u32)]) {
    let types = &types[..types.len().min(pbblob::MAX_ITEMS)];
    u32_(b, types.len() as u32);
    for (t, n) in types {
        str_(b, cut_type(t));
        u32_(b, *n);
    }
}

/// The longest text one message carries (`SetClipboard`, `Clipboard`): longer text is cut by `str_`, so a sender with more
/// uses `PasteboardBlob` instead and never sends the cut text.
pub const MAX_INLINE_TEXT: usize = MAX_PACKET - 64;

fn str_(b: &mut Vec<u8>, s: &str) {
    let max = MAX_INLINE_TEXT;
    let mut n = s.len().min(max);
    while !s.is_char_boundary(n) {
        n -= 1;
    }
    u32_(b, n as u32);
    b.extend_from_slice(&s.as_bytes()[..n]);
}

/// The longest window title on the wire, in bytes.
pub const MAX_WINDOW_TITLE: usize = 512;

/// A window title cut to `MAX_WINDOW_TITLE` bytes on a character boundary (the encoder side).
fn short_title(title: &str) -> String {
    let mut end = title.len().min(MAX_WINDOW_TITLE);
    while !title.is_char_boundary(end) {
        end -= 1;
    }
    title[..end].to_string()
}

/// A decoded window title; a title over the limit is refused.
fn checked_title(title: String) -> Result<String, DecodeError> {
    if title.len() > MAX_WINDOW_TITLE { Err(DecodeError::BadValue) } else { Ok(title) }
}

/// `WindowInput` after the window id: a kind byte, then the payload (`0` key, `1` command, `2` menu item, `3` marked
/// text, `4` unmark, `5` delete surrounding, `6` pointer: the `ToApp::Pointer` kind byte and payload, see `pointer`).
fn window_input_encode(b: &mut Vec<u8>, input: &WindowInput) {
    match input {
        WindowInput::Key(KeyInput::Text(s)) => {
            b.push(0);
            b.push(0);
            str_(b, s);
        }
        WindowInput::Key(KeyInput::Code(c)) => {
            b.push(0);
            b.push(1);
            u32_(b, *c);
        }
        WindowInput::Command(c) => {
            b.push(1);
            u32_(b, *c as u32);
        }
        WindowInput::MenuAction(id) => {
            b.push(2);
            u32_(b, *id);
        }
        WindowInput::SetMarkedText { text, sel_start, sel_len } => {
            b.push(3);
            str_(b, text);
            u32_(b, *sel_start);
            u32_(b, *sel_len);
        }
        WindowInput::UnmarkText { commit } => {
            b.push(4);
            b.push(*commit as u8);
        }
        WindowInput::DeleteSurrounding { before, after } => {
            b.push(5);
            u32_(b, *before);
            u32_(b, *after);
        }
        WindowInput::Pointer(ev) => {
            b.push(6);
            pointer::encode_event(ev, b);
        }
    }
}

fn window_input_decode(r: &mut Reader) -> Result<WindowInput, DecodeError> {
    Ok(match r.u8()? {
        0 => match r.u8()? {
            0 => WindowInput::Key(KeyInput::Text(r.str()?)),
            1 => WindowInput::Key(KeyInput::Code(r.u32()?)),
            _ => return Err(DecodeError::BadValue),
        },
        1 => WindowInput::Command(Command::from_wire(r.u32()?).ok_or(DecodeError::BadValue)?),
        2 => WindowInput::MenuAction(r.u32()?),
        3 => {
            let text = r.str()?;
            let (sel_start, sel_len) = (r.u32()?, r.u32()?);
            let end = sel_start as u64 + sel_len as u64;
            if end > text.len() as u64 || !text.is_char_boundary(sel_start as usize) || !text.is_char_boundary(end as usize) {
                return Err(DecodeError::BadValue);
            }
            WindowInput::SetMarkedText { text, sel_start, sel_len }
        }
        4 => WindowInput::UnmarkText { commit: r.u8()? != 0 },
        5 => WindowInput::DeleteSurrounding { before: r.u32()?, after: r.u32()? },
        6 => WindowInput::Pointer(pointer::decode_event(r)?),
        _ => return Err(DecodeError::BadValue),
    })
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        if self.0.len() < n {
            return Err(DecodeError::Truncated);
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Ok(a)
    }
    fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> Result<u32, DecodeError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn f32(&mut self) -> Result<f32, DecodeError> {
        Ok(f32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    /// The trailing `ModifierFlags` of a touch; none held when a sender built before them left them out.
    fn modifiers(&mut self) -> Result<ModifierFlags, DecodeError> {
        if self.0.is_empty() {
            return Ok(ModifierFlags::EMPTY);
        }
        Ok(ModifierFlags::from_bits_truncate(self.u32()?))
    }
    fn items(&mut self) -> Result<Vec<(String, Vec<u8>)>, DecodeError> {
        let n = self.u32()? as usize;
        if n > 16 {
            return Err(DecodeError::BadValue);
        }
        let mut v = Vec::with_capacity(n);
        for _ in 0..n {
            let t = self.str()?;
            if t.len() > MAX_PB_TYPE {
                return Err(DecodeError::BadValue);
            }
            let l = self.u32()? as usize;
            let data = self.take(l)?.to_vec();
            // a type with a control character cannot be offered to other programs: the item is dropped, not the message
            if valid_pb_type(&t) {
                v.push((t, data));
            }
        }
        Ok(v)
    }
    /// The layout of a blob message: at most 16 types of at most 128 bytes, the lengths within the blob limits.
    fn layout(&mut self) -> Result<Vec<(String, u32)>, DecodeError> {
        let n = self.u32()? as usize;
        if n > pbblob::MAX_ITEMS {
            return Err(DecodeError::BadValue);
        }
        let mut v = Vec::with_capacity(n);
        for _ in 0..n {
            let t = self.str()?;
            if t.len() > MAX_PB_TYPE || !valid_pb_type(&t) {
                return Err(DecodeError::BadValue);
            }
            v.push((t, self.u32()?));
        }
        pbblob::check(&v).map_err(|_| DecodeError::BadValue)?;
        Ok(v)
    }
    fn str(&mut self) -> Result<String, DecodeError> {
        let n = self.u32()? as usize;
        String::from_utf8(self.take(n)?.to_vec()).map_err(|_| DecodeError::BadUtf8)
    }
    /// A secret; one byte over `MAX_KEYCHAIN_VALUE` is still read so the store can answer `Param`.
    fn bytes(&mut self) -> Result<Vec<u8>, DecodeError> {
        let n = self.u32()? as usize;
        if n > MAX_KEYCHAIN_VALUE + 1 {
            return Err(DecodeError::BadValue);
        }
        Ok(self.take(n)?.to_vec())
    }
    fn grant(&mut self) -> Result<GrantId, DecodeError> {
        Ok(self.take(16)?.try_into().unwrap())
    }
    /// A short text (title, file name): longer than `MAX_DOC_TEXT` characters is refused, not cut.
    fn short(&mut self) -> Result<String, DecodeError> {
        let s = self.str()?;
        if s.chars().count() > MAX_DOC_TEXT {
            return Err(DecodeError::BadValue);
        }
        Ok(s)
    }
    fn filters(&mut self) -> Result<Vec<String>, DecodeError> {
        let n = self.u32()? as usize;
        if n > MAX_DOC_FILTERS {
            return Err(DecodeError::BadValue);
        }
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            let f = self.str()?;
            if f.is_empty() || f.len() > MAX_DOC_FILTER {
                return Err(DecodeError::BadValue);
            }
            out.push(f);
        }
        Ok(out)
    }
}

impl ToApp {
    pub fn encode(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(32);
        match self {
            ToApp::Configure(c) => {
                u32_(&mut b, tag::CONFIGURE);
                u32_(&mut b, c.w);
                u32_(&mut b, c.h);
                f32_(&mut b, c.scale);
                f32_(&mut b, c.safe_top);
                f32_(&mut b, c.safe_bottom);
                b.push(c.dark as u8);
                f32_(&mut b, c.text_scale);
                b.push(c.reduce_motion as u8);
            }
            ToApp::Context{x,y}=>{u32_(&mut b,tag::CONTEXT);f32_(&mut b,*x);f32_(&mut b,*y);},
            ToApp::ContextAction(id)=>{u32_(&mut b,tag::CONTEXT_ACTION);u32_(&mut b,*id);},
            ToApp::MenuAction(id)=>{u32_(&mut b,tag::MENU_ACTION);u32_(&mut b,*id);},
            ToApp::QuickAction{kind,user_info}=>{u32_(&mut b,tag::QUICK_ACTION);str_(&mut b,kind);u32_(&mut b,user_info.len() as u32);for (k,v) in user_info {str_(&mut b,k);str_(&mut b,v);}},
            ToApp::Touch { kind, id, x, y, t_ms, mods } => {
                u32_(&mut b, tag::TOUCH);
                b.push(*kind as u8);
                u32_(&mut b, *id);
                f32_(&mut b, *x);
                f32_(&mut b, *y);
                u32_(&mut b, *t_ms);
                // the modifiers ride at the end: a decoder built before them stops at `t_ms`
                u32_(&mut b, mods.bits());
            }
            ToApp::Key(k) => {
                u32_(&mut b, tag::KEY);
                match k {
                    KeyInput::Text(s) => {
                        b.push(0);
                        str_(&mut b, s);
                    }
                    KeyInput::Code(c) => {
                        b.push(1);
                        u32_(&mut b, *c);
                    }
                }
            }
            ToApp::Release { buf, generation } => {
                u32_(&mut b, tag::RELEASE);
                u32_(&mut b, *buf);
                u32_(&mut b, *generation);
            }
            ToApp::WindowMoved { window, x, y } => {
                u32_(&mut b, tag::WINDOW_MOVED);
                u32_(&mut b, *window);
                f32_(&mut b, *x);
                f32_(&mut b, *y);
            }
            ToApp::WindowTitleDoubleClick { window } => {
                u32_(&mut b, tag::WINDOW_TITLE_DOUBLE_CLICK);
                u32_(&mut b, *window);
            }
            ToApp::WindowClosed { id } => {
                u32_(&mut b, tag::WINDOW_CLOSED);
                u32_(&mut b, *id);
            }
            ToApp::FullScreen { on } => {
                u32_(&mut b, tag::FULL_SCREEN);
                b.push(*on as u8);
            }
            ToApp::WindowConfigure { id, w, h } => {
                u32_(&mut b, tag::WINDOW_CONFIGURE);
                u32_(&mut b, *id);
                f32_(&mut b, *w);
                f32_(&mut b, *h);
            }
            ToApp::WindowTouch { id, touch, kind, x, y, t_ms, mods } => {
                u32_(&mut b, tag::WINDOW_TOUCH);
                u32_(&mut b, *id);
                u32_(&mut b, *touch);
                b.push(match kind {
                    TouchKind::Down => 0,
                    TouchKind::Move => 1,
                    TouchKind::Up => 2,
                    TouchKind::Cancel => 3,
                });
                f32_(&mut b, *x);
                f32_(&mut b, *y);
                u32_(&mut b, *t_ms);
                u32_(&mut b, mods.bits());
            }
            ToApp::WindowRelease { id, buf, generation } => {
                u32_(&mut b, tag::WINDOW_RELEASE);
                u32_(&mut b, *id);
                u32_(&mut b, *buf);
                u32_(&mut b, *generation);
            }
            ToApp::WindowFocus { window, focused } => {
                u32_(&mut b, tag::WINDOW_FOCUS);
                u32_(&mut b, *window);
                b.push(*focused as u8);
            }
            ToApp::WindowInput { window, input } => {
                u32_(&mut b, tag::WINDOW_INPUT);
                u32_(&mut b, *window);
                window_input_encode(&mut b, input);
            }
            ToApp::Resume => u32_(&mut b, tag::RESUME),
            ToApp::Suspend => u32_(&mut b, tag::SUSPEND),
            ToApp::AxSet { id, action } => {
                u32_(&mut b, tag::AX_SET);u32_(&mut b, *id as u32);u32_(&mut b, (*id >> 32) as u32);
                match action {
                    AxAction::Press => b.push(0),
                    AxAction::Focus => b.push(2),
                    AxAction::Increment => b.push(3),
                    AxAction::Decrement => b.push(4),
                    AxAction::SetValue(v) => {b.push(1);str_(&mut b, &short(v));}
                }
            }
            ToApp::SetMarkedText { text, sel_start, sel_len } => {
                u32_(&mut b, tag::SET_MARKED_TEXT);
                str_(&mut b, text);
                u32_(&mut b, *sel_start);
                u32_(&mut b, *sel_len);
            }
            ToApp::UnmarkText { commit } => {
                u32_(&mut b, tag::UNMARK_TEXT);
                b.push(*commit as u8);
            }
            ToApp::DeleteSurrounding { before, after } => {
                u32_(&mut b, tag::DELETE_SURROUNDING);
                u32_(&mut b, *before);
                u32_(&mut b, *after);
            }
            ToApp::AccessoryAction(id) => {
                u32_(&mut b, tag::ACCESSORY_ACTION);
                u32_(&mut b, *id);
            }
            ToApp::InputSource { japanese } => {
                u32_(&mut b, tag::INPUT_SOURCE);
                b.push(*japanese as u8);
            }
            ToApp::AccentHold { letter } => {
                u32_(&mut b, tag::ACCENT_HOLD);
                u32_(&mut b, *letter as u32);
            }
            ToApp::Activate(id) => {u32_(&mut b,tag::ACTIVATE);u32_(&mut b,*id as u32);u32_(&mut b,(*id>>32) as u32);}
            ToApp::Command(command) => {u32_(&mut b,tag::COMMAND);u32_(&mut b,*command as u32);}
            ToApp::Quit => u32_(&mut b, tag::QUIT),
            ToApp::Open { url } => {
                u32_(&mut b, tag::OPEN);
                str_(&mut b, url);
            }
            ToApp::Pasteboard { change_count, items } => {
                u32_(&mut b, tag::PASTEBOARD);
                u32_(&mut b, *change_count);
                items_(&mut b, items);
            }
            ToApp::PasteboardChanged { change_count } => {
                u32_(&mut b, tag::PASTEBOARD_CHANGED);
                u32_(&mut b, *change_count);
            }
            ToApp::PasteboardBlob { change_count, types } => {
                u32_(&mut b, tag::PASTEBOARD_BLOB);
                u32_(&mut b, *change_count);
                layout_(&mut b, types);
            }
            ToApp::Clipboard { text } => {
                u32_(&mut b, tag::CLIPBOARD);
                str_(&mut b, text);
            }
            ToApp::Appearance { flags } => {
                u32_(&mut b, tag::APPEARANCE);
                u32_(&mut b, *flags);
            }
            ToApp::KeyboardConfig(c) => {
                u32_(&mut b, tag::KEYBOARD_CONFIG);
                b.push(c.layout as u8);
                b.push(c.return_key as u8);
                u32_(&mut b, c.flags);
                b.push(c.languages);
                b.push(c.language);
            }
            ToApp::DocGranted { request, grant, mode, name, size } => {
                u32_(&mut b, tag::DOC_GRANTED);
                u32_(&mut b, *request);
                b.extend_from_slice(grant);
                b.push(*mode as u8);
                str_(&mut b, &short(name));
                b.extend_from_slice(&size.to_le_bytes());
            }
            ToApp::DocError { request, code } => {
                u32_(&mut b, tag::DOC_ERROR);
                u32_(&mut b, *request);
                b.push(*code as u8);
            }
            ToApp::KeychainReply { request, status, value } => {
                u32_(&mut b, tag::KEYCHAIN_REPLY);
                u32_(&mut b, *request);
                b.push(*status as u8);
                bytes_(&mut b, value);
            }
            ToApp::DocBatch { request, count } => {
                u32_(&mut b, tag::DOC_BATCH);
                u32_(&mut b, *request);
                u32_(&mut b, *count);
            }
            m @ (ToApp::LoginItemStatus { .. } | ToApp::StatusItemAction { .. }) => extras::encode_app(m, &mut b),
            m @ (ToApp::ContinueActivity { .. } | ToApp::ReindexAll | ToApp::ReindexIdentifiers { .. }) => quicksearch::encode_app(m, &mut b),
            m @ (ToApp::GuidedAccessState { .. } | ToApp::GuidedRestriction { .. }) => guided::encode_app(m, &mut b),
            m @ ToApp::Touches(_) => touch::encode_app(m, &mut b),
            m @ ToApp::Pointer(_) => pointer::encode_app(m, &mut b),
            m @ (ToApp::WorkspaceReply { .. } | ToApp::WorkspaceEvent { .. }) => workspace::encode_app(m, &mut b),
            m @ (ToApp::NotificationAction { .. } | ToApp::NotificationStatus { .. } | ToApp::NotificationList { .. } | ToApp::NotificationSettings { .. } | ToApp::NotificationAuthorized { .. } | ToApp::NotificationWillPresent { .. } | ToApp::ShareDone { .. } | ToApp::DistNotify { .. } | ToApp::ServiceReply { .. } | ToApp::ServiceEvent { .. } | ToApp::DragDest { .. } | ToApp::DragEnded { .. }) => rich::encode_app(m, &mut b),
        }
        b
    }

    pub fn decode(p: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader(p);
        Ok(match r.u32()? {
            tag::CONFIGURE => ToApp::Configure(Configure {
                w: r.u32()?,
                h: r.u32()?,
                scale: r.f32()?,
                safe_top: r.f32()?,
                safe_bottom: r.f32()?,
                dark: r.u8()? != 0,
                text_scale: r.f32()?,
                reduce_motion: r.u8()? != 0,
            }),
            tag::TOUCH => {
                let kind = match r.u8()? {
                    0 => TouchKind::Down,
                    1 => TouchKind::Move,
                    2 => TouchKind::Up,
                    3 => TouchKind::Cancel,
                    _ => return Err(DecodeError::BadValue),
                };
                let (id, x, y, t_ms) = (r.u32()?, r.f32()?, r.f32()?, r.u32()?);
                ToApp::Touch { kind, id, x, y, t_ms, mods: r.modifiers()? }
            }
            tag::KEY => match r.u8()? {
                0 => ToApp::Key(KeyInput::Text(r.str()?)),
                1 => ToApp::Key(KeyInput::Code(r.u32()?)),
                _ => return Err(DecodeError::BadValue),
            },
            tag::RELEASE => ToApp::Release { buf: r.u32()?, generation: r.u32()? },
            tag::WINDOW_CLOSED => ToApp::WindowClosed { id: r.u32()? },
            tag::FULL_SCREEN => ToApp::FullScreen { on: r.u8()? != 0 },
            tag::WINDOW_MOVED => {
                let window = r.u32()?;
                let (x, y) = (r.f32()?, r.f32()?);
                if !x.is_finite() || !y.is_finite() {
                    return Err(DecodeError::BadValue);
                }
                ToApp::WindowMoved { window, x, y }
            }
            tag::WINDOW_TITLE_DOUBLE_CLICK => ToApp::WindowTitleDoubleClick { window: r.u32()? },
            tag::WINDOW_CONFIGURE => {
                let (id, w, h) = (r.u32()?, r.f32()?, r.f32()?);
                if !w.is_finite() || !h.is_finite() || w < 0.0 || h < 0.0 {
                    return Err(DecodeError::BadValue);
                }
                ToApp::WindowConfigure { id, w, h }
            }
            tag::WINDOW_TOUCH => {
                let id = r.u32()?;
                let touch = r.u32()?;
                let kind = match r.u8()? {
                    0 => TouchKind::Down,
                    1 => TouchKind::Move,
                    2 => TouchKind::Up,
                    3 => TouchKind::Cancel,
                    _ => return Err(DecodeError::BadValue),
                };
                let (x, y) = (r.f32()?, r.f32()?);
                if !x.is_finite() || !y.is_finite() {
                    return Err(DecodeError::BadValue);
                }
                let t_ms = r.u32()?;
                ToApp::WindowTouch { id, touch, kind, x, y, t_ms, mods: r.modifiers()? }
            }
            tag::WINDOW_RELEASE => ToApp::WindowRelease { id: r.u32()?, buf: r.u32()?, generation: r.u32()? },
            tag::WINDOW_FOCUS => {
                let window = r.u32()?;
                let focused = match r.u8()? {
                    0 => false,
                    1 => true,
                    _ => return Err(DecodeError::BadValue),
                };
                ToApp::WindowFocus { window, focused }
            }
            tag::WINDOW_INPUT => {
                let window = r.u32()?;
                ToApp::WindowInput { window, input: window_input_decode(&mut r)? }
            }
            tag::RESUME => ToApp::Resume,
            tag::SUSPEND => ToApp::Suspend,
            tag::CONTEXT=>{let x=r.f32()?;let y=r.f32()?;if !x.is_finite()||!y.is_finite(){return Err(DecodeError::BadValue);}ToApp::Context{x,y}},
            tag::CONTEXT_ACTION=>ToApp::ContextAction(r.u32()?),
            tag::MENU_ACTION=>ToApp::MenuAction(r.u32()?),
            tag::QUICK_ACTION=>{let kind=r.str()?;let n=r.u32()?;if n>MAX_QUICK_INFO{return Err(DecodeError::BadValue);}let mut user_info=Vec::new();for _ in 0..n{user_info.push((r.str()?,r.str()?));}ToApp::QuickAction{kind,user_info}},
            tag::AX_SET => {
                let low = r.u32()? as u64;
                let id = low | ((r.u32()? as u64) << 32);
                let action = match r.u8()? {
                    0 => AxAction::Press,
                    1 => AxAction::SetValue(r.str()?),
                    2 => AxAction::Focus,
                    3 => AxAction::Increment,
                    4 => AxAction::Decrement,
                    _ => return Err(DecodeError::BadValue),
                };
                ToApp::AxSet { id, action }
            }
            tag::SET_MARKED_TEXT => {
                let text = r.str()?;
                let (sel_start, sel_len) = (r.u32()?, r.u32()?);
                let end = sel_start as u64 + sel_len as u64;
                if end > text.len() as u64
                    || !text.is_char_boundary(sel_start as usize)
                    || !text.is_char_boundary(end as usize)
                {
                    return Err(DecodeError::BadValue);
                }
                ToApp::SetMarkedText { text, sel_start, sel_len }
            }
            tag::UNMARK_TEXT => ToApp::UnmarkText { commit: r.u8()? != 0 },
            tag::DELETE_SURROUNDING => ToApp::DeleteSurrounding { before: r.u32()?, after: r.u32()? },
            tag::ACCESSORY_ACTION => ToApp::AccessoryAction(r.u32()?),
            tag::INPUT_SOURCE => ToApp::InputSource { japanese: r.u8()? != 0 },
            tag::ACCENT_HOLD => ToApp::AccentHold { letter: char::from_u32(r.u32()?).ok_or(DecodeError::BadValue)? },
            tag::ACTIVATE => {let low=r.u32()? as u64;ToApp::Activate(low|((r.u32()? as u64)<<32))},
            tag::COMMAND => ToApp::Command(Command::from_wire(r.u32()?).ok_or(DecodeError::BadValue)?),
            tag::QUIT => ToApp::Quit,
            tag::OPEN => ToApp::Open { url: r.str()? },
            tag::PASTEBOARD => ToApp::Pasteboard { change_count: r.u32()?, items: r.items()? },
            tag::PASTEBOARD_CHANGED => ToApp::PasteboardChanged { change_count: r.u32()? },
            tag::PASTEBOARD_BLOB => ToApp::PasteboardBlob { change_count: r.u32()?, types: r.layout()? },
            tag::CLIPBOARD => ToApp::Clipboard { text: r.str()? },
            tag::APPEARANCE => ToApp::Appearance { flags: r.u32()? },
            tag::KEYBOARD_CONFIG => {
                let layout = match r.u8()? {
                    0 => KeyboardLayout::Docked,
                    1 => KeyboardLayout::Split,
                    2 => KeyboardLayout::Floating,
                    _ => return Err(DecodeError::BadValue),
                };
                let return_key = match r.u8()? {
                    0 => ReturnKey::Return,
                    1 => ReturnKey::Search,
                    2 => ReturnKey::Go,
                    3 => ReturnKey::Done,
                    4 => ReturnKey::Next,
                    5 => ReturnKey::Send,
                    _ => return Err(DecodeError::BadValue),
                };
                let flags = r.u32()?;
                ToApp::KeyboardConfig(KeyboardConfig { layout, return_key, flags, languages: r.u8()?, language: r.u8()? })
            }
            tag::DOC_GRANTED => {
                let request = r.u32()?;
                let grant = r.grant()?;
                let mode = mode_(r.u8()?)?;
                let name = r.short()?;
                let size = u64::from_le_bytes(r.take(8)?.try_into().unwrap());
                ToApp::DocGranted { request, grant, mode, name, size }
            }
            tag::DOC_ERROR => {
                let request = r.u32()?;
                ToApp::DocError { request, code: DocErrorCode::from_wire(r.u8()?).ok_or(DecodeError::BadValue)? }
            }
            tag::KEYCHAIN_REPLY => {
                let request = r.u32()?;
                let status = KeychainStatus::from_wire(r.u8()?).ok_or(DecodeError::BadValue)?;
                ToApp::KeychainReply { request, status, value: r.bytes()? }
            }
            tag::DOC_BATCH => {
                let request = r.u32()?;
                let count = r.u32()?;
                if count > MAX_DOC_BATCH {
                    return Err(DecodeError::BadValue);
                }
                ToApp::DocBatch { request, count }
            }
            t @ (50..=59 | 0x1d0..=0x1df) => rich::decode_app(t, &mut r)?,
            t @ 0x1b0..=0x1bf => extras::decode_app(t, &mut r)?,
            t @ 0x1c0..=0x1cf => quicksearch::decode_app(t, &mut r)?,
            t @ 0x171..=0x172 => guided::decode_app(t, &mut r)?,
            t @ 0x1f1 => pointer::decode_app(t, &mut r)?,
            t @ 0x1f0..=0x1ff => touch::decode_app(t, &mut r)?,
            t @ 100..=109 => workspace::decode_app(t, &mut r)?,
            t => return Err(DecodeError::UnknownTag(t)),
        })
    }
}

impl ToShell {
    pub fn encode(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(32);
        match self {
            ToShell::Hello { sdk_version, app_id } => {
                u32_(&mut b, tag::HELLO);
                u32_(&mut b, *sdk_version);
                str_(&mut b, app_id);
            }
            ToShell::Buffers { w, h, generation } => {
                u32_(&mut b, tag::BUFFERS);
                u32_(&mut b, *w);
                u32_(&mut b, *h);
                u32_(&mut b, *generation);
            }
            ToShell::WindowOpen { id, title, w, h, position, flags } => {
                u32_(&mut b, tag::WINDOW_OPEN);
                u32_(&mut b, *id);
                str_(&mut b, &short_title(title));
                f32_(&mut b, *w);
                f32_(&mut b, *h);
                match position {
                    Some((x, y)) => {
                        b.push(1);
                        f32_(&mut b, *x);
                        f32_(&mut b, *y);
                    }
                    None => b.push(0),
                }
                u32_(&mut b, *flags);
            }
            ToShell::WindowMove { window, x, y } => {
                u32_(&mut b, tag::WINDOW_MOVE);
                u32_(&mut b, *window);
                f32_(&mut b, *x);
                f32_(&mut b, *y);
            }
            ToShell::WindowCollapse { window, collapsed } => {
                u32_(&mut b, tag::WINDOW_COLLAPSE);
                u32_(&mut b, *window);
                b.push(*collapsed as u8);
            }
            ToShell::WindowClose { id } => {
                u32_(&mut b, tag::WINDOW_CLOSE);
                u32_(&mut b, *id);
            }
            ToShell::WindowTitle { id, title } => {
                u32_(&mut b, tag::WINDOW_TITLE);
                u32_(&mut b, *id);
                str_(&mut b, &short_title(title));
            }
            ToShell::WindowResize { id, w, h } => {
                u32_(&mut b, tag::WINDOW_RESIZE);
                u32_(&mut b, *id);
                f32_(&mut b, *w);
                f32_(&mut b, *h);
            }
            ToShell::WindowBuffers { id, w, h, generation } => {
                u32_(&mut b, tag::WINDOW_BUFFERS);
                u32_(&mut b, *id);
                u32_(&mut b, *w);
                u32_(&mut b, *h);
                u32_(&mut b, *generation);
            }
            ToShell::WindowFrame { id, buf, damage } => {
                u32_(&mut b, tag::WINDOW_FRAME);
                u32_(&mut b, *id);
                u32_(&mut b, *buf);
                let n = damage.len().min((MAX_PACKET - 16) / 16);
                u32_(&mut b, n as u32);
                for r in &damage[..n] {
                    for v in r {
                        u32_(&mut b, *v);
                    }
                }
            }
            ToShell::Frame { buf, damage } => {
                u32_(&mut b, tag::FRAME);
                u32_(&mut b, *buf);
                let n = damage.len().min((MAX_PACKET - 16) / 16);
                u32_(&mut b, n as u32);
                for r in &damage[..n] {
                    for v in r {
                        u32_(&mut b, *v);
                    }
                }
            }
            ToShell::WantsKeyboard { on } => {
                u32_(&mut b, tag::WANTS_KEYBOARD);
                b.push(*on as u8);
            }
            ToShell::KeyboardOut(out) => {
                u32_(&mut b, tag::KEYBOARD_OUT);
                match out {
                    KeyboardOut::Text(t) => {
                        b.push(0);
                        str_(&mut b, t);
                    }
                    KeyboardOut::Backspace => b.push(1),
                    KeyboardOut::Enter => b.push(2),
                    KeyboardOut::Hide => b.push(3),
                    KeyboardOut::Named(n) => {
                        b.push(4);
                        let name: String = n.chars().take(16).collect();
                        str_(&mut b, &name);
                    }
                    KeyboardOut::Height(h) => {
                        b.push(5);
                        u32_(&mut b, *h);
                    }
                    KeyboardOut::Click(k) => {
                        b.push(6);
                        b.push(*k);
                    }
                    KeyboardOut::Language(l) => {
                        b.push(7);
                        b.push(*l);
                    }
                    KeyboardOut::Preedit { text, sel_start, sel_len } => {
                        b.push(8);
                        str_(&mut b, text);
                        u32_(&mut b, *sel_start);
                        u32_(&mut b, *sel_len);
                    }
                    KeyboardOut::Unmark { commit } => {
                        b.push(9);
                        b.push(*commit as u8);
                    }
                }
            }
            ToShell::ContextMenu{x,y,items}=>{u32_(&mut b,tag::CONTEXT_MENU);f32_(&mut b,*x);f32_(&mut b,*y);u32_(&mut b,items.len().min(32) as u32);for item in items.iter().take(32){u32_(&mut b,item.id);b.push(item.enabled as u8);let label=item.label.chars().take(128).collect::<String>();str_(&mut b,&label);}},
            ToShell::AppMenus(menus) => {
                // Menus that do not fit one packet are cut (whole items, then whole menus), never split.
                let budget = MAX_PACKET - 1024;
                u32_(&mut b, tag::APP_MENUS);
                let count_at = b.len();
                u32_(&mut b, 0);
                let mut kept = 0u32;
                for m in menus.iter().take(MAX_APP_MENUS) {
                    if b.len() + 80 > budget {
                        break;
                    }
                    str_(&mut b, &m.title.chars().take(48).collect::<String>());
                    let n_at = b.len();
                    u32_(&mut b, 0);
                    let mut n = 0u32;
                    for i in m.items.iter().take(MAX_APP_MENU_ITEMS) {
                        let title = i.title.chars().take(MAX_APP_MENU_TITLE).collect::<String>();
                        let key = i.key.chars().take(24).collect::<String>();
                        if b.len() + 4 + 4 + title.len() + 4 + key.len() + 2 > budget {
                            break;
                        }
                        u32_(&mut b, i.id);
                        str_(&mut b, &title);
                        str_(&mut b, &key);
                        b.push(i.enabled as u8 | (i.checked as u8) << 1 | (i.separator as u8) << 2 | (i.submenu as u8) << 3 | (i.prints as u8) << 4);
                        b.push(i.depth.min(MAX_APP_MENU_DEPTH));
                        n += 1;
                    }
                    b[n_at..n_at + 4].copy_from_slice(&n.to_le_bytes());
                    kept += 1;
                }
                b[count_at..count_at + 4].copy_from_slice(&kept.to_le_bytes());
            }
            ToShell::RemoteSharing { on } => {u32_(&mut b,tag::REMOTE_SHARING);b.push(*on as u8);}
            ToShell::Controls(nodes) => {
                u32_(&mut b,tag::CONTROLS);u32_(&mut b,0);let mut count=0u32;
                for node in nodes {
                    let mut item=Vec::new();u32_(&mut item,node.id as u32);u32_(&mut item,(node.id>>32) as u32);u32_(&mut item,node.role as u32);item.push(node.enabled as u8);
                    for v in [node.bounds.x,node.bounds.y,node.bounds.w,node.bounds.h]{f32_(&mut item,v);}
                    str_(&mut item,&node.label);str_(&mut item,&node.value);
                    if b.len()+item.len()>MAX_PACKET||count>=128 {break;}b.extend_from_slice(&item);count+=1;
                }b[4..8].copy_from_slice(&count.to_le_bytes());
            }
            m @ (ToShell::LoginItem { .. } | ToShell::StatusItemSet { .. } | ToShell::StatusItemRemove { .. }) => extras::encode_shell(m, &mut b),
            m @ (ToShell::QuickSearchIndex(_) | ToShell::QuickSearchDelete { .. }) => quicksearch::encode_shell(m, &mut b),
            m @ (ToShell::WorkspaceRequest { .. } | ToShell::WorkspaceSubscribe { .. }) => workspace::encode_shell(m, &mut b),
            m @ (ToShell::Preview { .. } | ToShell::PreviewClose) => peek::encode_shell(m, &mut b),
            m @ (ToShell::AxText(_) | ToShell::AxMeta(_) | ToShell::Announce { .. } | ToShell::AxNotify(_)) => axtext::encode_shell(m, &mut b),
            m @ ToShell::AxRotors(_) => axrotor::encode_shell(m, &mut b),
            m @ ToShell::GuidedAccessRequest { .. } => guided::encode_shell(m, &mut b),
            m @ ToShell::PointerInterest { .. } => pointer::encode_shell(m, &mut b),
            m @ (ToShell::NotifyCategories(_) | ToShell::NotifyRequest { .. } | ToShell::NotifyRemove { .. } | ToShell::NotifyQuery { .. } | ToShell::NotifySettings { .. } | ToShell::NotifyAuthorize { .. } | ToShell::NotifyPresentDelegate { .. } | ToShell::NotifyPresentReply { .. } | ToShell::NotifySchedule { .. } | ToShell::Share { .. } | ToShell::DistPost { .. } | ToShell::ServiceCall { .. } | ToShell::DragBegin { .. } | ToShell::DragRegister { .. } | ToShell::DragReply { .. } | ToShell::DragPerformed { .. } | ToShell::DragCancel { .. }) => rich::encode_shell(m, &mut b),
            ToShell::ControlsTree(list) => {
                u32_(&mut b, tag::CONTROLS_TREE);
                let n = list.len().min(MAX_NODE_EXT);
                u32_(&mut b, n as u32);
                for e in list.iter().take(n) {
                    u32_(&mut b, e.id as u32);u32_(&mut b, (e.id >> 32) as u32);
                    let p = e.parent.unwrap_or(0);
                    u32_(&mut b, p as u32);u32_(&mut b, (p >> 32) as u32);
                    // bit0 has parent, bit1 focused, bit2 checkable, bit3 checked
                    b.push(e.parent.is_some() as u8 | (e.focused as u8) << 1 | (e.checked.is_some() as u8) << 2 | (e.checked == Some(true)) as u8 * 8);
                }
            }
            ToShell::TextState(t) => {
                u32_(&mut b, tag::TEXT_STATE);
                let (text, cut) = surrounding_window(&t.text, t.cursor.min(t.anchor) as usize);
                str_(&mut b, text);
                let adj = |v: u32| (v as usize).saturating_sub(cut).min(text.len()) as u32;
                u32_(&mut b, adj(t.cursor));
                u32_(&mut b, adj(t.anchor));
                match t.marked {
                    Some((s, e)) => {
                        b.push(1);
                        u32_(&mut b, adj(s));
                        u32_(&mut b, adj(e));
                    }
                    None => b.push(0),
                }
            }
            ToShell::CaretRect { x, y, w, h } => {
                u32_(&mut b, tag::CARET_RECT);
                for v in [x, y, w, h] {
                    f32_(&mut b, *v);
                }
            }
            ToShell::InputTraits(t) => {
                u32_(&mut b, tag::INPUT_TRAITS);
                b.push(t.return_key as u8);
                b.push(t.content as u8);
                u32_(&mut b, t.flags);
            }
            ToShell::SuddenTermination { enabled } => {
                u32_(&mut b, tag::SUDDEN_TERMINATION);
                b.push(*enabled as u8);
            }
            ToShell::Activity { options } => {
                u32_(&mut b, tag::ACTIVITY);
                u32_(&mut b, *options);
            }
            ToShell::WindowLevel(level) => {
                u32_(&mut b, tag::WINDOW_LEVEL);
                b.push(*level as u8);
            }
            ToShell::WindowLevelOf { id, level } => {
                u32_(&mut b, tag::WINDOW_LEVEL_OF);
                u32_(&mut b, *id);
                b.push(*level as u8);
            }
            ToShell::InputAccessory(items) => {
                u32_(&mut b, tag::INPUT_ACCESSORY);
                let n = items.len().min(MAX_ACCESSORY_ITEMS);
                u32_(&mut b, n as u32);
                for i in items.iter().take(n) {
                    u32_(&mut b, i.id);
                    b.push(i.enabled as u8);
                    str_(&mut b, &i.label.chars().take(32).collect::<String>());
                }
            }
            ToShell::Commands(commands) => {u32_(&mut b,tag::COMMANDS);u32_(&mut b,commands.0);}
            ToShell::UndoTitles { undo, redo } => {u32_(&mut b,tag::UNDO_TITLES);str_(&mut b,&undo.chars().take(64).collect::<String>());str_(&mut b,&redo.chars().take(64).collect::<String>());}
            ToShell::DocumentState { title, edited, represented } => {
                u32_(&mut b, tag::DOCUMENT_STATE);
                str_(&mut b, &short(title));
                b.push(*edited as u8);
                str_(&mut b, &short(represented));
            }
            ToShell::Haptic => u32_(&mut b, tag::HAPTIC),
            ToShell::SetBadge { label } => {
                u32_(&mut b, tag::SET_BADGE);
                str_(&mut b, label);
            }
            ToShell::SetFullScreen { on } => {
                u32_(&mut b, tag::SET_FULL_SCREEN);
                b.push(*on as u8);
            }
            ToShell::OpenUrl { url } => {
                u32_(&mut b, tag::OPEN_URL);
                str_(&mut b, url);
            }
            ToShell::PlaySystemSound { name } => {
                u32_(&mut b, tag::PLAY_SYSTEM_SOUND);
                str_(&mut b, name);
            }
            ToShell::Notify { title, body } => {
                u32_(&mut b, tag::NOTIFY);
                str_(&mut b, title);
                str_(&mut b, body);
            }
            ToShell::Log { text } => {
                u32_(&mut b, tag::LOG);
                str_(&mut b, text);
            }
            ToShell::PasteboardSet { items } => {
                u32_(&mut b, tag::PASTEBOARD_SET);
                items_(&mut b, items);
            }
            ToShell::PasteboardBlob { types } => {
                u32_(&mut b, tag::PASTEBOARD_BLOB);
                layout_(&mut b, types);
            }
            ToShell::PasteboardRead => u32_(&mut b, tag::PASTEBOARD_READ),
            ToShell::SetClipboard { text } => {
                u32_(&mut b, tag::SET_CLIPBOARD);
                str_(&mut b, text);
            }
            ToShell::DocOpen { request, mode, title, filters } => {
                u32_(&mut b, tag::DOC_OPEN);
                u32_(&mut b, *request);
                b.push(*mode as u8);
                str_(&mut b, &short(title));
                filters_(&mut b, filters);
            }
            ToShell::DocOpenEx { request, mode, title, filters, options } => {
                u32_(&mut b, tag::DOC_OPEN_EX);
                u32_(&mut b, *request);
                b.push(*mode as u8);
                str_(&mut b, &short(title));
                filters_(&mut b, filters);
                u32_(&mut b, *options);
            }
            ToShell::DocSave { request, suggested_name, filters } => {
                u32_(&mut b, tag::DOC_SAVE);
                u32_(&mut b, *request);
                str_(&mut b, &short(suggested_name));
                filters_(&mut b, filters);
            }
            ToShell::DocCommit { request } => {
                u32_(&mut b, tag::DOC_COMMIT);
                u32_(&mut b, *request);
            }
            ToShell::DocAbort { request } => {
                u32_(&mut b, tag::DOC_ABORT);
                u32_(&mut b, *request);
            }
            ToShell::DocReopen { request, grant } => {
                u32_(&mut b, tag::DOC_REOPEN);
                u32_(&mut b, *request);
                b.extend_from_slice(grant);
            }
            ToShell::DocSaveGrant { request, grant } => {
                u32_(&mut b, tag::DOC_SAVE_GRANT);
                u32_(&mut b, *request);
                b.extend_from_slice(grant);
            }
            ToShell::DocRelease { grant } => {
                u32_(&mut b, tag::DOC_RELEASE);
                b.extend_from_slice(grant);
            }
            ToShell::DocGrantPath { request, path } => {
                u32_(&mut b, tag::DOC_GRANT_PATH);
                u32_(&mut b, *request);
                str_(&mut b, path);
            }
            ToShell::Keychain { request, op } => {
                u32_(&mut b, tag::KEYCHAIN);
                u32_(&mut b, *request);
                let (kind, service, account, value) = match op {
                    KeychainOp::Add { service, account, value } => (0u8, service, account, Some(value)),
                    KeychainOp::Get { service, account } => (1, service, account, None),
                    KeychainOp::Update { service, account, value } => (2, service, account, Some(value)),
                    KeychainOp::Delete { service, account } => (3, service, account, None),
                };
                b.push(kind);
                str_(&mut b, service);
                str_(&mut b, account);
                if let Some(v) = value {
                    bytes_(&mut b, v);
                }
            }
            ToShell::Materials(list) => {
                u32_(&mut b, tag::MATERIALS);
                let n = list.len().min(MAX_MATERIALS);
                u32_(&mut b, n as u32);
                for m in &list[..n] {
                    b.push(m.kind as u8 | (m.state as u8) << 6);
                    for v in [m.rect.x, m.rect.y, m.rect.w, m.rect.h, m.radius] {
                        f32_(&mut b, v);
                    }
                    b.push(m.emphasized as u8 | ((m.blending == BlendingMode::WithinWindow) as u8) << 1 | (m.corners.is_some() as u8) << 2);
                    if let Some(c) = m.corners {
                        for v in [c.tl, c.tr, c.br, c.bl] {
                            f32_(&mut b, v);
                        }
                    }
                }
            }
        }
        b
    }

    pub fn decode(p: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader(p);
        Ok(match r.u32()? {
            tag::CONTEXT_MENU=>{let x=r.f32()?;let y=r.f32()?;let count=r.u32()?;if !x.is_finite()||!y.is_finite()||count>32{return Err(DecodeError::BadValue);}let mut items=Vec::new();for _ in 0..count {let id=r.u32()?;let enabled=r.u8()?!=0;let label=r.str()?;if label.len()>512{return Err(DecodeError::BadValue);}items.push(ContextItem{id,label,enabled});}ToShell::ContextMenu{x,y,items}},
            tag::REMOTE_SHARING => ToShell::RemoteSharing {on:r.u8()?!=0},
            tag::CONTROLS => {
                let count=r.u32()?;if count>128{return Err(DecodeError::BadValue);}let mut nodes=Vec::new();
                for _ in 0..count {let low=r.u32()? as u64;let id=low|((r.u32()? as u64)<<32);let role=match r.u32()?{0=>Role::Button,1=>Role::TextField,2=>Role::List,3=>Role::Row,4=>Role::Toggle,5=>Role::Checkbox,6=>Role::Slider,7=>Role::SegmentedControl,8=>Role::StaticText,9=>Role::MenuItem,10=>Role::Window,11=>Role::ProgressIndicator,_=>return Err(DecodeError::BadValue)};let enabled=r.u8()?!=0;let bounds=UiRect{x:r.f32()?,y:r.f32()?,w:r.f32()?,h:r.f32()?};if ![bounds.x,bounds.y,bounds.w,bounds.h].iter().all(|v|v.is_finite())||bounds.w<0.||bounds.h<0.{return Err(DecodeError::BadValue);}nodes.push(Node{id,role,enabled,bounds,label:r.str()?,value:r.str()?});}ToShell::Controls(nodes)
            }
            tag::COMMANDS => ToShell::Commands(Commands(r.u32()?)),
            tag::UNDO_TITLES => ToShell::UndoTitles { undo: r.str()?, redo: r.str()? },
            tag::HELLO => ToShell::Hello { sdk_version: r.u32()?, app_id: r.str()? },
            tag::BUFFERS => ToShell::Buffers { w: r.u32()?, h: r.u32()?, generation: r.u32()? },
            tag::FRAME => {
                let buf = r.u32()?;
                let n = r.u32()? as usize;
                if n > MAX_PACKET / 16 {
                    return Err(DecodeError::BadValue);
                }
                let mut damage = Vec::with_capacity(n);
                for _ in 0..n {
                    damage.push([r.u32()?, r.u32()?, r.u32()?, r.u32()?]);
                }
                ToShell::Frame { buf, damage }
            }
            tag::WINDOW_OPEN => {
                let id = r.u32()?;
                let title = checked_title(r.str()?)?;
                let (w, h) = (r.f32()?, r.f32()?);
                if !w.is_finite() || !h.is_finite() || w < 0.0 || h < 0.0 {
                    return Err(DecodeError::BadValue);
                }
                let position = match r.u8()? {
                    0 => None,
                    1 => {
                        let (x, y) = (r.f32()?, r.f32()?);
                        if !x.is_finite() || !y.is_finite() {
                            return Err(DecodeError::BadValue);
                        }
                        Some((x, y))
                    }
                    _ => return Err(DecodeError::BadValue),
                };
                let flags = r.u32()?;
                ToShell::WindowOpen { id, title, w, h, position, flags }
            }
            tag::WINDOW_CLOSE => ToShell::WindowClose { id: r.u32()? },
            tag::WINDOW_MOVE => {
                let window = r.u32()?;
                let (x, y) = (r.f32()?, r.f32()?);
                if !x.is_finite() || !y.is_finite() {
                    return Err(DecodeError::BadValue);
                }
                ToShell::WindowMove { window, x, y }
            }
            tag::WINDOW_COLLAPSE => {
                let window = r.u32()?;
                let collapsed = match r.u8()? {
                    0 => false,
                    1 => true,
                    _ => return Err(DecodeError::BadValue),
                };
                ToShell::WindowCollapse { window, collapsed }
            }
            tag::WINDOW_TITLE => {
                let id = r.u32()?;
                ToShell::WindowTitle { id, title: checked_title(r.str()?)? }
            }
            tag::WINDOW_RESIZE => {
                let (id, w, h) = (r.u32()?, r.f32()?, r.f32()?);
                if !w.is_finite() || !h.is_finite() || w < 0.0 || h < 0.0 {
                    return Err(DecodeError::BadValue);
                }
                ToShell::WindowResize { id, w, h }
            }
            tag::WINDOW_BUFFERS => ToShell::WindowBuffers { id: r.u32()?, w: r.u32()?, h: r.u32()?, generation: r.u32()? },
            tag::WINDOW_FRAME => {
                let id = r.u32()?;
                let buf = r.u32()?;
                let n = r.u32()? as usize;
                if n > MAX_PACKET / 16 {
                    return Err(DecodeError::BadValue);
                }
                let mut damage = Vec::with_capacity(n);
                for _ in 0..n {
                    damage.push([r.u32()?, r.u32()?, r.u32()?, r.u32()?]);
                }
                ToShell::WindowFrame { id, buf, damage }
            }
            tag::WANTS_KEYBOARD => ToShell::WantsKeyboard { on: r.u8()? != 0 },
            tag::KEYBOARD_OUT => ToShell::KeyboardOut(match r.u8()? {
                0 => KeyboardOut::Text(r.str()?),
                1 => KeyboardOut::Backspace,
                2 => KeyboardOut::Enter,
                3 => KeyboardOut::Hide,
                4 => KeyboardOut::Named(r.str()?),
                5 => KeyboardOut::Height(r.u32()?),
                6 => KeyboardOut::Click(r.u8()?.min(2)),
                7 => KeyboardOut::Language(r.u8()?),
                8 => {
                    let text = r.str()?;
                    let (sel_start, sel_len) = (r.u32()?, r.u32()?);
                    let end = sel_start as u64 + sel_len as u64;
                    if end > text.len() as u64 || !text.is_char_boundary(sel_start as usize) || !text.is_char_boundary(end as usize) {
                        return Err(DecodeError::BadValue);
                    }
                    KeyboardOut::Preedit { text, sel_start, sel_len }
                }
                9 => KeyboardOut::Unmark { commit: r.u8()? != 0 },
                _ => return Err(DecodeError::BadValue),
            }),
            tag::HAPTIC => ToShell::Haptic,
            tag::SET_BADGE => ToShell::SetBadge { label: r.str()? },
            tag::SET_FULL_SCREEN => ToShell::SetFullScreen { on: r.u8()? != 0 },
            tag::DOCUMENT_STATE => ToShell::DocumentState { title: r.str()?, edited: r.u8()? != 0, represented: r.str()? },
            tag::OPEN_URL => ToShell::OpenUrl { url: r.str()? },
            tag::PLAY_SYSTEM_SOUND => ToShell::PlaySystemSound { name: r.str()? },
            tag::NOTIFY => ToShell::Notify { title: r.str()?, body: r.str()? },
            tag::LOG => ToShell::Log { text: r.str()? },
            tag::PASTEBOARD_SET => ToShell::PasteboardSet { items: r.items()? },
            tag::PASTEBOARD_BLOB => ToShell::PasteboardBlob { types: r.layout()? },
            tag::PASTEBOARD_READ => ToShell::PasteboardRead,
            tag::SET_CLIPBOARD => ToShell::SetClipboard { text: r.str()? },
            tag::DOC_OPEN => {
                let request = r.u32()?;
                let mode = mode_(r.u8()?)?;
                ToShell::DocOpen { request, mode, title: r.short()?, filters: r.filters()? }
            }
            tag::DOC_OPEN_EX => {
                let request = r.u32()?;
                let mode = mode_(r.u8()?)?;
                ToShell::DocOpenEx { request, mode, title: r.short()?, filters: r.filters()?, options: r.u32()? }
            }
            tag::DOC_SAVE => {
                let request = r.u32()?;
                ToShell::DocSave { request, suggested_name: r.short()?, filters: r.filters()? }
            }
            tag::DOC_COMMIT => ToShell::DocCommit { request: r.u32()? },
            tag::DOC_ABORT => ToShell::DocAbort { request: r.u32()? },
            tag::DOC_REOPEN => {
                let request = r.u32()?;
                ToShell::DocReopen { request, grant: r.grant()? }
            }
            tag::DOC_SAVE_GRANT => {
                let request = r.u32()?;
                ToShell::DocSaveGrant { request, grant: r.grant()? }
            }
            tag::DOC_RELEASE => ToShell::DocRelease { grant: r.grant()? },
            tag::DOC_GRANT_PATH => {
                let request = r.u32()?;
                let path = r.str()?;
                if path.len() > MAX_DOC_PATH {
                    return Err(DecodeError::BadValue);
                }
                ToShell::DocGrantPath { request, path }
            }
            tag::APP_MENUS => {
                let n = r.u32()? as usize;
                if n > MAX_APP_MENUS {
                    return Err(DecodeError::BadValue);
                }
                let mut menus = Vec::new();
                for _ in 0..n {
                    let title = r.str()?;
                    let count = r.u32()? as usize;
                    if title.len() > 512 || count > MAX_APP_MENU_ITEMS {
                        return Err(DecodeError::BadValue);
                    }
                    let mut items = Vec::new();
                    for _ in 0..count {
                        let id = r.u32()?;
                        let title = r.str()?;
                        let key = r.str()?;
                        let flags = r.u8()?;
                        let depth = r.u8()?;
                        if title.len() > 512 || key.len() > 128 || depth > MAX_APP_MENU_DEPTH || flags > 31 {
                            return Err(DecodeError::BadValue);
                        }
                        items.push(AppMenuItem { id, title, key, enabled: flags & 1 != 0, checked: flags & 2 != 0, separator: flags & 4 != 0, submenu: flags & 8 != 0, depth, prints: flags & 16 != 0 });
                    }
                    menus.push(AppMenu { title, items });
                }
                ToShell::AppMenus(menus)
            }
            tag::KEYCHAIN => {
                let request = r.u32()?;
                let kind = r.u8()?;
                let service = r.str()?;
                let account = r.str()?;
                if service.len() > MAX_KEYCHAIN_NAME * 4 || account.len() > MAX_KEYCHAIN_NAME * 4 {
                    return Err(DecodeError::BadValue);
                }
                let op = match kind {
                    0 => KeychainOp::Add { service, account, value: r.bytes()? },
                    1 => KeychainOp::Get { service, account },
                    2 => KeychainOp::Update { service, account, value: r.bytes()? },
                    3 => KeychainOp::Delete { service, account },
                    _ => return Err(DecodeError::BadValue),
                };
                ToShell::Keychain { request, op }
            }
            tag::TEXT_STATE => {
                let text = r.str()?;
                if text.len() > MAX_SURROUNDING {
                    return Err(DecodeError::BadValue);
                }
                let (cursor, anchor) = (r.u32()?, r.u32()?);
                let ok = |v: u32| v as usize <= text.len() && text.is_char_boundary(v as usize);
                let marked = match r.u8()? {
                    0 => None,
                    1 => {
                        let (a, b) = (r.u32()?, r.u32()?);
                        if a > b || !ok(a) || !ok(b) {
                            return Err(DecodeError::BadValue);
                        }
                        Some((a, b))
                    }
                    _ => return Err(DecodeError::BadValue),
                };
                if !ok(cursor) || !ok(anchor) {
                    return Err(DecodeError::BadValue);
                }
                ToShell::TextState(TextState { text, cursor, anchor, marked })
            }
            tag::CARET_RECT => {
                let (x, y, w, h) = (r.f32()?, r.f32()?, r.f32()?, r.f32()?);
                if ![x, y, w, h].iter().all(|v| v.is_finite()) || w < 0. || h < 0. {
                    return Err(DecodeError::BadValue);
                }
                ToShell::CaretRect { x, y, w, h }
            }
            tag::INPUT_TRAITS => ToShell::InputTraits(InputTraits {
                return_key: ReturnKeyType::from_wire(r.u8()?).ok_or(DecodeError::BadValue)?,
                content: TextContentKind::from_wire(r.u8()?).ok_or(DecodeError::BadValue)?,
                flags: r.u32()?,
            }),
            tag::SUDDEN_TERMINATION => ToShell::SuddenTermination { enabled: r.u8()? != 0 },
            tag::ACTIVITY => ToShell::Activity { options: r.u32()? },
            tag::WINDOW_LEVEL => ToShell::WindowLevel(WindowLevel::from_wire(r.u8()?).ok_or(DecodeError::BadValue)?),
            tag::WINDOW_LEVEL_OF => {
                let id = r.u32()?;
                ToShell::WindowLevelOf { id, level: WindowLevel::from_wire(r.u8()?).ok_or(DecodeError::BadValue)? }
            }
            tag::INPUT_ACCESSORY => {
                let n = r.u32()? as usize;
                if n > MAX_ACCESSORY_ITEMS {
                    return Err(DecodeError::BadValue);
                }
                let mut items = Vec::with_capacity(n);
                for _ in 0..n {
                    let id = r.u32()?;
                    let enabled = r.u8()? != 0;
                    items.push(AccessoryItem { id, enabled, label: r.str()? });
                }
                ToShell::InputAccessory(items)
            }
            tag::CONTROLS_TREE => {
                let n = r.u32()? as usize;
                if n > MAX_NODE_EXT {
                    return Err(DecodeError::BadValue);
                }
                let mut list = Vec::with_capacity(n);
                for _ in 0..n {
                    let id = r.u32()? as u64 | ((r.u32()? as u64) << 32);
                    let parent = r.u32()? as u64 | ((r.u32()? as u64) << 32);
                    let f = r.u8()?;
                    if f & !0xf != 0 {
                        return Err(DecodeError::BadValue);
                    }
                    list.push(NodeExt { id, parent: (f & 1 != 0).then_some(parent), focused: f & 2 != 0, checked: (f & 4 != 0).then_some(f & 8 != 0) });
                }
                ToShell::ControlsTree(list)
            }
            tag::MATERIALS => {
                let n = r.u32()? as usize;
                if n > MAX_MATERIALS {
                    return Err(DecodeError::BadValue);
                }
                let mut list = Vec::with_capacity(n);
                for _ in 0..n {
                    let byte = r.u8()?;
                    let kind = *MaterialKind::ALL.get((byte & 0x3f) as usize).ok_or(DecodeError::BadValue)?;
                    let state = match byte >> 6 {
                        0 => MaterialState::FollowsWindowActiveState,
                        1 => MaterialState::Active,
                        2 => MaterialState::Inactive,
                        _ => return Err(DecodeError::BadValue),
                    };
                    let rect = UiRect { x: r.f32()?, y: r.f32()?, w: r.f32()?, h: r.f32()? };
                    let radius = r.f32()?;
                    if ![rect.x, rect.y, rect.w, rect.h, radius].iter().all(|v| v.is_finite()) || rect.w < 0. || rect.h < 0. || radius < 0. {
                        return Err(DecodeError::BadValue);
                    }
                    let flags = r.u8()?;
                    if flags & !7 != 0 {
                        return Err(DecodeError::BadValue);
                    }
                    let corners = if flags & 4 != 0 {
                        let c = Corners { tl: r.f32()?, tr: r.f32()?, br: r.f32()?, bl: r.f32()? };
                        if ![c.tl, c.tr, c.br, c.bl].iter().all(|v| v.is_finite() && *v >= 0.) {
                            return Err(DecodeError::BadValue);
                        }
                        Some(c)
                    } else {
                        None
                    };
                    let blending = if flags & 2 != 0 { BlendingMode::WithinWindow } else { BlendingMode::BehindWindow };
                    list.push(Material { kind, rect, radius, state, blending, emphasized: flags & 1 != 0, corners });
                }
                ToShell::Materials(list)
            }
            t @ (0x140..=0x14f | 0x1d0..=0x1df) => rich::decode_shell(t, &mut r)?,
            t @ 0x1b0..=0x1bf => extras::decode_shell(t, &mut r)?,
            t @ 0x1c0..=0x1cf => quicksearch::decode_shell(t, &mut r)?,
            t @ 0x230..=0x23f => workspace::decode_shell(t, &mut r)?,
            t @ 0x1e0..=0x1ef => peek::decode_shell(t, &mut r)?,
            t @ 0x213 => axrotor::decode_shell(t, &mut r)?,
            t @ 0x170 => guided::decode_shell(t, &mut r)?,
            t @ 0x1f1 => pointer::decode_shell(t, &mut r)?,
            t @ 0x210..=0x21f => axtext::decode_shell(t, &mut r)?,
            t => return Err(DecodeError::UnknownTag(t)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag_of(p: &[u8]) -> u32 {
        u32::from_le_bytes(p[..4].try_into().unwrap())
    }

    /// Each rich message owns one tag and decodes back to itself; the 0x1d0 block is shared by both directions, so a
    /// tag must decode as the variant that encodes to it (ToShell 0x1d0/0x1d2, ToApp 0x1d1..0x1d3).
    #[test]
    fn distributed_and_service_messages_round_trip_on_their_tags() {
        let post = ToShell::DistPost { name: "n".into(), user_info: vec![("k".into(), "v".into())] };
        assert_eq!(tag_of(&post.encode()), 0x1d0);
        assert_eq!(ToShell::decode(&post.encode()), Ok(post));

        let call = ToShell::ServiceCall { request: 7, service: "dock".into(), method: "badge".into(), body: "null".into() };
        assert_eq!(tag_of(&call.encode()), 0x1d2);
        assert_eq!(ToShell::decode(&call.encode()), Ok(call));

        let dist = ToApp::DistNotify { name: "n".into(), user_info: vec![] };
        assert_eq!(tag_of(&dist.encode()), 0x1d1);
        assert_eq!(ToApp::decode(&dist.encode()), Ok(dist));

        let reply = ToApp::ServiceReply { request: 7, ok: false, body: "no such method".into() };
        assert_eq!(tag_of(&reply.encode()), 0x1d2);
        assert_eq!(ToApp::decode(&reply.encode()), Ok(reply));

        let event = ToApp::ServiceEvent { service: "now-playing".into(), event: "command".into(), body: "{}".into() };
        assert_eq!(tag_of(&event.encode()), 0x1d3);
        assert_eq!(ToApp::decode(&event.encode()), Ok(event));
    }

    #[test]
    fn keychain_messages_round_trip() {
        let add = |v| KeychainOp::Add { service: "svc".into(), account: "me".into(), value: v };
        for m in [
            ToShell::Keychain { request: 1, op: add(vec![1, 2, 3]) },
            ToShell::Keychain { request: 2, op: KeychainOp::Get { service: "s".into(), account: "a".into() } },
            ToShell::Keychain { request: 3, op: KeychainOp::Update { service: "s".into(), account: "a".into(), value: vec![] } },
            ToShell::Keychain { request: 4, op: KeychainOp::Delete { service: "s".into(), account: "a".into() } },
        ] {
            assert_eq!(ToShell::decode(&m.encode()).unwrap(), m);
        }
        let r = ToApp::KeychainReply { request: 9, status: KeychainStatus::Duplicate, value: vec![7; 5] };
        assert_eq!(ToApp::decode(&r.encode()).unwrap(), r);
        let big = ToShell::Keychain { request: 1, op: add(vec![0; MAX_KEYCHAIN_VALUE + 100]) };
        let Ok(ToShell::Keychain { op: KeychainOp::Add { value, .. }, .. }) = ToShell::decode(&big.encode()) else { panic!("decodes") };
        assert_eq!(value.len(), MAX_KEYCHAIN_VALUE + 1, "cut to one over the limit, which the store refuses");
    }

    #[test]
    fn document_messages_round_trip_and_are_bounded() {
        let g: GrantId = [7; 16];
        for m in [
            ToShell::DocOpen { request: 1, mode: DocMode::Write, title: "Open".into(), filters: vec!["public.text".into(), "md".into()] },
            ToShell::DocOpenEx { request: 5, mode: DocMode::Read, title: "Choose".into(), filters: vec!["public.image".into()], options: docopt::MULTIPLE | docopt::DIRECTORIES | docopt::CREATE_DIRS },
            ToShell::DocSave { request: 2, suggested_name: "notes.md".into(), filters: vec![] },
            ToShell::DocCommit { request: 2 },
            ToShell::DocAbort { request: 3 },
            ToShell::DocReopen { request: 4, grant: g },
            ToShell::DocRelease { grant: g },
            ToShell::DocSaveGrant { request: 5, grant: g },
            ToShell::DocGrantPath { request: 6, path: "/home/a/Notes/b.txt".into() },
        ] {
            assert_eq!(ToShell::decode(&m.encode()), Ok(m));
        }
        for m in [
            ToApp::DocGranted { request: 1, grant: g, mode: DocMode::Read, name: "a.txt".into(), size: 1 << 40 },
            ToApp::DocError { request: 9, code: DocErrorCode::Stale },
            ToApp::DocBatch { request: 5, count: 3 },
        ] {
            assert_eq!(ToApp::decode(&m.encode()), Ok(m));
        }
        // the encoder drops empty and oversized filters and keeps at most MAX_DOC_FILTERS; long titles are cut
        let many: Vec<String> = (0..40).map(|i| format!("ext{i}")).chain(["".into(), "x".repeat(65)]).collect();
        let ToShell::DocOpen { filters, title, .. } =
            ToShell::decode(&ToShell::DocOpen { request: 0, mode: DocMode::Read, title: "t".repeat(1000), filters: many }.encode()).unwrap()
        else {
            panic!()
        };
        assert_eq!((filters.len(), title.chars().count()), (MAX_DOC_FILTERS, MAX_DOC_TEXT));
        // a hand-built packet that breaks the bounds is refused
        let mut bad = Vec::new();
        u32_(&mut bad, tag::DOC_OPEN);
        u32_(&mut bad, 0);
        bad.push(0);
        str_(&mut bad, "t");
        u32_(&mut bad, 17);
        assert_eq!(ToShell::decode(&bad), Err(DecodeError::BadValue));
        let mut bad_mode = ToShell::DocOpen { request: 0, mode: DocMode::Read, title: String::new(), filters: vec![] }.encode();
        bad_mode[8] = 2;
        assert_eq!(ToShell::decode(&bad_mode), Err(DecodeError::BadValue));
        let mut too_many = ToApp::DocBatch { request: 1, count: 1 }.encode();
        too_many[8..12].copy_from_slice(&(MAX_DOC_BATCH + 1).to_le_bytes());
        assert_eq!(ToApp::decode(&too_many), Err(DecodeError::BadValue));
        let mut bad_code = ToApp::DocError { request: 0, code: DocErrorCode::Denied }.encode();
        bad_code[8] = 6;
        assert_eq!(ToApp::decode(&bad_code), Err(DecodeError::BadValue));
        assert_eq!(ToShell::decode(&ToShell::DocRelease { grant: g }.encode()[..10]), Err(DecodeError::Truncated));
    }

    #[test]
    fn chords_round_trip_as_codes_and_plain_codes_are_not_chords() {
        let k = KeyInput::chord(49, CHORD_META | CHORD_SHIFT);
        assert_eq!(k.as_chord(), Some((49, CHORD_META | CHORD_SHIFT)));
        assert_eq!(KeyInput::Code(49).as_chord(), None, "a plain key is not a chord");
        assert_eq!(KeyInput::Text("n".into()).as_chord(), None);
        // a chord is an ordinary Code on the wire, so an SDK 1 app decodes it without error
        let m = ToApp::Key(k.clone()).encode();
        assert_eq!(ToApp::decode(&m).unwrap(), ToApp::Key(k));
    }

    #[test]
    fn controls_are_bounded_and_invalid_commands_rejected() {
        let nodes=(0..200).map(|id|Node{id,role:Role::Button,label:"x".repeat(200),value:"y".repeat(200),bounds:UiRect{x:0.,y:0.,w:44.,h:44.},enabled:true}).collect();
        let packet=ToShell::Controls(nodes).encode();assert!(packet.len()<=MAX_PACKET);
        let ToShell::Controls(decoded)=ToShell::decode(&packet).unwrap() else{panic!("wrong message")};assert!(!decoded.is_empty());assert!(decoded.len()<200);
        let mut packet=ToApp::Command(Command::Cut).encode();packet[4..8].copy_from_slice(&99u32.to_le_bytes());assert_eq!(ToApp::decode(&packet),Err(DecodeError::BadValue));
    }

    #[test]
    fn semantic_control_roles_round_trip_with_stable_wire_values() {
        let roles = [Role::Button, Role::TextField, Role::List, Role::Row, Role::Toggle,
            Role::Checkbox, Role::Slider, Role::SegmentedControl, Role::StaticText,
            Role::MenuItem, Role::Window, Role::ProgressIndicator];
        for (i, role) in roles.into_iter().enumerate() {
            assert_eq!(role as u32, i as u32, "append-only role wire value");
            let node = Node { id: i as u64, role, label: format!("role-{i}"), value: String::new(),
                bounds: UiRect { x: 0., y: 0., w: 40., h: 28. }, enabled: true };
            assert_eq!(ToShell::decode(&ToShell::Controls(vec![node.clone()]).encode()),
                Ok(ToShell::Controls(vec![node])));
        }
        let node = Node { id: 0, role: Role::Button, label: String::new(), value: String::new(),
            bounds: UiRect { x: 0., y: 0., w: 1., h: 1. }, enabled: true };
        let mut invalid = ToShell::Controls(vec![node]).encode();
        invalid[16..20].copy_from_slice(&99u32.to_le_bytes());
        assert_eq!(ToShell::decode(&invalid), Err(DecodeError::BadValue));
    }

    #[test]
    fn to_app_round_trips() {
        let cfg = Configure { w: 1200, h: 1920, scale: 1.5625, safe_top: 24.0, safe_bottom: 20.0, dark: true, text_scale: 1.1, reduce_motion: false };
        let all = vec![
            ToApp::Configure(cfg),
            ToApp::Touch { kind: TouchKind::Down, id: 3, x: 10.5, y: -2.0, t_ms: 123456, mods: ModifierFlags::EMPTY },
            ToApp::Touch { kind: TouchKind::Down, id: 3, x: 1.0, y: 2.0, t_ms: 7, mods: ModifierFlags::SHIFT | ModifierFlags::COMMAND },
            ToApp::Touch { kind: TouchKind::Cancel, id: 0, x: 0.0, y: 0.0, t_ms: 0, mods: ModifierFlags::EMPTY },
            ToApp::Key(KeyInput::Text("héllo ✓".into())),
            ToApp::Key(KeyInput::Code(66)),
            ToApp::Release { buf: 1, generation: 5 },
            ToApp::Command(Command::Paste),
            ToApp::KeyboardConfig(KeyboardConfig::default()),
            ToApp::KeyboardConfig(KeyboardConfig { layout: KeyboardLayout::Floating, return_key: ReturnKey::Send, flags: kbflags::PREDICTIVE | kbflags::SHOWN, languages: 3, language: 1 }),
            ToApp::Activate(u64::MAX),
            ToApp::Resume,
            ToApp::Suspend,
            ToApp::Quit,
            ToApp::Open { url: "https://x.is/?a=1".into() },
            ToApp::Clipboard { text: String::new() },
            ToApp::Appearance { flags: appearance::INCREASE_CONTRAST | appearance::SHOW_BORDERS | 0x8000_0000 },
            ToApp::InputSource { japanese: true },
            ToApp::InputSource { japanese: false },
            ToApp::AccentHold { letter: 'é' },
        ];
        for m in all {
            assert_eq!(ToApp::decode(&m.encode()), Ok(m.clone()), "{m:?}");
        }
    }

    #[test]
    fn to_shell_round_trips() {
        let all = vec![
            ToShell::Hello { sdk_version: SDK_VERSION, app_id: "is.olibuijr.calculator".into() },
            ToShell::Buffers { w: 1200, h: 1920, generation: 2 },
            ToShell::Frame { buf: 1, damage: vec![[0, 0, 1200, 1920], [5, 6, 7, 8]] },
            ToShell::Frame { buf: 0, damage: vec![] },
            ToShell::WantsKeyboard { on: true },
            ToShell::KeyboardOut(KeyboardOut::Text("é😀".into())),
            ToShell::KeyboardOut(KeyboardOut::Backspace),
            ToShell::KeyboardOut(KeyboardOut::Enter),
            ToShell::KeyboardOut(KeyboardOut::Hide),
            ToShell::KeyboardOut(KeyboardOut::Named("Escape".into())),
            ToShell::KeyboardOut(KeyboardOut::Height(824)),
            ToShell::KeyboardOut(KeyboardOut::Click(2)),
            ToShell::KeyboardOut(KeyboardOut::Language(1)),
            ToShell::Commands(Commands(15)),
            ToShell::UndoTitles { undo: "Undo Typing".into(), redo: String::new() },
            ToShell::Controls(vec![Node{id:u64::MAX,role:Role::TextField,label:"Search".into(),value:"café".into(),bounds:UiRect{x:1.,y:2.,w:100.,h:44.},enabled:true}]),
            ToShell::RemoteSharing{on:true},
            ToShell::RemoteSharing{on:false},
            ToShell::Haptic,
            ToShell::SetBadge { label: "12".into() },
            ToShell::SetBadge { label: String::new() },
            ToShell::SetFullScreen { on: true },
            ToShell::SetFullScreen { on: false },
            ToShell::Materials(vec![Material { kind: MaterialKind::Sidebar, rect: UiRect { x: 0., y: 0., w: 220., h: 600. }, radius: 17., state: MaterialState::FollowsWindowActiveState, blending: BlendingMode::BehindWindow, emphasized: false, corners: None },
                Material { kind: MaterialKind::Titlebar, rect: UiRect { x: 220., y: 0., w: 580., h: 52. }, radius: 0., state: MaterialState::Active, blending: BlendingMode::WithinWindow, emphasized: true, corners: Some(Corners { tl: 1., tr: 2., br: 3., bl: 4. }) }]),
            ToShell::Materials(MaterialKind::ALL.iter().map(|&kind| Material { kind, rect: UiRect { x: 1., y: 2., w: 3., h: 4. }, radius: 5., state: MaterialState::Inactive, blending: BlendingMode::BehindWindow, emphasized: false, corners: None }).collect()),
            ToShell::Materials(vec![]),
            ToShell::OpenUrl { url: "https://bifrost.olibuijr.com".into() },
            ToShell::PlaySystemSound { name: "Bell".into() },
            ToShell::Notify { title: "T".into(), body: "B ü".into() },
            ToShell::Log { text: "hi".into() },
            ToShell::SetClipboard { text: "1,5".into() },
        ];
        for m in all {
            assert_eq!(ToShell::decode(&m.encode()), Ok(m.clone()), "{m:?}");
        }
    }

    #[test]
    fn exact_layout() {
        assert_eq!(ToApp::Release { buf: 1, generation: 2 }.encode(), [4, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0]);
        assert_eq!(ToShell::Haptic.encode(), [5, 1, 0, 0]);
        assert_eq!(ToApp::Key(KeyInput::Code(7)).encode(), [3, 0, 0, 0, 1, 7, 0, 0, 0]);
    }

    #[test]
    fn rejects_bad_input() {
        assert_eq!(ToApp::decode(&[]), Err(DecodeError::Truncated));
        assert_eq!(ToApp::decode(&[99, 0, 0, 0]), Err(DecodeError::UnknownTag(99)));
        assert_eq!(ToApp::decode(&[4, 0, 0, 0, 1]), Err(DecodeError::Truncated));
        assert_eq!(ToApp::decode(&[8, 0, 0, 0, 2, 0, 0, 0, 0xff, 0xfe]), Err(DecodeError::BadUtf8));
        assert_eq!(ToShell::decode(&[0x03, 1, 0, 0, 0, 0, 0, 0, 0xff, 0xff, 0xff, 0xff]), Err(DecodeError::BadValue));
        // keyboard enums reject unknown values and a truncated Text
        assert_eq!(ToApp::decode(&[15, 0, 0, 0, 9, 0, 0, 0, 0, 0, 0, 0]), Err(DecodeError::BadValue));
        assert_eq!(ToApp::decode(&[15, 0, 0, 0, 0, 9, 0, 0, 0, 0, 0, 0]), Err(DecodeError::BadValue));
        assert_eq!(ToShell::decode(&[0x0f, 1, 0, 0, 99]), Err(DecodeError::BadValue));
        assert_eq!(ToShell::decode(&[0x0f, 1, 0, 0, 0, 5, 0, 0, 0]), Err(DecodeError::Truncated));
        assert_eq!(ToApp::decode(&[2, 0, 0, 0, 9, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]), Err(DecodeError::BadValue));
    }

    #[test]
    fn long_text_is_clamped_on_a_char_boundary() {
        let m = ToShell::Log { text: "é".repeat(MAX_PACKET) };
        let b = m.encode();
        assert!(b.len() <= MAX_PACKET);
        assert!(matches!(ToShell::decode(&b), Ok(ToShell::Log { .. })));
    }
}

#[cfg(test)]
mod sys_tests {
    use super::sys::*;
    use super::*;

    #[test]
    fn packets_and_fds_cross_a_socketpair() {
        let (a, b) = socketpair_pair().unwrap();
        let shm = shm_create("test", 4096).unwrap();
        let mut m1 = Map::new(&shm, 4096).unwrap();
        m1.pixels_mut()[3] = 0xdead_beef;
        a.send(&ToShell::Buffers { w: 32, h: 32, generation: 1 }.encode(), &[shm.0]).unwrap();
        a.send(&ToShell::Haptic.encode(), &[]).unwrap();
        let Recv::Packet(p, fds) = b.recv(true).unwrap() else { panic!() };
        assert_eq!(ToShell::decode(&p), Ok(ToShell::Buffers { w: 32, h: 32, generation: 1 }));
        assert_eq!(fds.len(), 1);
        let m2 = Map::new(&fds[0], 4096).unwrap();
        assert_eq!(m2.pixels()[3], 0xdead_beef, "the same memory");
        let Recv::Packet(p, fds) = b.recv(false).unwrap() else { panic!() };
        assert_eq!(ToShell::decode(&p), Ok(ToShell::Haptic));
        assert!(fds.is_empty());
        assert!(matches!(b.recv(false).unwrap(), Recv::Empty));
        drop(a);
        assert!(matches!(b.recv(false).unwrap(), Recv::Closed));
    }
}

#[cfg(test)] mod desktop_context_tests {
 use super::*;
 #[test] fn context_roundtrip_and_limits(){let packet=ToShell::ContextMenu{x:12.,y:34.,items:vec![ContextItem{id:7,label:"Open".into(),enabled:true}]};assert_eq!(ToShell::decode(&packet.encode()),Ok(packet));for message in [ToApp::Context{x:12.,y:34.},ToApp::ContextAction(7)]{assert_eq!(ToApp::decode(&message.encode()),Ok(message));}let bad=ToShell::ContextMenu{x:f32::NAN,y:0.,items:vec![]};assert_eq!(ToShell::decode(&bad.encode()),Err(DecodeError::BadValue));let large=ToShell::ContextMenu{x:0.,y:0.,items:(0..100).map(|id|ContextItem{id,label:"x".repeat(10000),enabled:true}).collect()};let bytes=large.encode();assert!(bytes.len()<MAX_PACKET);let ToShell::ContextMenu{items,..}=ToShell::decode(&bytes).unwrap()else{panic!()};assert_eq!(items.len(),32);}
    #[test]
    fn context_items_carry_separators_and_key_equivalents_inside_the_wire_fields() {
        let plain = ContextItem::new(7, "Open");
        assert_eq!(plain, ContextItem { id: 7, label: "Open".into(), enabled: true });
        assert_eq!((plain.title(), plain.key_combo(), plain.is_separator()), ("Open", None, false));
        let keyed = ContextItem::new(9, "Get Info").key("cmd+i").enabled(false);
        assert_eq!((keyed.title(), keyed.key_combo(), keyed.enabled), ("Get Info", Some("cmd+i"), false));
        assert_eq!(ContextItem::new(9, "Get Info").key("cmd+i").key("cmd+j").label, "Get Info\tcmd+j", "a second key replaces the first");
        let line = ContextItem::separator();
        assert!(line.is_separator() && !line.enabled);
        assert!(!ContextItem::new(0, "Remove").is_separator(), "id 0 with a title is a row");
        // an app built before keys and separators sends plain titles: they stay rows without a key
        assert_eq!((ContextItem { id: 3, label: "Remove".into(), enabled: true }).key_combo(), None);
        let packet = ToShell::ContextMenu { x: 1., y: 2., items: vec![keyed.clone(), line.clone(), plain.clone()] };
        let ToShell::ContextMenu { items, .. } = ToShell::decode(&packet.encode()).unwrap() else { panic!() };
        assert_eq!(items, vec![keyed, line, plain], "separators and keys survive the wire");
    }
    #[test]
    fn app_menus_roundtrip_and_limits() {
        let menus = vec![AppMenu { title: "Note".into(), items: vec![
            AppMenuItem::new(1, "New Note").key("cmd+n"),
            AppMenuItem::separator(),
            AppMenuItem::new(2, "Pin").checked(true).enabled(false),
            AppMenuItem::submenu("Sort By"),
            AppMenuItem::new(3, "Name").depth(1),
        ] }];
        let packet = ToShell::AppMenus(menus);
        assert_eq!(ToShell::decode(&packet.encode()), Ok(packet));
        let ds = ToShell::DocumentState { title: "Untitled 2".into(), edited: true, represented: "a.txt".into() };
        assert_eq!(ToShell::decode(&ds.encode()), Ok(ds));
        let pb = ToShell::PasteboardSet { items: vec![("public.html".into(), b"<b>x</b>".to_vec())] };
        assert_eq!(ToShell::decode(&pb.encode()), Ok(pb));
        let pa = ToApp::Pasteboard { change_count: 3, items: vec![("public.png".into(), vec![1, 2, 3])] };
        assert_eq!(ToApp::decode(&pa.encode()), Ok(pa));
        assert_eq!(ToApp::decode(&ToApp::MenuAction(9).encode()), Ok(ToApp::MenuAction(9)));
        assert_eq!(ToShell::decode(&ToShell::AppMenus(vec![]).encode()), Ok(ToShell::AppMenus(vec![])));
        let many = ToShell::AppMenus((0..20).map(|_| AppMenu { title: "x".into(), items: (0..200).map(|i| AppMenuItem::new(i, &"t".repeat(1000))).collect() }).collect());
        let bytes = many.encode();
        assert!(bytes.len() < MAX_PACKET);
        let ToShell::AppMenus(m) = ToShell::decode(&bytes).unwrap() else { panic!() };
        assert!(!m.is_empty() && m.len() <= MAX_APP_MENUS);
        assert_eq!((m[0].items.len(), m[0].items[0].title.chars().count()), (MAX_APP_MENU_ITEMS, MAX_APP_MENU_TITLE));
    }
    #[test]
    fn the_apps_own_print_item_keeps_its_flag_over_the_wire() {
        let print = AppMenuItem::print(7);
        assert_eq!((print.title.as_str(), print.key.as_str(), print.prints), ("Print\u{2026}", "cmd+p", true));
        assert!(!AppMenuItem::new(8, "Print Address Labels\u{2026}").prints, "only the declared item is the app's Print");
        let packet = ToShell::AppMenus(vec![AppMenu { title: "File".into(), items: vec![print.clone().enabled(false), AppMenuItem::new(8, "Export")] }]);
        let bytes = packet.encode();
        assert_eq!(ToShell::decode(&bytes), Ok(packet));
        // a flag byte past the five known bits is refused
        let flags_at = 4 + 4 + (4 + 4) + 4 + 4 + (4 + "Print\u{2026}".len()) + (4 + 5);
        assert_eq!(bytes[flags_at], 16, "disabled, with the print bit");
        let mut bad = bytes.clone();
        bad[flags_at] = 32;
        assert_eq!(ToShell::decode(&bad), Err(DecodeError::BadValue));
    }
}

#[cfg(test)]
mod ax_tree_tests {
    use super::*;

    #[test]
    fn text_input_messages_round_trip() {
        let to_app = [
            ToApp::SetMarkedText { text: "にほん".into(), sel_start: 3, sel_len: 3 },
            ToApp::SetMarkedText { text: String::new(), sel_start: 0, sel_len: 0 },
            ToApp::UnmarkText { commit: true },
            ToApp::UnmarkText { commit: false },
            ToApp::DeleteSurrounding { before: 2, after: 1 },
            ToApp::AccessoryAction(9),
        ];
        for m in to_app {
            assert_eq!(ToApp::decode(&m.encode()), Ok(m));
        }
        let to_shell = [
            ToShell::TextState(TextState { text: "héllo wörld".into(), cursor: 3, anchor: 7, marked: Some((1, 3)) }),
            ToShell::TextState(TextState::default()),
            ToShell::CaretRect { x: 1.0, y: 2.0, w: 0.0, h: 18.0 },
            ToShell::InputTraits(InputTraits { return_key: ReturnKeyType::Continue, content: TextContentKind::Email, flags: tiflags::AUTOCORRECT | tiflags::SECURE }),
            ToShell::InputAccessory(vec![AccessoryItem { id: 1, label: "Done".into(), enabled: true }, AccessoryItem { id: 2, label: "Next".into(), enabled: false }]),
            ToShell::InputAccessory(vec![]),
            ToShell::SuddenTermination { enabled: true },
            ToShell::SuddenTermination { enabled: false },
            ToShell::Activity { options: 0x3 },
            ToShell::Activity { options: 0 },
            ToShell::WindowLevel(WindowLevel::Floating),
            ToShell::WindowLevel(WindowLevel::Normal),
            ToShell::WindowLevelOf { id: 2, level: WindowLevel::Floating },
            ToShell::WindowLevelOf { id: 2, level: WindowLevel::Normal },
        ];
        for m in to_shell {
            assert_eq!(ToShell::decode(&m.encode()), Ok(m));
        }
    }

    #[test]
    fn a_window_level_of_one_window_is_id_then_one_byte() {
        let m = ToShell::WindowLevelOf { id: 3, level: WindowLevel::Floating }.encode();
        assert_eq!(&m[..4], &0x24cu32.to_le_bytes());
        assert_eq!(&m[4..8], &3u32.to_le_bytes());
        assert_eq!(*m.last().unwrap(), 1);
        let mut bad = m.clone();
        *bad.last_mut().unwrap() = 9;
        assert_eq!(ToShell::decode(&bad), Err(DecodeError::BadValue));
    }

    #[test]
    fn window_level_is_one_byte_and_rejects_an_unknown_level() {
        let m = ToShell::WindowLevel(WindowLevel::Floating).encode();
        assert_eq!(m.len(), 4 + 1, "the tag, then the level byte");
        assert_eq!(*m.last().unwrap(), 1);
        let mut bad = m.clone();
        let n = bad.len();
        bad[n - 1] = 7;
        assert_eq!(ToShell::decode(&bad), Err(DecodeError::BadValue));
    }

    #[test]
    fn keyboard_preedit_round_trips_and_rejects_a_bad_selection() {
        for o in [KeyboardOut::Preedit { text: "にほん".into(), sel_start: 3, sel_len: 3 }, KeyboardOut::Unmark { commit: true }] {
            let m = ToShell::KeyboardOut(o);
            assert_eq!(ToShell::decode(&m.encode()), Ok(m));
        }
        let mut p = ToShell::KeyboardOut(KeyboardOut::Preedit { text: "é".into(), sel_start: 0, sel_len: 2 }).encode();
        let n = p.len();
        p[n - 4] = 1;
        assert_eq!(ToShell::decode(&p), Err(DecodeError::BadValue));
    }

    #[test]
    fn text_input_messages_reject_bad_values() {
        // a selection that ends past the text
        let mut p = ToApp::SetMarkedText { text: "ab".into(), sel_start: 0, sel_len: 2 }.encode();
        let n = p.len();
        p[n - 4] = 9;
        assert_eq!(ToApp::decode(&p), Err(DecodeError::BadValue));
        // a selection that cuts a character in half
        let mut p = ToApp::SetMarkedText { text: "é".into(), sel_start: 0, sel_len: 2 }.encode();
        let n = p.len();
        p[n - 4] = 1;
        assert_eq!(ToApp::decode(&p), Err(DecodeError::BadValue));
        let mut p = ToShell::InputTraits(InputTraits::default()).encode();
        p[4] = 99;
        assert_eq!(ToShell::decode(&p), Err(DecodeError::BadValue));
        assert_eq!(ToShell::decode(&ToShell::CaretRect { x: f32::NAN, y: 0., w: 1., h: 1. }.encode()), Err(DecodeError::BadValue));
    }

    #[test]
    fn text_state_window_is_cut_around_the_caret() {
        let text = "é".repeat(MAX_SURROUNDING); // twice the limit in bytes
        let cursor = MAX_SURROUNDING as u32; // a character boundary in the middle
        let m = ToShell::TextState(TextState { text, cursor, anchor: cursor, marked: None });
        let Ok(ToShell::TextState(t)) = ToShell::decode(&m.encode()) else { panic!("decode") };
        assert!(t.text.len() <= MAX_SURROUNDING);
        assert!(t.cursor as usize <= t.text.len());
        assert_eq!(t.cursor, t.anchor);
    }

    #[test]
    fn return_key_types_map_to_the_keyboard_labels() {
        for v in 0..=11u8 {
            let t = ReturnKeyType::from_wire(v).unwrap();
            assert_eq!(t as u8, v);
            let _ = t.basic();
        }
        assert_eq!(ReturnKeyType::from_wire(12), None);
        assert_eq!(ReturnKeyType::Yahoo.basic(), ReturnKey::Search);
        assert_eq!(ReturnKeyType::Continue.basic(), ReturnKey::Next);
    }

    #[test]
    fn controls_tree_round_trips() {
        let list = vec![
            NodeExt { id: 1, parent: None, focused: false, checked: None },
            NodeExt { id: u64::MAX, parent: Some(1 << 40), focused: true, checked: Some(true) },
            NodeExt { id: 7, parent: Some(0), focused: false, checked: Some(false) },
        ];
        assert_eq!(ToShell::decode(&ToShell::ControlsTree(list.clone()).encode()), Ok(ToShell::ControlsTree(list)));
    }

    #[test]
    fn controls_tree_rejects_unknown_flags_and_oversize() {
        let mut p = ToShell::ControlsTree(vec![NodeExt { id: 1, parent: None, focused: false, checked: None }]).encode();
        *p.last_mut().unwrap() = 0x80;
        assert_eq!(ToShell::decode(&p), Err(DecodeError::BadValue));
        let big = ToShell::ControlsTree(vec![NodeExt { id: 1, parent: None, focused: false, checked: None }; 300]).encode();
        let Ok(ToShell::ControlsTree(l)) = ToShell::decode(&big) else { panic!("decode") };
        assert_eq!(l.len(), MAX_NODE_EXT);
    }

    #[test]
    fn ax_set_round_trips() {
        for action in [AxAction::Press, AxAction::Focus, AxAction::Increment, AxAction::Decrement, AxAction::SetValue("héllo".into())] {
            let m = ToApp::AxSet { id: 0x1_0000_0002, action };
            assert_eq!(ToApp::decode(&m.encode()), Ok(m));
        }
        let mut bad = ToApp::AxSet { id: 1, action: AxAction::Press }.encode();
        *bad.last_mut().unwrap() = 9;
        assert_eq!(ToApp::decode(&bad), Err(DecodeError::BadValue));
    }
}

#[cfg(test)]
mod material_tests {
    use super::sys::*;
    use super::*;

    #[test]
    fn every_material_kind_sits_at_its_wire_value() {
        for (i, k) in MaterialKind::ALL.iter().enumerate() {
            assert_eq!(*k as usize, i);
        }
    }

    #[test]
    fn an_older_apps_material_byte_decodes_with_the_default_state() {
        let m = Material { kind: MaterialKind::Titlebar, rect: UiRect { x: 0., y: 0., w: 10., h: 10. }, radius: 0., state: MaterialState::FollowsWindowActiveState, blending: BlendingMode::BehindWindow, emphasized: false, corners: None };
        let bytes = ToShell::Materials(vec![m]).encode();
        // the kind byte of a state-less sender is the bare kind
        assert!(bytes.windows(1).any(|b| b[0] == 1));
        assert_eq!(ToShell::decode(&bytes), Ok(ToShell::Materials(vec![m])));
    }

    #[test]
    fn the_extended_material_round_trips_through_the_request() {
        let mut m = Material::new(MaterialKind::HudWindow, UiRect { x: 1., y: 2., w: 30., h: 40. }, 6.);
        m.blending = BlendingMode::WithinWindow;
        m.emphasized = true;
        m.corners = Some(Corners { tl: 1., tr: 2., br: 3., bl: 4. });
        assert_eq!(ToShell::decode(&ToShell::Materials(vec![m]).encode()), Ok(ToShell::Materials(vec![m])));
        let back = Material::from_request(&m.request());
        assert_eq!((back.kind, back.blending, back.emphasized, back.corners), (m.kind, m.blending, true, m.corners));
        // unknown flag bits and negative corners are refused
        let mut bad = ToShell::Materials(vec![Material::new(MaterialKind::Menu, UiRect { x: 0., y: 0., w: 1., h: 1. }, 0.)]).encode();
        *bad.last_mut().unwrap() = 8;
        assert_eq!(ToShell::decode(&bad), Err(DecodeError::BadValue));
    }

    #[test]
    fn legacy_kinds_are_the_two_an_older_shell_decodes() {
        for k in MaterialKind::ALL {
            assert!(matches!(k.legacy(), MaterialKind::Sidebar | MaterialKind::Titlebar));
        }
        assert_eq!(MaterialKind::Sidebar.legacy(), MaterialKind::Sidebar);
    }

    #[test]
    fn an_unknown_kind_or_state_is_refused() {
        let m = Material { kind: MaterialKind::Selection, rect: UiRect { x: 0., y: 0., w: 1., h: 1. }, radius: 0., state: MaterialState::Inactive, blending: BlendingMode::BehindWindow, emphasized: false, corners: None };
        let good = ToShell::Materials(vec![m]).encode();
        let at = good.iter().position(|&b| b == (13 | 2 << 6)).unwrap();
        for bad_byte in [14u8, 3 << 6] {
            let mut bad = good.clone();
            bad[at] = bad_byte;
            assert_eq!(ToShell::decode(&bad), Err(DecodeError::BadValue));
        }
    }

    #[test]
    fn pasteboard_encode_never_panics_or_shortens() {
        // a multibyte char straddling byte 128 of the type (IPC-2)
        let m = ToApp::Pasteboard { change_count: 1, items: vec![("é".repeat(100), vec![])] };
        let Ok(ToApp::Pasteboard { items, .. }) = ToApp::decode(&m.encode()) else { panic!("decode") };
        assert!(items[0].0.len() <= 128 && !items[0].0.is_empty());
        // a payload that does not fit is left out whole, never cut (IPC-5)
        let big = ToShell::PasteboardSet { items: vec![("public.png".into(), vec![7; 50_000]), ("public.utf8-plain-text".into(), b"hi".to_vec())] };
        let bytes = big.encode();
        assert!(bytes.len() < MAX_PACKET);
        let Ok(ToShell::PasteboardSet { items }) = ToShell::decode(&bytes) else { panic!("decode") };
        assert_eq!(items, vec![("public.utf8-plain-text".to_string(), b"hi".to_vec())]);
        // a type over 128 bytes is refused on decode
        let mut p = Vec::new();
        u32_(&mut p, tag::PASTEBOARD_SET);
        u32_(&mut p, 1);
        str_(&mut p, &"t".repeat(129));
        u32_(&mut p, 0);
        assert_eq!(ToShell::decode(&p), Err(DecodeError::BadValue));
    }

    #[test]
    fn pasteboard_blob_messages_round_trip_and_refuse_bad_layouts() {
        let types = vec![("public.png".to_string(), 100_000u32), ("public.utf8-plain-text".to_string(), 2)];
        let a = ToApp::PasteboardBlob { change_count: 9, types: types.clone() };
        assert_eq!(ToApp::decode(&a.encode()), Ok(a));
        let s = ToShell::PasteboardBlob { types: types.clone() };
        assert_eq!(ToShell::decode(&s.encode()), Ok(s));
        let c = ToApp::PasteboardChanged { change_count: 4 };
        assert_eq!(ToApp::decode(&c.encode()), Ok(c));
        assert_eq!(ToShell::decode(&ToShell::PasteboardRead.encode()), Ok(ToShell::PasteboardRead));
        // a layout over the blob limits, with a control character, or with too many items is refused
        for bad in [
            vec![("t".to_string(), (pbblob::MAX_ITEM + 1) as u32)],
            vec![("t\0".to_string(), 1)],
            vec![("t".to_string(), 1); pbblob::MAX_ITEMS + 1],
        ] {
            let mut p = Vec::new();
            u32_(&mut p, tag::PASTEBOARD_BLOB);
            u32_(&mut p, bad.len() as u32);
            for (t, n) in &bad {
                str_(&mut p, t);
                u32_(&mut p, *n);
            }
            assert_eq!(ToShell::decode(&p), Err(DecodeError::BadValue));
        }
    }

    #[test]
    fn a_pasteboard_type_with_a_control_character_is_left_out() {
        let m = ToShell::PasteboardSet { items: vec![("\0/x".into(), b"a".to_vec()), ("public.utf8-plain-text".into(), b"ok".to_vec())] };
        // the sender leaves it out ...
        assert_eq!(ToShell::decode(&m.encode()), Ok(ToShell::PasteboardSet { items: vec![("public.utf8-plain-text".into(), b"ok".to_vec())] }));
        // ... and so does the receiver, whatever a hostile sender put on the wire
        let mut p = Vec::new();
        u32_(&mut p, tag::PASTEBOARD_SET);
        u32_(&mut p, 2);
        str_(&mut p, "a\u{7}/b");
        u32_(&mut p, 1);
        p.push(1);
        str_(&mut p, "public.png");
        u32_(&mut p, 1);
        p.push(2);
        assert_eq!(ToShell::decode(&p), Ok(ToShell::PasteboardSet { items: vec![("public.png".into(), vec![2])] }));
    }

    #[test]
    fn oversized_service_reply_is_refused_not_cut() {
        let m = ToApp::ServiceReply { request: 3, ok: true, body: "x".repeat(20_000) };
        let bytes = m.encode();
        assert!(bytes.len() <= MAX_PACKET);
        assert_eq!(ToApp::decode(&bytes), Ok(ToApp::ServiceReply { request: 3, ok: false, body: "body too large".into() }));
        let fits = ToApp::ServiceReply { request: 3, ok: true, body: "y".repeat(15_000) };
        assert_eq!(ToApp::decode(&fits.encode()), Ok(fits));
    }

    #[test]
    fn short_memfd_is_measurable_and_shrink_seal_holds() {
        let fd = shm_create("seal-test", 4096).unwrap();
        assert_eq!(shm_len(&fd).unwrap(), 4096);
        seal_shrink(&fd).unwrap();
        assert!(shm_resize(&fd, 0).is_err(), "a sealed memfd cannot shrink");
        assert!(shm_resize(&fd, 8192).is_ok());
    }

    #[test]
    fn try_send_does_not_block_and_oversized_packets_are_errors() {
        let (a, b) = socketpair_pair().unwrap();
        set_nonblocking(a.fd()).unwrap();
        let pkt = vec![0u8; 16 * 1024];
        let mut sent = 0;
        let err = loop {
            match a.try_send(&pkt, &[]) {
                Ok(()) => sent += 1,
                Err(e) => break e,
            }
            assert!(sent < 100_000, "a never-read socket must fill up");
        };
        assert_eq!(err.kind(), std::io::ErrorKind::WouldBlock);
        drop(b);
        let (c, d) = socketpair_pair().unwrap();
        c.send(&vec![1u8; 20 * 1024], &[]).unwrap();
        assert!(d.recv(true).is_err(), "a 20 KiB packet is a protocol error, not a cut Packet");
    }
}

#[cfg(test)]
mod frame_damage_tests {
    use super::*;

    #[test]
    fn a_frame_with_more_damage_than_a_packet_holds_is_clamped_and_still_decodes() {
        let rects: Vec<Rect> = (0..5000u32).map(|i| [i, i, 1, 1]).collect();
        let bytes = ToShell::Frame { buf: 1, damage: rects.clone() }.encode();
        assert!(bytes.len() <= MAX_PACKET, "the encoder never builds a packet the decoder refuses");
        let ToShell::Frame { buf, damage } = ToShell::decode(&bytes).unwrap() else { panic!("not a frame") };
        assert_eq!(buf, 1);
        assert_eq!(damage.len(), (MAX_PACKET - 16) / 16, "the clamp keeps the first rectangles");
        assert_eq!(damage[..], rects[..damage.len()]);
    }

    #[test]
    fn a_frame_claiming_more_rectangles_than_a_packet_can_hold_is_refused() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&tag::FRAME.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&((MAX_PACKET / 16 + 1) as u32).to_le_bytes());
        bytes.resize(bytes.len() + (MAX_PACKET / 16 + 1) * 16, 0);
        assert_eq!(ToShell::decode(&bytes), Err(DecodeError::BadValue));
    }

    #[test]
    fn an_empty_frame_is_a_small_packet() {
        let bytes = ToShell::Frame { buf: 0, damage: Vec::new() }.encode();
        assert_eq!(bytes.len(), 12, "tag, buffer and a zero count");
        assert_eq!(ToShell::decode(&bytes), Ok(ToShell::Frame { buf: 0, damage: Vec::new() }));
    }
}

#[cfg(test)]
mod quick_action_tests {
    use super::*;

    #[test]
    fn a_chosen_quick_action_round_trips_with_its_user_info() {
        let message = ToApp::QuickAction { kind: "compose".into(), user_info: vec![("account".into(), "work".into())] };
        assert_eq!(ToApp::decode(&message.encode()), Ok(message));
        let bare = ToApp::QuickAction { kind: "search".into(), user_info: Vec::new() };
        assert_eq!(ToApp::decode(&bare.encode()), Ok(bare));
    }

    #[test]
    fn a_quick_action_with_too_much_user_info_is_refused() {
        let many: Vec<(String, String)> = (0..=MAX_QUICK_INFO).map(|n| (format!("k{n}"), "v".into())).collect();
        let bytes = ToApp::QuickAction { kind: "x".into(), user_info: many }.encode();
        assert_eq!(ToApp::decode(&bytes), Err(DecodeError::BadValue));
    }

    #[test]
    fn window_messages_round_trip_through_the_codec() {
        let shell = [
            ToShell::WindowOpen { id: 3, title: "Ada Lovelace".into(), w: 480.0, h: 560.0, position: None, flags: 0 },
            ToShell::WindowClose { id: 3 },
            ToShell::WindowTitle { id: 3, title: "Grace".into() },
            ToShell::WindowResize { id: 3, w: 320.0, h: 240.0 },
            ToShell::WindowBuffers { id: 3, w: 4, h: 3, generation: 2 },
            ToShell::WindowFrame { id: 3, buf: 1, damage: vec![[0, 0, 4, 3]] },
            ToShell::WindowFrame { id: 3, buf: 0, damage: vec![] },
        ];
        for m in shell {
            assert_eq!(ToShell::decode(&m.encode()), Ok(m.clone()), "{m:?}");
        }
        let app = [
            ToApp::WindowClosed { id: 3 },
            ToApp::FullScreen { on: true },
            ToApp::WindowConfigure { id: 3, w: 320.0, h: 240.0 },
            ToApp::WindowTouch { id: 3, touch: 1, kind: TouchKind::Move, x: 10.5, y: -2.0, t_ms: 99, mods: ModifierFlags::OPTION },
            ToApp::WindowRelease { id: 3, buf: 1, generation: 5 },
        ];
        for m in app {
            assert_eq!(ToApp::decode(&m.encode()), Ok(m.clone()), "{m:?}");
        }
    }

    #[test]
    fn key_window_messages_round_trip_through_the_codec() {
        let app = [
            ToApp::WindowFocus { window: 3, focused: true },
            ToApp::WindowFocus { window: 0, focused: false },
            ToApp::WindowInput { window: 3, input: WindowInput::Key(KeyInput::Text("é😀".into())) },
            ToApp::WindowInput { window: 3, input: WindowInput::Key(KeyInput::Code(30)) },
            ToApp::WindowInput { window: 3, input: WindowInput::Command(Command::SelectAll) },
            ToApp::WindowInput { window: 3, input: WindowInput::MenuAction(7) },
            ToApp::WindowInput { window: 3, input: WindowInput::SetMarkedText { text: "にほん".into(), sel_start: 3, sel_len: 3 } },
            ToApp::WindowInput { window: 3, input: WindowInput::UnmarkText { commit: true } },
            ToApp::WindowInput { window: 3, input: WindowInput::DeleteSurrounding { before: 2, after: 0 } },
            ToApp::WindowInput {
                window: 3,
                input: WindowInput::Pointer(bifrost_ui::pointer_event::PointerEvent::Wheel {
                    x: 12.5,
                    y: 40.0,
                    dx: 0.0,
                    dy: -19.2,
                    precise: false,
                    phase: bifrost_ui::wheel::Phase::None,
                    momentum: bifrost_ui::wheel::Phase::None,
                }),
            },
            ToApp::WindowInput { window: 3, input: WindowInput::Pointer(bifrost_ui::pointer_event::PointerEvent::Hover { x: 1.0, y: 2.0 }) },
            ToApp::WindowInput { window: 3, input: WindowInput::Pointer(bifrost_ui::pointer_event::PointerEvent::HoverExit) },
        ];
        for m in app {
            assert_eq!(ToApp::decode(&m.encode()), Ok(m.clone()), "{m:?}");
        }
    }

    #[test]
    fn window_input_has_the_documented_layout() {
        // tag 0x250, window 3, kind 0 (key), key kind 1 (code), code 30
        let bytes = ToApp::WindowInput { window: 3, input: WindowInput::Key(KeyInput::Code(30)) }.encode();
        assert_eq!(&bytes[..4], &0x250u32.to_le_bytes());
        assert_eq!(&bytes[4..8], &3u32.to_le_bytes());
        assert_eq!(&bytes[8..10], &[0, 1]);
        assert_eq!(&bytes[10..14], &30u32.to_le_bytes());
        // tag 0x24f, window 3, focused
        let focus = ToApp::WindowFocus { window: 3, focused: true }.encode();
        assert_eq!(&focus[..4], &0x24fu32.to_le_bytes());
        assert_eq!(&focus[4..9], &[3, 0, 0, 0, 1]);
    }

    #[test]
    fn window_input_for_the_main_window_uses_the_legacy_messages() {
        assert_eq!(WindowInput::Key(KeyInput::Code(9)).to_app(0), ToApp::Key(KeyInput::Code(9)));
        assert_eq!(WindowInput::Command(Command::Copy).to_app(0), ToApp::Command(Command::Copy));
        assert_eq!(WindowInput::MenuAction(4).to_app(0), ToApp::MenuAction(4));
        assert_eq!(WindowInput::UnmarkText { commit: false }.to_app(0), ToApp::UnmarkText { commit: false });
        assert_eq!(WindowInput::MenuAction(4).to_app(2), ToApp::WindowInput { window: 2, input: WindowInput::MenuAction(4) });
        let hover = bifrost_ui::pointer_event::PointerEvent::Hover { x: 3.0, y: 4.0 };
        assert_eq!(WindowInput::Pointer(hover).to_app(0), ToApp::Pointer(hover));
        assert_eq!(WindowInput::Pointer(hover).to_app(5), ToApp::WindowInput { window: 5, input: WindowInput::Pointer(hover) });
    }

    #[test]
    fn a_window_pointer_event_has_the_pointer_layout_after_kind_six() {
        use bifrost_ui::pointer_event::PointerEvent;
        let ev = PointerEvent::Hover { x: 1.5, y: 2.5 };
        let bytes = ToApp::WindowInput { window: 4, input: WindowInput::Pointer(ev) }.encode();
        assert_eq!(&bytes[..4], &0x250u32.to_le_bytes());
        assert_eq!(&bytes[4..8], &4u32.to_le_bytes());
        // kind 6, then the `ToApp::Pointer` bytes without their tag: kind 1 (hover) and the two floats
        let main = ToApp::Pointer(ev).encode();
        assert_eq!(bytes[8], 6);
        assert_eq!(&bytes[9..], &main[4..]);
        // a NaN location is refused here as it is for the main window
        let mut bad = bytes.clone();
        bad[10..14].copy_from_slice(&f32::NAN.to_le_bytes());
        assert_eq!(ToApp::decode(&bad), Err(DecodeError::BadValue));
    }

    #[test]
    fn a_bad_window_focus_or_window_input_is_refused() {
        let mut focus = ToApp::WindowFocus { window: 1, focused: true }.encode();
        focus[8] = 2;
        assert_eq!(ToApp::decode(&focus), Err(DecodeError::BadValue));
        // unknown input kind
        let mut kind = ToApp::WindowInput { window: 1, input: WindowInput::MenuAction(1) }.encode();
        kind[8] = 9;
        assert_eq!(ToApp::decode(&kind), Err(DecodeError::BadValue));
        // unknown key kind inside a key
        let mut key = ToApp::WindowInput { window: 1, input: WindowInput::Key(KeyInput::Code(1)) }.encode();
        key[9] = 7;
        assert_eq!(ToApp::decode(&key), Err(DecodeError::BadValue));
        // a command number the table does not know
        let mut cmd = ToApp::WindowInput { window: 1, input: WindowInput::Command(Command::Copy) }.encode();
        cmd[9..13].copy_from_slice(&99u32.to_le_bytes());
        assert_eq!(ToApp::decode(&cmd), Err(DecodeError::BadValue));
        // a marked-text selection that ends inside a character
        let mut marked = ToApp::WindowInput { window: 1, input: WindowInput::SetMarkedText { text: "é".into(), sel_start: 0, sel_len: 2 } }.encode();
        // kind 3 at byte 8, the text length at 9..13, "é" at 13..15, then sel_start and sel_len
        marked[15..19].copy_from_slice(&0u32.to_le_bytes());
        marked[19..23].copy_from_slice(&1u32.to_le_bytes());
        assert_eq!(ToApp::decode(&marked), Err(DecodeError::BadValue));
    }

    #[test]
    fn window_open_has_the_documented_layout() {
        let bytes = ToShell::WindowOpen { id: 2, title: "ab".into(), w: 1.0, h: 2.0, position: None, flags: 0 }.encode();
        // tag 0x240, id 2, title length 2 and "ab", then the two floats
        assert_eq!(&bytes[..4], &0x240u32.to_le_bytes());
        assert_eq!(&bytes[4..8], &2u32.to_le_bytes());
        assert_eq!(&bytes[8..12], &2u32.to_le_bytes());
        assert_eq!(&bytes[12..14], b"ab");
    }

    #[test]
    fn window_placement_and_collapse_round_trip() {
        let shell = [
            ToShell::WindowOpen { id: 4, title: "Note".into(), w: 200.0, h: 150.0, position: Some((12.5, 40.0)), flags: window_flags::TITLE_DOUBLE_CLICK },
            ToShell::WindowOpen { id: 5, title: "Plain".into(), w: 1.0, h: 1.0, position: None, flags: 0 },
            ToShell::WindowMove { window: 4, x: -3.0, y: 88.0 },
            ToShell::WindowCollapse { window: 4, collapsed: true },
            ToShell::WindowCollapse { window: 4, collapsed: false },
        ];
        for m in shell {
            assert_eq!(ToShell::decode(&m.encode()), Ok(m.clone()), "{m:?}");
        }
        let app = [ToApp::WindowMoved { window: 4, x: 10.0, y: 20.0 }, ToApp::WindowTitleDoubleClick { window: 4 }];
        for m in app {
            assert_eq!(ToApp::decode(&m.encode()), Ok(m.clone()), "{m:?}");
        }
    }

    #[test]
    fn window_placement_refuses_a_bad_flag_or_a_non_finite_position() {
        let mut bad = ToShell::WindowCollapse { window: 4, collapsed: true }.encode();
        let n = bad.len();
        bad[n - 1] = 2;
        assert_eq!(ToShell::decode(&bad), Err(DecodeError::BadValue));
        let nan = ToShell::WindowMove { window: 4, x: f32::NAN, y: 0.0 }.encode();
        assert_eq!(ToShell::decode(&nan), Err(DecodeError::BadValue));
        let open = ToShell::WindowOpen { id: 4, title: "a".into(), w: 1.0, h: 1.0, position: Some((f32::INFINITY, 0.0)), flags: 0 }.encode();
        assert_eq!(ToShell::decode(&open), Err(DecodeError::BadValue));
    }

    #[test]
    fn an_overlong_or_non_finite_window_message_is_refused() {
        let long = ToShell::WindowTitle { id: 1, title: "x".repeat(MAX_WINDOW_TITLE + 1) };
        // the encoder cuts the title, so the decoder never sees an overlong one from this crate
        let ToShell::WindowTitle { title, .. } = ToShell::decode(&long.encode()).unwrap() else { panic!() };
        assert_eq!(title.len(), MAX_WINDOW_TITLE);
        let mut bad = ToShell::WindowResize { id: 1, w: 1.0, h: 1.0 }.encode();
        bad[8..12].copy_from_slice(&f32::NAN.to_le_bytes());
        assert!(ToShell::decode(&bad).is_err());
        let mut bad_touch = ToApp::WindowTouch { id: 1, touch: 0, kind: TouchKind::Down, x: 0.0, y: 0.0, t_ms: 0, mods: ModifierFlags::EMPTY }.encode();
        bad_touch[12] = 9;
        assert!(ToApp::decode(&bad_touch).is_err());
    }
}

#[cfg(test)]
mod modifier_flag_tests {
    use super::*;

    #[test]
    fn modifier_flags_use_the_reference_raw_values() {
        assert_eq!(ModifierFlags::CAPS_LOCK.bits(), 1 << 16);
        assert_eq!(ModifierFlags::SHIFT.bits(), 1 << 17);
        assert_eq!(ModifierFlags::CONTROL.bits(), 1 << 18);
        assert_eq!(ModifierFlags::OPTION.bits(), 1 << 19);
        assert_eq!(ModifierFlags::COMMAND.bits(), 1 << 20);
        assert_eq!(ModifierFlags::from_bits_truncate(u32::MAX), ModifierFlags::ALL, "unknown bits are dropped");
        let both = ModifierFlags::SHIFT | ModifierFlags::COMMAND;
        assert!(both.shift() && both.command() && !both.option() && !both.control() && !both.caps_lock());
        assert!(ModifierFlags::default().is_empty());
    }

    #[test]
    fn chord_bits_and_held_keys_map_to_the_same_flags() {
        assert_eq!(ModifierFlags::from_chord(CHORD_SHIFT | CHORD_META), ModifierFlags::SHIFT | ModifierFlags::COMMAND);
        assert_eq!(ModifierFlags::from_chord(CHORD_CTRL | CHORD_ALT), ModifierFlags::CONTROL | ModifierFlags::OPTION);
        assert_eq!(ModifierFlags::from_held(true, false, false, true, false), ModifierFlags::SHIFT | ModifierFlags::COMMAND);
        assert_eq!(ModifierFlags::from_held(false, false, false, false, true), ModifierFlags::CAPS_LOCK);
    }

    #[test]
    fn a_touch_carries_its_modifiers_and_one_without_them_decodes_as_none_held() {
        let m = ToApp::Touch { kind: TouchKind::Down, id: 0, x: 4.0, y: 5.0, t_ms: 9, mods: ModifierFlags::SHIFT };
        assert_eq!(ToApp::decode(&m.encode()), Ok(m.clone()));
        // a sender built before the flags stops after `t_ms`
        let mut old = m.encode();
        old.truncate(old.len() - 4);
        let Ok(ToApp::Touch { mods, t_ms, .. }) = ToApp::decode(&old) else { panic!() };
        assert_eq!((mods, t_ms), (ModifierFlags::EMPTY, 9));
        let w = ToApp::WindowTouch { id: 2, touch: 1, kind: TouchKind::Up, x: 1.0, y: 1.0, t_ms: 3, mods: ModifierFlags::COMMAND };
        assert_eq!(ToApp::decode(&w.encode()), Ok(w.clone()));
        let mut old = w.encode();
        old.truncate(old.len() - 4);
        let Ok(ToApp::WindowTouch { mods, .. }) = ToApp::decode(&old) else { panic!() };
        assert!(mods.is_empty());
    }
}
