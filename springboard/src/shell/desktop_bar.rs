//! Desktop menus backed by actual app, network and power commands.
use super::Shell;
use super::helpbook;
use super::ui::Weight;
#[cfg(feature = "desktop-wayland")]
use super::hwkeys::TermKey;
#[cfg(feature = "desktop-wayland")]
use super::{AppId,Screen,Effect,Event,desktop_network,ui::{WHITE,SECONDARY,CELL,LABEL,MENU_TINT,MENU_SOLID,MENU_RADIUS_PT}};
#[cfg(feature = "desktop-wayland")]
use crate::gfx::{Image,Surface};
/// The overlay surface (the whole screen) the system sheets and alerts are drawn into on the desktop (`Shell::modal_layer`).
#[cfg(feature = "desktop-wayland")]
pub(super) const MODAL_CHROME_ID:u32=15;
/// Width of a menu extra's button, in pt (Golden Gate spaces menu extras about 8 pt apart).
#[cfg(feature = "desktop-wayland")]
const STATUS_ICON_PT:f32=30.;
#[cfg(feature = "desktop-wayland")]
const STATUS_PAD_PT:f32=10.;
/// Width of the leading system-menu button (the BifrOSt mark), in pt.
#[cfg(feature = "desktop-wayland")]
const SYSTEM_MENU_PT:f32=34.;
/// How far (pt) a status glyph's ink centre sits below its drawing origin: battery 0, plug 1, ethernet 1.5, Wi-Fi 1.
#[cfg(feature = "desktop-wayland")]
fn status_icon_lift(battery:bool,ethernet:bool)->f32 {if battery{0.}else if ethernet{1.5}else{1.}}

// Only the desktop-wayland bar builds these; other builds never construct them.
#[cfg_attr(not(feature = "desktop-wayland"), allow(dead_code))]
#[derive(Clone,Copy,PartialEq,Eq,Debug)] pub(super) enum Menu {
    /// the leading system menu (the menu equivalent), drawn as the BifrOSt mark
    System,
    /// the system menu's Recent Items submenu: it opens in place of the system menu, under the same bar item
    Recent,
    App,File,Edit,View,Window,Help,Network,Power,
    /// Photos' Image menu: shown on the bar only while Photos is in front
    Image,
    /// the Bluetooth menu extra: the radio, the paired and connected devices, and the Bluetooth pane
    Bluetooth,
    /// the recording stop item, shown while a screen recording runs: a red dot whose menu has Stop Recording
    Recording,
    /// the Script menu extra: the scripts in the bifrost scripts folder, shown while the folder holds one (`scripts_menu`)
    Scripts,
    /// the location arrow: shown while an app uses location (`services::location::in_use_now`); its menu lists the apps
    Location,
    /// Control Center menu extra: a press opens the Control Center sheet, it has no rows
    ControlCenter,
    /// CPU and RAM details; no longer placed on the bar (Golden Gate shows no readouts), kept for their rows
    Cpu,Memory,Dock,Context,
    /// the n-th menu the frontmost SDK app published (`ToShell::AppMenus`)
    Custom(u8),
    /// the n-th menu extra an app published (`NativeStatusItem`), in the order of `status_entries_now`
    Status(u8),
    /// the input source menu extra: the active source's short label, its rows the enabled sources (shown with two or more)
    Input,
    /// the Now Playing extra: shown while an app has a session, its menu is `now_playing_menu::panel`
    NowPlaying,
    /// the Game Mode extra: shown while Game Mode is on; its one row turns it off for this game (`gamemode`)
    GameMode,
    /// the open or save panel (`docpanel`): drawn and hit-tested by the panel, not as rows
    Document,
    /// the window menu under a window's green button (`desk::Desk::arrange`): Fill, Centre, Move & Resize, Full Screen
    Arrange}
#[cfg(feature = "desktop-wayland")]
#[derive(Clone)] pub(super) enum Action {Submenu(Menu),Window(super::utility_window::Kind),Script(u8),Foreign(u32),Settings(&'static str),Desktop,Arrange(super::AppId,super::desk::Arrange),Close,Quit,Minimize,Zoom,ShowAll,Open(super::AppId),Context(u8,u32),QuickAction(u8,u8),AppMenu(u8,u32),Dock(super::desktop_dock::Change),DockMenu(u8,u32),Shortcut(u16),Command(bifrost_proto::Command),Term(super::hwkeys::TermKey),Wifi(bool),Scan,Select(String),Power(String),Lock,Sleep,Halt(super::power_menu::PowerAction),StatusItem(u8,u32,u32),Tile(super::desk::Tile),RestoreSize,CycleWindow,Print,InputSource(u8),InputMethod(String),Recent(u8),ClearRecent,CharacterViewer,KeyboardViewer,OnScreenKeyboard,Media(bifrost_ui::media_session::RemoteCommand),Bluetooth(crate::bare::bluetooth::Cmd),Portal(String),RecordStop,Service(u8),Photo(super::photos::commands::PhotoCommand),PhotoEditWith(String),GameModeOff,LowPowerMode(bool),HelpItem(Menu,usize),HelpTopic(String,Option<String>),Help(String)}
// Only the desktop-wayland bar reads these fields; other builds never build the menu.
#[cfg_attr(not(feature = "desktop-wayland"), allow(dead_code))]
pub(super) struct ContextMenu {pub slot:u8,pub x:f32,pub y:f32,pub items:Vec<bifrost_proto::ContextItem>}
#[cfg(feature = "desktop-wayland")]
pub(super) struct Row {pub(super) text:String,pub(super) action:Option<Action>,sep:bool,key:Option<String>,symbol:Option<crate::symbols::Symbol>,indent:bool,search:bool,header:bool}
/// How an app's menu extra (`NativeStatusItem`) shows on the bar.
#[cfg(any(test, feature = "desktop-wayland"))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum StatusFace {
    /// the app's own title
    Text(String),
    /// the name of the symbol it registered, one the bar can draw
    Icon(String),
}
/// The symbols a menu extra can be drawn with: the shell's symbol set and the hand-drawn glyphs of `draw_status_symbol`.
#[cfg(any(test, feature = "desktop-wayland"))]
fn status_symbol_known(name:&str)->bool {matches!(name,"square.and.pencil"|"checklist")||crate::symbols::Symbol::ALL.iter().any(|s|s.name()==name)}
/// What the bar draws for an app's menu extra: its title when it has one, else its symbol when the bar can draw it, else
/// nothing. An item is never stood in for by a letter of the app's name.
#[cfg(any(test, feature = "desktop-wayland"))]
pub(super) fn status_face(title:&str,symbol:&str)->Option<StatusFace> {
    if !title.is_empty() {Some(StatusFace::Text(title.to_string()))}
    else if status_symbol_known(symbol) {Some(StatusFace::Icon(symbol.to_string()))}
    else {None}
}
/// Row, separator and edge padding of a bar menu, in pt (the HIG groups menu items with separators).
#[cfg(feature = "desktop-wayland")]
const MENU_ROW_PT:f32=24.;
#[cfg(feature = "desktop-wayland")]
const MENU_SEP_PT:f32=11.;
#[cfg(feature = "desktop-wayland")]
const MENU_PAD_PT:f32=5.;
/// Height of the Help menu's search field, in pt.
#[cfg(feature = "desktop-wayland")]
const HELP_FIELD_PT:f32=30.;
/// The gap between the menu bar and an open menu (Golden Gate).
#[cfg(feature = "desktop-wayland")]
const MENU_GAP_PT:f32=6.;
/// A shortcut as Golden Gate menus show it: modifier glyphs, a space, then the key ("⌘ ,", "⇧⌘ Z").
#[cfg(feature = "desktop-wayland")]
fn shortcut_spaced(k:&str)->String {let mut c:Vec<char>=k.chars().collect();match c.pop(){Some(last) if !c.is_empty()=>format!("{} {last}",c.into_iter().collect::<String>()),_=>k.to_string()}}
/// The screen rectangle (x, y, w, h in px) an open desktop popup owes a repaint: the popup and its shadow
/// `reach` px out on the sides and bottom, and `reach` px up on top, clamped to the bottom of the menu bar
/// (the bar repaints its own strip). The shadow above the popup lies in the gap under the bar, so a
/// rectangle that starts at the popup's top edge would leave a thin shadow line behind when the menu closes.
#[cfg(any(test, feature = "desktop-wayland"))]
pub(super) fn popup_damage_rect(popup:(usize,usize,usize,usize),bar_h:i32,reach:i32,lip:i32)->(i32,i32,i32,i32) {
    let (x,y,w,h)=(popup.0 as i32,popup.1 as i32,popup.2 as i32,popup.3 as i32);
    let left=(x-reach).max(0);let right=x+w+reach;
    let top=(y-reach).max(bar_h);let bottom=y+h+reach+lip;
    (left,top,right-left,bottom-top)
}
/// The chrome surface an open popup is exported as under the window server, and the popup inside it. The surface is the
/// popup grown by its shadow (`popup_damage_rect`) and clipped to the `screen` (w, h), so the shadow is not cut off at
/// the popup's edge. The array is the popup's own rectangle (x, y, w, h) in surface px: the only part of the surface that
/// takes the pointer, so a click on the shadow reaches what is below, as it did when the surface was the popup's size.
#[cfg(any(test, feature = "desktop-wayland"))]
pub(super) fn popup_surface_rect(popup:(usize,usize,usize,usize),bar_h:i32,reach:i32,lip:i32,screen:(usize,usize))->((usize,usize,usize,usize),[i32;4]) {
    let (x,y,w,h)=popup_damage_rect(popup,bar_h,reach,lip);
    let (x,y)=(x.max(0),y.max(0));
    let right=(x+w).min(screen.0 as i32);let bottom=(y+h).min(screen.1 as i32);
    ((x as usize,y as usize,(right-x).max(0) as usize,(bottom-y).max(0) as usize),[popup.0 as i32-x,popup.1 as i32-y,popup.2 as i32,popup.3 as i32])
}
/// Where a context menu of `size` (w, h in px) opens for a pointer at `at`, inside `area` (x, y, w, h in px: the screen
/// below the menu bar and above the Dock). Its top-left corner is the pointer. A menu that would pass the right edge of
/// the area opens to the left of the pointer instead, and one that would pass the bottom edge opens above it; a menu
/// that fits on neither side is held inside the area, top-left first.
#[cfg(any(test, feature = "desktop-wayland"))]
pub(super) fn context_menu_origin(at:(f32,f32),size:(usize,usize),area:(i32,i32,i32,i32))->(usize,usize) {
    let (ax,ay,aw,ah)=area;
    // one axis: the pointer's coordinate `p`, the menu's length `len`, the area's span `lo..hi`
    let fit=|p:i32,len:i32,lo:i32,hi:i32|{let p=if p+len>hi {p-len} else {p};p.min(hi-len).max(lo)};
    let x=fit(at.0 as i32,size.0 as i32,ax,ax+aw);
    let y=fit(at.1 as i32,size.1 as i32,ay,ay+ah);
    (x.max(0) as usize,y.max(0) as usize)
}
/// Top and height in pt (from the popup's top edge) of each row, and the popup's total height.
#[cfg(feature = "desktop-wayland")]
pub(super) fn menu_spans(rows:&[Row])->(Vec<(f32,f32)>,f32) {let mut y=MENU_PAD_PT;let v=rows.iter().map(|r|{let h=if r.sep{MENU_SEP_PT}else if r.search{HELP_FIELD_PT}else{MENU_ROW_PT};let s=(y,h);y+=h;s}).collect();(v,y+MENU_PAD_PT)}
/// Key equivalents the shell really handles (`NativeMenuItem.keyEquivalent`), as `bifrost_ui::keys` combos; the menus show them
/// in Golden Gate notation and `menu_key_equivalents_parse_and_do_not_conflict` checks the table.
#[cfg(feature = "desktop-wayland")]
pub(super) const MENU_KEYS:&[(&str,&str)]=&[("Close Window","cmd+w"),("Minimize","cmd+m"),("Minimize Window","cmd+m"),("Undo","cmd+z"),("Redo","cmd+shift+z"),("Cut","cmd+x"),("Copy","cmd+c"),("Paste","cmd+v"),("Select All","cmd+a"),("Quit","cmd+q"),("Lock Screen","ctrl+cmd+q"),("Force Quit\u{2026}","alt+cmd+escape"),("Log Out","shift+cmd+q"),("Settings…","cmd+,"),("Search","cmd+space"),("App Switcher","cmd+tab"),("Back","escape"),("Print\u{2026}","cmd+p")];
#[cfg(feature = "desktop-wayland")]
impl Row {fn indented(mut self)->Self {self.indent=true;self} fn symbol(mut self,sym:crate::symbols::Symbol)->Self {self.symbol=Some(sym);self} fn key(mut self,combo:&str)->Self {self.key=bifrost_ui::keys::KeyCombo::parse(combo).map(|c|c.to_string());self} fn separator()->Self {Self{text:String::new(),action:None,sep:true,key:None,symbol:None,indent:false,search:false,header:false}} fn action(text:impl Into<String>,action:Action)->Self {Self{text:text.into(),action:Some(action),sep:false,key:None,symbol:None,indent:false,search:false,header:false}} fn label(text:impl Into<String>)->Self {Self{text:text.into(),action:None,sep:false,key:None,symbol:None,indent:false,search:false,header:false}}}
#[cfg(feature = "desktop-wayland")]
impl Row {fn search(text: impl Into<String>)->Self {Self{text:text.into(),action:None,sep:false,key:None,symbol:None,indent:false,search:true,header:false}} fn header(text: impl Into<String>)->Self {Self{text:text.into(),action:None,sep:false,key:None,symbol:None,indent:false,search:false,header:true}}}
/// One row of a Photos menu: its title (with a check mark for a setting that is on), its key equivalent, and its
/// action when the command can run now (greyed out otherwise).
#[cfg(feature = "desktop-wayland")]
fn photo_row(p:&super::photos::Photos,c:super::photos::commands::PhotoCommand)->Row{
    let title=p.command_title(c);
    let text=if c.checkable(){format!("{} {}",if p.command_checked(c){"\u{2713}"}else{" "},title)}else{title};
    let mut r=if p.command_enabled(c){Row::action(text,Action::Photo(c))}else{Row::label(text)};
    if let Some(k)=c.key_text(){r=r.key(k);}
    r
}
/// Accessors the chrome menu bar model reads (chrome_host/menubar.rs): the separator flag and the shortcut text.
#[cfg(feature = "desktop-wayland")]
impl Row {pub(super) fn is_separator(&self)->bool {self.sep} pub(super) fn key_text(&self)->Option<&str> {self.key.as_deref()}}
/// Rows of one app-published menu (`ToShell::AppMenus`): disabled items and submenu headers are labels, a checked item
/// starts with a check mark, a submenu's children are indented. Invalid key equivalents are dropped.
#[cfg(feature = "desktop-wayland")]
pub(super) fn app_menu_rows(slot:u8,menu:&bifrost_proto::AppMenu)->Vec<Row> {
    menu.items.iter().map(|i|{
        if i.separator {return Row::separator();}
        let text=format!("{}{}{}",if i.depth>0{"    "}else{""},if i.checked{"\u{2713} "}else{""},i.title);
        let row=if i.enabled&&!i.submenu&&i.id!=0 {Row::action(text,Action::AppMenu(slot,i.id))}else{Row::label(text)};
        if i.key.is_empty(){row}else{row.key(&i.key)}
    }).collect()
}
/// Rows of an app's context menu (`ToShell::ContextMenu`): a separator row for `ContextItem::separator`, a disabled item as a
/// label (secondary ink), an enabled one as a choice that goes back to the app as `ToApp::ContextAction`, and a key
/// equivalent right-aligned. A separator at either end or next to another is dropped, so an app may build its groups
/// without checking which are empty; an invalid key equivalent is left out (as in `app_menu_rows`).
#[cfg(feature = "desktop-wayland")]
pub(super) fn context_rows(slot:u8,items:&[bifrost_proto::ContextItem])->Vec<Row> {
    let mut rows:Vec<Row>=Vec::new();
    for item in items {
        if item.is_separator() {
            if rows.last().is_some_and(|r|!r.sep) {rows.push(Row::separator());}
            continue;
        }
        let row=if item.enabled {Row::action(item.title(),Action::Context(slot,item.id))} else {Row::label(item.title())};
        rows.push(match item.key_combo() {Some(combo)=>row.key(combo),None=>row});
    }
    if rows.last().is_some_and(|r|r.sep) {rows.pop();}
    rows
}
/// The enabled item whose key equivalent is `ch` with the chord modifiers `mods` (`CHORD_*`), `NativeMenu.performKeyEquivalent:`.
pub(super) fn app_menu_chord(menus:&[bifrost_proto::AppMenu],ch:char,mods:u32)->Option<u32> {
    use bifrost_proto::{CHORD_ALT,CHORD_CTRL,CHORD_META,CHORD_SHIFT};
    let ch=ch.to_ascii_lowercase();
    menus.iter().flat_map(|m|m.items.iter()).filter(|i|i.enabled&&!i.separator&&!i.submenu&&i.id!=0).find(|i|{
        bifrost_ui::keys::KeyCombo::parse(&i.key).is_some_and(|c|c.key==bifrost_ui::keys::Key::Char(ch)&&c.cmd==(mods&CHORD_META!=0)&&c.ctrl==(mods&CHORD_CTRL!=0)&&c.alt==(mods&CHORD_ALT!=0)&&c.shift==(mods&CHORD_SHIFT!=0))
    }).map(|i|i.id)
}
/// Whether the key `code` with the chord modifiers `mods` is a key equivalent of an enabled item in `menus`. Key equivalents go to the
/// app in front first: the shell closes or minimizes a window with Cmd+W or Cmd+M only when the app does not take the chord (a tabbed
/// app's Close Tab keeps Cmd+W while it has a tab to close).
pub(super) fn chord_in_menus(menus:&[bifrost_proto::AppMenu],code:u16,mods:u32)->bool {
    super::evkeys::char_for(code,false).and_then(|c|app_menu_chord(menus,c,mods)).is_some()
}
/// What quitting (Cmd+Q, the App menu's Quit) does to the app in front. Cmd+W only ever closes the window.
#[cfg(feature = "desktop-wayland")]
#[derive(Clone,Debug,PartialEq,Eq)]
pub(super) enum QuitStep {
    /// an installed SDK app, by slot: its desk windows go and it is sent `Quit`, so it saves its state and exits (the host kills it
    /// after the grace period), the way the Dock's Quit ends it
    App(u8),
    /// the toplevels of a native or Linux app: each is closed through the window server, as the Dock's Quit does
    Windows(Vec<u32>),
    /// a built-in window (Settings, Terminal, ...): it closes, as it always did
    Window(AppId),
    /// nothing is in front
    Nothing,
}
/// The Cmd+Q decision, pure. `foreign` are the window server's windows as (toplevel id, app id, active); `sdk_slot` names the running
/// direct SDK app whose toplevels carry an app id; `front` is the front window of the shell's own desk. A window server window in
/// front wins over the desk's, as it does for Cmd+W (`foreign_active`). An extra window of an SDK app quits its app.
#[cfg(feature = "desktop-wayland")]
pub(super) fn quit_step(foreign:&[(u32,&str,bool)],sdk_slot:impl Fn(&str)->Option<u8>,front:Option<AppId>)->QuitStep {
    if let Some(&(_,app_id,_))=foreign.iter().find(|w|w.2) {
        return match sdk_slot(app_id) {
            Some(slot)=>QuitStep::App(slot),
            None=>QuitStep::Windows(foreign.iter().filter(|w|w.1==app_id).map(|w|w.0).collect()),
        };
    }
    match front.map(AppId::parent) {
        Some(AppId::External(slot))=>QuitStep::App(slot),
        Some(app)=>QuitStep::Window(app),
        None=>QuitStep::Nothing,
    }
}
/// Menus that sit right after Edit instead of after View (the document apps' Format menus).
#[cfg(feature = "desktop-wayland")]
fn follows_edit(title:&str)->bool {
    ["Format","Font","Text","Insert"].iter().any(|t|t.eq_ignore_ascii_case(title.trim()))
}
/// The bar after the app's name menu, left to right (Golden Gate): File, Edit, the document apps' Format menus, Image (Photos only),
/// View, the app's other own menus (History, Bookmarks, Develop of the Browser, ...) and last Window and Help. An app menu titled like
/// a shell menu merges into it, so only the others get a `Custom` button. The app's order is kept within each group.
#[cfg(feature = "desktop-wayland")]
pub(super) fn bar_after_app(app_menus:&[bifrost_proto::AppMenu],image:bool)->Vec<Menu> {
    let custom=|after_edit:bool|app_menus.iter().enumerate().filter(move|(_,m)|shell_menu(&m.title).is_none()&&follows_edit(&m.title)==after_edit).map(|(i,_)|Menu::Custom(i as u8)).collect::<Vec<Menu>>();
    let mut bar=vec![Menu::File,Menu::Edit];
    bar.extend(custom(true));
    if image{bar.push(Menu::Image);}
    bar.push(Menu::View);
    bar.extend(custom(false));
    bar.extend([Menu::Window,Menu::Help]);
    bar
}
/// Header plus the five largest app groups by `key`; empty when there are no app readings.
#[cfg(feature = "desktop-wayland")]
fn top_app_rows(apps:&[super::desktop_metrics::AppUsage],header:&str,key:impl Fn(&super::desktop_metrics::AppUsage)->u64,show:impl Fn(u64)->String)->Vec<Row> {
    let mut sorted:Vec<_>=apps.iter().collect();sorted.sort_by_key(|a|std::cmp::Reverse(key(a)));
    if sorted.is_empty() {return Vec::new();}
    let mut rows=vec![Row::label(header)];rows.extend(sorted.into_iter().take(5).map(|a|Row::label(format!("{}: {}",a.id,show(key(a))))));rows
}
/// The screen rectangles (x, y, w, h; the bar's full `height`, `pad` px more on each side, kept inside `[0, width]`) of
/// the bar items that differ between two layouts: an item whose label or place changed (old and new place both), one
/// that appeared or went. A reading that changes only the CPU label returns that label's rectangle, not the bar.
#[cfg(feature = "desktop-wayland")]
pub(super) fn changed_bar_rects(before:&[(Menu,f32,f32,String)],after:&[(Menu,f32,f32,String)],width:i32,height:i32,pad:i32)->Vec<(i32,i32,i32,i32)> {
    let width=width.max(0);
    let span=|left:f32,w:f32|->(i32,i32) {(((left.floor() as i32)-pad).clamp(0,width),(((left+w).ceil() as i32)+pad).clamp(0,width))};
    let mut spans:Vec<(i32,i32)>=Vec::new();
    for item in after {
        match before.iter().find(|b|b.0==item.0) {
            Some(old) if old.1==item.1&&old.2==item.2&&old.3==item.3=>{}
            Some(old)=>{spans.push(span(old.1,old.2));spans.push(span(item.1,item.2));}
            None=>spans.push(span(item.1,item.2)),
        }
    }
    for old in before {
        if !after.iter().any(|a|a.0==old.0) {spans.push(span(old.1,old.2));}
    }
    spans.retain(|s|s.1>s.0);
    spans.sort();
    // overlapping or touching spans are one rectangle
    let mut merged:Vec<(i32,i32)>=Vec::new();
    for s in spans {
        if let Some(last)=merged.last_mut() {
            if s.0<=last.1 {last.1=last.1.max(s.1);continue;}
        }
        merged.push(s);
    }
    merged.into_iter().map(|(a,b)|(a,0,b-a,height)).collect()
}
impl Shell {
    /// The Window menu's name for an open window: an extra window's title from its app's table, else the app's name.
    pub(super) fn window_menu_name(&self, app: super::AppId) -> String {
        if let super::AppId::ExternalWindow(slot, id) = app {
            if let Some(Some(h)) = self.ext.get(slot as usize) {
                if let Some(w) = h.windows.of_app(&h.id).into_iter().find(|w| w.id == u32::from(id)) {
                    return w.title.clone();
                }
            }
        }
        super::apps().iter().find(|a| a.id == app.parent()).map_or("Window", |a| a.name).to_string()
    }

