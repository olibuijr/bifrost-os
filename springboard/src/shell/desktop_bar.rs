//! Desktop menus backed by actual app, network and power commands.
use super::Shell;
#[cfg(feature = "desktop-wayland")]
use super::hwkeys::TermKey;
#[cfg(feature = "desktop-wayland")]
use super::{AppId,Screen,Effect,Event,desktop_network,ui::{Weight,WHITE,SECONDARY,CELL,LABEL,MENU_TINT,MENU_SOLID,MENU_RADIUS_PT}};
#[cfg(feature = "desktop-wayland")]
use crate::gfx::{Image,Surface};
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
    App,File,Edit,View,Window,Help,Network,Power,
    /// the Bluetooth menu extra: the radio, the paired and connected devices, and the Bluetooth pane
    Bluetooth,
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
    /// the open or save panel (`docpanel`): drawn and hit-tested by the panel, not as rows
    Document}
#[cfg(feature = "desktop-wayland")]
#[derive(Clone)] pub(super) enum Action {Foreign(u32),Settings(&'static str),Desktop,Close,Minimize,Zoom,ShowAll,Open(super::AppId),Context(u8,u32),AppMenu(u8,u32),Dock(super::desktop_dock::Change),Shortcut(u16),Command(bifrost_proto::Command),Term(super::hwkeys::TermKey),Wifi(bool),Scan,Select(String),Power(String),Lock,Sleep,Halt(super::power_menu::PowerAction),StatusItem(u8,u32,u32),Tile(super::desk::Tile),RestoreSize,CycleWindow,Print,InputSource(u8),Recent(u8),ClearRecent,CharacterViewer,KeyboardViewer,Media(bifrost_ui::media_session::RemoteCommand),Bluetooth(crate::bare::bluetooth::Cmd),Portal(String),Service(u8)}
// Only the desktop-wayland bar reads these fields; other builds never build the menu.
#[cfg_attr(not(feature = "desktop-wayland"), allow(dead_code))]
pub(super) struct ContextMenu {pub slot:u8,pub x:f32,pub y:f32,pub items:Vec<bifrost_proto::ContextItem>}
#[cfg(feature = "desktop-wayland")]
pub(super) struct Row {pub(super) text:String,pub(super) action:Option<Action>,sep:bool,key:Option<String>,symbol:Option<crate::symbols::Symbol>}
/// Row, separator and edge padding of a bar menu, in pt (the HIG groups menu items with separators).
#[cfg(feature = "desktop-wayland")]
const MENU_ROW_PT:f32=24.;
#[cfg(feature = "desktop-wayland")]
const MENU_SEP_PT:f32=11.;
#[cfg(feature = "desktop-wayland")]
const MENU_PAD_PT:f32=5.;
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
/// Top and height in pt (from the popup's top edge) of each row, and the popup's total height.
#[cfg(feature = "desktop-wayland")]
pub(super) fn menu_spans(rows:&[Row])->(Vec<(f32,f32)>,f32) {let mut y=MENU_PAD_PT;let v=rows.iter().map(|r|{let h=if r.sep{MENU_SEP_PT}else{MENU_ROW_PT};let s=(y,h);y+=h;s}).collect();(v,y+MENU_PAD_PT)}
/// Key equivalents the shell really handles (`NativeMenuItem.keyEquivalent`), as `bifrost_ui::keys` combos; the menus show them
/// in Golden Gate notation and `menu_key_equivalents_parse_and_do_not_conflict` checks the table.
#[cfg(feature = "desktop-wayland")]
pub(super) const MENU_KEYS:&[(&str,&str)]=&[("Close Window","cmd+w"),("Minimize","cmd+m"),("Minimize Window","cmd+m"),("Undo","cmd+z"),("Redo","cmd+shift+z"),("Cut","cmd+x"),("Copy","cmd+c"),("Paste","cmd+v"),("Select All","cmd+a"),("Quit","cmd+q"),("Settings…","cmd+,"),("Search","cmd+space"),("App Switcher","cmd+tab"),("Back","escape"),("Print\u{2026}","cmd+p")];
#[cfg(feature = "desktop-wayland")]
impl Row {fn symbol(mut self,sym:crate::symbols::Symbol)->Self {self.symbol=Some(sym);self} fn key(mut self,combo:&str)->Self {self.key=bifrost_ui::keys::KeyCombo::parse(combo).map(|c|c.to_string());self} fn separator()->Self {Self{text:String::new(),action:None,sep:true,key:None,symbol:None}} fn action(text:impl Into<String>,action:Action)->Self {Self{text:text.into(),action:Some(action),sep:false,key:None,symbol:None}} fn label(text:impl Into<String>)->Self {Self{text:text.into(),action:None,sep:false,key:None,symbol:None}}}
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
/// The enabled item whose key equivalent is `ch` with the chord modifiers `mods` (`CHORD_*`), `NativeMenu.performKeyEquivalent:`.
pub(super) fn app_menu_chord(menus:&[bifrost_proto::AppMenu],ch:char,mods:u32)->Option<u32> {
    use bifrost_proto::{CHORD_ALT,CHORD_CTRL,CHORD_META,CHORD_SHIFT};
    let ch=ch.to_ascii_lowercase();
    menus.iter().flat_map(|m|m.items.iter()).filter(|i|i.enabled&&!i.separator&&!i.submenu&&i.id!=0).find(|i|{
        bifrost_ui::keys::KeyCombo::parse(&i.key).is_some_and(|c|c.key==bifrost_ui::keys::Key::Char(ch)&&c.cmd==(mods&CHORD_META!=0)&&c.ctrl==(mods&CHORD_CTRL!=0)&&c.alt==(mods&CHORD_ALT!=0)&&c.shift==(mods&CHORD_SHIFT!=0))
    }).map(|i|i.id)
}
/// Header plus the five largest app groups by `key`; empty when there are no app readings.
#[cfg(feature = "desktop-wayland")]
fn top_app_rows(apps:&[super::desktop_metrics::AppUsage],header:&str,key:impl Fn(&super::desktop_metrics::AppUsage)->u64,show:impl Fn(u64)->String)->Vec<Row> {
    let mut sorted:Vec<_>=apps.iter().collect();sorted.sort_by_key(|a|std::cmp::Reverse(key(a)));
    if sorted.is_empty() {return Vec::new();}
    let mut rows=vec![Row::label(header)];rows.extend(sorted.into_iter().take(5).map(|a|Row::label(format!("{}: {}",a.id,show(key(a))))));rows
}
impl Shell {
    pub(super) fn desktop_bar_height(&self)->usize {self.ui.pt(28.).ceil() as usize}
    /// Without the desktop there is no menu bar, so nothing shows CPU or RAM readings.
    #[cfg(not(feature="desktop-wayland"))]
    pub(super) fn metrics_wanted(&self)->bool {false}
    #[cfg(not(feature="desktop-wayland"))]
    pub(super) fn metrics_menu_open(&self)->bool {false}
}
#[cfg(feature="desktop-wayland")]
impl Shell {
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
    fn menu_extra_shown(&self,menu:Menu)->bool {
        use super::options::MenuExtra as E;
        // the input menu shows only when there is a choice of keyboards, as the reference input menu does
        if menu==Menu::Input {return super::kb_layouts::enabled().len()>1;}
        let e=match menu {Menu::Cpu=>E::Cpu,Menu::Memory=>E::Memory,Menu::Power=>E::Battery,Menu::Network=>E::Network,Menu::ControlCenter=>E::ControlCenter,_=>return true};
        self.options().menu_extras&e.bit()!=0
    }
    /// A CPU or RAM menu is open: it prints the full reading.
    pub(super) fn metrics_menu_open(&self)->bool {matches!(self.desktop_menu,Some(Menu::Cpu|Menu::Memory))}
    /// Whether anything on screen shows the CPU and RAM readings: the bar readouts or their menus. Otherwise nothing samples or wakes.
    pub(super) fn metrics_wanted(&self)->bool {self.metrics_menu_open()||self.menu_extra_shown(Menu::Cpu)||self.menu_extra_shown(Menu::Memory)}
    pub(super) fn menu_buttons(&self)->Vec<(Menu,f32,f32,String)> {
        let mut buttons=vec![(Menu::System,self.ui.pt(6.),self.ui.pt(SYSTEM_MENU_PT),String::from("system"))];let mut x=self.ui.pt(6.+SYSTEM_MENU_PT);
        let name=self.app_menu_name();
        let name=self.ui.fit(&name,13.,self.ui.pt(160.));
        // the order of the Golden Gate menu bar: App, File, Edit, View, Window, then Help at the trailing end
        // the app's own menus follow its name menu (`NativeApp.mainMenu`), before the shell's File, Edit, View, Window and Help
        let mut bar:Vec<(Menu,String)>=vec![(Menu::App,name.clone())];
        bar.extend(self.app_menus().iter().enumerate().filter(|(_,m)|shell_menu(&m.title).is_none()).map(|(i,m)|(Menu::Custom(i as u8),self.ui.fit(&m.title,13.,self.ui.pt(120.)))));
        bar.extend([(Menu::File,"File"),(Menu::Edit,"Edit"),(Menu::View,"View"),(Menu::Window,"Window"),(Menu::Help,"Help")].map(|(m,l)|(m,l.to_string())));
        for (kind,label) in bar.iter().map(|(k,l)|(*k,l.as_str())) {
            let width=self.ui.measure(label,13.,Weight::Semibold)+self.ui.pt(24.);buttons.push((kind,x,width,label.into()));x+=width;
        }
        let network=self.info.desktop_network.as_ref();let wired=network.is_some_and(|n|n.interfaces.iter().any(|i|i.kind=="ethernet"&&i.connected()));
        let nw=self.ui.pt(STATUS_ICON_PT);let pw=self.ui.pt(STATUS_ICON_PT);let clock=self.ui.measure(&self.clock_text(),12.,Weight::Medium);let right=self.w as f32-self.ui.pt(16.)-clock-self.ui.pt(2.);
        // Golden Gate menu extras, placed from the clock leftwards: Control Center, Wi-Fi or ethernet, battery or power; an item that would run into the app menus is left out
        let limit=x+self.ui.pt(8.);let mut edge=right;let mut tail=Vec::new();
        // the owner's CPU and RAM readouts sit left of the system extras (like iStat Menus), dropped first when space runs out
        let cpu=format!("CPU {:.0}%",self.desktop_metrics.cpu);let memory=format!("RAM {:.0}%",self.desktop_metrics.memory_percent());
        let cw=self.ui.measure(&cpu,12.,Weight::Regular)+self.ui.pt(2.*STATUS_PAD_PT);let mw=self.ui.measure(&memory,12.,Weight::Regular)+self.ui.pt(2.*STATUS_PAD_PT);
        // the input source sits left of Control Center (the extras are placed from the clock leftwards)
        let source=super::kb_layouts::active().short_name();let input=self.ui.measure(source,12.,Weight::Regular)+self.ui.pt(2.*STATUS_PAD_PT);
        let mut extras=vec![(Menu::ControlCenter,self.ui.pt(STATUS_ICON_PT),String::from("control")),(Menu::Input,input,source.to_string()),(Menu::Network,nw,if wired {"ethernet"}else{"wifi"}.into()),(Menu::Power,pw,String::from("power")),(Menu::Memory,mw,memory),(Menu::Cpu,cw,cpu)];
        // the Bluetooth extra sits left of Wi-Fi, once the radio exists (it stays hidden without a controller)
        if self.info.bt.state!=crate::bare::bluetooth::State::Unavailable {extras.insert(2,(Menu::Bluetooth,self.ui.pt(STATUS_ICON_PT),String::from("bluetooth")));}
        // the Now Playing extra sits left of the system extras while an app has a session, and right of the app extras
        if let Some(p)=super::now_playing_menu::panel(){let label=self.ui.fit(&p.label,12.,self.ui.pt(160.));let w=self.ui.measure(&label,12.,Weight::Regular)+self.ui.pt(2.*STATUS_PAD_PT);extras.push((Menu::NowPlaying,w,label));}
        // menu extras of running apps (`NativeStatusItem`) sit left of the system extras; an empty title shows the app's initial
        for (i,e) in self.status_entries_now().iter().enumerate() {let label=if e.title.is_empty(){super::apphost::installed().get(e.slot as usize).and_then(|a|a.manifest.name.chars().next()).unwrap_or('•').to_string()}else{e.title.clone()};let w=self.ui.measure(&label,12.,Weight::Regular)+self.ui.pt(2.*STATUS_PAD_PT);extras.push((Menu::Status(i as u8),w,label));}
        for (menu,width,label) in extras {
            if !self.menu_extra_shown(menu){continue;}
            if edge-width<limit {break;}
            edge-=width;tail.push((menu,edge,width,label));
        }
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
    fn draw_status_glyph(&self,s:&mut Surface,menu:Menu,label:&str,x:f32,y:f32,ink:u32) {
        let p=self.ui.pt(1.);
        // the glyphs are drawn from different origins; lift each so its ink centres on the bar like the text beside it
        let y=y-p*if matches!(menu,Menu::System|Menu::ControlCenter){0.}else{status_icon_lift(menu==Menu::Power&&self.info.battery_present,label=="ethernet")};let rect=|s:&mut Surface,dx:f32,dy:f32,w:f32,h:f32|s.fill_rrect(x+dx*p,y+dy*p,w*p,h*p,p,ink);
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
    pub(super) fn menu_rows(&self,menu:Menu)->Vec<Row> {
        let mut rows=self.own_menu_rows(menu);
        if let Screen::App(AppId::External(slot))=self.screen {
            let merged:Vec<Row>=self.app_menus().iter().filter(|m|shell_menu(&m.title)==Some(menu)).flat_map(|m|app_menu_rows(slot,m)).collect();
            if !merged.is_empty() {if !rows.is_empty(){rows.insert(0,Row::separator());}for r in merged.into_iter().rev(){rows.insert(0,r);}}
        }
        rows
    }
    fn own_menu_rows(&self,menu:Menu)->Vec<Row> {
        match menu {
            Menu::Document=>Vec::new(),
            // each enabled source, the active one checked; picking one makes it the source (`kb_layouts::set_active`)
            Menu::Input=>{let mut v:Vec<Row>=super::kb_layouts::Layout::ALL.iter().enumerate().filter(|(_,l)|super::kb_layouts::is_enabled(**l)).map(|(i,l)|Row::action(format!("{} {}",if *l==super::kb_layouts::active(){"✓"}else{" "},l.name()),Action::InputSource(i as u8))).collect();v.extend([Row::separator(),Row::action("Show Keyboard Viewer",Action::KeyboardViewer)]);v},
            Menu::Status(i)=>self.status_entries_now().get(i as usize).map(|e|e.menu.iter().map(|m|if m.separator{Row::separator()}else if m.enabled{Row::action(format!("{} {}",if m.checked{"✓"}else{" "},m.title),Action::StatusItem(e.slot,e.item,m.id))}else{Row::label(format!("{} {}",if m.checked{"✓"}else{" "},m.title))}).collect()).unwrap_or_default(),
            Menu::Custom(i)=>{let slot=match self.screen {Screen::App(AppId::External(n))=>n,_=>return Vec::new()};self.app_menus().get(i as usize).map(|m|app_menu_rows(slot,m)).unwrap_or_default()},
            Menu::Context=>self.desktop_context.as_ref().map(|c|c.items.iter().map(|item|if item.enabled {Row::action(&item.label,Action::Context(c.slot,item.id))}else{Row::label(&item.label)}).collect()).unwrap_or_default(),
            Menu::Dock=>{use super::desktop_dock::{Change,Position};let p=self.dock_preferences();let d=self.desk.as_ref().unwrap();let mut rows=Vec::new();if let Some(index)=d.dock_menu_target{let name=super::apps().get(index).map(|a|a.name).unwrap_or("Application");rows.push(Row::label(name));rows.push(Row::action("Open / Show",Action::Dock(Change::Open(index))));let pinned=self.dock_app_key(index).is_some_and(|key|p.pinned.contains(&key));rows.push(Row::action(if pinned{"Remove from Dock"}else{"Keep in Dock"},Action::Dock(Change::Pin(index))));rows.push(Row::action("Quit",Action::Dock(Change::Quit(index))));}rows.push(Row::label("Dock"));for (label,position) in [("Bottom",Position::Bottom),("Left",Position::Left),("Right",Position::Right)] {rows.push(Row::action(format!("{} {}",if p.position==position{"✓"}else{" "},label),Action::Dock(Change::Position(position))));}rows.push(Row::action(format!("{} Automatically hide and show",if p.autohide{"✓"}else{" "}),Action::Dock(Change::Autohide)));rows.push(Row::action("Smaller Icons",Action::Dock(Change::Size(-8.))));rows.push(Row::action("Larger Icons",Action::Dock(Change::Size(8.))));rows},
            Menu::App=>{let enabled=self.foreign_active().is_some()||self.desk.as_ref().and_then(|d|d.front()).is_some();let name=self.app_menu_name();let quit=format!("Quit {name}");let mut rows=vec![Row::action(format!("About {name}"),Action::Settings("About")),Row::separator(),Row::action("Settings…",Action::Settings("")).symbol(crate::symbols::Symbol::GearOutline).key("cmd+,")];let services=super::services_route::menu_entries();if !services.is_empty(){rows.push(Row::separator());rows.push(Row::label("Services"));rows.extend(services.into_iter().map(|(i,title)|Row::action(title,Action::Service(i))));}rows.push(Row::separator());rows.push((if enabled{Row::action(quit,Action::Close)}else{Row::label(quit)}).key("cmd+q"));rows},
            Menu::System=>{use super::power_menu::PowerAction;let mut rows=vec![Row::action("About This Computer",Action::Settings("About")),Row::separator(),Row::action("System Settings…",Action::Settings("")),Row::action("App Store…",Action::Open(AppId::AppStore)),Row::separator()];rows.push(Row::label("Recent Items"));let recent=self.recent_titles();for (i,title) in recent.iter().enumerate(){rows.push(Row::action(title.clone(),Action::Recent(i as u8)));}rows.push(if recent.is_empty(){Row::label("Clear Menu")}else{Row::action("Clear Menu",Action::ClearRecent)});rows.push(Row::separator());rows.extend(vec![Row::action("Sleep",Action::Sleep),Row::action("Restart…",Action::Halt(PowerAction::Restart)),Row::action("Shut Down…",Action::Halt(PowerAction::Off)),Row::separator(),Row::action("Lock Screen",Action::Lock)]);rows},
            Menu::ControlCenter=>Vec::new(),
            // the Bluetooth extra: the radio, then each paired or connected device (connect or disconnect), then its pane
            Menu::Bluetooth=>{let bt=&self.info.bt;let mut rows=vec![Row::action(if bt.on(){"Turn Bluetooth Off"}else{"Turn Bluetooth On"},Action::Bluetooth(crate::bare::bluetooth::Cmd::Enable(!bt.on())))];if bt.on(){rows.push(Row::separator());for d in bt.devices.iter().filter(|d|d.paired||d.connected).take(8) {if bt.busy.as_deref()==Some(d.addr.as_str()){rows.push(Row::label(format!("{}…",d.label())));}else{let cmd=if d.connected{crate::bare::bluetooth::Cmd::Disconnect(d.addr.clone())}else{crate::bare::bluetooth::Cmd::Connect(d.addr.clone())};rows.push(Row::action(format!("{} {}",if d.connected{"✓"}else{" "},d.label()),Action::Bluetooth(cmd)));}}}rows.push(Row::separator());rows.push(Row::action("Bluetooth Settings…",Action::Settings("Bluetooth")));rows},
            // title and position as labels, then the transport: an enabled row sends its remote command to the owner
            Menu::NowPlaying=>super::now_playing_menu::panel().map(|p|{let mut rows=vec![Row::label(p.title),Row::label(p.position),Row::separator()];rows.extend(p.transport.into_iter().map(|(c,text,on)|if on {Row::action(text,Action::Media(c))} else {Row::label(text)}));rows}).unwrap_or_default(),
            // View holds the Dock's display options (also in the Dock menu), as Files and Mail keep theirs in View
            Menu::View=>{use super::desktop_dock::Change;let autohide=self.desk.as_ref().is_some_and(|d|d.dock_preferences.autohide);let mut v=vec![Row::action(format!("{} Automatically Hide Dock",if autohide{"✓"}else{" "}),Action::Dock(Change::Autohide)),Row::action("Smaller Icons",Action::Dock(Change::Size(-8.))),Row::action("Larger Icons",Action::Dock(Change::Size(8.)))];if self.term_front(){v.splice(0..0,[Row::action("Actual Size",Action::Term(TermKey::ActualSize)).key("cmd+0"),Row::action("Bigger",Action::Term(TermKey::Bigger)).key("cmd+="),Row::action("Smaller",Action::Term(TermKey::Smaller)).key("cmd+-"),Row::separator()]);}v},
            // Window: Minimize and Zoom (also in File), Bring All to Front, then every open window by name, the front one checked
            Menu::Window=>{let Some(d)=self.desk.as_ref() else{return Vec::new();};let enabled=self.foreign_active().is_some()||d.front().is_some();let front=d.front().map(|w|w.app);let mut rows=vec![(if enabled{Row::action("Minimize",Action::Minimize)}else{Row::label("Minimize")}).key("cmd+m"),if enabled{Row::action("Zoom",Action::Zoom)}else{Row::label("Zoom")},Row::separator(),if d.wins.iter().any(|w|w.minimized){Row::action("Bring All to Front",Action::ShowAll)}else{Row::label("Bring All to Front")},Row::action("Show Desktop",Action::Desktop)];let tile_on=self.foreign_active().is_none()&&d.front().is_some();rows.push(Row::separator());rows.extend([("Left Half",super::desk::Tile::Left,"ctrl+cmd+left"),("Right Half",super::desk::Tile::Right,"ctrl+cmd+right"),("Fill",super::desk::Tile::Fill,"ctrl+cmd+up"),("Center",super::desk::Tile::Center,"ctrl+cmd+down")].map(|(l,t,k)|if tile_on&&(t==super::desk::Tile::Fill||!super::desk::compositor_places_windows()){Row::action(l,Action::Tile(t)).key(k)}else{Row::label(l).key(k)}));rows.push(if tile_on{Row::action("Return to Previous Size",Action::RestoreSize)}else{Row::label("Return to Previous Size")});rows.push(if d.wins.len()>=2{Row::action("Next Window",Action::CycleWindow).key("cmd+`")}else{Row::label("Next Window")});let mut open:Vec<(String,super::AppId)>=d.wins.iter().map(|w|(super::apps().iter().find(|a|a.id==w.app).map(|a|a.name).unwrap_or("Window").to_string(),w.app)).collect();open.sort_by_key(|(name,_)|name.to_lowercase());let foreign:Vec<&crate::bare::wayland::ForeignWindow>=d.native_windows.iter().chain(d.linux_apps.iter().flat_map(|a|a.windows.iter())).collect();if !open.is_empty()||!foreign.is_empty(){rows.push(Row::separator());}rows.extend(open.into_iter().take(12).map(|(name,app)|Row::action(format!("{} {}",if front==Some(app)&&self.foreign_active().is_none(){"✓"}else{" "},name),Action::Open(app))));rows.extend(foreign.into_iter().take(12).map(|w|Row::action(format!("{} {}",if w.active{"✓"}else{" "},self.ui.fit(if w.title.is_empty(){w.app_id.as_str()}else{w.title.as_str()},13.,self.ui.pt(300.))),Action::Foreign(w.id))));rows},
            Menu::File=>{let enabled=self.foreign_active().is_some()||self.desk.as_ref().and_then(|d|d.front()).is_some();let mut rows=vec![(if enabled{Row::action("Close Window",Action::Close)}else{Row::label("Close Window")}).key("cmd+w"),(if enabled{Row::action("Minimize Window",Action::Minimize)}else{Row::label("Minimize Window")}).key("cmd+m"),if enabled {Row::action("Maximize / Restore Window",Action::Zoom)}else{Row::label("Maximize / Restore Window")}];if self.term_front(){rows.splice(0..0,[Row::action("New Tab",Action::Term(TermKey::NewTab)).key("cmd+t"),Row::separator()]);rows[2]=Row::action("Close Tab",Action::Term(TermKey::CloseTab)).key("cmd+w");}if self.foreign_active().is_none(){rows.push(Row::separator());rows.push((if enabled{Row::action("Print\u{2026}",Action::Print)}else{Row::label("Print\u{2026}")}).key("cmd+p"));}rows},
            // Undo and Redo first, as in every Golden Gate Edit menu. Native Linux apps get the shortcut (Ctrl+key); Redo has no
            // single key, so it stays a disabled label for them.
            Menu::Edit=>{let mut v:Vec<Row>=[("Undo",Some(44),bifrost_proto::Command::Undo),("Redo",None,bifrost_proto::Command::Redo),("Cut",Some(45),bifrost_proto::Command::Cut),("Copy",Some(46),bifrost_proto::Command::Copy),("Paste",Some(47),bifrost_proto::Command::Paste),("Select All",Some(30),bifrost_proto::Command::SelectAll)].into_iter().map(|(label,key,command)|if let Some(commands)=self.sdk_commands(){if commands.supports(command){Row::action(label,Action::Command(command))}else{Row::label(label)}}else if let (Some(key),true)=(key,self.foreign_active().is_some()||self.screen==Screen::App(AppId::Terminal)){Row::action(label,Action::Shortcut(key))}else{Row::label(label)}).collect();for r in &mut v{if let Some((_,k))=MENU_KEYS.iter().find(|(l,_)|*l==r.text){*r=std::mem::replace(r,Row::separator()).key(k);}}v.insert(2,Row::separator());if let Some(t)=self.sdk_undo_titles(){for r in &mut v{if r.action.is_some(){let t=match r.text.as_str(){"Undo"=>&t.0,"Redo"=>&t.1,_=>continue};if !t.is_empty(){r.text=t.clone();}}}}if self.term_front(){v.extend([Row::separator(),Row::action("Find…",Action::Term(TermKey::Find)).key("cmd+f"),Row::action("Find Next",Action::Term(TermKey::FindNext)).key("cmd+g"),Row::action("Find Previous",Action::Term(TermKey::FindPrev)).key("cmd+shift+g"),Row::separator(),Row::action("Clear Scrollback",Action::Term(TermKey::ClearScrollback)).key("cmd+k")]);}v.extend([Row::separator(),Row::action("Emoji & Symbols",Action::CharacterViewer)]);v},
            Menu::Help=>vec![Row::action("BifrOSt Information",Action::Settings("About")),Row::separator(),Row::label("Search").key("cmd+space"),Row::label("Input Source").key("ctrl+space"),Row::label("App Switcher").key("cmd+tab"),Row::label("Back").key("escape"),Row::label("Home: Cmd (Meta) alone")],
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
    pub(super) fn desktop_popup_rect(&self)->Option<(usize,usize,usize,usize)> {
        let menu=self.desktop_menu?;if menu==Menu::Dock{return self.dock_popup_rect();}if menu==Menu::Document{return self.doc_panel_rect();}if menu==Menu::Context{let c=self.desktop_context.as_ref()?;let width=self.ui.pt(264.) as usize;let height=(self.ui.pt(8.)+c.items.len() as f32*self.ui.pt(36.)) as usize;return Some(((c.x.max(0.) as usize).min((self.w as usize).saturating_sub(width)),(c.y.max(0.) as usize).min((self.h as usize).saturating_sub(height)).max(self.desktop_bar_height()),width,height));}let (_,x,_,_)=self.menu_buttons().into_iter().find(|b|b.0==menu)?;
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
        if self.perf.enabled||self.perf_drawn{let (x,y,w,h)=self.perf_rect();let g=self.ui.pt(2.) as i32;v.push((x as i32-g,y as i32-g,w as i32+2*g,h as i32+2*g));}
        Some(v)
    }
    pub(super) fn draw_desktop_bar(&self,s:&mut Surface) {
        let height=self.desktop_bar_height();if s.y0>=height{return;}
        // the backdrop depends only on the size and the wallpaper: blur it once, not every frame
        if bifrost_render::glass::gpu() {
            // Golden Gate: the bar is transparent over the wallpaper the presenter draws
            *self.bar_backdrop.borrow_mut()=None;
            s.fill_rect(0,0,self.w as usize,height,0);
            self.draw_desktop_bar_items(s,height);return;
        }
        let key=(self.w as usize,height,self.home_still().px.as_ptr() as usize);
        let mut cache=self.bar_backdrop.borrow_mut();
        if cache.as_ref().is_none_or(|(k,_)|*k!=key) {
            let mut image=Image::new(self.w as usize,height);let mut bg=Surface{w:image.w,h:image.h,stride:image.w,px:&mut image.px,y0:0};
            bg.fill_rect(0,0,bg.w,bg.h,CELL);{let wp=self.home_still();bg.copy(wp,(self.w-wp.w as i32)/2,0);}
            *cache=Some((key,image));
        }
        let (_,image)=cache.as_ref().unwrap();s.blit(image,0,0,255,255);drop(cache);
        self.draw_desktop_bar_items(s,height);
    }
    fn draw_desktop_bar_items(&self,s:&mut Surface,height:usize) {
        for (menu,x,width,label) in self.menu_buttons() {if self.desktop_menu==Some(menu){let h=height as f32-self.ui.pt(4.);s.fill_rrect(x,self.ui.pt(2.),width,h,h/2.,self.bar_ink().capsule);}if matches!(menu,Menu::System|Menu::Network|Menu::Power|Menu::ControlCenter|Menu::Bluetooth) {self.draw_desktop_status_icon(s,menu,&label,x+width/2.,height as f32/2.);continue;}if matches!(menu,Menu::Cpu|Menu::Memory|Menu::Status(_)|Menu::Input|Menu::NowPlaying){self.bar_label(s,&label,12.,Weight::Regular,|im|((x+self.ui.pt(STATUS_PAD_PT)) as i32,(height as i32-im.h as i32)/2));continue;}self.bar_label(s,&label,13.,if menu==Menu::App{Weight::Semibold}else{Weight::Regular},|im|((x+self.ui.pt(12.)) as i32,(height as i32-im.h as i32)/2));}
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
        let width=self.ui.measure(&self.clock_text(),12.,Weight::Medium);let left=self.w as f32-self.ui.pt(16.)-width;
        y<self.desktop_bar_height() as f32&&x>=left-self.ui.pt(8.)&&x<self.w as f32
    }
    pub(super) fn draw_desktop_popup(&self,s:&mut Surface) {
        let Some((x,y,w,h))=self.desktop_popup_rect() else{return;};let menu=self.desktop_menu.unwrap();
        if bifrost_render::glass::gpu()&&bifrost_render::appearance::reduce_transparency() {
            let r=self.ui.pt(MENU_RADIUS_PT);
            s.fill_rect(x,y,w,h,0);super::desk::soft_shadow(s,&self.ui,x as f32,y as f32,w as f32,h as f32,r);s.fill_rrect(x as f32,y as f32,w as f32,h as f32,r,MENU_SOLID.now());
        } else if bifrost_render::glass::gpu() {
            let r=self.ui.pt(MENU_RADIUS_PT);
            bifrost_render::glass::push(super::control_center::shell_glass_rect(&bifrost_ui::material_request::ShellSurface::MenuBarMenu.request((x as f32,y as f32,w as f32,h as f32),r)));
            s.fill_rect(x,y,w,h,0);super::desk::soft_shadow(s,&self.ui,x as f32,y as f32,w as f32,h as f32,r);s.fill_rrect(x as f32,y as f32,w as f32,h as f32,r,bifrost_render::glass::tint(MENU_TINT.now()));
        } else {let mut image=Image::new(w,h);let mut bg=Surface{w,h,stride:w,px:&mut image.px,y0:0};bg.fill_rect(0,0,w,h,CELL);{let wp=self.home_still();bg.copy(wp,(self.w-wp.w as i32)/2-x as i32,-(y as i32));}image.blur_material(self.ui.pt(10.) as usize);s.blit(&image,x as i32,y as i32,255,255);s.fill_rrect(x as f32,y as f32,w as f32,h as f32,self.ui.pt(MENU_RADIUS_PT),MENU_TINT.now());}
        if menu==Menu::Document{self.draw_doc_panel(s,x,y,w,h);return;}
        let sel=self.menu_row_of(menu);let rows=self.menu_rows(menu);let (spans,_)=menu_spans(&rows);
        // Golden Gate: a row's symbol sits at the text edge and pushes only that row's title right
        let lead=16.;
        for (i,row) in rows.iter().enumerate(){let (top,rh)=spans[i];
            if row.sep{s.fill_rect(x+self.ui.pt(12.) as usize,(y as f32+self.ui.pt(top+rh/2.)) as usize,w.saturating_sub(self.ui.pt(24.) as usize),1,super::ui::SEPARATOR_C.now());continue;}
            let hi=sel==Some(i)&&row.action.is_some();
            if hi{s.fill_rrect(x as f32+self.ui.pt(5.),y as f32+self.ui.pt(top),w as f32-self.ui.pt(10.),self.ui.pt(rh),self.ui.pt(8.),super::ui::MENU_HIGHLIGHT);}
            let mid=y as f32+self.ui.pt(top+rh/2.);
            let ink=if row.action.is_some(){if hi{super::ui::MENU_HIGHLIGHT_TEXT}else{LABEL.now()}}else{SECONDARY};
            if let Some(sym)=row.symbol{if let Some(im)=self.ui.symbols.render(sym,self.ui.pt(15.),ink){s.blit(&im,x as i32+self.ui.pt(lead+7.5) as i32-im.w as i32/2,(mid-im.h as f32/2.) as i32,255,255);}}
            let text_x=if row.symbol.is_some(){lead+20.}else{lead};
            self.label(&row.text,13.,Weight::Regular,ink,|im|s.blit(im,x as i32+self.ui.pt(text_x) as i32,(mid-im.h as f32/2.) as i32,255,255));
            if let Some(k)=&row.key{let k=shortcut_spaced(k);self.label(&k,13.,Weight::Regular,if hi{super::ui::MENU_HIGHLIGHT_TEXT}else{super::ui::MENU_SHORTCUT},|im|s.blit(im,(x+w) as i32-self.ui.pt(16.) as i32-im.w as i32,(mid-im.h as f32/2.) as i32,255,255));}}
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
            Event::Down(x,y) if y<self.desktop_bar_height() as f32 && (!self.cc_reveal_zone(x,y)||self.menu_buttons().iter().any(|(_,left,w,_)|x>=*left&&x<*left+*w))=>{self.desktop_menu_row=None;self.desktop_menu=self.menu_buttons().into_iter().find(|(_,left,w,_)|x>=*left&&x<*left+*w).map(|b|b.0);if self.desktop_menu==Some(Menu::ControlCenter){self.desktop_menu=None;self.open_control_center(t);}self.dirty=true;Some(vec![])},
            Event::Down(x,y) if self.desktop_menu.is_some()=>{if !self.desktop_popup_rect().is_some_and(|(left,top,w,h)|x>=left as f32&&x<(left+w) as f32&&y>=top as f32&&y<(top+h) as f32){self.desktop_menu=None;self.dirty=true;}Some(vec![])},
            Event::Up(x,y) if self.desktop_menu.is_some()=>{let menu=self.desktop_menu.unwrap();if y<self.desktop_bar_height() as f32{return Some(vec![]);}
                let action=self.desktop_popup_rect().and_then(|(left,top,w,h)|if x>=left as f32&&x<(left+w) as f32&&y>=top as f32&&y<(top+h) as f32 {self.menu_row_at(menu,y-top as f32).and_then(|i|self.menu_rows(menu).get(i).and_then(|r|r.action.clone()))}else{None});
                self.desktop_menu=None;self.dirty=true;let mut effects=Vec::new();self.desktop_menu_row=None;if let Some(action)=action {effects=self.run_menu_action(action,t);}Some(effects)
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
        let lead=if rows.iter().any(|r|r.symbol.is_some()){self.ui.pt(20.)}else{0.};let need=rows.iter().map(|r|{let t=self.ui.measure(&r.text,13.,Weight::Regular);let k=r.key.as_ref().map_or(0.,|k|self.ui.pt(24.)+self.ui.measure(&shortcut_spaced(k),13.,Weight::Regular));t+k}).fold(0.,f32::max)+self.ui.pt(32.)+lead;
        need.max(self.ui.pt(264.)).ceil() as usize
    }
    /// The row under `rel` px below the popup's top edge (separators count as rows, but have no action).
    fn menu_row_at(&self,menu:Menu,rel:f32)->Option<usize> {let (spans,_)=menu_spans(&self.menu_rows(menu));spans.iter().position(|(t,h)|{let t=self.ui.pt(*t);rel>=t&&rel<t+self.ui.pt(*h)})}
    pub(super) fn menu_row_of(&self,menu:Menu)->Option<usize> {self.desktop_menu_row.filter(|(m,_)|*m==menu).map(|(_,i)|i)}
    /// Move the highlight to the next row with an action in direction `dir`, wrapping (Full Keyboard Access, HIG "The menu bar").
    pub(super) fn menu_step(&self,menu:Menu,dir:i32)->Option<usize> {
        let rows=self.menu_rows(menu);let n=rows.len() as i32;if n==0{return None;}
        let start=self.menu_row_of(menu).map_or(if dir>0{-1}else{n},|i|i as i32);
        (1..=n).map(|k|(start+dir*k).rem_euclid(n) as usize).find(|&i|rows[i].action.is_some())
    }
    /// Keys while a bar menu is open: Up/Down move, Left/Right switch menus, Return chooses, Esc closes. None when no menu is open.
    pub(super) fn desktop_menu_key(&mut self,code:u16,t:f64)->Option<Vec<Effect>> {
        let menu=self.desktop_menu?;if !self.desk_active()||self.is_locked(){return None;}
        if menu==Menu::Document{self.doc_key(code,t);return Some(vec![]);}
        let mut bar=vec![Menu::System,Menu::App];bar.extend(self.app_menus().iter().enumerate().filter(|(_,m)|shell_menu(&m.title).is_none()).map(|(i,_)|Menu::Custom(i as u8)));bar.extend([Menu::File,Menu::Edit,Menu::View,Menu::Window,Menu::Help]);
        self.dirty=true;
        match code {
            103|108=>{if let Some(i)=self.menu_step(menu,if code==108{1}else{-1}){self.desktop_menu_row=Some((menu,i));}},
            105|106=>{if let Some(at)=bar.iter().position(|m|*m==menu){let n=bar.len();self.desktop_menu=Some(bar[(at+if code==106{1}else{n-1})%n]);self.desktop_menu_row=None;}},
            28|96=>{let action=self.menu_row_of(menu).and_then(|i|self.menu_rows(menu).get(i).and_then(|r|r.action.clone()));if let Some(action)=action{self.desktop_menu=None;self.desktop_menu_row=None;return Some(self.run_menu_action(action,t));}},
            1=>{self.desktop_menu=None;self.desktop_menu_row=None;},
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
        let Some(menu)=self.desktop_menu else{return;};if matches!(menu,Menu::Dock|Menu::Context|Menu::Document){return;}
        if y<self.desktop_bar_height() as f32 {
            if let Some(b)=self.menu_buttons().into_iter().find(|(_,l,w,_)|x>=*l&&x<*l+*w) {if b.0!=menu&&b.0!=Menu::ControlCenter{self.desktop_menu=Some(b.0);self.desktop_menu_row=None;self.dirty=true;}}
            return;
        }
        let row=self.desktop_popup_rect().and_then(|(l,t,w,h)|if x>=l as f32&&x<(l+w) as f32&&y>=t as f32&&y<(t+h) as f32{self.menu_row_at(menu,y-t as f32)}else{None}).filter(|i|self.menu_rows(menu).get(*i).is_some_and(|r|r.action.is_some()));
        let new=row.map(|i|(menu,i));if self.desktop_menu_row!=new{self.desktop_menu_row=new;self.dirty=true;}
    }
    /// Carry out a chosen menu row (a click or Return). The menu is already closed.
    pub(super) fn run_menu_action(&mut self,action:Action,t:f64)->Vec<Effect> {
        let mut effects=Vec::new();match action {
                    Action::Settings(page)=>{self.desk_open(AppId::Settings,t);if !page.is_empty(){if let Some(st)=&mut self.settings {effects.extend(st.open_page(&self.ui,page,t));}}},
                    Action::Context(slot,id)=>{if id>=super::sharing::MENU_BASE&&self.share_choose(id,t){self.desktop_context=None;}else if self.screen==Screen::App(AppId::External(slot)){if let Some(Some(h))=self.ext.get_mut(slot as usize){h.send(&bifrost_proto::ToApp::ContextAction(id));}}self.desktop_context=None;},
                    Action::StatusItem(slot,item,id)=>{if let Some(Some(h))=self.ext.get_mut(slot as usize){h.send(&bifrost_proto::ToApp::StatusItemAction{item,id});}},
                    Action::AppMenu(slot,id)=>{if self.screen==Screen::App(AppId::External(slot)){if let Some(Some(h))=self.ext.get_mut(slot as usize){h.send(&bifrost_proto::ToApp::MenuAction(id));}}},
                    Action::Dock(change)=>effects.extend(self.dock_change(change,t)),
                    Action::Tile(t)=>{if self.foreign_active().is_none(){self.desk_tile_front(t);}},
                    Action::RestoreSize=>{if self.foreign_active().is_none(){self.desk_restore_front();}},
                    Action::CycleWindow=>{self.desk_cycle_window();let t=self.last_input;self.cmdtab_release(t);},
                    Action::InputSource(i)=>{if let Some(l)=super::kb_layouts::Layout::ALL.get(i as usize) {super::kb_layouts::set_active(*l);}self.dirty=true;},
                    Action::Zoom=>{if let Some(id)=self.foreign_active(){let max=self.desk.as_ref().is_some_and(|d|d.native_windows.iter().chain(d.linux_apps.iter().flat_map(|a|a.windows.iter())).any(|w|w.id==id&&w.maximized));effects.push(Effect::ForeignWindowMaximize(id,!max));}else if let Some(i)=self.desk.as_ref().and_then(|d|d.wins.iter().rposition(|w|!w.minimized)){self.desk_zoom_window(i);}},
                    Action::Foreign(id)=>effects.push(Effect::ActivateForeign(id)),
                    Action::Print=>self.print_sheet_open(t),
                    Action::Service(i)=>{if let Some(service)=bifrost_ui::services_menu::Service::ALL.get(i as usize).copied(){effects.extend(self.run_service(service,t));}},
                    Action::CharacterViewer=>self.char_viewer_open(),
                    Action::KeyboardViewer=>self.keyboard_viewer_open(),
                    Action::Recent(n)=>{if let Some(app)=self.recent_app_at(n as usize){self.desk_open(app,t);}},
                    Action::ClearRecent=>self.recent_clear(),
                    Action::Desktop=>self.desk_show_desktop(),
                    Action::Open(app)=>self.desk_open(app,t),
                    Action::ShowAll=>{if let Some(d)=&mut self.desk {for w in &mut d.wins {w.minimized=false;}}self.desk_refocus();},
                    Action::Close|Action::Minimize=>effects.extend(self.desk_window_close_or_minimize(matches!(action,Action::Minimize))),
                    Action::Command(command)=>{if let Screen::App(AppId::External(n))=self.screen {if let Some(Some(h))=self.ext.get_mut(n as usize){if h.commands.supports(command){h.send(&bifrost_proto::ToApp::Command(command));}}}},
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
                }effects
    }
    /// Close or minimize the front window (the File menu and Ctrl+W / Ctrl+M).
    pub(super) fn desk_window_close_or_minimize(&mut self,minimize:bool)->Vec<Effect> {
        let mut effects=Vec::new();
        if let Some(id)=self.foreign_active(){effects.push(Effect::ForeignWindowAction(id,minimize));}else if let Some(app)=self.desk.as_ref().and_then(|d|d.front()).map(|w|w.app){if minimize {let t=self.last_input;if let Some(i)=self.desk.as_ref().and_then(|d|d.index_of(app)){self.desk_minimize_window(i,t);}else{self.desk_refocus();}}else{self.desk_close_app(app);}}
        effects
    }
    /// Control Center (chrome id 4) and Notification Center (id 5) as overlay surfaces above every window, each with
    /// room for its shadow: drawn only into the desktop frame they would sit under the windows of the session.
    fn desk_panel_overlays(&self)->Vec<(u32,usize,usize,usize,usize)> {
        let g=self.ui.pt(super::desk::SHADOW_REACH_PT);
        [(4,self.cc_desk_rect()),(5,self.center_desk_rect())].into_iter().filter_map(|(id,r)|r.map(|r|(id,panel_overlay_rect(r,g)))).map(|(id,(x,y,w,h))|(id,x,y,w,h)).collect()
    }
    #[cfg(feature="desktop-wayland")]
    pub fn desktop_chrome_frames(&self,pixels:&[u32],width:usize,height:usize)->Vec<crate::bare::wayland::ChromeFrame> {
        if !self.desk_active()||self.is_locked()||self.anim.is_some(){return vec![];}
        let mut frames=Vec::new();for (id,x,y,w,h) in std::iter::once((0,0,0,width,self.desktop_bar_height())).chain({let r=self.dock_overlay_rect();std::iter::once((2,r.x.max(0) as usize,r.y.max(0) as usize,r.w.max(0) as usize,r.h.max(0) as usize))}).chain(self.desktop_popup_rect().map(|(x,y,w,h)|(1,x,y,w,h))).chain(self.desk_panel_overlays()) {
            let w=w.min(width.saturating_sub(x));let h=h.min(height.saturating_sub(y));if w==0||h==0{continue;}let mut data=Vec::with_capacity(w*h);for row in y..y+h {data.extend_from_slice(&pixels[row*width+x..row*width+x+w]);}if id==2&&!self.dock_is_visible(){data.fill(0);}if id==1 {let mask=crate::gfx::rrect_mask(w,h,self.ui.pt(MENU_RADIUS_PT));let mut image=Image{w,h,px:data};image.apply_mask(&mask);data=image.px;}frames.push(crate::bare::wayland::ChromeFrame{id,x:x as i32,y:y as i32,width:w,height:h,pixels:data});
        }
        // Search: id 3, an overlay surface above every window that takes the keyboard while it is open
        if let Some(((x,y,w,h),mask))=self.quicksearch.desk_overlay(&self.ui,self.w) {
            let w=w.min(width.saturating_sub(x));let h=h.min(height.saturating_sub(y));
            if w>0&&h>0 {
                let mut data=Vec::with_capacity(w*h);for row in y..y+h {data.extend_from_slice(&pixels[row*width+x..row*width+x+w]);}
                let full=mask.len()/h.max(1);let mask:Vec<u8>=(0..w*h).map(|i|mask.get((i/w)*full+i%w).copied().unwrap_or(0)).collect();
                let mut image=Image{w,h,px:data};image.apply_mask(&mask);
                frames.push(crate::bare::wayland::ChromeFrame{id:3,x:x as i32,y:y as i32,width:w,height:h,pixels:image.px});
            }
        }
        frames
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
    use super::popup_damage_rect;

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

#[cfg(test)]
mod panel_overlay_tests {
    #[test]
    fn a_panel_overlay_covers_the_panel_and_its_shadow() {
        assert_eq!(super::panel_overlay_rect((100.,50.,200.,300.),10.),(90,40,220,320));
        assert_eq!(super::panel_overlay_rect((5.,5.,20.,20.),10.),(0,0,35,35),"clipped at the top-left corner");
    }
}