    pub(super) fn desktop_bar_height(&self)->usize {self.ui.pt(28.).ceil() as usize}
    /// Advance width of a bar label (`Ui::measure`), from the chrome text cache: the bar lays its menus out on every frame.
    pub(super) fn measure_chrome(&self,text:&str,pt:f32,weight:Weight)->f32 {self.chrome_text.borrow_mut().width(&self.ui,text,pt,weight)}
    /// Without the desktop there is no menu bar, so nothing shows CPU or RAM readings.
    #[cfg(not(feature="desktop-wayland"))]
    pub(super) fn metrics_wanted(&self)->bool {false}
    #[cfg(not(feature="desktop-wayland"))]
    pub(super) fn metrics_menu_open(&self)->bool {false}
    /// Without the desktop's chrome text cache, a label is fitted by the shared text measure.
    #[cfg(not(feature="desktop-wayland"))]
    pub(super) fn fit_chrome(&self,text:&str,pt:f32,max_px:f32)->String {self.ui.fit(text,pt,max_px)}
    /// Photos' commands come from the desktop menus and its key equivalents, which exist only with the desktop.
    #[cfg(not(feature="desktop-wayland"))]
    pub(super) fn photo_command(&mut self,command:super::photos::commands::PhotoCommand,t:f64)->Vec<super::Effect>{
        if let Some(p)=self.photos.as_mut(){p.command(&self.ui,command,t);let _=p.take_print();}
        self.dirty=true;
        let _=t;
        Vec::new()
    }
}
#[cfg(feature="desktop-wayland")]
impl Shell {
    /// A bar label fitted into `max_px` (`Ui::fit`), from the chrome text cache.
    pub(super) fn fit_chrome(&self,text:&str,pt:f32,max_px:f32)->String {self.chrome_text.borrow_mut().fit(&self.ui,text,pt,max_px)}
    fn foreign_active(&self)->Option<u32> {
        self.desk.as_ref().and_then(|d|d.native_windows.iter().chain(d.linux_apps.iter().flat_map(|a|a.windows.iter())).find(|w|w.active).map(|w|w.id))
    }
    fn sdk_commands(&self)->Option<bifrost_proto::Commands> {
        if self.foreign_active().is_some(){return None;}
        match self.screen {Screen::App(AppId::External(n))=>self.ext.get(n as usize).and_then(Option::as_ref).map(|h|h.commands),_=>None}
    }
    /// The Undo and Redo titles the frontmost SDK app reported (`undoMenuItemTitle`), when it has any.
    fn sdk_undo_titles(&self)->Option<(String,String)> {
        if self.foreign_active().is_some(){return None;}
        match self.screen {Screen::App(AppId::External(n))=>self.ext.get(n as usize).and_then(Option::as_ref).map(|h|h.undo_titles.clone()),_=>None}
    }
    /// The menus the frontmost SDK app published, empty for shell apps and foreign windows.
    pub(super) fn app_menus(&self)->&[bifrost_proto::AppMenu] {
        if self.foreign_active().is_some(){return &[];}
        match self.screen {Screen::App(AppId::External(n))=>self.ext.get(n as usize).and_then(Option::as_ref).map_or(&[][..],|h|&h.app_menus[..]),_=>&[]}
    }
    /// The name of the app whose menus the bar shows: the foreign app in front, else the built-in app on screen, else BifrOSt.
    fn app_menu_name(&self)->String {
        self.foreign_active().and_then(|id|self.desk.as_ref().and_then(|d|d.native_windows.iter().find(|w|w.id==id))).and_then(|w|super::apphost::installed().iter().find(|a|a.manifest.wayland_app_id==w.app_id).map(|a|a.manifest.name.clone())).unwrap_or_else(||match self.screen {Screen::App(app)=>super::apps().iter().find(|a|a.id==app).map(|a|a.name).unwrap_or("BifrOSt").into(),_=>"BifrOSt".into()})
    }
    /// Settings › Menu Bar: whether a menu extra is turned on (the clock always shows).
    pub(super) fn menu_extra_shown(&self,menu:Menu)->bool {
        use super::options::MenuExtra as E;
        // the input menu shows when there is a choice of keyboards or input-method modes, while the on-screen keyboard is open,
        // or when the user turned on Settings > Keyboard > Input Sources > Show Input menu in menu bar
        if menu==Menu::Input {return input_menu_visible(super::kb_layouts::enabled().len(),self.keyboard_opened_explicitly(),self.options().input_menu)||self.input_providers.has_modes();}
        if menu==Menu::Scripts {return super::scripts_menu::folder().is_some_and(|d|!super::scripts_menu::list(&d).is_empty());}
        // a desktop without a battery has nothing to show for it: no battery and no power plug in the bar
        if menu==Menu::Power&&self.info.desktop_network.is_some()&&!self.info.battery_present {return false;}
        let e=match menu {Menu::Cpu=>E::Cpu,Menu::Memory=>E::Memory,Menu::Power=>E::Battery,Menu::Network=>E::Network,Menu::ControlCenter=>E::ControlCenter,_=>return true};
        self.options().menu_extras&e.bit()!=0
    }
    /// The percentage the battery menu bar item shows beside its glyph: only when the Control Center pref asks for it
    /// (`BATTERY_PERCENT_PREF`) and the machine has a battery.
    pub(super) fn battery_bar_label(&self)->Option<String> {
        (self.info.battery_present && super::control_center::battery_percent_shown(self.prefs.get(super::control_center::BATTERY_PERCENT_PREF))).then(|| format!("{}%",self.info.battery))
    }
    /// A CPU or RAM menu is open: it prints the full reading.
    pub(super) fn metrics_menu_open(&self)->bool {matches!(self.desktop_menu,Some(Menu::Cpu|Menu::Memory))}
    /// Whether anything on screen shows the CPU and RAM readings: the bar readouts or their menus. Otherwise nothing samples or wakes.
    pub(super) fn metrics_wanted(&self)->bool {self.metrics_menu_open()||self.menu_extra_shown(Menu::Cpu)||self.menu_extra_shown(Menu::Memory)}
    pub(super) fn menu_buttons(&self)->Vec<(Menu,f32,f32,String)> {
        let mut buttons=vec![(Menu::System,self.ui.pt(6.),self.ui.pt(SYSTEM_MENU_PT),String::from("system"))];let mut x=self.ui.pt(6.+SYSTEM_MENU_PT);
        let name=self.app_menu_name();
        let name=self.fit_chrome(&name,13.,self.ui.pt(160.));
        // the order of the Golden Gate menu bar: App, File, Edit, View, then the app's own menus (the Browser's History, Bookmarks and
        // Develop), Window, and Help at the trailing end; the document apps' Format menus sit right after Edit (`bar_after_app`)
        let mut bar:Vec<(Menu,String)>=vec![(Menu::App,name.clone())];
        let app_menus=self.app_menus();
        bar.extend(bar_after_app(app_menus,self.photos_front()).into_iter().map(|m|{
            let label=match m {
                Menu::Custom(i)=>app_menus.get(i as usize).map(|am|self.fit_chrome(&am.title,13.,self.ui.pt(120.))).unwrap_or_default(),
                Menu::File=>"File".to_string(),
                Menu::Edit=>"Edit".to_string(),
                Menu::Image=>"Image".to_string(),
                Menu::View=>"View".to_string(),
                Menu::Window=>"Window".to_string(),
                _=>"Help".to_string(),
            };
            (m,label)
        }));
        for (kind,label) in bar.iter().map(|(k,l)|(*k,l.as_str())) {
            let width=self.measure_chrome(label,13.,Weight::Semibold)+self.ui.pt(24.);buttons.push((kind,x,width,label.into()));x+=width;
        }
        let network=self.info.desktop_network.as_ref();let wired=network.is_some_and(|n|n.interfaces.iter().any(|i|i.kind=="ethernet"&&i.connected()));
        let nw=self.ui.pt(STATUS_ICON_PT);let pw=self.ui.pt(STATUS_ICON_PT)+self.battery_bar_label().map_or(0.,|t|self.measure_chrome(&t,12.,Weight::Regular)+self.ui.pt(4.));let clock=self.measure_chrome(&self.clock_text(),12.,Weight::Medium);let right=self.w as f32-self.ui.pt(16.)-clock-self.ui.pt(2.);
        // Golden Gate menu extras, placed from the clock leftwards: Control Center, Wi-Fi or ethernet, battery or power; an item that would run into the app menus is left out
        let limit=x+self.ui.pt(8.);let mut edge=right;let mut tail=Vec::new();
        // the owner's CPU and RAM readouts sit left of the system extras (like iStat Menus), dropped first when space runs out
        let cpu=format!("CPU {:.0}%",self.desktop_metrics.cpu);let memory=format!("RAM {:.0}%",self.desktop_metrics.memory_percent());
        let cw=self.measure_chrome(&cpu,12.,Weight::Regular)+self.ui.pt(2.*STATUS_PAD_PT);let mw=self.measure_chrome(&memory,12.,Weight::Regular)+self.ui.pt(2.*STATUS_PAD_PT);
        // the input source sits left of Control Center (the extras are placed from the clock leftwards)
        let source_label=self.input_providers.label().unwrap_or_else(||super::kb_layouts::active().short_name().to_string());let source=source_label.as_str();let input=self.measure_chrome(source,12.,Weight::Regular)+self.ui.pt(2.*STATUS_PAD_PT);
        let mut extras=vec![(Menu::ControlCenter,self.ui.pt(STATUS_ICON_PT),String::from("control")),(Menu::Input,input,source.to_string()),(Menu::Network,nw,if wired {"ethernet"}else{"wifi"}.into()),(Menu::Power,pw,String::from("power")),(Menu::Memory,mw,memory),(Menu::Cpu,cw,cpu)];
        // the Script menu extra sits left of the input source; `menu_extra_shown` hides it while the folder is empty
        let script_label=String::from("Script");let script_w=self.measure_chrome(&script_label,12.,Weight::Regular)+self.ui.pt(2.*STATUS_PAD_PT);extras.insert(1,(Menu::Scripts,script_w,script_label));
        // the recording stop item sits left of Control Center while a screen recording runs
        if self.recording_active() {extras.insert(1,(Menu::Recording,self.ui.pt(STATUS_ICON_PT),String::from("stop")));}
        // the location arrow sits left of the other system extras while an app uses location
        if !super::services::location::in_use_now().is_empty() {extras.insert(1,(Menu::Location,self.ui.pt(STATUS_ICON_PT),String::from("location")));}
        // the Bluetooth extra sits left of Wi-Fi, once the radio exists (it stays hidden without a controller)
        if self.info.bt.state!=crate::bare::bluetooth::State::Unavailable {extras.insert(2,(Menu::Bluetooth,self.ui.pt(STATUS_ICON_PT),String::from("bluetooth")));}
        // the Now Playing extra sits left of the system extras while an app has a session, and right of the app extras
        if self.game_mode_on(){let label=String::from("Game Mode");let w=self.measure_chrome(&label,12.,Weight::Regular)+self.ui.pt(2.*STATUS_PAD_PT);extras.push((Menu::GameMode,w,label));}
        if let Some(p)=super::now_playing_menu::panel(){let label=self.fit_chrome(&p.label,12.,self.ui.pt(160.));let w=self.measure_chrome(&label,12.,Weight::Regular)+self.ui.pt(2.*STATUS_PAD_PT);extras.push((Menu::NowPlaying,w,label));}
        // menu extras of running apps (`NativeStatusItem`) sit left of the system extras: the app's title, or its symbol when it
        // registered a symbol the bar draws; an item with neither is not shown (no letter stands in for it)
        for (i,e) in self.status_entries_now().iter().enumerate() {match status_face(&e.title,&e.symbol){
            Some(StatusFace::Text(label))=>{let w=self.measure_chrome(&label,12.,Weight::Regular)+self.ui.pt(2.*STATUS_PAD_PT);extras.push((Menu::Status(i as u8),w,label));},
            Some(StatusFace::Icon(name))=>extras.push((Menu::Status(i as u8),self.ui.pt(STATUS_ICON_PT),name)),
            None=>{},
        }}
        for (menu,width,label) in extras {
            if !self.menu_extra_shown(menu){continue;}
            // the recording indicator is never left out for lack of room: a running screen recording is always shown
            if edge-width<limit {if menu==Menu::Recording {edge-=width;tail.push((menu,edge,width,label));continue;} break;}
            edge-=width;tail.push((menu,edge,width,label));
        }
        // when the first extra (Control Center) already ran out of room the loop ended before the indicator: put it in
        if self.recording_active()&&!tail.iter().any(|t|t.0==Menu::Recording) {let w=self.ui.pt(STATUS_ICON_PT);edge-=w;tail.push((Menu::Recording,edge,w,String::from("stop")));}
        tail.reverse();buttons.extend(tail);buttons

    }
    fn draw_desktop_status_icon(&self,s:&mut Surface,menu:Menu,label:&str,x:f32,y:f32) {
        // legibility shadow (or halo) over the wallpaper, then the glyph
        let ink=self.bar_ink();
        self.draw_status_glyph(s,menu,label,x,y+self.ui.pt(0.5),crate::gfx::scale(ink.shadow,ink.shadow_alpha));self.draw_status_glyph(s,menu,label,x,y,crate::gfx::scale(ink.text,ink.text_alpha));
    }
    /// The menu bar's ink: white with a dark shadow, or near-black with a light halo over a light wallpaper.
    pub(super) fn bar_ink(&self)->BarInk {
        use super::ui::*;
        if self.bar_on_light {BarInk{text:BAR_TEXT_ON_LIGHT,text_alpha:BAR_TEXT_ON_LIGHT_ALPHA,shadow:BAR_TEXT_HALO,shadow_alpha:BAR_TEXT_HALO_ALPHA,capsule:BAR_OPEN_CAPSULE_ON_LIGHT}}
        else {BarInk{text:BAR_TEXT,text_alpha:255,shadow:BAR_TEXT_SHADOW,shadow_alpha:BAR_TEXT_SHADOW_ALPHA,capsule:BAR_OPEN_CAPSULE}}
    }
    /// Whether the bar item for `menu` is an app's menu extra drawn as a symbol (its title is empty), not as text.
    pub(super) fn status_item_is_icon(&self,menu:Menu)->bool {
        let Menu::Status(i)=menu else{return false;};
        self.status_entries_now().get(i as usize).is_some_and(|e|matches!(status_face(&e.title,&e.symbol),Some(StatusFace::Icon(_))))
    }
    /// An app's menu extra symbol, centred on (`x`, `y`): a shell symbol when it has one of that name, else a hand-drawn glyph.
    fn draw_status_symbol(&self,s:&mut Surface,name:&str,x:f32,y:f32,ink:u32) {
        let p=self.ui.pt(1.);
        if let Some(sym)=crate::symbols::Symbol::ALL.iter().copied().find(|sym|sym.name()==name) {
            if let Some(im)=self.ui.symbols.render(sym,self.ui.pt(15.),ink) {s.blit(&im,(x-im.w as f32/2.) as i32,(y-im.h as f32/2.) as i32,255,255);}
            return;
        }
        match name {
            // a square open at its top right, with a pencil stroke across the gap
            "square.and.pencil"=>{
                s.stroke_rrect(x-8.*p,y-6.*p,13.*p,13.*p,3.*p,1.2*p,ink);
                for k in 0..=8 {let f=k as f32/8.;s.fill_rrect(x-2.*p+f*9.*p,y+1.*p-f*9.*p,2.*p,2.*p,p,ink);}
            },
            // three list rows: a dot and a line each
            "checklist"=>for k in 0..3 {let ry=y+(k as f32*5.-6.)*p;s.fill_rrect(x-8.*p,ry,3.*p,3.*p,1.5*p,ink);s.fill_rrect(x-3.*p,ry+0.5*p,11.*p,2.*p,p,ink);},
            _=>{},
        }
    }
    fn draw_status_glyph(&self,s:&mut Surface,menu:Menu,label:&str,x:f32,y:f32,ink:u32) {
        let p=self.ui.pt(1.);
        // an app's menu extra drawn as its symbol (the label is the symbol's name, see `status_face`)
        if matches!(menu,Menu::Status(_)) {self.draw_status_symbol(s,label,x,y,ink);return;}
        // the glyphs are drawn from different origins; lift each so its ink centres on the bar like the text beside it
        let y=y-p*if matches!(menu,Menu::System|Menu::ControlCenter){0.}else{status_icon_lift(menu==Menu::Power&&self.info.battery_present,label=="ethernet")};let rect=|s:&mut Surface,dx:f32,dy:f32,w:f32,h:f32|s.fill_rrect(x+dx*p,y+dy*p,w*p,h*p,p,ink);
        // the recording stop item: a red dot, as the recording indicator
        if menu==Menu::Recording {s.fill_rrect(x-4.*p,y-4.*p,8.*p,8.*p,4.*p,super::ui::SYSTEM_RED);return;}
        // the location arrow: a north-pointing triangle, drawn as ten rows that widen from the tip
        if menu==Menu::Location {for i in 0..10 {let w=1.+i as f32*0.9;rect(s,4.5-w/2.,i as f32,w,1.);}return;}
        if menu==Menu::System {
            // the BifrOSt mark: a ring (the sky) over the bridge deck
            s.stroke_rrect(x-6.*p,y-7.*p,12.*p,12.*p,6.*p,1.5*p,ink);rect(s,-8.,6.,16.,2.);return;
        }
        if menu==Menu::ControlCenter {
            // two toggle capsules, as the Golden Gate Control Center extra
            for dy in [-5.,1.] {s.stroke_rrect(x-8.*p,y+dy*p,16.*p,6.*p,3.*p,1.2*p,ink);}
            s.fill_rrect(x+2.*p,y-4.*p,4.*p,4.*p,2.*p,ink);s.fill_rrect(x-6.*p,y+2.*p,4.*p,4.*p,2.*p,ink);
            // the input indicator: an orange dot while any input stream or tap runs (audio policy)
            if super::sound_page::input_active() {s.fill_rrect(x+6.*p,y-9.*p,5.*p,5.*p,2.5*p,super::ui::SYSTEM_ORANGE);}
            return;
        }
        if menu==Menu::Power {if self.info.battery_present {s.stroke_rrect(x-9.*p,y-5.*p,17.*p,10.*p,2.*p,p,ink);rect(s,9.,-2.,2.,4.);rect(s,-7.,-3.,13.*self.info.battery as f32/100.,6.);}else{rect(s,-5.,-2.,10.,6.);rect(s,-4.,-7.,2.,5.);rect(s,2.,-7.,2.,5.);rect(s,-1.,4.,2.,5.);}}else if menu==Menu::Bluetooth {
            // the Bluetooth rune: a stem and two chevrons, drawn as dots along each stroke
            let strokes:[((f32,f32),(f32,f32));4]=[((0.,-8.),(0.,8.)),((0.,-8.),(5.,-3.)),((5.,-3.),(-5.,3.)),((-5.,3.),(0.,8.))];
            for ((ax,ay),(bx,by)) in strokes {for k in 0..=8 {let f=k as f32/8.;rect(s,ax+(bx-ax)*f-1.,ay+(by-ay)*f-1.,2.,2.);}}
        }else if label=="ethernet" {s.stroke_rrect(x-7.*p,y-7.*p,14.*p,10.*p,p,p,ink);rect(s,-1.,3.,2.,4.);rect(s,-9.,6.,18.,2.);for dx in [-9.,-1.,7.] {rect(s,dx,6.,2.,4.);}}else{for i in 0..3 {let w=(16-i*5) as f32;rect(s,-w/2.,-6.+i as f32*4.,w,2.);}rect(s,-1.,6.,2.,2.);}
    }
    /// The rows of a bar menu as drawn and navigated. The Help menu starts with its search field; typing replaces
    /// its items with the grouped results.
    pub(super) fn menu_rows(&self,menu:Menu)->Vec<Row> {if menu==Menu::Help {self.help_rows()} else {self.plain_rows(menu)}}
    /// The rows of a bar menu without the Help search. The Help search also searches these.
    pub(super) fn plain_rows(&self,menu:Menu)->Vec<Row> {
        let mut rows=self.own_menu_rows(menu);
        if self.photos_front(){rows=self.photo_overlay(menu,rows);}
        if let Screen::App(AppId::External(slot))=self.screen {
            let merged:Vec<Row>=self.app_menus().iter().filter(|m|shell_menu(&m.title)==Some(menu)).flat_map(|m|app_menu_rows(slot,m)).collect();
            if !merged.is_empty() {if !rows.is_empty(){rows.insert(0,Row::separator());}for r in merged.into_iter().rev(){rows.insert(0,r);}}
        }
        rows
    }
    /// The Help menu's rows: the search field, then the menu's own items, or while a query is typed the grouped results.
    fn help_rows(&self)->Vec<Row> {
        let query=self.help_search.query.clone();
        let mut rows=vec![Row::search(query.clone())];
        if query.trim().is_empty() {rows.extend(self.plain_rows(Menu::Help));return rows;}
        let entries=self.help_entries();
        let hits:Vec<usize>=help_hits(&entries,&query).iter().filter_map(|(m,r)|entries.iter().position(|e|e.0==*m&&e.1==*r)).collect();
        let topics=self.help_topics(&query);
        for group in help_groups(&hits,topics.len()) {
            rows.push(match group {
                HelpRow::Header(title)=>Row::header(title),
                HelpRow::Item(i)=>{let (menu,row,item)=&entries[i];let text=format!("{} \u{203a} {}",item.menu,item.title);if item.enabled {Row::action(text,Action::HelpItem(*menu,*row))} else {Row::label(text)}},
                HelpRow::Topic(i)=>{let hit=&topics[i];Row::action(format!("{} \u{203a} {}",hit.app_name,hit.title),Action::HelpTopic(hit.app.clone(),hit.anchor.clone()))},
            });
        }
        rows
    }
    /// Every menu row the Help search searches, with its menu and its row index in that menu's plain rows: the bar
    /// menus of the shell and of the app in front.
    fn help_entries(&self)->Vec<(Menu,usize,helpbook::MenuItem)> {
        let mut menus:Vec<(Menu,String)>=vec![(Menu::App,self.app_menu_name()),(Menu::File,"File".to_string()),(Menu::Edit,"Edit".to_string())];
        if self.photos_front(){menus.push((Menu::Image,"Image".to_string()));}
        menus.extend([(Menu::View,"View"),(Menu::Window,"Window"),(Menu::Help,"Help")].map(|(m,t)|(m,t.to_string())));
        menus.extend(self.app_menus().iter().enumerate().filter(|(_,m)|shell_menu(&m.title).is_none()).map(|(i,m)|(Menu::Custom(i as u8),m.title.clone())));
        let mut out=Vec::new();
        for (menu,title) in &menus {
            for (i,row) in self.plain_rows(*menu).iter().enumerate() {
                if row.is_separator()||row.text.trim().is_empty() {continue;}
                out.push((*menu,i,help_item(title,&row.text,row.action.is_some())));
            }
        }
        out
    }
    /// The help topics the Help search finds: the front app's book and the system book.
    fn help_topics(&self,query:&str)->Vec<helpbook::Hit> {
        let hits=helpbook::with_index(std::path::Path::new(super::store::install::ROOT),|ix|ix.search(query,64));
        help_topic_hits(hits,self.help_book_id().as_deref(),8)
    }
    /// The help book of the app in front: a hosted app's id, or a built-in app's id (its icon name is its id).
    fn help_book_id(&self)->Option<String> {
        match self.screen {
            Screen::App(AppId::External(slot))=>self.ext.get(slot as usize).and_then(|h|h.as_ref()).map(|h|h.id.clone()),
            Screen::App(app)=>super::apps().iter().find(|a|a.id==app).map(|a|a.icon.to_string()),
            _=>None,
        }
    }
    /// True when a point is on the Help menu's search field (a press there keeps the menu open).
    fn help_field_at(&self,x:f32,y:f32)->bool {
        let Some((left,top,w,h))=self.desktop_popup_rect() else {return false;};
        if !(x>=left as f32&&x<(left+w) as f32&&y>=top as f32&&y<(top+h) as f32) {return false;}
        self.menu_row_at(Menu::Help,y-top as f32).is_some_and(|i|self.menu_rows(Menu::Help).get(i).is_some_and(|r|r.search))
    }
    /// Keys the Help search takes while the Help menu is open: typing adds to the query, Backspace removes from it, and
    /// Escape clears a query. None for the keys the menu handles itself (arrows, Return, Escape with no query).
    fn help_key(&mut self,code:u16)->Option<Vec<Effect>> {
        match code {
            14=>{if self.help_search.query.pop().is_some(){self.help_query_changed();}Some(vec![])},
            1=>{if self.help_search.query.is_empty() {return None;}self.help_reset();self.help_query_changed();Some(vec![])},
            103|108|105|106|28|96=>None,
            _=>{let c=super::evkeys::char_for(code,false)?;if c.is_alphanumeric()||c==' ' {self.help_search.query.push(c);self.help_query_changed();Some(vec![])} else {None}},
        }
    }
    /// The query changed: the first result is highlighted, and the pointer marker goes.
    fn help_query_changed(&mut self) {
        self.help_search.marker=false;
        self.desktop_menu_row=None;
        if !self.help_search.query.trim().is_empty() {if let Some(i)=self.menu_step(Menu::Help,1) {self.desktop_menu_row=Some((Menu::Help,i));}}
        self.dirty=true;
    }
    /// Drops the Help search's query and marker: the menu closed, or the open menu changed.
    fn help_reset(&mut self) {self.help_search=HelpSearch::default();}
    /// Shift+Cmd+/ (`hwkeys`): opens the Help menu with its search field ready for typing.
    pub(super) fn help_search_open(&mut self) {self.help_reset();self.desktop_menu=Some(Menu::Help);self.desktop_menu_row=None;self.dirty=true;}
    /// The Help search field: a capsule with a magnifier, the query (or its placeholder) and a caret after the query.
    fn draw_help_field(&self,s:&mut Surface,x:usize,y:usize,w:usize,top:f32,rh:f32,query:&str) {
        let fx=x as f32+self.ui.pt(8.);let fy=y as f32+self.ui.pt(top+3.);
        let fw=w as f32-self.ui.pt(16.);let fh=self.ui.pt(rh-6.);
        s.fill_rrect(fx,fy,fw,fh,fh/2.,super::ui::FIELD_BG.now());
        if let Some(im)=self.ui.symbols.render(crate::symbols::Symbol::Magnifyingglass,self.ui.pt(12.),SECONDARY) {s.blit(&im,(fx+self.ui.pt(9.)) as i32,(fy+fh/2.-im.h as f32/2.) as i32,255,255);}
        let tx=(fx+self.ui.pt(28.)) as i32;
        if query.is_empty() {
            self.label("Search",13.,Weight::Regular,SECONDARY,|im|s.blit(im,tx,(fy+fh/2.-im.h as f32/2.) as i32,255,255));
        } else {
            self.label(query,13.,Weight::Regular,LABEL.now(),|im|s.blit(im,tx,(fy+fh/2.-im.h as f32/2.) as i32,255,255));
            let cx=fx+self.ui.pt(28.)+self.measure_chrome(query,13.,Weight::Regular);
            s.fill_rect(cx as usize,(fy+self.ui.pt(6.)) as usize,1,(fh-self.ui.pt(12.)).max(1.) as usize,LABEL.now());
        }
    }
    fn own_menu_rows(&self,menu:Menu)->Vec<Row> {
        match menu {
            Menu::Document=>Vec::new(),
            Menu::Image=>self.photo_image_rows(),
            // each enabled source, the active one checked; picking one makes it the source (`kb_layouts::set_active`)
            Menu::Scripts=>super::scripts_menu::folder().map(|d|super::scripts_menu::list(&d).iter().enumerate().map(|(i,n)|Row::action(super::scripts_menu::title(n),Action::Script(i as u8))).collect()).unwrap_or_default(),
            Menu::Input=>{let mut v:Vec<Row>=super::kb_layouts::Layout::ALL.iter().enumerate().filter(|(_,l)|super::kb_layouts::is_enabled(**l)).map(|(i,l)|Row::action(format!("{} {}",if *l==super::kb_layouts::active()&&!self.input_providers.is_method_active(){"✓"}else{" "},l.name()),Action::InputSource(i as u8))).collect();v.extend(self.input_providers.modes().into_iter().map(|(id,label,on)|Row::action(format!("{} {}",if on{"✓"}else{" "},label),Action::InputMethod(id))));v.extend([Row::separator(),Row::action("Show Keyboard Viewer",Action::KeyboardViewer),Row::action(if self.keyboard_opened_explicitly(){"Hide On-Screen Keyboard"}else{"Show On-Screen Keyboard"},Action::OnScreenKeyboard)]);v},
            Menu::Status(i)=>self.status_entries_now().get(i as usize).map(|e|e.menu.iter().map(|m|if m.separator{Row::separator()}else if m.enabled{Row::action(format!("{} {}",if m.checked{"✓"}else{" "},m.title),Action::StatusItem(e.slot,e.item,m.id))}else{Row::label(format!("{} {}",if m.checked{"✓"}else{" "},m.title))}).collect()).unwrap_or_default(),
            Menu::Custom(i)=>{let slot=match self.screen {Screen::App(AppId::External(n))=>n,_=>return Vec::new()};self.app_menus().get(i as usize).map(|m|app_menu_rows(slot,m)).unwrap_or_default()},
            Menu::Context=>self.desktop_context.as_ref().map(|c|context_rows(c.slot,&c.items)).unwrap_or_default(),
            Menu::Arrange=>self.arrange_rows(),
            Menu::Dock=>{use super::desktop_dock::{Change,Position};let p=self.dock_preferences();let d=self.desk.as_ref().unwrap();let mut rows=Vec::new();if let Some(index)=d.dock_menu_target{let name=super::apps().get(index).map(|a|a.name).unwrap_or("Application");rows.push(Row::label(name));rows.push(Row::action("Open / Show",Action::Dock(Change::Open(index))));let pinned=self.dock_app_key(index).is_some_and(|key|p.pinned.contains(&key));rows.push(Row::action(if pinned{"Remove from Dock"}else{"Keep in Dock"},Action::Dock(Change::Pin(index))));rows.push(Row::action("Quit",Action::Dock(Change::Quit(index))));if let Some(AppId::External(slot))=super::apps().get(index).map(|a|a.id){if let Some(Some(host))=self.ext.get(slot as usize){for (id,title) in host.windows.menu_items(&host.id){if let Ok(n)=u16::try_from(id){rows.push(Row::action(title,Action::Open(AppId::ExternalWindow(slot,n))));}}if !host.windows.of_app(&host.id).is_empty(){rows.push(Row::separator());}let quick=self.quick_actions_for(slot as usize);for (i,item) in quick.iter().enumerate(){rows.push(Row::action(item.title.clone(),Action::QuickAction(slot,i as u8)));}if !quick.is_empty(){rows.push(Row::separator());}for item in &host.dock_menu{rows.push(if item.enabled{Row::action(item.title.clone(),Action::DockMenu(slot,item.id))}else{Row::label(item.title.clone())});}if !host.dock_menu.is_empty(){rows.push(Row::separator());}}}}rows.push(Row::label("Dock"));for (label,position) in [("Bottom",Position::Bottom),("Left",Position::Left),("Right",Position::Right)] {rows.push(Row::action(format!("{} {}",if p.position==position{"✓"}else{" "},label),Action::Dock(Change::Position(position))));}rows.push(Row::action(format!("{} Automatically hide and show",if p.autohide{"✓"}else{" "}),Action::Dock(Change::Autohide)));rows.push(Row::action("Smaller Icons",Action::Dock(Change::Size(-8.))));rows.push(Row::action("Larger Icons",Action::Dock(Change::Size(8.))));rows},
            Menu::App=>{let enabled=self.foreign_active().is_some()||self.desk.as_ref().and_then(|d|d.front()).is_some();let name=self.app_menu_name();let quit=format!("Quit {name}");let mut rows=vec![Row::action(format!("About {name}"),Action::Settings("About")),Row::separator(),Row::action("Settings…",Action::Settings("")).symbol(crate::symbols::Symbol::GearOutline).key("cmd+,")];let services=super::services_route::menu_entries();if !services.is_empty(){rows.push(Row::separator());rows.push(Row::label("Services"));rows.extend(services.into_iter().map(|(i,title)|Row::action(title,Action::Service(i))));}rows.push(Row::separator());rows.push((if enabled{Row::action(quit,Action::Quit)}else{Row::label(quit)}).key("cmd+q"));rows},
            // About This Computer, the settings and the store, Recent Items (a submenu), Force Quit, the power rows, then Lock Screen
            // and Log Out with the account's name (Golden Gate's system menu)
            Menu::System=>{use super::power_menu::PowerAction;use super::utility_window::Kind;let user=self.fit_chrome(super::utility_window::user_name(),13.,self.ui.pt(180.));vec![Row::action("About This Computer",Action::Window(Kind::About)),Row::separator(),Row::action("System Settings…",Action::Settings("")),Row::action("App Store…",Action::Open(AppId::AppStore)),Row::separator(),Row::action("Recent Items",Action::Submenu(Menu::Recent)),Row::separator(),Row::action("Force Quit…",Action::Window(Kind::ForceQuit)).key("alt+cmd+escape"),Row::separator(),Row::action("Sleep",Action::Sleep),Row::action("Restart…",Action::Halt(PowerAction::Restart)),Row::action("Shut Down…",Action::Halt(PowerAction::Off)),Row::separator(),Row::action("Lock Screen",Action::Lock).key("ctrl+cmd+q"),Row::action(format!("Log Out {user}…"),Action::Window(Kind::LogOut)).key("shift+cmd+q")]},
            // the Recent Items submenu: a way back, the apps opened last (newest first), then Clear Menu
            Menu::Recent=>{let recent=self.recent_titles();let mut rows=vec![Row::action("Recent Items",Action::Submenu(Menu::System)).symbol(crate::symbols::Symbol::ChevronLeft),Row::separator()];if recent.is_empty(){rows.push(Row::label("No Recent Items"));}for (i,title) in recent.iter().enumerate(){rows.push(Row::action(title.clone(),Action::Recent(i as u8)));}rows.push(Row::separator());rows.push(if recent.is_empty(){Row::label("Clear Menu")}else{Row::action("Clear Menu",Action::ClearRecent)});rows},
            Menu::ControlCenter=>Vec::new(),
            Menu::Recording=>vec![Row::action("Stop Recording",Action::RecordStop)],
            Menu::GameMode=>vec![Row::action("Turn Off Game Mode",Action::GameModeOff)],
            // the apps using location now, by name: a label, not a choice (nothing here changes the grant)
            Menu::Location=>{let mut rows:Vec<Row>=super::services::location::in_use_now().into_iter().map(|id|{let name=super::apphost::installed().iter().find(|a|a.manifest.id==id).map_or_else(|| id.clone(),|a|a.manifest.name.clone());Row::label(name)}).collect();rows.push(Row::separator());rows.push(Row::action("Location Services Settings\u{2026}",Action::Settings("Location Services")));rows},
            // the Bluetooth extra: the radio, then each paired or connected device (connect or disconnect), then its pane
            Menu::Bluetooth=>{let bt=&self.info.bt;let mut rows=vec![Row::action(if bt.on(){"Turn Bluetooth Off"}else{"Turn Bluetooth On"},Action::Bluetooth(crate::bare::bluetooth::Cmd::Enable(!bt.on())))];if bt.on(){rows.push(Row::separator());for d in bt.devices.iter().filter(|d|d.paired||d.connected).take(8) {if bt.busy.as_deref()==Some(d.addr.as_str()){rows.push(Row::label(format!("{}…",d.label())));}else{let cmd=if d.connected{crate::bare::bluetooth::Cmd::Disconnect(d.addr.clone())}else{crate::bare::bluetooth::Cmd::Connect(d.addr.clone())};rows.push(Row::action(format!("{} {}",if d.connected{"✓"}else{" "},d.label()),Action::Bluetooth(cmd)));}}}rows.push(Row::separator());rows.push(Row::action("Bluetooth Settings…",Action::Settings("Bluetooth")));rows},
            // title and position as labels, then the transport: an enabled row sends its remote command to the owner
            Menu::NowPlaying=>super::now_playing_menu::panel().map(|p|{let mut rows=vec![Row::label(p.title),Row::label(p.position),Row::separator()];rows.extend(p.transport.into_iter().map(|(c,text,on)|if on {Row::action(text,Action::Media(c))} else {Row::label(text)}));rows}).unwrap_or_default(),
            // View holds the Dock's display options (also in the Dock menu), as Files and Mail keep theirs in View
            Menu::View=>{use super::desktop_dock::Change;let autohide=self.desk.as_ref().is_some_and(|d|d.dock_preferences.autohide);let mut v=vec![Row::action(format!("{} Automatically Hide Dock",if autohide{"✓"}else{" "}),Action::Dock(Change::Autohide)),Row::action("Smaller Icons",Action::Dock(Change::Size(-8.))),Row::action("Larger Icons",Action::Dock(Change::Size(8.)))];if self.term_front(){v.splice(0..0,[Row::action("Actual Size",Action::Term(TermKey::ActualSize)).key("cmd+0"),Row::action("Bigger",Action::Term(TermKey::Bigger)).key("cmd+="),Row::action("Smaller",Action::Term(TermKey::Smaller)).key("cmd+-"),Row::separator()]);}v},
            // Window: Minimize and Zoom (also in File), Bring All to Front, then every open window by name, the front one checked
            Menu::Window=>{let Some(d)=self.desk.as_ref() else{return Vec::new();};let enabled=self.foreign_active().is_some()||d.front().is_some();let front=d.front().map(|w|w.app);let mut rows=vec![(if enabled{Row::action("Minimize",Action::Minimize)}else{Row::label("Minimize")}).key("cmd+m"),if enabled{Row::action("Zoom",Action::Zoom)}else{Row::label("Zoom")},match d.front().map(|w|(w.app,w.full)).filter(|_|enabled&&self.foreign_active().is_none()){Some((a,full))=>Row::action(if full{"Exit Full Screen"}else{"Enter Full Screen"},Action::Arrange(a,super::desk::Arrange::FullScreen(!full))),None=>Row::label("Enter Full Screen")},Row::separator(),if d.wins.iter().any(|w|w.minimized){Row::action("Bring All to Front",Action::ShowAll)}else{Row::label("Bring All to Front")},Row::action("Show Desktop",Action::Desktop)];let tile_on=self.foreign_active().is_none()&&d.front().is_some();rows.push(Row::separator());rows.extend([("Left Half",super::desk::Tile::Left,"ctrl+cmd+left"),("Right Half",super::desk::Tile::Right,"ctrl+cmd+right"),("Fill",super::desk::Tile::Fill,"ctrl+cmd+up"),("Center",super::desk::Tile::Center,"ctrl+cmd+down")].map(|(l,t,k)|if tile_on&&(t==super::desk::Tile::Fill||!super::desk::compositor_places_windows()){Row::action(l,Action::Tile(t)).key(k)}else{Row::label(l).key(k)}));rows.push(if tile_on{Row::action("Return to Previous Size",Action::RestoreSize)}else{Row::label("Return to Previous Size")});rows.push(if d.wins.len()>=2{Row::action("Next Window",Action::CycleWindow).key("cmd+`")}else{Row::label("Next Window")});let front_parent=d.front().map(|w|w.app.parent());let mut open:Vec<(String,super::AppId)>=d.wins.iter().filter(|w|Some(w.app.parent())==front_parent).map(|w|(self.window_menu_name(w.app),w.app)).collect();open.sort_by_key(|(name,_)|name.to_lowercase());let foreign:Vec<&crate::bare::wayland::ForeignWindow>=d.native_windows.iter().chain(d.linux_apps.iter().flat_map(|a|a.windows.iter())).collect();if !open.is_empty()||!foreign.is_empty(){rows.push(Row::separator());}rows.extend(open.into_iter().take(12).map(|(name,app)|Row::action(format!("{} {}",if front==Some(app)&&self.foreign_active().is_none(){"✓"}else{" "},name),Action::Open(app))));rows.extend(foreign.into_iter().take(12).map(|w|Row::action(format!("{} {}",if w.active{"✓"}else{" "},self.fit_chrome(if w.title.is_empty(){w.app_id.as_str()}else{w.title.as_str()},13.,self.ui.pt(300.))),Action::Foreign(w.id))));rows},
            Menu::File=>{let enabled=self.foreign_active().is_some()||self.desk.as_ref().and_then(|d|d.front()).is_some();
            // an app that keeps Cmd+W for its own Close Tab leaves the window the shifted chord (Close Window is Shift+Cmd+W then)
            let close_key=if self.app_owns_close(){"cmd+shift+w"}else{"cmd+w"};let mut rows=vec![(if enabled{Row::action("Close Window",Action::Close)}else{Row::label("Close Window")}).key(close_key),(if enabled{Row::action("Minimize Window",Action::Minimize)}else{Row::label("Minimize Window")}).key("cmd+m"),if enabled {Row::action("Maximize / Restore Window",Action::Zoom)}else{Row::label("Maximize / Restore Window")}];if self.term_front(){rows.splice(0..0,[Row::action("New Tab",Action::Term(TermKey::NewTab)).key("cmd+t"),Row::separator()]);rows[2]=Row::action("Close Tab",Action::Term(TermKey::CloseTab)).key("cmd+w");}if self.foreign_active().is_none(){rows.push(Row::separator());rows.push((if enabled{Row::action("Print\u{2026}",Action::Print)}else{Row::label("Print\u{2026}")}).key("cmd+p"));}rows},
            // Undo and Redo first, as in every Golden Gate Edit menu. Native Linux apps get the shortcut (Ctrl+key); Redo has no
            // single key, so it stays a disabled label for them.
            Menu::Edit=>{let mut v:Vec<Row>=[("Undo",Some(44),bifrost_proto::Command::Undo),("Redo",None,bifrost_proto::Command::Redo),("Cut",Some(45),bifrost_proto::Command::Cut),("Copy",Some(46),bifrost_proto::Command::Copy),("Paste",Some(47),bifrost_proto::Command::Paste),("Select All",Some(30),bifrost_proto::Command::SelectAll)].into_iter().map(|(label,key,command)|if let Some(commands)=self.sdk_commands(){if commands.supports(command){Row::action(label,Action::Command(command))}else{Row::label(label)}}else if let (Some(key),true)=(key,self.foreign_active().is_some()||self.screen==Screen::App(AppId::Terminal)){Row::action(label,Action::Shortcut(key))}else{Row::label(label)}).collect();for r in &mut v{if let Some((_,k))=MENU_KEYS.iter().find(|(l,_)|*l==r.text){*r=std::mem::replace(r,Row::separator()).key(k);}}v.insert(2,Row::separator());if let Some(t)=self.sdk_undo_titles(){for r in &mut v{if r.action.is_some(){let t=match r.text.as_str(){"Undo"=>&t.0,"Redo"=>&t.1,_=>continue};if !t.is_empty(){r.text=t.clone();}}}}if self.term_front(){v.extend([Row::separator(),Row::action("Find…",Action::Term(TermKey::Find)).key("cmd+f"),Row::action("Find Next",Action::Term(TermKey::FindNext)).key("cmd+g"),Row::action("Find Previous",Action::Term(TermKey::FindPrev)).key("cmd+shift+g"),Row::separator(),Row::action("Clear Scrollback",Action::Term(TermKey::ClearScrollback)).key("cmd+k")]);v.extend([Row::separator(),Row::label("Copy Special"),Row::action("Plain Text",Action::Term(TermKey::Copy)).indented(),Row::action("Text with Colors",Action::Term(TermKey::CopyStyled)).indented(),Row::action("Without Background Colour",Action::Term(TermKey::CopyStyledPlain)).indented()]);}v.extend([Row::separator(),Row::action("Emoji & Symbols",Action::CharacterViewer)]);v},
            Menu::Help=>{let mut r:Vec<Row>=self.help_book().map(|(id,name)|vec![Row::action(format!("{name} Help"),Action::Help(id)),Row::separator()]).unwrap_or_default();r.extend(vec![Row::action("BifrOSt Information",Action::Settings("About")),Row::separator(),Row::label("Search").key("cmd+space"),Row::label("Input Source").key("ctrl+space"),Row::label("App Switcher").key("cmd+tab"),Row::label("Back").key("escape")]);if self.form().is_tablet() {r.push(Row::label("Home: Cmd (Meta) alone"));}r},
            Menu::Network=>{
                let mut rows=Vec::new();if let Some(n)=&self.info.desktop_network {
                    for i in n.interfaces.iter().filter(|i|i.kind=="ethernet") {rows.push(Row::label(format!("{}: {}",i.name,if i.connected(){"Connected"}else{"Disconnected"})));for ip in &i.ipv4 {rows.push(Row::label(ip.clone()));}}
                    if n.wifi_available {rows.push(Row::action(if n.wifi_on{"Turn Wi-Fi Off"}else{"Turn Wi-Fi On"},Action::Wifi(!n.wifi_on)));if n.wifi_on {rows.push(Row::label(n.ssid.clone().unwrap_or_else(||"Not Connected".into())));rows.push(Row::action("Scan for Networks",Action::Scan));for net in n.networks.iter().filter(|net|net.saved&&n.ssid.as_deref()!=Some(net.ssid.as_str())).take(8) {rows.push(Row::action(net.ssid.clone(),Action::Select(net.ssid.clone())));}}}
                    else {rows.push(Row::label("No Wi-Fi adapter"));}
                }if let Some(url)=&self.info.captive_portal {rows.push(Row::action(format!("Sign In to {}…",self.info.current_ssid.as_deref().unwrap_or("Network")),Action::Portal(url.clone())));}rows.push(Row::action("Network Settings…",Action::Settings("Network")));rows
            },
            Menu::Cpu=>{let m=&self.desktop_metrics;let mut rows=vec![Row::label(format!("CPU usage: {:.1}%",m.cpu)),Row::label(format!("{} logical processors",m.cores)),Row::label(format!("Load: {}",m.load)),Row::label(format!("Uptime: {}h {}m",m.uptime/3600,m.uptime/60%60))];rows.extend(top_app_rows(&m.apps,"Top apps by CPU time",|a|a.cpu_usec,|v|format!("{:.1} s",v as f64/1e6)));rows},
            Menu::Memory=>{let m=&self.desktop_metrics;let gb=|n:u64|format!("{:.2} GB",n as f64/1073741824.);let mut rows=vec![Row::label(format!("Used: {} ({:.0}%)",gb(m.used),m.memory_percent())),Row::label(format!("Available: {}",gb(m.available))),Row::label(format!("Installed: {}",gb(m.total))),Row::label(format!("Cached: {}",gb(m.cached))),Row::label(format!("Swap: {} / {}",gb(m.swap_total.saturating_sub(m.swap_free)),gb(m.swap_total)))];rows.extend(top_app_rows(&m.apps,"Top apps by memory",|a|a.memory,gb));rows},
            Menu::Power=>{
                let mut rows=vec![Row::label(if self.info.battery_present&&!self.info.charging {"On battery power"}else{"Connected to AC power"})];
                // the charge level, with the charging state, when there is a battery
                if self.info.battery_present {rows.push(Row::label(format!("Battery {}%{}",self.info.battery,if self.info.charging {", charging"} else {""})));}
                let base=rows.len();if let Some(n)=&self.info.desktop_network {for p in &n.power_profiles {rows.push(Row::action(format!("{} {}",if n.power==*p{"✓"}else{" "},match p.as_str(){"performance"=>"Performance","power-saver"=>"Low Power",_=>"Balanced"}),Action::Power(p.clone())));}}
                if rows.len()==base {rows.push(Row::label("Power profiles unavailable"));}
                // Low Power Mode is a switch of its own (Settings > Battery and the low-battery alert turn it on too).
                rows.push(Row::action(format!("{} Low Power Mode",if self.low_power(){"✓"}else{" "}),Action::LowPowerMode(!self.low_power())));
                rows.push(Row::action("Lock Screen",Action::Lock));rows.push(Row::action("Restart",Action::Halt(super::power_menu::PowerAction::Restart)));rows.push(Row::action("Shut Down",Action::Halt(super::power_menu::PowerAction::Off)));rows
            },
        }
    }
    #[cfg(test)]
    pub(super) fn menu_row_keys(&self,menu:Menu)->Vec<String> {self.menu_rows(menu).into_iter().filter_map(|r|r.key).collect()}
    #[cfg(test)]
    pub(super) fn row_top_pt(&self,menu:Menu,i:usize)->f32 {menu_spans(&self.menu_rows(menu)).0[i].0}
    #[cfg(test)]
    pub(super) fn menu_row_texts(&self,menu:Menu)->Vec<String> {self.menu_rows(menu).into_iter().map(|r|r.text).collect()}
    /// The rows of the green-button menu for the window it is open on (`desk::arrange_lines`); the choices under the
    /// Move & Resize header are indented.
    fn arrange_rows(&self)->Vec<Row> {
        use super::desk::ArrangeLine;
        let Some(d)=self.desk.as_ref() else{return Vec::new();};let Some(a)=d.arrange else{return Vec::new();};
        let Some(w)=d.index_of(a.app).and_then(|i|d.wins.get(i)) else{return Vec::new();};
        let others=d.wins.iter().any(|o|o.app.parent()!=a.app.parent()&&!o.minimized);
        let mut under_header=false;
        super::desk::arrange_lines(w.full,others).into_iter().map(|l|match l {
            ArrangeLine::Header(t)=>{under_header=true;Row::header(t)},
            ArrangeLine::Separator=>{under_header=false;Row::separator()},
            ArrangeLine::Item(t,what)=>{let r=Row::action(t,Action::Arrange(a.app,what));if under_header{r.indented()}else{r}},
        }).collect()
    }
    pub(super) fn desktop_popup_rect(&self)->Option<(usize,usize,usize,usize)> {
        let menu=self.desktop_menu?;if menu==Menu::Dock{return self.dock_popup_rect();}if menu==Menu::Arrange{
            // under the green button, kept on the screen like a context menu (the size is the drawn rows')
            let a=self.desk.as_ref()?.arrange?;let rows=self.menu_rows(menu);let width=self.menu_width(&rows);let height=self.ui.pt(menu_spans(&rows).1).ceil() as usize;
            let area=super::desk::Desk::area(&self.desk_metrics());
            let (x,y)=context_menu_origin((a.x,a.y),(width,height),(0,area.y,self.w,area.h));
            return Some((x,y,width,height));
        }if menu==Menu::Document{return self.doc_panel_rect();}if menu==Menu::Context{
            // the size is the drawn rows' (`menu_spans`, the same call the bar menus use), so the surface fits its items
            let c=self.desktop_context.as_ref()?;let rows=self.menu_rows(menu);let width=self.menu_width(&rows);let height=self.ui.pt(menu_spans(&rows).1).ceil() as usize;
            let area=super::desk::Desk::area(&self.desk_metrics());
            let (x,y)=context_menu_origin((c.x,c.y),(width,height),(0,area.y,self.w,area.h));
            return Some((x,y,width,height));
        }// the Recent Items submenu hangs from the system menu's bar item
        let anchor=if menu==Menu::Recent{Menu::System}else{menu};let (_,x,_,_)=self.menu_buttons().into_iter().find(|b|b.0==anchor)?;
        let rows=self.menu_rows(menu);let width=self.menu_width(&rows);let height=self.ui.pt(menu_spans(&rows).1).ceil() as usize;
        Some(((x as usize).min((self.w as usize).saturating_sub(width)),self.desktop_bar_height()+self.ui.pt(MENU_GAP_PT) as usize,width,height))
    }
    /// The parts of a desktop frame that are not transparent when the GPU presenter draws the
    /// wallpaper: the menu bar, an open menu, the dock with room for its hover label. None on the CPU path.
    pub(super) fn desk_regions_now(&self)->Option<Vec<(i32,i32,i32,i32)>> {
        if self.is_locked(){return None;}
        // the CPU path redraws the whole frame; the region is the whole screen so the draw compares it with the
        // previous frame and only the pixels that changed are presented
        if !bifrost_render::glass::gpu(){return Some(vec![(0,0,self.w,self.h)]);}
        let mut v=vec![(0,0,self.w,self.desktop_bar_height() as i32)];
        // the open menu owes its shadow on every side, the top one up into the gap under the bar; the same rect is
        // cleared on the frame after it closes (desk_regions keeps it), so no shadow line is left behind
        if let Some(popup)=self.desktop_popup_rect(){let g=self.ui.pt(super::desk::SHADOW_REACH_PT) as i32+2;v.push(popup_damage_rect(popup,self.desktop_bar_height() as i32,g,self.ui.pt(2.) as i32));}
        let d=self.dock_overlay_rect();let m=self.ui.pt(120.) as i32;
        v.push(((d.x-m).max(0),(d.y-m).max(0),d.w+2*m,d.h+2*m));
        // the desktop Control Center and Notification Center panels, with room for their shadow
        let g=self.ui.pt(super::desk::SHADOW_REACH_PT);
        for r in [self.cc_desk_rect(),self.center_desk_rect()].into_iter().flatten() {let (x,y,w,h)=r;v.push(((x-g).max(0.) as i32,(y-g).max(0.) as i32,(w+2.*g) as i32,(h+2.*g) as i32));}
        if let Some(((x,y,w,h),_))=self.quicksearch.desk_overlay(&self.ui,self.w){v.push((x as i32,y as i32,w as i32,h as i32));}
        if let Some(((x,y,w,h),_))=self.assistant.desk_overlay(&self.ui,self.w){v.push((x as i32,y as i32,w as i32,h as i32));}
        // the Quick Look card floats over the windows: the GPU presenter keeps its region (cleared again once it closes)
        if let Some((x,y,w,h))=self.quick_look_rect(){v.push((x as i32,y as i32,w as i32,h as i32));}
        // the Cmd+Tab strip floats over the windows the same way (cleared again once it closes)
        if let Some((x,y,w,h))=self.cmdtab_rect(){v.push((x as i32,y as i32,w as i32,h as i32));}
        // the system menu's card (Force Quit, About, Log Out) is an overlay surface above the windows as well
        if let Some((x,y,w,h))=self.utility_rect(){v.push((x as i32,y as i32,w as i32,h as i32));}
        // a tablet's on-screen keyboard, from its resting top to the bottom edge: drawn here while it slides, and its
        // rows cleared again on the frame after it is gone
        if self.desk_keyboard_visible(){let top=self.kb_top().max(0.) as i32;v.push((0,top,self.w,self.h-top));}
        if self.perf.enabled||self.perf_drawn{let (x,y,w,h)=self.perf_rect();let g=self.ui.pt(2.) as i32;v.push((x as i32-g,y as i32-g,w as i32+2*g,h as i32+2*g));}
        Some(v)
    }
    pub(super) fn draw_desktop_bar(&self,s:&mut Surface) {
        let height=self.desktop_bar_height();if s.y0>=height{return;}
        // a full-screen window covers the bar until the pointer reaches the top (`desk::bar_hidden`)
        if self.desk_bar_hidden(){return;}
        // the backdrop depends only on the size and the wallpaper: blur it once, not every frame
        if bifrost_render::glass::gpu() {
            // Golden Gate: the bar is transparent over the wallpaper the presenter draws
            *self.bar_backdrop.borrow_mut()=None;
            // over a full-screen window (the pointer is at the top, or the bar would not be here) the bar has a ground of its
            // own: transparent, the window's toolbar and traffic lights would show through it and collide with its items
            let ground=if self.desk.as_ref().is_some_and(|d|d.front_full()){MENU_SOLID.now()}else{0};
            s.fill_rect(0,0,self.w as usize,height,ground);
            self.draw_desktop_bar_items(s,height);return;
        }
        let key=(self.w as usize,height,self.wallpaper_frame().px.as_ptr() as usize);
        let mut cache=self.bar_backdrop.borrow_mut();
        if cache.as_ref().is_none_or(|(k,_)|*k!=key) {
            let mut image=Image::new(self.w as usize,height);let mut bg=Surface{w:image.w,h:image.h,stride:image.w,px:&mut image.px,y0:0};
            bg.fill_rect(0,0,bg.w,bg.h,CELL);{let wp=self.wallpaper_frame();bg.copy(wp,(self.w-wp.w as i32)/2,0);}
            *cache=Some((key,image));
        }
        let (_,image)=cache.as_ref().unwrap();s.blit(image,0,0,255,255);drop(cache);
        self.draw_desktop_bar_items(s,height);
    }
    fn draw_desktop_bar_items(&self,s:&mut Surface,height:usize) {
        for (menu,x,width,label) in self.menu_buttons() {if self.desktop_menu==Some(menu)||(menu==Menu::System&&self.desktop_menu==Some(Menu::Recent)){let h=height as f32-self.ui.pt(4.);s.fill_rrect(x,self.ui.pt(2.),width,h,h/2.,self.bar_ink().capsule);}if matches!(menu,Menu::System|Menu::Network|Menu::Power|Menu::ControlCenter|Menu::Bluetooth|Menu::Location)||self.status_item_is_icon(menu) {
            // the battery item with its percentage: the glyph stays at the item's left, the percentage follows it
            let percent=if menu==Menu::Power {self.battery_bar_label()} else {None};
            let glyph_x=if percent.is_some() {x+self.ui.pt(STATUS_ICON_PT)/2.} else {x+width/2.};
            self.draw_desktop_status_icon(s,menu,&label,glyph_x,height as f32/2.);
            if let Some(t)=percent {self.bar_label(s,&t,12.,Weight::Regular,|im|((x+self.ui.pt(STATUS_ICON_PT)+self.ui.pt(4.)) as i32,(height as i32-im.h as i32)/2));}
            continue;
        }if matches!(menu,Menu::Cpu|Menu::Memory|Menu::Status(_)|Menu::Input|Menu::NowPlaying|Menu::Scripts){self.bar_label(s,&label,12.,Weight::Regular,|im|((x+self.ui.pt(STATUS_PAD_PT)) as i32,(height as i32-im.h as i32)/2));continue;}self.bar_label(s,&label,13.,if menu==Menu::App{Weight::Semibold}else{Weight::Regular},|im|((x+self.ui.pt(12.)) as i32,(height as i32-im.h as i32)/2));}
        let text=self.clock_text();
        self.bar_label(s,&text,12.,Weight::Medium,|im|(self.w-im.w as i32-self.ui.pt(16.) as i32,(height as i32-im.h as i32)/2));
    }
    /// A menu bar label in the bar's ink, with its soft shadow (or halo) drawn first for legibility.
    fn bar_label(&self,s:&mut Surface,text:&str,pt:f32,weight:Weight,place:impl FnOnce(&Image)->(i32,i32)) {
        let ink=self.bar_ink();
        self.label(text,pt,weight,WHITE,|im|{
            let (x,y)=place(im);let radius=self.ui.pt(1.).max(1.) as usize;
            let mut shadows=self.bar_shadows.borrow_mut();
            if shadows.len()>64{shadows.clear();}
            let shadow=shadows.entry((text.to_string(),pt.to_bits(),std::mem::discriminant(&weight),radius)).or_insert_with(||{let mut sh=im.clone();sh.blur(radius);sh});
            s.blit_colored(shadow,x,y+self.ui.pt(0.5).max(1.) as i32,ink.shadow,ink.shadow_alpha);s.blit_colored(im,x,y,ink.text,ink.text_alpha);
        });
    }
    pub(super) fn clock_text(&self)->String {self.clock_style.text(self.info.wday,self.info.mday,self.info.mon,self.info.hour,self.info.min)}
    /// The date and time at the right end of the bar; a tap toggles Notification Center.
    fn clock_hit(&self,x:f32,y:f32)->bool {
        let width=self.measure_chrome(&self.clock_text(),12.,Weight::Medium);let left=self.w as f32-self.ui.pt(16.)-width;
        y<self.desktop_bar_height() as f32&&x>=left-self.ui.pt(8.)&&x<self.w as f32
    }
    pub(super) fn draw_desktop_popup(&self,s:&mut Surface) {
        let Some((x,y,w,h))=self.desktop_popup_rect() else{return;};let menu=self.desktop_menu.unwrap();
        if menu==Menu::Document{self.draw_doc_dim(s);}
        if bifrost_render::glass::gpu()&&bifrost_render::appearance::reduce_transparency() {
            let r=self.ui.pt(MENU_RADIUS_PT);
            s.fill_rect(x,y,w,h,0);super::desk::soft_shadow(s,&self.ui,x as f32,y as f32,w as f32,h as f32,r);s.fill_rrect(x as f32,y as f32,w as f32,h as f32,r,MENU_SOLID.now());
        } else if bifrost_render::glass::gpu() {
            let r=self.ui.pt(MENU_RADIUS_PT);
            bifrost_render::glass::push(super::control_center::shell_glass_rect(&bifrost_ui::material_request::ShellSurface::MenuBarMenu.request((x as f32,y as f32,w as f32,h as f32),r)));
            s.fill_rect(x,y,w,h,0);super::desk::soft_shadow(s,&self.ui,x as f32,y as f32,w as f32,h as f32,r);s.fill_rrect(x as f32,y as f32,w as f32,h as f32,r,bifrost_render::glass::tint(MENU_TINT.now()));
        } else {
            // no glass on the CPU path: the blurred wallpaper under the menu, rounded to its corners, over the shadow
            let r=self.ui.pt(MENU_RADIUS_PT);
            super::desk::soft_shadow(s,&self.ui,x as f32,y as f32,w as f32,h as f32,r);
            let mut image=Image::new(w,h);let mut bg=Surface{w,h,stride:w,px:&mut image.px,y0:0};bg.fill_rect(0,0,w,h,CELL);{let wp=self.wallpaper_frame();bg.copy(wp,(self.w-wp.w as i32)/2-x as i32,-(y as i32));}image.blur_material(self.ui.pt(10.) as usize);image.apply_mask(&crate::gfx::rrect_mask(w,h,r));s.blit(&image,x as i32,y as i32,255,255);s.fill_rrect(x as f32,y as f32,w as f32,h as f32,r,MENU_TINT.now());
        }
        self.draw_popup_edge(s,x,y,w,h);
        if menu==Menu::Document{self.draw_doc_panel(s,x,y,w,h);return;}
        let sel=self.menu_row_of(menu);let rows=self.menu_rows(menu);let (spans,_)=menu_spans(&rows);
        // Golden Gate: a row's symbol sits at the text edge and pushes only that row's title right
        let lead=16.;
        for (i,row) in rows.iter().enumerate(){let (top,rh)=spans[i];
            if row.sep{s.fill_rect(x+self.ui.pt(12.) as usize,(y as f32+self.ui.pt(top+rh/2.)) as usize,w.saturating_sub(self.ui.pt(24.) as usize),1,super::ui::SEPARATOR_C.now());continue;}
            if row.search{self.draw_help_field(s,x,y,w,top,rh,&row.text);continue;}
            if row.header{self.label(&row.text,10.,Weight::Regular,SECONDARY,|im|s.blit(im,x as i32+self.ui.pt(lead) as i32,(y as f32+self.ui.pt(top+rh/2.)-im.h as f32/2.) as i32,255,255));continue;}
            let hi=sel==Some(i)&&row.action.is_some();
            if hi{s.fill_rrect(x as f32+self.ui.pt(5.),y as f32+self.ui.pt(top),w as f32-self.ui.pt(10.),self.ui.pt(rh),self.ui.pt(8.),super::ui::MENU_HIGHLIGHT);}
            let mid=y as f32+self.ui.pt(top+rh/2.);
            if hi&&self.help_search.marker {if let Some(im)=self.ui.symbols.render(crate::symbols::Symbol::ChevronRight,self.ui.pt(10.),super::ui::MENU_HIGHLIGHT_TEXT) {s.blit(&im,x as i32+self.ui.pt(6.) as i32,(mid-im.h as f32/2.) as i32,255,255);}}
            let ink=if row.action.is_some(){if hi{super::ui::MENU_HIGHLIGHT_TEXT}else{LABEL.now()}}else{SECONDARY};
            if let Some(sym)=row.symbol{if let Some(im)=self.ui.symbols.render(sym,self.ui.pt(15.),ink){s.blit(&im,x as i32+self.ui.pt(lead+7.5) as i32-im.w as i32/2,(mid-im.h as f32/2.) as i32,255,255);}}
            let text_x=(if row.symbol.is_some(){lead+20.}else{lead})+if row.indent{12.}else{0.};
            self.label(&row.text,13.,Weight::Regular,ink,|im|s.blit(im,x as i32+self.ui.pt(text_x) as i32,(mid-im.h as f32/2.) as i32,255,255));
            // a row that opens a submenu ends in a chevron
            if matches!(row.action,Some(Action::Submenu(Menu::Recent))) {if let Some(im)=self.ui.symbols.render(crate::symbols::Symbol::ChevronRight,self.ui.pt(11.),ink) {s.blit(&im,(x+w) as i32-self.ui.pt(16.) as i32-im.w as i32,(mid-im.h as f32/2.) as i32,255,255);}}
            if let Some(k)=&row.key{let k=shortcut_spaced(k);self.label(&k,13.,Weight::Regular,if hi{super::ui::MENU_HIGHLIGHT_TEXT}else{super::ui::MENU_SHORTCUT},|im|s.blit(im,(x+w) as i32-self.ui.pt(16.) as i32-im.w as i32,(mid-im.h as f32/2.) as i32,255,255));}}
    }
    /// The one-pixel edge every popup gets (a menu, the Dock's menu, an open panel), in the separator colour, so the
    /// translucent menu still has an outline over white content (E2E N6). Its shadow is `soft_shadow`, drawn before the fill.
    fn draw_popup_edge(&self,s:&mut Surface,x:usize,y:usize,w:usize,h:usize) {
        s.stroke_rrect(x as f32,y as f32,w as f32,h as f32,self.ui.pt(MENU_RADIUS_PT),self.ui.pt(0.5).max(1.),super::ui::separator_now());
    }
    pub(super) fn desktop_bar_event(&mut self,event:Event,t:f64)->Option<Vec<Effect>> {
        if !self.desk_active()||self.is_locked(){return None;}
        // the preview panel takes the pointer inside its card while it is up
        match event {Event::Down(x,y)=>{if self.quick_look_pointer(x,y,false){return Some(vec![]);}},Event::Up(x,y)=>{if self.quick_look_pointer(x,y,true){return Some(vec![]);}},_=>{}}
        // an open or save panel is modal over the desktop: it takes the pointer, a press elsewhere does nothing
        if self.desktop_menu==Some(Menu::Document) {
            if let (Event::Up(x,y),Some((left,top,_,_)))=(event,self.desktop_popup_rect()) {self.doc_click(x-left as f32,y-top as f32,t);}
            return Some(vec![]);
        }
        match event {
            // Golden Gate: the clock opens Notification Center (notifications and widgets), as on Golden Gate
            Event::Down(x,y) if self.clock_hit(x,y)&&self.desktop_menu.is_none()&&self.desk.is_some()=>{if self.notify.center.is_hidden(){self.open_notification_center(t);}else{self.notify.center.hide();}self.dirty=true;Some(vec![])},
            Event::Down(x,y) if y<self.desktop_bar_height() as f32 && (!self.cc_reveal_zone(x,y)||self.menu_buttons().iter().any(|(_,left,w,_)|x>=*left&&x<*left+*w))=>{self.desktop_menu_row=None;self.help_reset();self.desktop_menu=self.menu_buttons().into_iter().find(|(_,left,w,_)|x>=*left&&x<*left+*w).map(|b|b.0);if self.desktop_menu==Some(Menu::ControlCenter){self.desktop_menu=None;self.toggle_desk_control_center(t);}else{self.close_desk_control_center(t);}self.dirty=true;Some(vec![])},
            // a press on the green button the menu hangs from is the button's own click (full screen), not a click outside
            Event::Down(x,y) if self.desktop_menu==Some(Menu::Arrange)&&self.arrange_on_button(x,y)=>{self.desktop_menu=None;self.dirty=true;None},
            Event::Down(x,y) if self.desktop_menu.is_some()=>{if !self.desktop_popup_rect().is_some_and(|(left,top,w,h)|x>=left as f32&&x<(left+w) as f32&&y>=top as f32&&y<(top+h) as f32){self.desktop_menu=None;self.dirty=true;}Some(vec![])},
            Event::Up(x,y) if self.desktop_menu.is_some()=>{let menu=self.desktop_menu.unwrap();if y<self.desktop_bar_height() as f32{return Some(vec![]);}
                if self.desktop_menu==Some(Menu::Help)&&self.help_field_at(x,y) {return Some(vec![]);}
                let action=self.desktop_popup_rect().and_then(|(left,top,w,h)|if x>=left as f32&&x<(left+w) as f32&&y>=top as f32&&y<(top+h) as f32 {self.menu_row_at(menu,y-top as f32).and_then(|i|self.menu_rows(menu).get(i).and_then(|r|r.action.clone()))}else{None});
                self.desktop_menu=None;self.help_reset();self.dirty=true;let mut effects=Vec::new();self.desktop_menu_row=None;if let Some(action)=action {effects=self.run_menu_action(action,t);}Some(effects)
            },_=>None,
        }
    }
    /// The selectable row of the open menu (keyboard or pointer), if it belongs to `menu`.
    /// Menu width in px: at least 264 pt, wider when a title and its key equivalent would touch (menus size to their content).
    fn menu_width(&self,rows:&[Row])->usize {
        use std::hash::{Hash,Hasher};
        let mut h=std::collections::hash_map::DefaultHasher::new();self.ui.pt(1.).to_bits().hash(&mut h);for r in rows{r.text.hash(&mut h);r.key.hash(&mut h);}
        let key=h.finish();if let Some((k,w))=self.desktop_menu_width.get(){if k==key{return w;}}
        let width=self.menu_width_measured(rows);self.desktop_menu_width.set(Some((key,width)));width
    }
    fn menu_width_measured(&self,rows:&[Row])->usize {
        let lead=if rows.iter().any(|r|r.symbol.is_some()){self.ui.pt(20.)}else{0.};let need=rows.iter().filter(|r|!r.search).map(|r|{let t=self.measure_chrome(&r.text,13.,Weight::Regular);let k=r.key.as_ref().map_or(0.,|k|self.ui.pt(24.)+self.measure_chrome(&shortcut_spaced(k),13.,Weight::Regular));t+k}).fold(0.,f32::max)+self.ui.pt(32.)+lead;
        need.max(self.ui.pt(264.)).ceil() as usize
    }
    /// The row under `rel` px below the popup's top edge (separators count as rows, but have no action).
    fn menu_row_at(&self,menu:Menu,rel:f32)->Option<usize> {let (spans,_)=menu_spans(&self.menu_rows(menu));spans.iter().position(|(t,h)|{let t=self.ui.pt(*t);rel>=t&&rel<t+self.ui.pt(*h)})}
    pub(super) fn menu_row_of(&self,menu:Menu)->Option<usize> {self.desktop_menu_row.filter(|(m,_)|*m==menu).map(|(_,i)|i)}
    /// Move the highlight to the next row with an action in direction `dir`, wrapping (Full Keyboard Access, HIG "The menu bar").
    pub(super) fn menu_step(&self,menu:Menu,dir:i32)->Option<usize> {
        let selectable:Vec<bool>=self.menu_rows(menu).iter().map(|r|r.action.is_some()).collect();
        step_index(&selectable,self.menu_row_of(menu),dir)
    }
    /// Keys while a bar menu is open: Up/Down move, Left/Right switch menus, Return chooses, Esc closes. None when no menu is open.
    pub(super) fn desktop_menu_key(&mut self,code:u16,t:f64)->Option<Vec<Effect>> {
        let menu=self.desktop_menu?;if !self.desk_active()||self.is_locked(){return None;}
        if menu==Menu::Document{self.doc_key(code,t);return Some(vec![]);}
        if menu==Menu::Help {if let Some(fx)=self.help_key(code) {return Some(fx);}}
        let mut bar=vec![Menu::System,Menu::App];bar.extend(bar_after_app(self.app_menus(),self.photos_front()));
        self.dirty=true;
        // Left leaves the Recent Items submenu; Right opens the submenu of the highlighted row
        if menu==Menu::Recent&&code==105 {self.desktop_menu=Some(Menu::System);self.desktop_menu_row=None;return Some(vec![]);}
        if code==106 {if let Some(Action::Submenu(sub))=self.menu_row_of(menu).and_then(|i|self.menu_rows(menu).get(i).and_then(|r|r.action.clone())) {self.desktop_menu=Some(sub);self.desktop_menu_row=None;return Some(vec![]);}}
        match code {
            103|108=>{if let Some(i)=self.menu_step(menu,if code==108{1}else{-1}){self.desktop_menu_row=Some((menu,i));}},
            105|106=>{if let Some(at)=bar.iter().position(|m|*m==menu){let n=bar.len();self.desktop_menu=Some(bar[(at+if code==106{1}else{n-1})%n]);self.desktop_menu_row=None;self.help_reset();}},
            28|96=>{let action=self.menu_row_of(menu).and_then(|i|self.menu_rows(menu).get(i).and_then(|r|r.action.clone()));if let Some(action)=action{self.desktop_menu=None;self.desktop_menu_row=None;self.help_reset();return Some(self.run_menu_action(action,t));}},
            1=>{self.desktop_menu=None;self.desktop_menu_row=None;self.help_reset();},
            _=>{if let Some(c)=super::evkeys::char_for(code,false).filter(|c|c.is_alphanumeric()){self.menu_type_select(menu,c,t);}},
        }
        Some(vec![])
    }
    /// Type-to-select (`NativeMenu`): typing a title's first letters highlights it; the same letter again cycles. Uses the kit's `Menu` model.
    fn menu_type_select(&mut self,menu:Menu,c:char,t:f64) {
        use bifrost_ui::menu::{Menu as Model,MenuItem};
        let rows=self.menu_rows(menu);
        let items:Vec<MenuItem>=rows.iter().enumerate().map(|(i,r)|if r.sep{MenuItem::separator()}else{MenuItem::new(i as u32,r.text.trim_start_matches(|c:char|c=='\u{2713}'||c==' ')).enabled(r.action.is_some())}).collect();
        let mut model=self.desktop_menu_typed.take().unwrap_or_else(||Model::new(Vec::new()));
        model.items=items;model.highlighted=self.menu_row_of(menu);
        if let Some(i)=model.type_select(c,t){self.desktop_menu_row=Some((menu,i));}
        self.desktop_menu_typed=Some(model);
    }
    /// Pointer movement with a menu open: highlight the row under it, and a bar title switches the open menu (as Golden Gate does).
    pub(super) fn desktop_menu_hover(&mut self,x:f32,y:f32) {
        let Some(menu)=self.desktop_menu else{return;};if matches!(menu,Menu::Dock|Menu::Document){return;}
        // a context menu stays open when the pointer crosses the bar; only a bar menu follows the pointer to a neighbour
        if y<self.desktop_bar_height() as f32&&!matches!(menu,Menu::Context|Menu::Arrange) {
            // the Recent Items submenu sits under the System bar item: crossing that item is not a move to another menu
            if let Some(b)=self.menu_buttons().into_iter().find(|(_,l,w,_)|x>=*l&&x<*l+*w) {let here=if menu==Menu::Recent{Menu::System}else{menu};if b.0!=here&&b.0!=Menu::ControlCenter{self.desktop_menu=Some(b.0);self.desktop_menu_row=None;self.help_reset();self.dirty=true;}}
            return;
        }
        let row=self.desktop_popup_rect().and_then(|(l,t,w,h)|if x>=l as f32&&x<(l+w) as f32&&y>=t as f32&&y<(t+h) as f32{self.menu_row_at(menu,y-t as f32)}else{None}).filter(|i|self.menu_rows(menu).get(*i).is_some_and(|r|r.action.is_some()));
        // a Menu Items result opens the menu it lives in and highlights its item there, with the pointer marker
        if menu==Menu::Help {if let Some(i)=row {if let Some(Action::HelpItem(target,j))=self.menu_rows(menu).get(i).and_then(|r|r.action.clone()) {if target!=Menu::Help {self.help_reset();self.help_search.marker=true;self.desktop_menu=Some(target);self.desktop_menu_row=Some((target,j));self.dirty=true;return;}}}}
        let new=row.map(|i|(menu,i));if self.desktop_menu_row!=new{self.desktop_menu_row=new;self.dirty=true;}
    }
    /// Carry out a chosen menu row (a click or Return). The menu is already closed.
    pub(super) fn run_menu_action(&mut self,action:Action,t:f64)->Vec<Effect> {
        let mut effects=Vec::new();match action {
                    Action::Settings(page)=>{self.desk_open(AppId::Settings,t);if !page.is_empty(){if let Some(st)=&mut self.settings {effects.extend(st.open_page(&self.ui,page,t));}}},
                    Action::Context(slot,id)=>{if id>=super::sharing::MENU_BASE&&self.share_choose(id,t){self.desktop_context=None;}else if self.screen==Screen::App(AppId::External(slot)){if let Some(Some(h))=self.ext.get_mut(slot as usize){h.send(&bifrost_proto::ToApp::ContextAction(id));}}self.desktop_context=None;},
                    Action::StatusItem(slot,item,id)=>{if let Some(Some(h))=self.ext.get_mut(slot as usize){h.send(&bifrost_proto::ToApp::StatusItemAction{item,id});}},
                    Action::AppMenu(slot,id)=>{if self.screen==Screen::App(AppId::External(slot)){if let Some(Some(h))=self.ext.get_mut(slot as usize){h.input(bifrost_proto::WindowInput::MenuAction(id));}}},
                    Action::Dock(change)=>effects.extend(self.dock_change(change,t)),
                    Action::QuickAction(slot,index)=>{if let Some(item)=self.quick_actions_for(slot as usize).get(index as usize).cloned(){self.record_quick_action(slot as usize,&item.kind);if let Some(Some(h))=self.ext.get_mut(slot as usize){h.send(&bifrost_proto::ToApp::QuickAction{kind:item.kind,user_info:item.user_info});}}},
                    Action::DockMenu(slot,id)=>{if let Some(Some(h))=self.ext.get_mut(slot as usize){h.send(&bifrost_proto::ToApp::ContextAction(id));}},
                    Action::Arrange(app,what)=>self.desk_arrange(app,what,t),
                    Action::Tile(t)=>{if self.foreign_active().is_none(){self.desk_tile_front(t);}},
                    Action::RestoreSize=>{if self.foreign_active().is_none(){self.desk_restore_front();}},
                    Action::CycleWindow=>{self.desk_cycle_window();let t=self.last_input;self.cmdtab_release(t);},
                    Action::Script(i)=>{if let Some(dir)=super::scripts_menu::folder(){if let Some(name)=super::scripts_menu::list(&dir).get(i as usize){if let Err(e)=super::scripts_menu::run(&dir,name){eprintln!("springboard: script {name}: {e}");}}}self.dirty=true;},
                    Action::InputSource(i)=>{if let Some(l)=super::kb_layouts::Layout::ALL.get(i as usize) {super::kb_layouts::set_active(*l);self.input_layout_selected(*l);self.show_input_source(*l,t);}self.dirty=true;},
                    Action::InputMethod(id)=>{let commits=self.input_providers.select(&id);self.apply_provider_commits(commits);},
                    Action::Zoom=>{if let Some(id)=self.foreign_active(){let max=self.desk.as_ref().is_some_and(|d|d.native_windows.iter().chain(d.linux_apps.iter().flat_map(|a|a.windows.iter())).any(|w|w.id==id&&w.maximized));effects.push(Effect::ForeignWindowMaximize(id,!max));}else if let Some(i)=self.desk.as_ref().and_then(|d|d.wins.iter().rposition(|w|!w.minimized)){self.desk_zoom_window(i);}},
                    Action::Foreign(id)=>effects.push(Effect::ActivateForeign(id)),
                    Action::Print=>self.print_sheet_open(t),
                    Action::Service(i)=>{if let Some(service)=bifrost_ui::services_menu::Service::ALL.get(i as usize).copied(){effects.extend(self.run_service(service,t));}else{self.run_app_service(i as usize,t);}},
                    Action::CharacterViewer=>self.char_viewer_open(),
                    Action::Help(id)=>self.open_help(&id,None),
                    Action::KeyboardViewer=>self.keyboard_viewer_open(),
                    Action::OnScreenKeyboard=>{if self.keyboard_opened_explicitly(){self.keyboard_close_explicit(t);}else{self.keyboard_open_explicit(t);}},
                    Action::Submenu(menu)=>{self.desktop_menu=Some(menu);self.desktop_menu_row=None;self.dirty=true;},
                    Action::Window(kind)=>self.utility_open(kind),
                    Action::Recent(n)=>{if let Some(app)=self.recent_app_at(n as usize){self.desk_open(app,t);}},
                    Action::ClearRecent=>self.recent_clear(),
                    Action::Desktop=>self.desk_show_desktop(),
                    Action::Open(super::AppId::ExternalWindow(slot,id))=>{let _=self.desk_focus_extra_window(slot,id);},
                    Action::Open(app)=>self.desk_open(app,t),
                    Action::ShowAll=>{if let Some(d)=&mut self.desk {for w in &mut d.wins {w.minimized=false;}}self.desk_refocus();},
                    Action::Close|Action::Minimize=>effects.extend(self.desk_window_close_or_minimize(matches!(action,Action::Minimize))),
                    Action::Quit=>effects.extend(self.desk_quit_front()),
                    Action::Command(command)=>{if let Screen::App(AppId::External(n))=self.screen {if let Some(Some(h))=self.ext.get_mut(n as usize){if h.commands.supports(command){h.input(bifrost_proto::WindowInput::Command(command));}}}},
                    Action::Shortcut(code)=>effects.push(Effect::DesktopShortcut(code)),
                    Action::Term(key)=>effects.extend(self.terminal_hw_key(key,t)),
                    Action::Wifi(on)=>desktop_network::send(desktop_network::Request::Wifi(super::WifiAction::Enable(on))),
                    Action::Scan=>desktop_network::send(desktop_network::Request::Wifi(super::WifiAction::Scan)),
                    Action::Select(ssid)=>desktop_network::send(desktop_network::Request::Wifi(super::WifiAction::Select(ssid))),
                    Action::Power(profile)=>desktop_network::send(desktop_network::Request::Power(profile)),
                    Action::Lock=>effects.extend(self.sleep_screen()),
                    Action::Sleep=>{effects.extend(self.sleep_screen());effects.push(super::Effect::SleepSystem)}
                    Action::Halt(power)=>effects.push(Effect::Power(power)),
                    Action::Media(command)=>{super::services::now_playing::send(command);},
                    Action::Bluetooth(command)=>effects.push(Effect::Bluetooth(command)),
                    Action::Portal(url)=>{let _=self.desktop_open_url(&url,t);},
                    Action::RecordStop=>self.recording_stop_from_menu(),
                    Action::GameModeOff=>{self.game_mode_turn_off();}
                    Action::LowPowerMode(on)=>self.set_low_power(on),
                    Action::Photo(command)=>effects.extend(self.photo_command(command,t)),
                    Action::PhotoEditWith(key)=>self.photo_edit_with_viewed(&key,t),
                    Action::HelpItem(menu,i)=>{if let Some(a)=self.plain_rows(menu).get(i).and_then(|r|r.action.clone()) {effects.extend(self.run_menu_action(a,t));}},
                    Action::HelpTopic(app,anchor)=>self.open_help(&app,anchor.as_deref()),
                }effects
    }
    /// Runs a Photos command from a menu row or a key equivalent, and opens the print sheet when it asked for one.
    pub(super) fn photo_command(&mut self,command:super::photos::commands::PhotoCommand,t:f64)->Vec<Effect>{
        if let Some(p)=self.photos.as_mut(){p.command(&self.ui,command,t);}
        self.photo_print_taken();
        self.dirty=true;
        Vec::new()
    }
    /// Opens the print sheet over the picture a Print command handed over (the picture is dropped without a print sheet).
    #[cfg(feature = "desktop-wayland")]
    fn photo_print_taken(&mut self) {
        if let Some(image)=self.photos.as_mut().and_then(|p|p.take_print()){self.print_image_open(image);}
    }
    #[cfg(not(feature = "desktop-wayland"))]
    fn photo_print_taken(&mut self) {
        if let Some(p)=self.photos.as_mut(){let _=p.take_print();}
    }
    /// The Image menu of Photos: what the open photo, or the selection, can do.
    fn photo_image_rows(&self)->Vec<Row>{
        use super::photos::commands::PhotoCommand as P;
        let Some(p)=self.photos.as_ref() else {return Vec::new();};
        let mut v=vec![photo_row(p,P::Favourite),photo_row(p,P::Hide),Row::separator()];
        v.extend([0,1,2,3,4,5].into_iter().map(|n|photo_row(p,P::Rate(n))));
        v.push(Row::separator());
        v.push(photo_row(p,P::Info));
        v.push(Row::separator());
        v.extend([P::RotateLeft,P::RotateRight,P::FlipHorizontal,P::FlipVertical,P::AutoEnhance].into_iter().map(|c|photo_row(p,c)));
        v.push(Row::separator());
        v.push(photo_row(p,P::RevertToOriginal));
        v.push(Row::separator());
        v.extend([P::Duplicate,P::CopyEdits,P::PasteEdits].into_iter().map(|c|photo_row(p,c)));
        v.push(Row::separator());
        let edit_with=self.photo_edit_with_rows();
        if !edit_with.is_empty(){v.extend(edit_with);v.push(Row::separator());}
        v.push(photo_row(p,P::MergeDuplicates));
        v.push(photo_row(p,P::Delete));
        v
    }
    /// Image > Edit With: one row per app that opens the open photo's type (not Photos), the default first.
    fn photo_edit_with_rows(&self)->Vec<Row>{
        let Some(photo)=self.photos.as_ref().and_then(|p|p.viewing()) else {return Vec::new();};
        self.edit_with_choices(&photo.name()).into_iter().map(|c|Row::action(format!("Edit With {}",super::defaultapps::app_name(&c.app)),Action::PhotoEditWith(c.app))).collect()
    }
    /// The File, Edit and View menus while Photos is in front: Photos' own commands lead File and View, and its Edit
    /// menu replaces the shell's. The shell's Print row is Photos' own, so it is dropped from File.
    fn photo_overlay(&self,menu:Menu,rows:Vec<Row>)->Vec<Row>{
        use super::photos::commands::PhotoCommand as P;
        let Some(p)=self.photos.as_ref() else {return rows;};
        match menu {
            Menu::File=>{
                let mut v=vec![photo_row(p,P::NewAlbum),photo_row(p,P::RenameAlbum),photo_row(p,P::DeleteAlbum),Row::separator(),photo_row(p,P::Import),photo_row(p,P::SwitchLibrary),photo_row(p,P::Export),photo_row(p,P::Print),Row::separator(),photo_row(p,P::PlaySlideshow),Row::separator()];
                let mut rest:Vec<Row>=rows.into_iter().filter(|r|!r.text.starts_with("Print")).collect();
                while rest.last().is_some_and(|r|r.sep){rest.pop();}
                v.extend(rest);
                v
            }
            Menu::Edit=>{
                let mut v=vec![photo_row(p,P::Undo),photo_row(p,P::Redo),Row::separator(),photo_row(p,P::Copy),Row::separator(),photo_row(p,P::SelectAll),photo_row(p,P::DeselectAll),Row::separator(),photo_row(p,P::Find),Row::separator()];
                v.push(Row::action("Emoji & Symbols",Action::CharacterViewer));
                v
            }
            Menu::View=>{
                let mut v=vec![photo_row(p,P::ZoomIn),photo_row(p,P::ZoomOut),Row::separator(),photo_row(p,P::NewestFirst),photo_row(p,P::OldestFirst),photo_row(p,P::AlbumsByName),Row::separator(),photo_row(p,P::ShowAll),photo_row(p,P::FavouritesOnly),photo_row(p,P::EditedOnly),photo_row(p,P::ScreenshotsOnly),photo_row(p,P::FilterDate),Row::separator(),photo_row(p,P::ShowHidden),photo_row(p,P::AspectGrid),Row::separator(),Row::action("Enter Full Screen",Action::Zoom).key("ctrl+cmd+f"),Row::separator()];
                v.extend(rows);
                v
            }
            _=>rows,
        }
    }
    /// Close or minimize the front window (the File menu and Ctrl+W / Ctrl+M).
    pub(super) fn desk_window_close_or_minimize(&mut self,minimize:bool)->Vec<Effect> {
        let mut effects=Vec::new();
        if let Some(id)=self.foreign_active(){effects.push(Effect::ForeignWindowAction(id,minimize));}else if let Some(app)=self.desk.as_ref().and_then(|d|d.front()).map(|w|w.app){if minimize {let t=self.last_input;if let Some(i)=self.desk.as_ref().and_then(|d|d.index_of(app)){self.desk_minimize_window(i,t);}else{self.desk_refocus();}}else{self.desk_close_app(app);}}
        effects
    }
    /// Quits the front app (Cmd+Q and the App menu's Quit); `quit_step` decides what that means for the window in front.
    pub(super) fn desk_quit_front(&mut self)->Vec<Effect> {
        let step={
            let desk=self.desk.as_ref();
            let foreign:Vec<(u32,&str,bool)>=desk.map(|d|d.native_windows.iter().chain(d.linux_apps.iter().flat_map(|a|a.windows.iter())).map(|w|(w.id,w.app_id.as_str(),w.active)).collect()).unwrap_or_default();
            let installed=super::apphost::installed();
            let sdk_slot=|app_id:&str|installed.iter().position(|a|a.manifest.id==app_id).filter(|slot|self.ext.get(*slot).and_then(Option::as_ref).is_some_and(|h|h.running()&&h.route()==super::apphost::Route::Direct)).map(|slot|slot as u8);
            quit_step(&foreign,sdk_slot,desk.and_then(|d|d.front()).map(|w|w.app))
        };
        match step {
            QuitStep::App(slot)=>{self.desk_quit_app(slot);Vec::new()}
            QuitStep::Windows(ids)=>ids.into_iter().map(|id|Effect::ForeignWindowAction(id,false)).collect(),
            QuitStep::Window(app)=>{self.desk_close_app(app);Vec::new()}
            QuitStep::Nothing=>Vec::new(),
        }
    }
    /// Control Center (chrome id 4) and Notification Center (id 5) as overlay surfaces above every window, each with
    /// room for its shadow: drawn only into the desktop frame they would sit under the windows of the session.
    fn desk_panel_overlays(&self)->Vec<(u32,usize,usize,usize,usize)> {
        let g=self.ui.pt(super::desk::SHADOW_REACH_PT);
        let bar=self.desktop_bar_height();
        // the surface starts under the menu bar: the shadow margin above a panel must not cover the bar's items
        [(4,self.cc_desk_rect()),(5,self.center_desk_rect())].into_iter().filter_map(|(id,r)|r.map(|r|(id,panel_overlay_rect(r,g)))).map(|(id,(x,y,w,h))|{let top=y.max(bar);(id,x,top,w,(y+h).saturating_sub(top))}).collect()
    }
    #[cfg(feature="desktop-wayland")]
    pub fn desktop_chrome_frames(&self,pixels:&[u32],width:usize,height:usize)->Vec<crate::bare::wayland::ChromeFrame> {
        if !self.desk_active()||self.is_locked()||self.anim.is_some(){return vec![];}
        // The popup (id 1) is exported with room for its shadow: a crop at the menu's own edge cut the shadow off, so a
        // menu over white content had no edge at all (E2E N6). Only the menu itself takes the pointer (`input`); a click
        // on the shadow reaches the window below, as when the surface was the menu's size.
        let popup=self.desktop_popup_rect().map(|p|popup_surface_rect(p,self.desktop_bar_height() as i32,self.ui.pt(super::desk::SHADOW_REACH_PT) as i32+2,self.ui.pt(2.) as i32,(width,height)));
        let mut frames=Vec::new();for (id,x,y,w,h) in std::iter::once((0,0,0,width,self.desktop_bar_height())).chain({let r=self.dock_overlay_rect();std::iter::once((2,r.x.max(0) as usize,r.y.max(0) as usize,r.w.max(0) as usize,r.h.max(0) as usize))}).chain(popup.map(|(r,_)|(1,r.0,r.1,r.2,r.3))).chain(self.desk_panel_overlays()) {
            let w=w.min(width.saturating_sub(x));let h=h.min(height.saturating_sub(y));if w==0||h==0{continue;}let mut data=Vec::with_capacity(w*h);for row in y..y+h {data.extend_from_slice(&pixels[row*width+x..row*width+x+w]);}if id==2&&!self.dock_is_visible(){data.fill(0);}let input=if id==1 {popup.map(|(_,menu)|menu)}else{None};frames.push(crate::bare::wayland::ChromeFrame{id,x:x as i32,y:y as i32,width:w,height:h,pixels:data,input});
        }
        // Search: id 3, an overlay surface above every window that takes the keyboard while it is open
        if let Some(((x,y,ow,oh),mask))=self.quicksearch.desk_overlay(&self.ui,self.w) {
            let w=ow.min(width.saturating_sub(x));let h=oh.min(height.saturating_sub(y));
            if w>0&&h>0 {
                let mut data=Vec::with_capacity(w*h);for row in y..y+h {data.extend_from_slice(&pixels[row*width+x..row*width+x+w]);}
                let mask=crop_mask(&mask,ow,w,h);
                let mut image=Image{w,h,px:data};image.apply_mask(&mask);
                frames.push(crate::bare::wayland::ChromeFrame{id:3,x:x as i32,y:y as i32,width:w,height:h,pixels:image.px,input:None});
            }
        }
        // the assistant panel: id 6, an overlay surface like Search (its keyboard is exclusive while it is open)
        if let Some(((x,y,ow,oh),mask))=self.assistant.desk_overlay(&self.ui,self.w) {
            let w=ow.min(width.saturating_sub(x));let h=oh.min(height.saturating_sub(y));
            if w>0&&h>0 {
                let mut data=Vec::with_capacity(w*h);for row in y..y+h {data.extend_from_slice(&pixels[row*width+x..row*width+x+w]);}
                let mask=crop_mask(&mask,ow,w,h);
                let mut image=Image{w,h,px:data};image.apply_mask(&mask);
                frames.push(crate::bare::wayland::ChromeFrame{id:6,x:x as i32,y:y as i32,width:w,height:h,pixels:image.px,input:None});
            }
        }
        // Quick Look: id 7, an overlay surface above every window (its keys come from the shell's own keyboard reader)
        if let Some(((x,y,w,h),mask))=self.quick_look_overlay() {
            let mut data=Vec::with_capacity(w*h);for row in y..y+h {data.extend_from_slice(&pixels[row*width+x..row*width+x+w]);}
            let mut image=Image{w,h,px:data};image.apply_mask(&mask);
            frames.push(crate::bare::wayland::ChromeFrame{id:7,x:x as i32,y:y as i32,width:w,height:h,pixels:image.px,input:None});
        }
        // The Cmd+Tab strip: id 13, an overlay surface above every window, listed only while it is open. It takes neither
        // the pointer (an empty input region, as the sweep's id 10) nor the keyboard (never exclusive): the modifier release
        // that commits the switch comes from the shell's own keyboard reader, so the strip must not pull the keys away.
        if let Some(((x,y,w,h),mask))=self.cmdtab_overlay() {
            let mut data=Vec::with_capacity(w*h);for row in y..y+h {data.extend_from_slice(&pixels[row*width+x..row*width+x+w]);}
            let mut image=Image{w,h,px:data};image.apply_mask(&mask);
            frames.push(crate::bare::wayland::ChromeFrame{id:13,x:x as i32,y:y as i32,width:w,height:h,pixels:image.px,input:Some([0,0,0,0])});
        }
        // The system menu's card (Force Quit, About, Log Out): an overlay surface above every window, like the file preview. It takes the
        // pointer over its whole card; its keys come from the shell's own keyboard reader.
        if let Some(((x,y,w,h),mask))=self.utility_overlay() {
            let mut data=Vec::with_capacity(w*h);for row in y..y+h {data.extend_from_slice(&pixels[row*width+x..row*width+x+w]);}
            let mut image=Image{w,h,px:data};image.apply_mask(&mask);
            frames.push(crate::bare::wayland::ChromeFrame{id:super::utility_window::CHROME_ID,x:x as i32,y:y as i32,width:w,height:h,pixels:image.px,input:None});
        }
        // The Apps grid: id 8, an overlay surface above every window that takes the keyboard while it is
        // open. It spans the veil only, so the menu bar and the Dock stay their own surfaces and stay usable. It is
        // listed only while the grid is open: the frame it closes in drops the surface (E2E L1, OV1).
        if let Some((x,y,w,h))=self.overview.grid.overlay_rect(self) {
            let w=w.min(width.saturating_sub(x));let h=h.min(height.saturating_sub(y));
            if w>0&&h>0 {
                let mut data=Vec::with_capacity(w*h);for row in y..y+h {data.extend_from_slice(&pixels[row*width+x..row*width+x+w]);}
                // the surface spans the veil so a click anywhere closes the grid, but only the capsule and the panel are opaque
                let mask=self.overview.grid.overlay_mask(self,(x,y,w,h));
                let mut image=Image{w,h,px:data};image.apply_mask(&mask);
                frames.push(crate::bare::wayland::ChromeFrame{id:8,x:x as i32,y:y as i32,width:w,height:h,pixels:image.px,input:None});
            }
        }
        // Overview: id 9, an overlay surface above every window that takes the keyboard while it is open,
        // on the same rectangle as the Apps grid (the menu bar and the Dock stay their own surfaces). Listed only while
        // Overview is open or animating; the frame it ends in drops the surface. A desktop switch's sweep is id 10: the same
        // rectangle with an empty input region, so it takes neither the pointer nor the keyboard.
        for (id,rect,input) in [(9,self.overview.overlay_rect(self),None),(10,self.overview.slide_overlay_rect(self),Some([0,0,0,0]))] {
            if let Some((x,y,w,h))=rect {
                let w=w.min(width.saturating_sub(x));let h=h.min(height.saturating_sub(y));
                if w>0&&h>0 {
                    let mut data=Vec::with_capacity(w*h);for row in y..y+h {data.extend_from_slice(&pixels[row*width+x..row*width+x+w]);}
                    frames.push(crate::bare::wayland::ChromeFrame{id,x:x as i32,y:y as i32,width:w,height:h,pixels:data,input});
                }
            }
        }
        // the screenshot toolbar's session (id 11, the whole screen) and the capture thumbnail (id 12): overlay surfaces above
        // the windows, drawn on their own ground so nothing of them is left behind (E2E SC3)
        // the system sheets and alerts (id 15, the whole screen): a surface above the windows that takes the pointer and the
        // keyboard until the sheet is answered. Drawn into the shell's own frame, under the windows, a sheet was hidden
        // behind them and left its veil on the wallpaper (MA6).
        for (id,layer) in [(11,&self.capture_layer),(12,&self.shot_layer),(MODAL_CHROME_ID,&self.modal_layer)] {
            if let Some(l)=layer {
                if l.image.w>0&&l.image.h>0 {frames.push(crate::bare::wayland::ChromeFrame{id,x:l.x,y:l.y,width:l.image.w,height:l.image.h,pixels:l.image.px.clone(),input:l.input});}
            }
        }
        // a tablet's on-screen keyboard: id 16 (7 is Quick Look), an overlay surface above every window from its resting top to the bottom
        // edge (drawn only into the desktop frame, the windows covered it). It stays up for the whole slide: the frame
        // is clear above the sliding keyboard, so the surface keeps one size and the slide plays inside it.
        if self.desk_keyboard_visible() {
            let top=(self.kb_top().max(0.0) as usize).min(height);
            if top<height&&width>0 {
                let data=pixels[top*width..height*width].to_vec();
                frames.push(crate::bare::wayland::ChromeFrame{id:16,x:0,y:top as i32,width,height:height-top,pixels:data,input:None});
            }
        }
        frames
    }
}

/// The top-left `w` x `h` of a mask that is `stride` px wide (its own width, before the card was clipped to the screen).
/// The stride is the mask's width, not its length over the clipped height: a card that runs off the bottom of the
/// screen has fewer rows than the mask, and dividing by them sheared the rows.
#[cfg(feature="desktop-wayland")]
fn crop_mask(mask:&[u8],stride:usize,w:usize,h:usize)->Vec<u8> {
    (0..w*h).map(|i|mask.get((i/w)*stride+i%w).copied().unwrap_or(0)).collect()
}

#[cfg(all(test,feature="desktop-wayland"))]
mod crop_tests {
    use super::crop_mask;
    #[test]
    fn a_clipped_card_keeps_its_mask_rows() {
        // a 4 x 3 mask, rows 0..3 hold 1, 2, 3
        let mask=[1,1,1,1, 2,2,2,2, 3,3,3,3];
        // clipped to 4 x 2 (the bottom row ran off the screen): rows 1 and 2 stay whole
        assert_eq!(crop_mask(&mask,4,4,2),vec![1,1,1,1,2,2,2,2]);
        // clipped to 3 x 3 (the right side ran off): the row stride is still 4
        assert_eq!(crop_mask(&mask,4,3,3),vec![1,1,1,2,2,2,3,3,3]);
    }
}

/// The menu bar for the element map (`axmap`): the six app menus with their rows. The first menu is `App` (its title is the
/// front app's name, which changes), the rest are named as shown. Separators are left out; a disabled row has no action.
#[cfg(feature = "desktop-wayland")]
/// The shell's own menus an app menu of the same title merges into (`File`, `Edit`, `View`, `Window`, `Help`).
pub(super) fn shell_menu(title:&str)->Option<Menu> {
    [(Menu::File,"File"),(Menu::Edit,"Edit"),(Menu::View,"View"),(Menu::Window,"Window"),(Menu::Help,"Help")].into_iter().find(|(_,t)|t.eq_ignore_ascii_case(title.trim())).map(|(m,_)|m)
}
#[cfg(feature = "desktop-wayland")]
const AX_MENUS: [(Menu, &str); 7] = [(Menu::System, "System"), (Menu::App, "App"), (Menu::File, "File"), (Menu::Edit, "Edit"), (Menu::View, "View"), (Menu::Window, "Window"), (Menu::Help, "Help")];
/// Without the desktop there is no menu bar to map.
#[cfg(not(feature = "desktop-wayland"))]
impl Shell {
    pub(super) fn ax_menus(&self) -> Vec<(usize, &'static str, Vec<(usize, String, bool)>)> {
        Vec::new()
    }
    pub(super) fn ax_menu_press(&mut self, _menu: usize, _row: usize, _t: f64) -> Result<Vec<super::Effect>, String> {
        Err("there is no menu bar".into())
    }
}
#[cfg(feature = "desktop-wayland")]
impl Shell {
    pub(super) fn ax_menus(&self) -> Vec<(usize, &'static str, Vec<(usize, String, bool)>)> {
        if self.desk.is_none() {
            return Vec::new();
        }
        AX_MENUS
            .iter()
            .enumerate()
            .map(|(m, (menu, name))| {
                let rows = self.menu_rows(*menu).into_iter().filter(|r| !r.sep).enumerate().map(|(i, r)| (i, r.text, r.action.is_some())).collect();
                (m, *name, rows)
            })
            .collect()
    }

    /// Chooses row `row` (counting without separators) of menu `menu` as a click would.
    pub(super) fn ax_menu_press(&mut self, menu: usize, row: usize, t: f64) -> Result<Vec<Effect>, String> {
        let (m, _) = *AX_MENUS.get(menu).ok_or("no such menu")?;
        let action = self.menu_rows(m).into_iter().filter(|r| !r.sep).nth(row).and_then(|r| r.action).ok_or("that menu row is not available")?;
        self.desktop_menu = None;
        self.desktop_menu_row = None;
        Ok(self.run_menu_action(action, t))
    }
}

/// Colours of the menu bar's titles, status items and open-title capsule.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg(feature = "desktop-wayland")]
pub(super) struct BarInk { pub text: u32, pub text_alpha: u32, pub shadow: u32, pub shadow_alpha: u32, pub capsule: u32 }

/// True when the wallpaper behind the menu bar (its top `rows` rows) is light enough that the bar needs dark text.
/// Lightness is the mean Rec. 709 luma (0..255) of every other pixel in the band, against `BAR_LIGHT_LUMA`.
pub(super) fn wallpaper_is_light(wp: crate::gfx::View, rows: usize) -> bool {
    let (mut sum, mut n) = (0u64, 0u64);
    for y in (0..rows.min(wp.h)).step_by(2) {
        for &p in wp.px[y * wp.w..(y + 1) * wp.w].iter().step_by(2) {
            let (r, g, b) = ((p & 0xff) as u64, (p >> 8 & 0xff) as u64, (p >> 16 & 0xff) as u64);
            sum += (2126 * r + 7152 * g + 722 * b) / 10_000;
            n += 1;
        }
    }
    n > 0 && sum / n >= BAR_LIGHT_LUMA
}

/// Mean luma above which white menu bar text loses contrast and the bar switches to dark text.
const BAR_LIGHT_LUMA: u64 = 160;

#[cfg(test)]
mod bar_ink_tests {
    use super::wallpaper_is_light;
    use crate::gfx::{pack, View};

    fn view(w: usize, h: usize, f: impl Fn(usize) -> u32) -> Vec<u32> {
        (0..w * h).map(|i| f(i / w)).collect()
    }

    #[test]
    fn white_and_light_grey_wallpapers_get_dark_bar_text() {
        let px = view(64, 40, |_| pack(255, 255, 255, 255));
        assert!(wallpaper_is_light(View { w: 64, h: 40, px: &px }, 8));
        let px = view(64, 40, |_| pack(200, 200, 200, 255));
        assert!(wallpaper_is_light(View { w: 64, h: 40, px: &px }, 8));
    }

    #[test]
    fn dark_and_mid_grey_wallpapers_keep_white_bar_text() {
        let px = view(64, 40, |_| pack(0, 0, 0, 255));
        assert!(!wallpaper_is_light(View { w: 64, h: 40, px: &px }, 8));
        let px = view(64, 40, |_| pack(128, 128, 128, 255));
        assert!(!wallpaper_is_light(View { w: 64, h: 40, px: &px }, 8));
    }

    #[test]
    fn only_the_band_under_the_bar_counts() {
        // a light sky over a dark landscape: the bar sits on the sky
        let px = view(64, 40, |y| if y < 8 { pack(235, 225, 210, 255) } else { pack(10, 10, 10, 255) });
        assert!(wallpaper_is_light(View { w: 64, h: 40, px: &px }, 8));
        let px = view(64, 40, |y| if y < 8 { pack(10, 10, 10, 255) } else { pack(250, 250, 250, 255) });
        assert!(!wallpaper_is_light(View { w: 64, h: 40, px: &px }, 8));
    }

    #[test]
    fn green_weighs_more_than_blue() {
        // perceived lightness, not a plain channel average: pure green is light, pure blue is dark
        let px = view(16, 8, |_| pack(0, 255, 0, 255));
        assert!(wallpaper_is_light(View { w: 16, h: 8, px: &px }, 8));
        let px = view(16, 8, |_| pack(0, 0, 255, 255));
        assert!(!wallpaper_is_light(View { w: 16, h: 8, px: &px }, 8));
    }

    #[test]
    fn an_empty_wallpaper_is_not_light() {
        assert!(!wallpaper_is_light(View { w: 0, h: 0, px: &[] }, 8));
    }
}

#[cfg(test)]
mod menu_damage_tests {
    use super::{popup_damage_rect, popup_surface_rect};

    const SCREEN: (usize, usize) = (1000, 800);

    #[test]
    fn the_exported_surface_is_the_popup_and_its_shadow() {
        // the damage rectangle of the popup at (100, 34) 200 x 150 is (88, 28, 224, 170)
        let (surface, menu) = popup_surface_rect((100, 34, 200, 150), 28, 12, 2, SCREEN);
        assert_eq!(surface, (88, 28, 224, 170));
        assert_eq!(menu, [12, 6, 200, 150], "the popup inside it: 12 px of shadow to its left, 6 px up to the bar");
    }

    #[test]
    fn the_surface_is_clipped_to_the_screen_and_the_menu_rectangle_follows() {
        // at the left edge the shadow is cut at x 0, so the popup starts 5 px in
        let (surface, menu) = popup_surface_rect((5, 34, 100, 40), 28, 12, 2, SCREEN);
        assert_eq!(surface, (0, 28, 117, 60));
        assert_eq!(menu, [5, 6, 100, 40]);
        // at the right edge it is cut at the screen's width: 888 + 112 = 1000
        let (surface, menu) = popup_surface_rect((900, 34, 90, 40), 28, 12, 2, SCREEN);
        assert_eq!(surface, (888, 28, 112, 60));
        assert_eq!(menu, [12, 6, 90, 40]);
        // at the bottom it is cut at the screen's height: 748 + 52 = 800
        let (surface, menu) = popup_surface_rect((100, 760, 200, 30), 28, 12, 2, SCREEN);
        assert_eq!(surface, (88, 748, 224, 52));
        assert_eq!(menu, [12, 12, 200, 30]);
    }

    #[test]
    fn the_menu_rectangle_stays_inside_the_surface_and_is_smaller_than_it() {
        for popup in [(100, 200, 264, 150), (0, 34, 300, 100), (700, 600, 300, 200)] {
            let ((sx, sy, sw, sh), menu) = popup_surface_rect(popup, 28, 12, 2, SCREEN);
            assert!(menu[0] >= 0 && menu[1] >= 0, "{popup:?}: the menu starts inside the surface");
            assert!(menu[0] + menu[2] <= sw as i32 && menu[1] + menu[3] <= sh as i32, "{popup:?}: and ends inside it");
            assert_eq!((sx as i32 + menu[0], sy as i32 + menu[1]), (popup.0 as i32, popup.1 as i32), "{popup:?}: at the popup's screen position");
            assert!(sw as i32 > menu[2] && sh as i32 > menu[3], "{popup:?}: there is shadow to show");
        }
    }

    // A menu bar 28 px tall, a shadow reach of 12 px (10 pt + 2 at scale 1) and a 2 px lip.
    #[test]
    fn a_menu_below_the_bar_owes_its_shadow_up_to_the_bar() {
        // popup at (100, 34) 200 x 150: the top shadow would start at y 22, clamped to the bar's bottom (28);
        // left 88, right 100 + 200 + 12 = 312 (width 224); bottom 34 + 150 + 12 + 2 = 198 (height 170)
        assert_eq!(popup_damage_rect((100, 34, 200, 150), 28, 12, 2), (88, 28, 224, 170));
    }

    #[test]
    fn a_popup_lower_on_screen_starts_its_damage_above_its_top_edge() {
        // top shadow starts at 70 - 12 = 58, which is inside the gap under the bar, so it is damaged
        let (_, top, _, _) = popup_damage_rect((10, 70, 100, 80), 28, 12, 2);
        assert_eq!(top, 58);
    }

    #[test]
    fn a_menu_at_the_left_edge_is_clamped_to_the_screen() {
        // x 5 - 12 is negative, so the rectangle starts at 0 and reaches 5 + 100 + 12 = 117 (width 117);
        // top 34 - 12 = 22 clamps to 28; bottom 34 + 40 + 12 + 2 = 88 (height 60)
        assert_eq!(popup_damage_rect((5, 34, 100, 40), 28, 12, 2), (0, 28, 117, 60));
    }

    #[test]
    fn the_rectangle_always_contains_the_popup() {
        let (x, y, w, h) = popup_damage_rect((300, 34, 180, 220), 28, 12, 2);
        assert!(x <= 300 && y <= 34);
        assert!(x + w >= 300 + 180 && y + h >= 34 + 220);
    }
}

/// A panel's overlay rectangle in whole px: the panel grown by its shadow reach `g`, clipped at the screen's top-left.
#[cfg(any(test, feature = "desktop-wayland"))]
fn panel_overlay_rect((x,y,w,h):(f32,f32,f32,f32),g:f32)->(usize,usize,usize,usize) {
    let (x0,y0)=((x-g).max(0.),(y-g).max(0.));
    (x0 as usize,y0 as usize,(x+w+g-x0).max(0.).ceil() as usize,(y+h+g-y0).max(0.).ceil() as usize)
}

#[cfg(all(test, feature = "desktop-wayland"))]
mod chrome_cache_tests {
    use crate::shell::ui::Weight;

    #[test]
    fn the_bar_lays_out_from_cached_widths_the_same_way_twice() {
        let sh = crate::shell::tests::desktop_shell();
        let first = sh.menu_buttons();
        assert_eq!(first, sh.menu_buttons(), "a frame that lays the bar out again gets the same menus");
        assert_eq!(sh.measure_chrome("Control", 13.0, Weight::Semibold), sh.ui.measure("Control", 13.0, Weight::Semibold));
        assert_eq!(sh.fit_chrome("a very long network name indeed", 13.0, 40.0), sh.ui.fit("a very long network name indeed", 13.0, 40.0));
    }
}

#[cfg(test)]
mod panel_overlay_tests {
    #[test]
    fn a_panel_overlay_covers_the_panel_and_its_shadow() {
        assert_eq!(super::panel_overlay_rect((100.,50.,200.,300.),10.),(90,40,220,320));
        assert_eq!(super::panel_overlay_rect((5.,5.,20.,20.),10.),(0,0,35,35),"clipped at the top-left corner");
    }

    /// E2E NC2: the column opens under the menu bar, and its overlay surface (shadow margin included) starts there too,
    /// so the bar's status items and clock stay visible and take their clicks.
    #[cfg(feature="desktop-wayland")]
    #[test]
    fn a_panel_overlay_starts_under_the_menu_bar() {
        let mut sh=crate::shell::tests::desktop_shell();
        sh.anim=None;
        // the fixture starts on the lock screen, where the column does not open
        sh.screen=crate::shell::Screen::Home;
        sh.open_desk_notification_center(1.0);
        let bar=sh.desktop_bar_height();
        let overlays=sh.desk_panel_overlays();
        let column=overlays.iter().find(|o|o.0==5).expect("the open column is an overlay surface");
        assert!(column.2>=bar,"the surface starts at y {} under the {bar} px bar",column.2);
        assert!(column.4>0,"and has height");
        let top=sh.center_desk_rect().expect("the column is open").1;
        assert!(top>=bar as f32,"the column itself is under the bar");
    }
}

/// One row of the Help menu's results: a group heading, a Menu Items hit (an index into the menu items), or a
/// Help Topics hit (an index into the topics).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum HelpRow { Header(&'static str), Item(usize), Topic(usize) }

/// What the Help menu's search field holds. `marker` is set while the highlighted row was reached by hovering a
/// Menu Items result, so the menu it opened shows the pointer marker beside that item.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct HelpSearch { pub query: String, pub marker: bool }

/// Menu item the Help search sees for one menu row: its menu's title and its text without the check mark or indent.
#[cfg_attr(not(feature = "desktop-wayland"), allow(dead_code))]
pub(super) fn help_item(menu_title: &str, text: &str, enabled: bool) -> helpbook::MenuItem {
    helpbook::MenuItem {
        menu: menu_title.to_string(),
        title: text.trim_start_matches(|c: char| c == '\u{2713}' || c == ' ').trim().to_string(),
        enabled,
    }
}

/// The menu rows a query finds, as `(menu, row index)`, best first. `entries` holds each searched row with its menu
/// and its index in that menu's rows; the search itself is `helpbook::search_menu`.
#[cfg_attr(not(feature = "desktop-wayland"), allow(dead_code))]
pub(super) fn help_hits(entries: &[(Menu, usize, helpbook::MenuItem)], query: &str) -> Vec<(Menu, usize)> {
    let items: Vec<helpbook::MenuItem> = entries.iter().map(|e| e.2.clone()).collect();
    helpbook::search_menu(&items, query).into_iter().map(|i| (entries[i].0, entries[i].1)).collect()
}

/// The Help menu's result groups: "Menu Items" first, then "Help Topics". A group with no hits is left out; no hits
/// at all gives one "No Results" heading.
#[cfg_attr(not(feature = "desktop-wayland"), allow(dead_code))]
pub(super) fn help_groups(item_hits: &[usize], topic_count: usize) -> Vec<HelpRow> {
    let mut rows = Vec::new();
    if !item_hits.is_empty() {
        rows.push(HelpRow::Header("Menu Items"));
        rows.extend(item_hits.iter().map(|&i| HelpRow::Item(i)));
    }
    if topic_count > 0 {
        rows.push(HelpRow::Header("Help Topics"));
        rows.extend((0..topic_count).map(HelpRow::Topic));
    }
    if rows.is_empty() {
        rows.push(HelpRow::Header("No Results"));
    }
    rows
}

/// Topic hits that belong to the front app's book or to the system book, at most `limit`, in the index's order.
#[cfg_attr(not(feature = "desktop-wayland"), allow(dead_code))]
pub(super) fn help_topic_hits(hits: Vec<helpbook::Hit>, front_book: Option<&str>, limit: usize) -> Vec<helpbook::Hit> {
    hits.into_iter()
        .filter(|h| h.app == helpbook::SYSTEM_BOOK || Some(h.app.as_str()) == front_book)
        .take(limit)
        .collect()
}

/// The row a keyboard step lands on: the next (`dir` > 0) or previous (`dir` < 0) row that can be chosen, wrapping
/// round the ends. `selectable[i]` says whether row i has an action; headers and labels are skipped.
#[cfg_attr(not(feature = "desktop-wayland"), allow(dead_code))]
pub(super) fn step_index(selectable: &[bool], current: Option<usize>, dir: i32) -> Option<usize> {
    let n = selectable.len() as i32;
    if n == 0 {
        return None;
    }
    let start = current.map_or(if dir > 0 { -1 } else { n }, |i| i as i32);
    (1..=n).map(|k| (start + dir * k).rem_euclid(n) as usize).find(|&i| selectable[i])
}

/// Whether the menu bar shows the Input menu: several enabled input sources, the on-screen keyboard open on purpose, or the
/// user's Show Input menu in menu bar switch.
pub(super) fn input_menu_visible(sources:usize,keyboard_open:bool,setting:bool)->bool {sources>1||keyboard_open||setting}

#[cfg(test)]
mod help_search_tests {
    use super::*;
    use crate::shell::helpbook::Hit;

    fn entry(menu: Menu, row: usize, title: &str, enabled: bool) -> (Menu, usize, helpbook::MenuItem) {
        (menu, row, help_item(match menu { Menu::Edit => "Edit", Menu::Window => "Window", _ => "App" }, title, enabled))
    }

    #[test]
    fn check_marks_and_indents_are_left_out_of_the_title() {
        let item = help_item("Edit", "\u{2713} Copy", true);
        assert_eq!(item.title, "Copy");
        assert_eq!(item.menu, "Edit");
        assert!(item.enabled);
        assert_eq!(help_item("View", "    Larger Icons", false).title, "Larger Icons");
    }

    #[test]
    fn menu_items_group_before_topics_and_empty_groups_are_left_out() {
        assert_eq!(
            help_groups(&[3, 1], 2),
            vec![HelpRow::Header("Menu Items"), HelpRow::Item(3), HelpRow::Item(1), HelpRow::Header("Help Topics"), HelpRow::Topic(0), HelpRow::Topic(1)]
        );
        assert_eq!(help_groups(&[], 1), vec![HelpRow::Header("Help Topics"), HelpRow::Topic(0)], "no Menu Items heading without hits");
        assert_eq!(help_groups(&[0], 0), vec![HelpRow::Header("Menu Items"), HelpRow::Item(0)]);
        assert_eq!(help_groups(&[], 0), vec![HelpRow::Header("No Results")]);
    }

    #[test]
    fn a_menu_query_resolves_to_the_row_that_holds_the_item() {
        let entries = vec![
            entry(Menu::Edit, 4, "Copy", true),
            entry(Menu::Edit, 5, "Paste", false),
            entry(Menu::Window, 2, "Copy Window", true),
        ];
        assert_eq!(help_hits(&entries, "copy"), vec![(Menu::Edit, 4), (Menu::Window, 2)], "whole-word titles in menu order");
        assert_eq!(help_hits(&entries, "paste"), vec![(Menu::Edit, 5)], "a disabled item is still found");
        assert!(help_hits(&entries, "print").is_empty());
        assert!(help_hits(&entries, "  ").is_empty());
    }

    #[test]
    fn topics_come_from_the_front_book_and_the_system_book_only() {
        let hit = |app: &str, title: &str| Hit { app: app.into(), app_name: app.into(), slug: title.into(), title: title.into(), anchor: None, score: 1 };
        let hits = vec![hit("is.olibuijr.other", "A"), hit("com.front", "B"), hit(helpbook::SYSTEM_BOOK, "C"), hit("com.front", "D")];
        let kept: Vec<String> = help_topic_hits(hits.clone(), Some("com.front"), 8).into_iter().map(|h| h.title).collect();
        assert_eq!(kept, vec!["B", "C", "D"]);
        let one: Vec<String> = help_topic_hits(hits, None, 1).into_iter().map(|h| h.title).collect();
        assert_eq!(one, vec!["C"], "no front book: only the system book, limited");
    }

    #[test]
    fn keyboard_steps_skip_headers_and_wrap() {
        let sel = [false, true, false, true];
        assert_eq!(step_index(&sel, None, 1), Some(1), "down from nothing lands on the first choice");
        assert_eq!(step_index(&sel, Some(1), 1), Some(3));
        assert_eq!(step_index(&sel, Some(3), 1), Some(1), "down wraps to the top");
        assert_eq!(step_index(&sel, None, -1), Some(3), "up from nothing lands on the last choice");
        assert_eq!(step_index(&sel, Some(1), -1), Some(3), "up wraps to the bottom");
        assert_eq!(step_index(&[false, false], None, 1), None, "no choices");
        assert_eq!(step_index(&[], Some(0), 1), None);
    }
}

/// Where a context menu opens: at the pointer, flipped left or up when it would leave the area, held inside when neither fits.
#[cfg(test)]
mod context_menu_origin_tests {
    use super::context_menu_origin;

    // the area under the bar and above the Dock: 1000 x 600 px, its top at y 30
    const AREA: (i32, i32, i32, i32) = (0, 30, 1000, 600);

    #[test]
    fn a_menu_that_fits_opens_with_its_corner_at_the_pointer() {
        assert_eq!(context_menu_origin((300., 200.), (264, 400), AREA), (300, 200));
    }

    #[test]
    fn a_menu_flips_left_at_the_right_edge_and_up_at_the_bottom_edge() {
        // 1000 - 264 = 736: a pointer at 800 leaves no room to the right
        assert_eq!(context_menu_origin((800., 200.), (264, 100), AREA), (536, 200), "its right edge is the pointer");
        // the area ends at 630: a 100 px menu at 560 would end at 660
        assert_eq!(context_menu_origin((300., 560.), (264, 100), AREA), (300, 460), "its bottom edge is the pointer");
        assert_eq!(context_menu_origin((800., 560.), (264, 100), AREA), (536, 460), "both at once");
        // a menu that ends exactly at the edge stays where it opened
        assert_eq!(context_menu_origin((736., 530.), (264, 100), AREA), (736, 530));
    }

    #[test]
    fn a_menu_larger_than_the_area_is_held_inside_it_from_the_top_left() {
        assert_eq!(context_menu_origin((300., 300.), (264, 700), AREA), (300, 30), "too tall to flip: held at the top");
        assert_eq!(context_menu_origin((10., 300.), (2000, 10), AREA), (0, 300), "wider than the screen: held at the left");
    }

    #[test]
    fn a_pointer_outside_the_area_is_pulled_in() {
        assert_eq!(context_menu_origin((-20., 5.), (264, 100), AREA), (0, 30));
    }
}

/// An app's context menu in the shell: the rows come from its items and the surface is as tall as they are.
#[cfg(all(test, feature = "desktop-wayland"))]
mod context_menu_shell_tests {
    use super::*;
    use bifrost_proto::ContextItem;

    fn items() -> Vec<ContextItem> {
        vec![
            ContextItem::new(1, "Open"),
            ContextItem::new(2, "Get Info").key("cmd+i"),
            ContextItem::separator(),
            ContextItem::new(3, "Paste").enabled(false).key("cmd+v"),
        ]
    }

    /// A desktop shell with `items()` open as a context menu for a pointer at (x, y).
    fn open_at(x: f32, y: f32) -> Shell {
        let mut sh = crate::shell::tests::desktop_shell();
        sh.desktop_context = Some(ContextMenu { slot: 0, x, y, items: items() });
        sh.desktop_menu = Some(Menu::Context);
        sh
    }

    #[test]
    fn the_surface_is_as_tall_as_its_rows_and_separators() {
        let sh = open_at(300., 200.);
        let (_, _, w, h) = sh.desktop_popup_rect().unwrap();
        // edge padding, three rows, one separator, edge padding
        let pt = 2. * MENU_PAD_PT + 3. * MENU_ROW_PT + MENU_SEP_PT;
        assert_eq!(h, sh.ui.pt(pt).ceil() as usize);
        assert_eq!(h, sh.ui.pt(menu_spans(&sh.menu_rows(Menu::Context)).1).ceil() as usize, "the height the rows are drawn with");
        assert!((h as f32) < sh.ui.pt(8. + 36. * 4.), "not a fixed 36 pt per item");
        assert!(w >= sh.ui.pt(264.) as usize, "as wide as the bar menus");
    }

    #[test]
    fn it_opens_at_the_pointer_and_flips_at_the_screen_edges() {
        let sh = open_at(300., 200.);
        let (x, y, _, _) = sh.desktop_popup_rect().unwrap();
        assert_eq!((x, y), (300, 200), "top-left at the pointer");
        // near the bottom of the work area: above the pointer
        let area = crate::shell::desk::Desk::area(&sh.desk_metrics());
        let low = area.y + area.h - 10;
        let sh = open_at(300., low as f32);
        let (x, y, _, h) = sh.desktop_popup_rect().unwrap();
        assert_eq!((x, y + h), (300, low as usize), "its bottom edge is the pointer");
        // near the right edge: left of the pointer
        let right = sh.w - 10;
        let sh = open_at(right as f32, 200.);
        let (x, y, w, _) = sh.desktop_popup_rect().unwrap();
        assert_eq!((x + w, y), (right as usize, 200), "its right edge is the pointer");
    }

    /// E2E N6: under the window server the popup is a chrome surface cropped to the menu, so its shadow was cut off and
    /// a menu over white content had no edge. The surface now has room for the shadow; only the menu takes input.
    #[test]
    fn the_exported_surface_has_room_for_the_shadow_and_only_the_menu_takes_input() {
        let mut sh = open_at(300., 400.);
        sh.anim = None;
        // the fixture starts on the lock screen, where no chrome is exported
        sh.screen = crate::shell::Screen::Home;
        let (w, h) = (sh.w as usize, sh.h as usize);
        let pixels = vec![0u32; w * h];
        let (px, py, pw, ph) = sh.desktop_popup_rect().unwrap();
        let frames = sh.desktop_chrome_frames(&pixels, w, h);
        let f = frames.iter().find(|f| f.id == 1).expect("an open menu is exported");
        assert_eq!(f.pixels.len(), f.width * f.height);
        assert_eq!(f.input, Some([px as i32 - f.x, py as i32 - f.y, pw as i32, ph as i32]), "the menu's own rectangle, in surface px");
        let reach = sh.ui.pt(crate::shell::desk::SHADOW_REACH_PT) as i32;
        assert!(f.x <= px as i32 - reach && f.y <= py as i32 - reach, "room for the shadow left of and above the menu");
        assert!(f.x + f.width as i32 >= (px + pw) as i32 + reach && f.y + f.height as i32 >= (py + ph) as i32 + reach, "and right of and below it");
        assert!(frames.iter().filter(|f| f.id != 1).all(|f| f.input.is_none()), "the other chrome takes the whole surface");
    }

    /// The shadow does not change what a click does: the menu's rectangle decides, a press on the shadow closes the menu.
    #[test]
    fn hit_testing_uses_the_menu_rectangle_not_the_shadow() {
        let mut sh = open_at(300., 400.);
        sh.anim = None;
        // the fixture starts on the lock screen, where no chrome is exported
        sh.screen = crate::shell::Screen::Home;
        let (px, py, _, _) = sh.desktop_popup_rect().unwrap();
        let (w, h) = (sh.w as usize, sh.h as usize);
        let pixels = vec![0u32; w * h];
        let f = sh.desktop_chrome_frames(&pixels, w, h).into_iter().find(|f| f.id == 1).unwrap();
        let on_shadow = (px as f32 - 3., py as f32 + 10.);
        assert!(f.x as f32 <= on_shadow.0, "the point is inside the exported surface");
        sh.desktop_bar_event(Event::Down(px as f32 + 5., py as f32 + 5.), 1.);
        assert!(sh.desktop_menu.is_some(), "a press inside the menu keeps it open");
        sh.desktop_bar_event(Event::Down(on_shadow.0, on_shadow.1), 2.);
        assert!(sh.desktop_menu.is_none(), "a press on the shadow, outside the menu, closes it");
    }

    /// Both draw paths give the menu a shadow around it and a one-pixel edge in the separator colour.
    #[test]
    fn the_popup_is_drawn_with_a_shadow_and_a_hairline_edge() {
        let sh = open_at(300., 400.);
        let (w, h) = (sh.w as usize, sh.h as usize);
        let (px, py, pw, ph) = sh.desktop_popup_rect().unwrap();
        let mut buf = vec![0u32; w * h];
        sh.draw_desktop_popup(&mut Surface { w, h, stride: w, px: &mut buf, y0: 0 });
        let alpha = |x: usize, y: usize| buf[y * w + x] >> 24;
        let cx = px + pw / 2;
        let reach = sh.ui.pt(crate::shell::desk::SHADOW_REACH_PT) as usize;
        assert!((1..=reach + 2).any(|d| alpha(cx, py + ph + d) > 0), "a shadow below the menu");
        assert!((1..=reach + 2).any(|d| alpha(px - d, py + ph / 2) > 0), "and beside it");
        assert_eq!(alpha(cx, py + ph + 3 * reach), 0, "none beyond its reach");
        let edge = buf[py * w + cx];
        let want = crate::shell::ui::separator_now();
        assert_eq!(edge >> 24, 0xff, "the edge is opaque");
        for shift in [0, 8, 16] {
            let (a, b) = ((edge >> shift & 0xff) as i32, (want >> shift & 0xff) as i32);
            assert!((a - b).abs() <= 3, "the top edge is the separator colour (channel at {shift}: {a} vs {b})");
        }
    }

    #[test]
    fn rows_follow_the_items_and_disabled_items_are_labels() {
        let mut list = vec![ContextItem::separator()];
        list.extend(items());
        list.extend([ContextItem::separator(), ContextItem::separator()]);
        let rows = context_rows(4, &list);
        assert_eq!(rows.len(), 4, "a separator at either end is dropped");
        assert!(matches!(rows[0].action, Some(Action::Context(4, 1))), "a choice goes back to the app with its id");
        assert!(rows[0].key_text().is_none());
        assert_eq!((rows[1].text.as_str(), rows[1].key_text()), ("Get Info", Some("\u{2318}I")));
        assert!(rows[2].is_separator());
        assert_eq!(rows[3].text, "Paste", "the key equivalent is not part of the title");
        assert!(rows[3].action.is_none(), "a disabled item is a label in the secondary ink, not a choice");
        assert_eq!(rows[3].key_text(), Some("\u{2318}V"));
        let doubled = context_rows(0, &[ContextItem::new(1, "A"), ContextItem::separator(), ContextItem::separator(), ContextItem::new(2, "B")]);
        assert_eq!(doubled.len(), 3, "two separators in a row are one");
    }

    #[test]
    fn hovering_a_row_highlights_it_and_the_bar_does_not_take_over_the_menu() {
        let mut sh = open_at(300., 200.);
        let (x, y, _, _) = sh.desktop_popup_rect().unwrap();
        let (spans, _) = menu_spans(&sh.menu_rows(Menu::Context));
        let (top, h) = spans[1];
        sh.desktop_menu_hover(x as f32 + 12., y as f32 + sh.ui.pt(top + h / 2.));
        assert_eq!(sh.menu_row_of(Menu::Context), Some(1));
        sh.desktop_menu_hover(x as f32 + 12., 4.);
        assert_eq!(sh.desktop_menu, Some(Menu::Context), "crossing the bar keeps the context menu");
        assert_eq!(sh.menu_row_of(Menu::Context), None);
    }
}

/// Which bar items a new reading repaints: the changed label's rectangle and what moved with it, not the whole bar.
#[cfg(all(test, feature = "desktop-wayland"))]
mod changed_bar_item_tests {
    use super::{changed_bar_rects, Menu};

    fn item(menu: Menu, left: f32, width: f32, label: &str) -> (Menu, f32, f32, String) {
        (menu, left, width, label.to_string())
    }

    #[test]
    fn the_same_layout_changes_nothing() {
        let bar = vec![item(Menu::App, 40., 80., "Music"), item(Menu::Cpu, 1900., 70., "CPU 3%")];
        assert!(changed_bar_rects(&bar, &bar, 2560, 44, 8).is_empty());
    }

    #[test]
    fn a_new_cpu_label_of_the_same_width_damages_only_that_label() {
        let before = vec![item(Menu::App, 40., 80., "Music"), item(Menu::Cpu, 1900., 70., "CPU 3%"), item(Menu::Memory, 1970., 70., "RAM 40%")];
        let after = vec![item(Menu::App, 40., 80., "Music"), item(Menu::Cpu, 1900., 70., "CPU 4%"), item(Menu::Memory, 1970., 70., "RAM 40%")];
        // the label's span, 8 px more on each side
        let rects = changed_bar_rects(&before, &after, 2560, 44, 8);
        assert_eq!(rects, vec![(1892, 0, 86, 44)]);
        let pixels: i32 = rects.iter().map(|r| r.2 * r.3).sum();
        assert!(pixels < 2560 * 44 / 10, "far less than the bar: {pixels}");
    }

    #[test]
    fn a_wider_label_also_damages_the_item_that_moved_with_it() {
        let before = vec![item(Menu::Cpu, 1900., 70., "CPU 9%"), item(Menu::Memory, 1970., 70., "RAM 9%")];
        // RAM grew by 10 px, so CPU moved left by 10 and both changed
        let after = vec![item(Menu::Cpu, 1890., 70., "CPU 9%"), item(Menu::Memory, 1960., 80., "RAM 10%")];
        let rects = changed_bar_rects(&before, &after, 2560, 44, 0);
        assert_eq!(rects, vec![(1890, 0, 150, 44)]);
    }

    #[test]
    fn items_that_appeared_or_went_and_the_screen_edges_are_handled() {
        let before = vec![item(Menu::App, 0., 50., "A")];
        let after = vec![item(Menu::Cpu, 2540., 40., "CPU 1%")];
        let rects = changed_bar_rects(&before, &after, 2560, 44, 8);
        assert_eq!(rects, vec![(0, 0, 58, 44), (2532, 0, 28, 44)], "a gone item and a new one, clipped to the screen");
    }
}

/// An app's menu extra shows its title or a symbol the bar draws, never the first letter of the app's name (E2E MB1).
#[cfg(test)]
mod status_face_tests {
    use super::*;

    #[test]
    fn a_title_wins_and_a_drawable_symbol_shows_as_an_icon() {
        assert_eq!(status_face("Quick", "square.and.pencil"), Some(StatusFace::Text("Quick".into())));
        assert_eq!(status_face("", "square.and.pencil"), Some(StatusFace::Icon("square.and.pencil".into())), "Notes' extra");
        assert_eq!(status_face("", "checklist"), Some(StatusFace::Icon("checklist".into())), "Reminders' extra");
        assert_eq!(status_face("", "wifi"), Some(StatusFace::Icon("wifi".into())), "a symbol of the shell's own set");
    }

    #[test]
    fn an_item_with_no_title_and_no_symbol_the_bar_draws_is_not_shown() {
        assert_eq!(status_face("", ""), None, "no letter stands in for it");
        assert_eq!(status_face("", "unknown.symbol.name"), None);
    }
}

/// The Recent Items submenu opens in place of the system menu and keeps the keyboard path (Right in, Left out).
#[cfg(all(test, feature = "desktop-wayland"))]
mod submenu_tests {
    use super::*;

    fn open_system() -> Shell {
        let mut sh = crate::shell::tests::desktop_shell();
        sh.screen = Screen::Home;
        sh.anim = None;
        sh.desktop_menu = Some(Menu::System);
        sh
    }

    #[test]
    fn right_opens_the_submenu_of_the_highlighted_row_and_left_goes_back() {
        let mut sh = open_system();
        let at = sh.menu_row_texts(Menu::System).iter().position(|t| t == "Recent Items").unwrap();
        sh.desktop_menu_row = Some((Menu::System, at));
        let _ = sh.desktop_menu_key(106, 1.);
        assert_eq!(sh.desktop_menu, Some(Menu::Recent));
        let _ = sh.desktop_menu_key(105, 2.);
        assert_eq!(sh.desktop_menu, Some(Menu::System));
        // Right on a row without a submenu still moves to the next bar menu
        sh.desktop_menu_row = Some((Menu::System, 0));
        let _ = sh.desktop_menu_key(106, 3.);
        assert_eq!(sh.desktop_menu, Some(Menu::App));
    }

    #[test]
    fn a_click_on_recent_items_opens_the_submenu_and_keeps_a_popup() {
        let mut sh = open_system();
        let rows = sh.menu_rows(Menu::System);
        let at = rows.iter().position(|r| r.text == "Recent Items").unwrap();
        let (x, top, w, _) = sh.desktop_popup_rect().unwrap();
        let (spans, _) = menu_spans(&rows);
        let y = top as f32 + sh.ui.pt(spans[at].0 + spans[at].1 / 2.);
        let fx = sh.desktop_bar_event(Event::Up((x + w / 2) as f32, y), 1.);
        assert!(fx.is_some());
        assert_eq!(sh.desktop_menu, Some(Menu::Recent), "the click opens the submenu instead of closing the menu");
        assert!(sh.desktop_popup_rect().is_some());
    }
}
