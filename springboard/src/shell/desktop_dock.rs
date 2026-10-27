//! Persistent desktop dock choices; app identities survive registry changes.
//! It also draws what an app sets on its Dock tile through the `dock` service: a progress bar and an attention bounce.
use super::{apps,Shell,Effect,AppId,desktop_bar::Menu,desk::{Rect,DOCK_TRASH},dock_stack::DOCK_STACK};
use super::ui::{FILL_GRAY,LINK,MAC_CALLOUT_PT};
use crate::gfx::Surface;
use bifrost_ui::values::Progress;
use bifrost_render::motion::{self,Anim,Curve,Kind};
pub(super) const DOCK_LABEL_MAX_WIDTH_PT: f32 = 240.0;
pub(super) const DOCK_LABEL_PAD_X_PT: f32 = 10.0;
pub(super) const DOCK_LABEL_PAD_Y_PT: f32 = 5.0;
pub(super) const DOCK_LABEL_GAP_PT: f32 = 8.0;
#[derive(Clone,Copy,PartialEq,Debug)] pub enum Position {Bottom,Left,Right}
#[derive(Clone,Debug)] pub struct Preferences {pub position:Position,pub size:f32,pub autohide:bool,pub pinned:Vec<String>,
 /// pointer magnification (`dock_magnification`) and the size, in pt, of the icon under the pointer (`dock_magnification_size`)
 pub magnification:bool,pub magnified:f32}
impl Default for Preferences {fn default()->Self{Self{position:Position::Bottom,size:48.,autohide:false,magnification:true,magnified:80.,pinned:super::dock_tiles::default_pinned(apps())}}}
impl Preferences {
 pub fn load(path:&std::path::Path)->Self {let mut p=Self::default();if let Ok(text)=std::fs::read_to_string(path.join("desktop-dock.ini")){for line in text.lines(){if let Some((key,value))=line.split_once('='){match key {"position"=>p.position=match value{"left"=>Position::Left,"right"=>Position::Right,_=>Position::Bottom},"size"=>p.size=value.parse::<f32>().ok().filter(|v|v.is_finite()).unwrap_or(48.).clamp(32.,80.),"autohide"=>p.autohide=value=="true","dock_magnification"=>p.magnification=value!="false","dock_magnification_size"=>p.magnified=value.parse::<f32>().ok().filter(|v|v.is_finite()).unwrap_or(80.).clamp(48.,128.),"pinned"=>p.pinned=value.split(',').filter(|s|!s.is_empty()).take(256).map(str::to_string).collect(),_=>{}}}}}for key in &mut p.pinned{if key=="is.olibuijr.browser"&&apps().iter().any(|a|a.icon=="is.olibuijr.browser.desktop"){*key="is.olibuijr.browser.desktop".into();}if key=="bifrost.files"&&apps().iter().any(|a|a.icon=="is.olibuijr.files.desktop"){*key="is.olibuijr.files.desktop".into();}}super::dock_tiles::normalize_pinned(&super::apphost::installed(),apps(),&mut p.pinned,false);p}
 pub fn save(&self,path:&std::path::Path){let text=format!("position={}\nsize={}\nautohide={}\ndock_magnification={}\ndock_magnification_size={}\npinned={}\n",match self.position{Position::Bottom=>"bottom",Position::Left=>"left",Position::Right=>"right"},self.size,self.autohide,self.magnification,self.magnified,self.pinned.join(","));let dest=path.join("desktop-dock.ini");let tmp=path.join("desktop-dock.ini.tmp");if std::fs::write(&tmp,text).and_then(|_|std::fs::rename(tmp,dest)).is_err(){eprintln!("BifrOSt: could not save dock preferences");}}
}
/// What an app set on its Dock tile through the `dock` service: a progress bar (`None` for none) and an attention
/// bounce as a window on the shell clock in seconds (`None` for none).
#[derive(Clone,Copy,Debug,Default,PartialEq)] pub struct TileState {pub progress:Option<Progress>,pub bounce:Option<(f64,f64)>}
/// The seconds one sweep of an indeterminate Dock progress bar takes.
const PROGRESS_PERIOD_S:f64=1.2;
/// One attention hop, lifting the icon `icon * 0.5` px at its peak: the same kinematics as the launch bounce.
pub(super) fn bounce_lift(now:f64,start:f64,end:f64,hop:f64,icon:i32)->i32 {
 if hop<=0.||now<start||now>=end {return 0;}
 let ph=((now-start)/hop).fract() as f32;(icon as f32*0.5*4.*ph*(1.-ph)).round() as i32
}
pub enum Pointer {Hover(f32,f32),Context(f32,f32)}
#[derive(Clone,Debug,PartialEq)] pub(super) enum Change {Position(Position),Size(f32),Autohide,Magnification(bool),MagnifiedSize(f32),Pin(usize),Open(usize),Quit(usize)}
impl Shell {
 pub(super) fn dock_preferences(&self)->&Preferences {&self.desk.as_ref().unwrap().dock_preferences}
 pub(super) fn dock_change(&mut self,change:Change,t:f64)->Vec<Effect>{
  match change {
   Change::Open(index)=>{self.dock_start_bounce(index,t);if index<apps().len(){self.desk_open(apps()[index].id,t);} #[cfg(feature="desktop-wayland")] if index>apps().len(){if let Some(w)=self.desk.as_ref().and_then(|d|d.linux_apps.get(index-apps().len()-1)).and_then(|a|a.windows.first()){return vec![Effect::ActivateForeign(w.id)];}else if let Some(a)=self.desk.as_ref().and_then(|d|d.linux_apps.get(index-apps().len()-1)){let id=a.app_id.clone();crate::bare::sys::spawn_named("launch-apps", move||{let _=std::process::Command::new("/usr/bin/bifrost-apps").arg(id).status();});}}},
   Change::Quit(index)=>{if index<apps().len(){let app=apps()[index].id;#[cfg(feature="desktop-wayland")]if let AppId::External(slot)=app {if let Some(p)=super::apphost::installed().get(slot as usize){let effects=self.desk.as_ref().unwrap().native_windows.iter().filter(|w|w.app_id==p.manifest.wayland_app_id).map(|w|Effect::ForeignWindowAction(w.id,false)).collect::<Vec<_>>();if !effects.is_empty(){return effects;}}}if let AppId::External(slot)=app{self.desk_quit_app(slot);}else{self.desk_close_app(app);}} #[cfg(feature="desktop-wayland")] if index>apps().len(){if let Some(a)=self.desk.as_ref().and_then(|d|d.linux_apps.get(index-apps().len()-1)){return a.windows.iter().map(|w|Effect::ForeignWindowAction(w.id,false)).collect();}}},
   Change::Pin(index)=>{let Some(key)=self.dock_app_key(index) else{return vec![];};let p=&mut self.desk.as_mut().unwrap().dock_preferences;if let Some(at)=p.pinned.iter().position(|id|id==&key){p.pinned.remove(at);}else{p.pinned.push(key);}},
   Change::Position(position)=>self.desk.as_mut().unwrap().dock_preferences.position=position,
   Change::Size(delta)=>{let p=&mut self.desk.as_mut().unwrap().dock_preferences;p.size=(p.size+delta).clamp(32.,80.);},
   Change::Autohide=>{let d=self.desk.as_mut().unwrap();d.dock_preferences.autohide=!d.dock_preferences.autohide;d.dock_visible=true;d.dock_vis_anim=None;
    // turning autohide off must clear the pending hide, or next_wake keeps returning a due time (busy loop)
    d.dock_hide_at=if d.dock_preferences.autohide{Some(t+700.)}else{None};},
   Change::Magnification(on)=>{let d=self.desk.as_mut().unwrap();d.dock_preferences.magnification=on;d.dock_ptr=None;d.dock_mag=None;},
   Change::MagnifiedSize(delta)=>{let p=&mut self.desk.as_mut().unwrap().dock_preferences;p.magnified=(p.magnified+delta).clamp(48.,128.);},
  }
  // Filled windows follow the newly available frame; ordinary windows keep the person's chosen size.
  self.reflow_filled_windows();
  self.dock_preferences().save(&self.data_dir);self.dirty=true;vec![]
 }
 /// Refits every filled (zoomed) window to the visible frame of its display; other windows keep their size.
 pub(super) fn reflow_filled_windows(&mut self){
  let m=self.desk_metrics();let mut resized=Vec::new();
  if let Some(d)=self.desk.as_mut(){for i in 0..d.wins.len(){let w=d.wins[i];let area=d.area_for(&m,w.r);if w.maximized&&!w.full&&w.r!=area{d.wins[i].r=area;resized.push(i);}}}
  for i in resized{self.desk_layout(i);}
 }
 pub(super) fn dock_app_key(&self,index:usize)->Option<String>{if let Some(a)=apps().get(index){return Some(a.icon.into());} #[cfg(feature="desktop-wayland")]if index>apps().len(){return self.desk.as_ref()?.linux_apps.get(index-apps().len()-1).map(|a|format!("linux:{}",a.app_id));}None}
#[cfg(feature = "desktop-wayland")]
 pub(super) fn dock_popup_rect(&self)->Option<(usize,usize,usize,usize)>{let d=self.desk.as_ref()?;let (x,y)=d.dock_menu_anchor;let width=self.ui.pt(260.) as usize;let items=d.dock_menu_target.and_then(|i|apps().get(i).map(|a|a.id)).and_then(|id|if let AppId::External(slot)=id{self.ext.get(slot as usize).and_then(Option::as_ref).map(|h|h.dock_menu.len())}else{None}).unwrap_or(0);let base=if d.dock_menu_target.is_some(){404.}else{260.};let extra=if items>0{items as f32*24.+10.}else{0.};let height=self.ui.pt(base+extra) as usize;Some(((x as usize).min((self.w as usize).saturating_sub(width)),(y as usize).min((self.h as usize).saturating_sub(height)).max(self.desktop_bar_height()),width,height))}
 pub(super) fn dock_pointer(&mut self,event:Pointer,t:f64)->Option<Vec<Effect>> {
  if !self.desk_active()||self.is_locked(){return None;}
  if let Pointer::Hover(x,y)=event {
   let m=self.desk_metrics();let (_,bar,rects)=self.desk_dock(&m);let now=t/1000.;
   let zone=self.dock_zone(bar);
   let p=self.dock_preferences();
   let edge=match p.position{Position::Bottom=>y>=self.h as f32-self.ui.pt(4.),Position::Left=>x<=self.ui.pt(4.),Position::Right=>x>=self.w as f32-self.ui.pt(4.)};
   let inside=bar.contains(x,y);
   let (_,live)=self.dock_live(bar,&rects,now);
   let in_zone=self.dock_is_visible()&&(inside||zone.contains(x,y)||self.dock_press(&live).iter().any(|r|r.contains(x,y)));
   let magnify=in_zone&&p.magnification;
   let menu=matches!(self.desktop_menu,Some(Menu::Dock|Menu::Stack));
   let cur=self.dock_mag_amount(now);
   let d=self.desk.as_mut().unwrap();
   if magnify!=d.dock_ptr.is_some(){d.dock_mag=Some(Anim{from:cur,to:if magnify{1.}else{0.},start:now,kind:Kind::DockMagnify,curve:Curve::EaseOut});self.dirty=true;}
   let moved=d.dock_ptr!=magnify.then_some((x,y));d.dock_ptr=magnify.then_some((x,y));if moved&&magnify{self.dirty=true;}
   // the hovered icon comes from the row magnified about this pointer, the one drawn and pressed next (W2-23): the
   // layout of the previous pointer put the label, and a quick press, on a neighbour; `dock_tick` re-tests it per frame
   let hov=if in_zone{self.dock_hit(bar,&rects,x,y,now)}else{None};
   self.dock_set_hover(hov,t);
   let d=self.desk.as_mut().unwrap();
   if d.dock_label_off.is_some_and(|p|p!=(x,y)){d.dock_label_off=None;self.dirty=true;}
   if edge||in_zone||menu {d.dock_hide_at=None;dock_reveal(d,now);if !d.dock_visible{self.dirty=true;}}
   else if d.dock_autohides()&&d.dock_hide_at.is_none(){d.dock_hide_at=Some(t+550.);}
   return Some(vec![]);
  }
  if let Pointer::Context(x,y)=event {let m=self.desk_metrics();let (list,bar,rects)=self.desk_dock(&m);let (bar,slid)=self.dock_slid(bar,&rects,t/1000.);let rects=self.dock_magnified(&slid,t/1000.);let plate=dock_plate(bar,&slid,&rects,self.dock_preferences().position!=Position::Bottom);let press=self.dock_press(&rects);if self.dock_is_visible()&&(plate.contains(x,y)||press.iter().any(|r|r.contains(x,y))){let target=press.iter().position(|r|r.contains(x,y)).map(|i|list[i]).filter(|i|*i!=apps().len()&&*i!=super::desk::DOCK_TRASH&&*i!=DOCK_STACK&&dock_min_index(*i).is_none());let d=self.desk.as_mut().unwrap();d.dock_menu_target=target;d.dock_menu_anchor=(x,y);self.desktop_menu=Some(Menu::Dock);self.dirty=true;return Some(vec![]);}}
  None
 }
 pub fn desktop_pointer(&mut self,x:f32,y:f32,context:bool,t:f64){if self.modal_layer.is_some(){return;}if !context{self.note_pointer_y(y);self.pointer_hover(x,y);self.hover_speech_at(x,y,t);self.notify_hover(x,y,t);}#[cfg(feature="desktop-wayland")] if !context&&self.desk_active()&&!self.is_locked(){self.arrange_hover(x,y,t);self.desktop_menu_hover_at(x,y,t);}if !context&&self.desk_active()&&!self.is_locked(){let m=self.desk_metrics();let revealed=self.desk_reveal_at(x,y);let d=self.desk.as_mut().unwrap();let over=match revealed{Some((i,super::desk::RevealHit::Light(_)))=>Some(d.wins[i].app),Some(_)=>None,None=>match d.pick(&m,x,y){Some((i,super::desk::Hit::Light(_)))=>Some(d.wins[i].app),_=>None}};if d.lights_hover!=over{d.lights_hover=over;self.dirty=true;}}if self.dock_pointer(if context{Pointer::Context(x,y)}else{Pointer::Hover(x,y)},t).is_some(){return;}if context&&self.desk_active()&&!self.is_locked(){let m=self.desk_metrics();let (picked,y)=match self.desk_reveal_at(x,y){Some((i,super::desk::RevealHit::App(ly)))=>(Some((i,super::desk::Hit::Content)),ly),Some(_)=>(None,y),None=>(self.desk.as_ref().unwrap().pick(&m,x,y),y)};if let Some((i,super::desk::Hit::Content))=picked{let w=self.desk.as_ref().unwrap().wins[i];let b=w.body(&m);if let AppId::External(slot)=w.app {if let Some(Some(host))=self.ext.get_mut(slot as usize){host.send(&bifrost_proto::ToApp::Context{x:x-b.x as f32,y:y-b.y as f32});}}}}}
 pub(super) fn dock_tick(&mut self,t:f64){
  let now=t/1000.;let menu_open=matches!(self.desktop_menu,Some(Menu::Dock|Menu::Stack));
  // the Downloads stack re-lists its folder every few seconds (never per frame)
  let stack_changed=self.desk.as_mut().is_some_and(|d|d.stack_refresh(now));if stack_changed{self.dirty=true;}
  let keep_flags:Vec<bool>=self.desk.as_ref().map(|d|d.dock_bounce.iter().map(|&(i,start)|{let el=now-start;el<BOUNCE_MAX_S&&(!self.dock_item_running(i)||el<motion::duration(Kind::DockBounce) as f64)}).collect()).unwrap_or_default();
  let Some(d)=self.desk.as_mut()else{return;};
  // a launch bounce lasts one hop once the app's first window maps (none under Reduce Motion), or ends after a timeout
  if !d.dock_bounce.is_empty(){let mut keep=keep_flags.into_iter();let before=d.dock_bounce.len();d.dock_bounce.retain(|_|keep.next().unwrap_or(false));if d.dock_bounce.len()!=before{self.dirty=true;}}
  let d=self.desk.as_mut().unwrap();
  if d.dock_autohides()&&d.dock_hide_at.is_some_and(|at|t>=at)&&d.grab_idle()&&!menu_open{
   d.dock_hide_at=None;
   if d.dock_visible&&!d.dock_vis_anim.as_ref().is_some_and(|a|a.to<0.5){let cur=d.dock_vis_anim.as_ref().map_or(1.,|a|a.value(now));d.dock_vis_anim=Some(Anim{from:cur,to:0.,start:now,kind:Kind::PanelReveal,curve:Curve::EaseInOut});}
   self.dirty=true;
  }
  let d=self.desk.as_mut().unwrap();
  if d.dock_vis_anim.as_ref().is_some_and(|a|a.done(now)){let hide=d.dock_vis_anim.as_ref().is_some_and(|a|a.to<0.5);if hide{d.dock_visible=false;}d.dock_vis_anim=None;self.dirty=true;}
  // the row keeps moving under a resting pointer while it grows, shrinks or slides in, so the hovered icon (its label,
  // a press latched from it) is hit-tested again against the row this frame draws, not the one of the last pointer event
  if let Some((x,y))=self.desk.as_ref().and_then(|d|d.dock_ptr){
   let m=self.desk_metrics();let (_,bar,rects)=self.desk_dock(&m);
   let hov=self.dock_hit(bar,&rects,x,y,now);self.dock_set_hover(hov,t);
  }
 }
 /// The Dock slot whose press area (`dock_press`) contains (`x`, `y`) in the row drawn at `now` seconds: slid, then
 /// magnified about the current pointer and clamped to the screen. Hover, labels, press, release and the context menu
 /// all use this one geometry.
 pub(super) fn dock_hit(&self,bar:Rect,rects:&[Rect],x:f32,y:f32,now:f64)->Option<usize>{
  self.dock_press(&self.dock_live(bar,rects,now).1).iter().position(|r|r.contains(x,y))
 }
 /// Sets the hovered Dock slot, starting the label cross-fade when it changes (`t` ms).
 fn dock_set_hover(&mut self,hov:Option<usize>,t:f64){
  let Some(d)=self.desk.as_mut()else{return;};
  if d.dock_hover!=hov{d.dock_label_last=d.dock_hover;d.dock_label_ms=t;d.dock_hover=hov;self.dirty=true;}
 }
 /// Whether a dock list entry is an app the user keeps in the Dock (not the Apps icon, Trash, the Downloads stack, a minimized window or a recent).
 pub(super) fn dock_removable(&self,entry:usize)->bool{entry!=apps().len()&&entry!=DOCK_TRASH&&dock_min_index(entry).is_none()&&self.dock_section(entry)==0}
 pub(super) fn dock_reorder(&mut self,from:usize,to:usize,after:bool){let Some(key)=self.dock_app_key(from)else{return;};let target=self.dock_app_key(to);let d=self.desk.as_mut().unwrap();d.dock_preferences.pinned.retain(|s|s!=&key);let at=target.and_then(|key|d.dock_preferences.pinned.iter().position(|s|s==&key)).map(|at|at+after as usize).unwrap_or(d.dock_preferences.pinned.len());d.dock_preferences.pinned.insert(at.min(d.dock_preferences.pinned.len()),key);d.dock_preferences.save(&self.data_dir);self.dirty=true;}
 pub(super) fn dock_is_visible(&self)->bool{self.desk.as_ref().is_some_and(|d|!d.dock_autohides()||d.dock_visible)}
 /// A stable surface with room for magnified icons, bounce, labels and shadow. Transparent margins do not take input.
 pub(super) fn dock_surface_rect(&self)->Rect {
  if !self.dock_is_visible(){return self.dock_overlay_rect();}
  let m=self.desk_metrics();let (_,bar,rects)=self.desk_dock(&m);let p=self.dock_preferences();
  let shadow=self.ui.pt(super::desk::SHADOW_REACH_PT).ceil() as i32;
  let label_h=self.ui.pt(2.*MAC_CALLOUT_PT+2.*DOCK_LABEL_PAD_Y_PT+DOCK_LABEL_GAP_PT).ceil() as i32;
  let label_w=self.ui.pt(DOCK_LABEL_MAX_WIDTH_PT+2.*DOCK_LABEL_PAD_X_PT+DOCK_LABEL_GAP_PT).ceil() as i32;
  let max=self.ui.pt(if p.magnification{p.magnified.max(p.size)}else{p.size}).ceil() as i32;
  let base=rects.first().map_or(max,|r|r.w.min(r.h));
  let grow=(max-base).max(0)+max/2;
  let vertical=p.position!=Position::Bottom;
  let along=self.dock_axis_room().max(if vertical{label_h/2}else{label_w/2})+shadow;
  let inward=grow+if vertical{label_w}else{label_h}+shadow;
  let r=match p.position{
   Position::Bottom=>Rect{x:bar.x-along,y:bar.y-inward,w:bar.w+2*along,h:bar.h+inward+shadow},
   Position::Left=>Rect{x:bar.x-shadow,y:bar.y-along,w:bar.w+inward+shadow,h:bar.h+2*along},
   Position::Right=>Rect{x:bar.x-inward,y:bar.y-along,w:bar.w+inward+shadow,h:bar.h+2*along},
  };
  let x=r.x.max(0);let y=r.y.max(m.menubar);
  Rect{x,y,w:(r.x+r.w).min(self.w).saturating_sub(x).max(1),h:(r.y+r.h).min(self.h).saturating_sub(y).max(1)}
 }
 /// The plate and magnified icons take input; labels and the surface's unused margins let clicks through.
 pub(super) fn dock_input_rect(&self,surface:Rect)->Option<[i32;4]>{
  if !self.dock_is_visible(){return None;}
  let m=self.desk_metrics();let (_,bar,rects)=self.desk_dock(&m);let (_,live)=self.dock_live(bar,&rects,self.desk_t/1000.);
  let mut p=self.dock_overlay_rect();for r in live{let x=p.x.min(r.x);let y=p.y.min(r.y);p=Rect{x,y,w:(p.x+p.w).max(r.x+r.w)-x,h:(p.y+p.h).max(r.y+r.h)-y};}
  let x=p.x.max(surface.x);let y=p.y.max(surface.y);let right=(p.x+p.w).min(surface.x+surface.w);let bottom=(p.y+p.h).min(surface.y+surface.h);
  Some([x-surface.x,y-surface.y,(right-x).max(0),(bottom-y).max(0)])
 }
 /// The Dock as it is drawn now: the plate (grown by magnification), or the strip at the edge that reveals a hidden Dock.
 pub(super) fn dock_overlay_rect(&self)->Rect {let m=self.desk_metrics();if self.dock_is_visible(){
  self.dock_plate_at(self.desk_t/1000.)
 }else{let p=self.ui.pt(3.) as i32;match self.dock_preferences().position {Position::Bottom=>Rect{x:0,y:self.h-p,w:self.w,h:p},Position::Left=>Rect{x:0,y:m.menubar,w:p,h:self.h-m.menubar},Position::Right=>Rect{x:self.w-p,y:m.menubar,w:p,h:self.h-m.menubar}}}}
}

/// Entries of the `desk_dock` list from this value down (below `DOCK_TRASH`) stand for minimized windows:
/// entry `DOCK_MIN_BASE - n` is window `n` of the desk.
pub(super) const DOCK_MIN_BASE:usize=usize::MAX-1;
const DOCK_MIN_SPAN:usize=4096;
/// The window index a minimized-window entry stands for.
pub(super) fn dock_min_index(entry:usize)->Option<usize>{(entry<=DOCK_MIN_BASE&&entry>DOCK_MIN_BASE-DOCK_MIN_SPAN).then(||DOCK_MIN_BASE-entry)}
/// How far (px) past the dock capsule a dragged icon must be released to leave the Dock.
pub(super) fn dock_drag_off(bar:Rect,x:f32,y:f32,reach:f32)->bool{
 let dx=(bar.x as f32-x).max(0.).max(x-(bar.x+bar.w) as f32);
 let dy=(bar.y as f32-y).max(0.).max(y-(bar.y+bar.h) as f32);
 (dx*dx+dy*dy).sqrt()>=reach.max(0.)
}
/// A launch bounce gives up after this long even if no window maps.
const BOUNCE_MAX_S:f64=10.;
/// The Dock plate for a magnified row: `bar` stretched along its axis (x, or y when `vertical`) so the first and last
/// `live` icons keep the padding the `resting` row has to the capsule's ends. It never shrinks below `bar`; its depth
/// (across the axis) stays the capsule's, as the Dock's does, so magnified icons rise out of it but never past its ends.
pub fn dock_plate(bar:Rect,resting:&[Rect],live:&[Rect],vertical:bool)->Rect{
 let (Some(r0),Some(r1),Some(l0),Some(l1))=(resting.first(),resting.last(),live.first(),live.last()) else {return bar;};
 let (start,len):(fn(&Rect)->i32,fn(&Rect)->i32)=if vertical{(|r|r.y,|r|r.h)}else{(|r|r.x,|r|r.w)};
 let (b0,b1)=(start(&bar),start(&bar)+len(&bar));
 let lo=(start(l0)-(start(r0)-b0)).min(b0);
 let hi=(start(l1)+len(l1)+(b1-start(r1)-len(r1))).max(b1);
 if vertical{Rect{y:lo,h:hi-lo,..bar}}else{Rect{x:lo,w:hi-lo,..bar}}
}
/// Pointer magnification: 1 at the pointer, 0 at `radius` and beyond, a cosine in between.
pub fn magnify_factor(dist:f32,radius:f32)->f32{if radius<=0.||dist>=radius{0.}else{0.5+0.5*(std::f32::consts::PI*dist.max(0.)/radius).cos()}}
/// Scales a premultiplied colour (all four channels) by `k`, for fades.
pub(super) fn fade(c:u32,k:f32)->u32{let k=k.clamp(0.,1.);let m=|shift:u32|((((c>>shift)&255) as f32*k).round() as u32)<<shift;m(0)|m(8)|m(16)|m(24)}
/// Magnifies a row of equal square icon `rects` around `ptr` (a coordinate along the dock axis: x unless `vertical`).
/// Sizes follow `magnify_factor` between the base size and `max`, quantised to 2 px so scaled icons cache well; gaps are
/// kept, and icons keep their far edge (`far`: bottom or right) or near edge, so they grow into the screen.
///
/// The row grows about the pointer, as the reference Dock does: the point of the resting row under the pointer (a
/// fraction of an icon, or of the gap between two) is the same fraction of the same icon or gap in the magnified row,
/// and it stays under the pointer, so the icons before it grow away to the start and those after it to the end. The
/// icon under the pointer is therefore the icon drawn under it at every magnification amount (E2E N5: the row used to
/// stay centred on its resting middle, which slid a neighbour under a pointer away from the middle). A pointer beyond
/// either end of the row keeps that end where it rests. `Shell::dock_magnified` clamps the ends to the screen after.
pub fn magnified_rects(rects:&[Rect],vertical:bool,far:bool,ptr:f32,max:i32,radius:f32,amount:f32)->Vec<Rect>{
 if rects.is_empty()||amount<=0.{return rects.to_vec();}
 let (start,len):(fn(&Rect)->i32,fn(&Rect)->i32)=if vertical{(|r|r.y,|r|r.h)}else{(|r|r.x,|r|r.w)};
 let base=len(&rects[0]);
 let sizes:Vec<i32>=rects.iter().map(|r|{let c=start(r) as f32+len(r) as f32/2.;let f=magnify_factor((c-ptr).abs(),radius)*amount.min(1.);base+(((max-base).max(0) as f32*f/2.).round() as i32)*2}).collect();
 let gaps:Vec<i32>=rects.windows(2).map(|w|start(&w[1])-start(&w[0])-len(&w[0])).collect();
 // the magnified row laid out from 0
 let mut at=0;let mut starts=Vec::with_capacity(rects.len());
 for (i,sz) in sizes.iter().enumerate(){starts.push(at);at+=sz+gaps.get(i).copied().unwrap_or(0);}
 // the anchor: a resting coordinate `a` and where the same point lies in the magnified row (`q`)
 let n=rects.len();let last=&rects[n-1];
 let (a,q)=if ptr<=start(&rects[0]) as f32{(start(&rects[0]) as f32,0.)}
  else if ptr>=(start(last)+len(last)) as f32{((start(last)+len(last)) as f32,(starts[n-1]+sizes[n-1]) as f32)}
  else{
   let mut anchor=(ptr,(starts[n-1]+sizes[n-1]) as f32);
   for (i,r) in rects.iter().enumerate(){
    let (s0,l)=(start(r) as f32,len(r) as f32);
    if ptr<s0+l{anchor=(ptr,starts[i] as f32+(ptr-s0).max(0.)/l.max(1.)*sizes[i] as f32);break;}
    let g=gaps.get(i).copied().unwrap_or(0) as f32;
    // gaps keep their width, so the pointer keeps its distance past the icon before it
    if ptr<s0+l+g{anchor=(ptr,(starts[i]+sizes[i]) as f32+(ptr-s0-l));break;}
   }
   anchor
  };
 let offset=(a-q).round() as i32;
 rects.iter().enumerate().map(|(i,r)|{let sz=sizes[i];let at=starts[i]+offset;if vertical{let x=if far{r.x+r.w-sz}else{r.x};Rect{x,y:at,w:sz,h:sz}}else{let y=if far{r.y+r.h-sz}else{r.y};Rect{x:at,y,w:sz,h:sz}}}).collect()
}
impl Shell {
 /// Where a dock list entry sits in the section order: 0 kept in the Dock, 1 recents (running, not kept), 2 minimized windows and the Trash.
 pub(super) fn dock_section(&self,entry:usize)->u8{
  if entry==DOCK_TRASH||entry==DOCK_STACK||dock_min_index(entry).is_some(){return 2;}
  let Some(d)=self.desk.as_ref()else{return 0;};
  if entry<apps().len(){return if d.dock_preferences.pinned.iter().any(|k|k==apps()[entry].icon){0}else{1};}
  if entry==apps().len(){return 0;}
  #[cfg(feature="desktop-wayland")]
  if let Some(a)=d.linux_apps.get(entry-apps().len()-1){return if d.dock_preferences.pinned.iter().any(|k|k.strip_prefix("linux:")==Some(a.app_id.as_str())){0}else{1};}
  1
 }
 /// Indices `k` in `list` with a divider between entry `k` and `k + 1`.
 pub(super) fn dock_seps(&self,list:&[usize])->Vec<usize>{(0..list.len().saturating_sub(1)).filter(|&k|self.dock_section(list[k])<self.dock_section(list[k+1])).collect()}
 #[cfg(feature="desktop-wayland")]
 pub(super) fn dock_linux_name(&self,entry:usize)->String{self.desk.as_ref().and_then(|d|d.linux_apps.get(entry.wrapping_sub(apps().len()+1))).map(|a|a.app_id.clone()).unwrap_or_default()}
 #[cfg(not(feature="desktop-wayland"))]
 pub(super) fn dock_linux_name(&self,_entry:usize)->String{String::new()}
 /// Whether the app behind a dock list entry has a window (or, for a native client, a mapped surface).
 pub(super) fn dock_item_running(&self,entry:usize)->bool{
  #[cfg_attr(not(feature="desktop-wayland"),allow(unused_variables))] let Some(d)=self.desk.as_ref()else{return false;};
  if entry<apps().len(){
   // one tile stands for every entry of the app (same icon key), so a running instance lights the pinned tile
   return apps().iter().enumerate().any(|(i,a)|(i==entry||a.icon==apps()[entry].icon)&&self.dock_entry_running(i));
  }
  #[cfg(feature="desktop-wayland")] if entry>apps().len(){return d.linux_apps.get(entry-apps().len()-1).is_some_and(|a|!a.windows.is_empty());}
  false
 }
 /// Whether the app of built-in or installed entry `entry` (an index into `apps()`) has a window (main or extra), or is an SDK app
 /// still running after it ordered its main window out (Cmd+W on its last document).
 pub(super) fn dock_entry_running(&self,entry:usize)->bool{
  let Some(d)=self.desk.as_ref()else{return false;};
  let Some(app)=apps().get(entry)else{return false;};
  // any window counts, an extra one too: an app whose main window was closed keeps its tile and dot while a compose or
  // document window of it is open, so that window is not an orphan the Dock knows nothing about
  let mut running=d.has_window_of(app.id);
  // the app keeps its tile and running dot with no window (E2E13 1); a click reopens its window (`desk_open`)
  if let AppId::External(slot)=app.id{running|=self.ext.get(slot as usize).and_then(Option::as_ref).is_some_and(|h|h.running()&&h.main_ordered_out);}
  #[cfg(feature="desktop-wayland")] if let AppId::External(slot)=app.id{running|=self.foreign_running(slot as usize)||super::apphost::installed().get(slot as usize).is_some_and(|p|!p.manifest.wayland_app_id.is_empty()&&d.native_windows.iter().any(|w|w.app_id==p.manifest.wayland_app_id));}
  running
 }
 /// Starts the launch bounce of a dock entry that has no window yet (it ends when the first window maps).
 pub(super) fn dock_start_bounce(&mut self,entry:usize,t:f64){
  if entry==apps().len()||entry==DOCK_TRASH||dock_min_index(entry).is_some()||self.dock_item_running(entry){return;}
  if let Some(d)=self.desk.as_mut(){if !d.dock_bounce.iter().any(|b|b.0==entry){d.dock_bounce.push((entry,t/1000.));self.dirty=true;}}
 }
 /// The bounce offset in px (dx, dy) of list entry `entry` at `now` seconds: the launch bounce plus the attention bounce.
 pub(super) fn dock_bounce_offset(&self,entry:usize,now:f64,icon:i32)->(i32,i32){
  let (lx,ly)=self.dock_launch_offset(entry,now,icon);let (ax,ay)=self.dock_tile_bounce(entry,now,icon);(lx+ax,ly+ay)
 }
 /// The Dock tile state of list entry `entry` when it is an installed app.
 fn dock_tile_state(&self,entry:usize)->Option<&TileState> {
  let Some(AppId::External(slot))=apps().get(entry).map(|a|a.id) else {return None;};
  self.ext.get(slot as usize).and_then(Option::as_ref).map(|h|&h.tile)
 }
 /// The attention bounce offset (px) of the Dock tile of `entry` at `now` seconds, while the app's `bounce` window is open.
 fn dock_tile_bounce(&self,entry:usize,now:f64,icon:i32)->(i32,i32){
  let Some((start,end))=self.dock_tile_state(entry).and_then(|st|st.bounce) else {return(0,0);};
  let lift=bounce_lift(now,start,end,motion::duration(Kind::DockBounce) as f64,icon);
  match self.dock_preferences().position{Position::Bottom=>(0,-lift),Position::Left=>(lift,0),Position::Right=>(-lift,0)}
 }
 /// Draws the progress of the Dock tile of `entry` (`dock` service `progress`): a thin bar across the lower part of the
 /// icon, filled to the fraction, or sweeping while indeterminate.
 pub(super) fn dock_tile_progress(&self,s:&mut Surface,entry:usize,r:&Rect,t:f64){
  let Some(p)=self.dock_tile_state(entry).and_then(|st|st.progress) else {return;};
  let ui=&self.ui;
  let inset=r.w as f32*0.15;let h=ui.pt(4.);
  let track=bifrost_ui::Rect{x:r.x as f32+inset,y:(r.y+r.h) as f32-ui.pt(12.),w:r.w as f32-2.*inset,h};
  s.fill_rrect(track.x,track.y,track.w,track.h,h/2.,FILL_GRAY);
  let fill=p.filled(track,t/1000.,PROGRESS_PERIOD_S);
  if fill.w>0.{s.fill_rrect(fill.x,fill.y,fill.w,fill.h,h/2.,LINK);}
 }
 /// The launch bounce offset of list entry `entry`: hops of `Kind::DockBounce` length, none under Reduce Motion.
 fn dock_launch_offset(&self,entry:usize,now:f64,icon:i32)->(i32,i32){
  let Some(d)=self.desk.as_ref()else{return(0,0);};
  let Some(&(_,start))=d.dock_bounce.iter().find(|b|b.0==entry)else{return(0,0);};
  let dur=motion::duration(Kind::DockBounce) as f64;let el=now-start;
  if dur<=0.||el<0.||el>BOUNCE_MAX_S{return(0,0);}
  let ph=(el/dur).fract() as f32;let lift=(icon as f32*0.5*4.*ph*(1.-ph)).round() as i32;
  match self.dock_preferences().position{Position::Bottom=>(0,-lift),Position::Left=>(lift,0),Position::Right=>(-lift,0)}
 }
 /// The area around the dock where the pointer magnifies it: the capsule plus the room magnified icons need.
 pub(super) fn dock_zone(&self,bar:Rect)->Rect{let p=self.dock_preferences();if !p.magnification{return bar;}let base=bar.w.min(bar.h) as f32*p.size/(p.size+2.*super::desk::DOCK_PAD_PT);let grow=(self.ui.pt(p.magnified)-base).max(0.).ceil() as i32;Rect{x:bar.x-grow,y:bar.y-grow,w:bar.w+2*grow,h:bar.h+2*grow}}
 /// Magnification amount 0..1 at `now` seconds.
 pub(super) fn dock_mag_amount(&self,now:f64)->f32{self.desk.as_ref().map_or(0.,|d|match &d.dock_mag{Some(a)=>a.value(now),None=>if d.dock_ptr.is_some(){1.}else{0.}})}
 /// The dock capsule and icon slots shifted by the autohide slide at `now` seconds.
 pub(super) fn dock_slid(&self,bar:Rect,rects:&[Rect],now:f64)->(Rect,Vec<Rect>){
  let p=self.desk.as_ref().and_then(|d|d.dock_vis_anim.as_ref()).map_or(1.,|a|a.value(now)).clamp(0.,1.);
  if p>=1.{return(bar,rects.to_vec());}
  let away=1.-p;
  let (dx,dy)=match self.dock_preferences().position{Position::Bottom=>(0,((self.h-bar.y) as f32*away) as i32),Position::Left=>(-(((bar.x+bar.w) as f32*away) as i32),0),Position::Right=>((((self.w-bar.x) as f32)*away) as i32,0)};
  let sh=|r:&Rect|Rect{x:r.x+dx,y:r.y+dy,..*r};
  (sh(&bar),rects.iter().map(sh).collect())
 }
 /// The press areas of the icon slots `live` (as `dock_live` gives them): each grown along the Dock's axis by half the
 /// gap to its neighbour (`desk::dock_press_rects`). Press, release, the hovered icon (its label) and the context menu
 /// all hit-test these, so the icon a pointer in the gap magnifies and names is the one a click opens (E2E N5).
 pub(super) fn dock_press(&self,live:&[Rect])->Vec<Rect>{super::desk::dock_press_rects(live,self.dock_preferences().position!=Position::Bottom)}
 /// Icon slots as drawn and hit-tested now: slid, then magnified around the pointer.
 pub(super) fn dock_live(&self,bar:Rect,rects:&[Rect],now:f64)->(Rect,Vec<Rect>){let (bar,slid)=self.dock_slid(bar,rects,now);let live=self.dock_magnified(&slid,now);(bar,live)}
 pub(super) fn dock_magnified(&self,rects:&[Rect],now:f64)->Vec<Rect>{
  let Some(d)=self.desk.as_ref()else{return rects.to_vec();};
  let p=&d.dock_preferences;
  let Some((px,py))=d.dock_ptr.filter(|_|p.magnification&&d.dock_grab_magnifies())else{return rects.to_vec();};
  let cross=if p.position==Position::Bottom{self.h-self.desktop_bar_height() as i32}else{self.w};
  let max=(self.ui.pt(p.magnified).round() as i32).min(cross*2/3).max(rects.first().map_or(1,|r|r.w));
  let (vertical,far,ptr)=match p.position{Position::Bottom=>(false,true,px),Position::Left=>(true,false,py),Position::Right=>(true,true,py)};
  let amount=self.dock_mag_amount(now);let radius=max as f32*1.6;
  let mut live=magnified_rects(rects,vertical,far,ptr,max,radius,amount);
  let edge=self.ui.pt(super::desk::DOCK_MARGIN_PT).round() as i32;
  let base=rects.first().map_or(1,|r|r.w);let pad=(self.ui.pt(super::desk::DOCK_PAD_PT)*base as f32/self.ui.pt(p.size).max(1.)).floor() as i32;
  let lo=if vertical{self.desktop_bar_height() as i32+edge+pad}else{edge+pad};
  let hi=if vertical{self.h-edge-pad}else{self.w-edge-pad};
  let span=|row:&[Rect]|row.first().zip(row.last()).map_or(0,|(a,b)|if vertical{b.y+b.h-a.y}else{b.x+b.w-a.x});
  if span(&live)>hi-lo{
   let(mut low,mut high)=(0.,amount);for _ in 0..10{let mid=(low+high)*0.5;let row=magnified_rects(rects,vertical,far,ptr,max,radius,mid);if span(&row)<=hi-lo{low=mid;live=row;}else{high=mid;}}
   if span(&live)>hi-lo{live=rects.to_vec();}
  }
  if let(Some(first),Some(last))=(live.first(),live.last()){
   let(start,end)=if vertical{(first.y,last.y+last.h)}else{(first.x,last.x+last.w)};
   let shift=if start<lo{lo-start}else if end>hi{hi-end}else{0};
   for r in &mut live{if vertical{r.y+=shift;}else{r.x+=shift;}}
  }
  live
 }
 /// The Dock plate drawn now: the capsule grown along the Dock's axis around the magnified row (W2-24).
 pub(super) fn dock_plate_at(&self,now:f64)->Rect{
  let m=self.desk_metrics();let (_,bar,rects)=self.desk_dock(&m);
  let (bar,slid)=self.dock_slid(bar,&rects,now);let live=self.dock_magnified(&slid,now);
  dock_plate(bar,&slid,&live,self.dock_preferences().position!=Position::Bottom)
 }
 /// How far (px) the plate can grow past the resting capsule at each end of its axis while magnified: the growth with
 /// the pointer on the middle icon (where the most icons are inside the radius) plus one magnified icon of slack.
 pub(super) fn dock_axis_room(&self)->i32{
  let p=self.dock_preferences();if !p.magnification{return 0;}
  let m=self.desk_metrics();let (_,bar,rects)=self.desk_dock(&m);
  let vertical=p.position!=Position::Bottom;let max=self.ui.pt(p.magnified).round() as i32;
  let Some(mid)=rects.get(rects.len()/2) else {return 0;};
  let ptr=if vertical{mid.y as f32+mid.h as f32/2.}else{mid.x as f32+mid.w as f32/2.};
  let live=magnified_rects(&rects,vertical,true,ptr,max,max as f32*1.6,1.);
  let plate=dock_plate(bar,&rects,&live,vertical);
  let (a,b)=if vertical{(plate.h,bar.h)}else{(plate.w,bar.w)};
  ((a-b).max(0)+1)/2+(max-rects[0].w.min(rects[0].h)).max(0)
 }
 /// The hover label of Dock list entry `entry`: the app's name, "Applications" for the Apps grid icon (as the reference
 /// labels it, E2E6 5), Trash, the Downloads stack, a minimized window's app, or a native client's id; empty for none.
 pub(super) fn dock_label_text(&self,entry:usize)->String{
  if entry<apps().len(){return apps()[entry].name.to_string();}
  if entry==apps().len(){return "Applications".into();}
  if entry==super::desk::DOCK_TRASH{return "Trash".into();}
  if entry==super::dock_stack::DOCK_STACK{return "Downloads".into();}
  if let Some(wi)=dock_min_index(entry){
   return self.desk.as_ref().and_then(|d|d.wins.get(wi)).and_then(|w|apps().iter().find(|a|a.id==w.app)).map_or(String::new(),|a|a.name.to_string());
  }
  self.dock_linux_name(entry)
 }
 /// True while a Dock context menu or the Downloads stack is open: the hover label is not drawn then, as in the
 /// reference (the stack's popover covered all but the tail of "Downloads", E2E8 7).
 pub(super) fn dock_label_hidden_by_menu(&self)->bool{matches!(self.desktop_menu,Some(Menu::Dock|Menu::Stack))}
 /// The hover label's slot and opacity at `t` ms: fading in for the hovered slot, out for the one just left.
 pub(super) fn dock_label_state(&self,t:f64)->(Option<usize>,f32){
  let Some(d)=self.desk.as_ref()else{return(None,0.);};
  let dur=motion::duration(Kind::CrossFade) as f64*1000.;let el=t-d.dock_label_ms;
  let k=if dur<=0.{1.}else{motion::ease(Curve::EaseOut,(el/dur) as f32)};
  match(d.dock_hover,d.dock_label_last){(Some(h),_)=>(Some(h),k),(None,Some(l))=>(Some(l),1.-k),_=>(None,0.)}
 }
 /// True while any dock animation (slide, magnify, label fade, launch bounce, app attention bounce or an indeterminate
 /// progress bar) needs frames; `next_wake` polls it.
 pub fn dock_animating(&self,t:f64)->bool{
  let Some(d)=self.desk.as_ref()else{return false;};let now=t/1000.;
  d.dock_vis_anim.as_ref().is_some_and(|a|!a.done(now))||d.dock_mag.as_ref().is_some_and(|a|!a.done(now))||(d.dock_label_ms>0.&&t-d.dock_label_ms<400.)||(motion::duration(Kind::DockBounce)>0.&&d.dock_bounce.iter().any(|b|now-b.1<BOUNCE_MAX_S))||self.ext.iter().flatten().any(|h|h.tile.bounce.is_some_and(|(_,end)|now<end)||h.tile.progress.is_some_and(|p|p.fraction().is_none()))
 }
 /// Option+Cmd+D: toggles Automatically Hide and Show; with it on, the dock slides away at the next tick.
 pub fn dock_toggle_autohide(&mut self)->bool{
  if !self.desk_active()||self.is_locked(){return false;}
  // t = -700 ms makes the hide deadline 0, i.e. already due, instead of the menu's 700 ms grace
  let _=self.dock_change(Change::Autohide,-700.);true
 }
 /// Where the dock shows `app` on screen: the rect the compositor shrinks a minimizing window into (and grows it back from).
 ///
 /// With `window = Some(n)` (index into the desk's windows) a minimized window's own slot to the right of the last
 /// divider is returned when it exists; otherwise the app's icon slot; otherwise the free slot just left of the Trash.
 /// The rect is the resting slot (not magnified), in screen px, for every dock position, and it is returned even
 /// when autohide has the dock slid away, so the compositor animates towards the dock's edge.
 pub fn dock_icon_rect(&self,app:AppId,window:Option<usize>)->Option<Rect>{
  self.desk.as_ref()?;
  let m=self.desk_metrics();let (list,_,rects)=self.desk_dock(&m);
  let slot=window.and_then(|wi|list.iter().position(|&e|dock_min_index(e)==Some(wi)))
   .or_else(||{let target=Self::dock_app(app);apps().iter().position(|a|a.id==app||a.id==target).and_then(|i|list.iter().position(|&e|e==i))});
  if let Some(k)=slot{return rects.get(k).copied();}
  let trash=*rects.last()?;
  Some(match self.dock_preferences().position{Position::Bottom=>Rect{x:trash.x-trash.w-trash.w/4,..trash},_=>Rect{y:trash.y-trash.h-trash.h/4,..trash}})
 }
}
impl Shell {
 /// A swipe up from the bottom edge of the tablet desktop shows the Dock (`tablet_gestures`), as a pointer at its edge does.
 pub(super) fn desk_reveal_dock(&mut self,t:f64){
  if let Some(d)=self.desk.as_mut(){d.dock_hide_at=None;dock_reveal(d,t/1000.);}
  self.dirty=true;
 }
}

/// Makes the dock visible again, sliding it in from wherever a slide-out had got to.
fn dock_reveal(d:&mut super::desk::Desk,now:f64){
 let hiding=d.dock_vis_anim.as_ref().is_some_and(|a|a.to<0.5);
 if !d.dock_visible||hiding{let cur=d.dock_vis_anim.as_ref().map_or(0.,|a|a.value(now));d.dock_visible=true;d.dock_vis_anim=Some(Anim{from:cur,to:1.,start:now,kind:Kind::PanelReveal,curve:Curve::EaseOut});}
}

#[cfg(test)]mod tests {
 use super::*;

 /// E2E6 5: the Apps grid icon labels "Applications" on hover like every other icon labels its name; Trash keeps its own.
 #[test]
 fn the_apps_grid_icon_is_labelled_applications(){
  let sh=desktop(2560,1440);
  assert_eq!(sh.dock_label_text(apps().len()),"Applications");
  assert_eq!(sh.dock_label_text(super::super::desk::DOCK_TRASH),"Trash");
  assert_eq!(sh.dock_label_text(0),apps()[0].name);
 }
 /// E2E8 7: the Downloads label must not show through the open stack, nor over a Dock context menu.
 #[test]
 fn an_open_stack_or_dock_menu_hides_the_hover_label(){
  let mut sh=desktop(2560,1440);
  assert!(!sh.dock_label_hidden_by_menu());
  sh.desktop_menu=Some(Menu::Stack);assert!(sh.dock_label_hidden_by_menu());
  sh.desktop_menu=Some(Menu::Dock);assert!(sh.dock_label_hidden_by_menu());
  sh.desktop_menu=None;assert!(!sh.dock_label_hidden_by_menu());
 }
 /// A click on an icon puts its name away: it stays away over a resting pointer and returns once the pointer moves.
 #[test]
 fn a_click_silences_the_hover_label_until_the_pointer_moves(){
  let mut sh=desktop(1280,800);
  {let p=&mut sh.desk.as_mut().unwrap().dock_preferences;p.position=Position::Bottom;p.magnification=false;}
  let m=sh.desk_metrics();let(_,_,rest)=sh.desk_dock(&m);
  let(x,y)=((rest[1].x+rest[1].w/2) as f32,(rest[1].y+rest[1].h/2) as f32);
  sh.desktop_pointer(x,y,false,1000.);
  sh.desk.as_mut().unwrap().dock_label_off=Some((x,y));
  sh.desktop_pointer(x,y,false,1100.);
  assert!(sh.desk.as_ref().unwrap().dock_label_off.is_some(),"a pointer at rest keeps the label away");
  sh.desktop_pointer(x+3.,y,false,1200.);
  assert!(sh.desk.as_ref().unwrap().dock_label_off.is_none(),"a moved pointer brings it back");
 }
 fn desktop(w:i32,h:i32)->Shell{
  let mut sh=crate::shell::tests::desktop_shell();sh.w=w;sh.h=h;
  sh.screen=crate::shell::Screen::Home;sh.anim=None;
  let d=sh.desk.as_mut().unwrap();d.wins.clear();d.dock_preferences.autohide=false;d.dock_preferences.magnification=true;
  sh
 }

 #[test]
 fn dock_keeps_a_small_edge_gap_at_every_size_and_fits_the_complete_row(){
  for(w,h)in[(640,480),(1280,800),(2560,1440)]{for size in[32.,48.,80.]{for position in[Position::Bottom,Position::Left,Position::Right]{
   let mut sh=desktop(w,h);let p=&mut sh.desk.as_mut().unwrap().dock_preferences;p.size=size;p.position=position;
   let m=sh.desk_metrics();let(list,bar,rects)=sh.desk_dock(&m);let edge=sh.ui.pt(super::super::desk::DOCK_MARGIN_PT).round() as i32;
   assert_eq!(list.len(),rects.len());assert_eq!(list.last(),Some(&DOCK_TRASH));assert!(list.contains(&DOCK_STACK));
   match position{
    Position::Bottom=>{assert_eq!(h-bar.y-bar.h,edge);assert!((2*bar.x+bar.w-w).abs()<=1);},
    Position::Left=>assert_eq!(bar.x,edge),
    Position::Right=>assert_eq!(w-bar.x-bar.w,edge),
   }
   assert!(bar.x>=0&&bar.y>=m.menubar&&bar.x+bar.w<=w&&bar.y+bar.h<=h,"{position:?}, {w}x{h}, size {size}: {bar:?}");
   let a=super::super::desk::Desk::area(&m);
   assert!(a.x+a.w<=bar.x||bar.x+bar.w<=a.x||a.y+a.h<=bar.y||bar.y+bar.h<=a.y,"the work area excludes the resting Dock");
  }}}
 }

 #[test]
 fn magnified_icons_stay_on_screen_and_their_top_takes_their_context_menu(){
  for position in[Position::Bottom,Position::Left,Position::Right]{
   let mut sh=desktop(1280,800);sh.desk.as_mut().unwrap().dock_preferences.position=position;
   let m=sh.desk_metrics();let(list,bar,rest)=sh.desk_dock(&m);let k=1;let r=rest[k];
   sh.desktop_pointer((r.x+r.w/2) as f32,(r.y+r.h/2) as f32,false,1000.);sh.desk_t=1300.;
   let surface=sh.dock_surface_rect();let(_,live)=sh.dock_live(bar,&rest,1.3);let r=live[k];
   for r in&live{assert!(r.x>=0&&r.y>=m.menubar&&r.x+r.w<=sh.w&&r.y+r.h<=sh.h);}
   let(x,y)=match position{Position::Bottom=>((r.x+r.w/2) as f32,r.y as f32+1.),Position::Left=>((r.x+r.w-2) as f32,(r.y+r.h/2) as f32),Position::Right=>(r.x as f32+1.,(r.y+r.h/2) as f32)};
   let input=sh.dock_input_rect(surface).unwrap();let hit=Rect{x:surface.x+input[0],y:surface.y+input[1],w:input[2],h:input[3]};
   assert!(hit.contains(x,y),"the visible magnified icon takes input above the resting plate");
   sh.dock_pointer(Pointer::Context(x,y),1300.);
   assert_eq!(sh.desk.as_ref().unwrap().dock_menu_target,Some(list[k]),"the drawn icon, not its neighbour");
  }
 }

 /// E2E N5: the gap between two icons belongs to the nearer one for the hovered label and the context menu too, as it
 /// does for press and release, so the icon named under the pointer is the one a click or a right-click acts on.
 #[test]
 fn the_gap_next_to_an_icon_names_it_and_takes_its_context_menu(){
  let mut sh=desktop(1280,800);
  {let p=&mut sh.desk.as_mut().unwrap().dock_preferences;p.position=Position::Bottom;p.magnification=false;}
  let m=sh.desk_metrics();let(list,_,rest)=sh.desk_dock(&m);let k=1;
  let gap=rest[k+1].x-(rest[k].x+rest[k].w);assert!(gap>=2,"icons sit apart: {gap}");
  let(x,y)=((rest[k].x+rest[k].w) as f32+0.5,(rest[k].y+rest[k].h/2) as f32);
  assert!(!rest[k].contains(x,y)&&!rest[k+1].contains(x,y),"the point is in the gap");
  sh.desktop_pointer(x,y,false,1000.);
  assert_eq!(sh.desk.as_ref().unwrap().dock_hover,Some(k),"the label names the nearer icon");
  sh.dock_pointer(Pointer::Context(x,y),1000.);
  assert_eq!(sh.desk.as_ref().unwrap().dock_menu_target,Some(list[k]),"its context menu, not the Dock's own");
 }

 #[test]
 fn a_dock_surface_contains_every_visible_icon_label_and_shadow(){
  struct Reset;impl Drop for Reset{fn drop(&mut self){bifrost_render::glass::set_thread_gpu(None);}}
  let _reset=Reset;bifrost_render::glass::set_thread_gpu(Some(true));
  for position in[Position::Bottom,Position::Left,Position::Right]{
   let mut sh=desktop(1280,800);sh.desk.as_mut().unwrap().dock_preferences.position=position;
   let m=sh.desk_metrics();let(_,_,rest)=sh.desk_dock(&m);let r=rest[1];
   sh.desktop_pointer((r.x+r.w/2) as f32,(r.y+r.h/2) as f32,false,1000.);sh.desk_t=1300.;
   let surface=sh.dock_surface_rect();let mut px=vec![0;sh.w as usize*sh.h as usize];
   let mut out=Surface{w:sh.w as usize,h:sh.h as usize,stride:sh.w as usize,px:&mut px,y0:0};
   sh.draw_desk_dock(&mut out,&m,1300.);
   // The menu bar owns the top strip; side-Dock shadows are clipped beneath it.
   for(y,row)in px.chunks(sh.w as usize).enumerate().skip(m.menubar as usize){for(x,p)in row.iter().enumerate(){if *p!=0{assert!(surface.contains(x as f32,y as f32),"{position:?}: painted pixel {x},{y} was cropped by {surface:?}");}}}
   let before=surface;sh.desk_t=1600.;assert_eq!(sh.dock_surface_rect(),before,"hover frames keep a stable surface");
  }
 }

 #[test]
 fn autohide_keeps_a_hovered_magnified_icon_visible_above_the_plate(){
  let mut sh=desktop(1280,800);sh.desk.as_mut().unwrap().dock_preferences.autohide=true;
  let m=sh.desk_metrics();let(_,bar,rest)=sh.desk_dock(&m);let r=rest[1];
  sh.desktop_pointer((r.x+r.w/2) as f32,(r.y+r.h/2) as f32,false,1000.);
  let r=sh.dock_live(bar,&rest,1.3).1[1];assert!(r.y<bar.y);
  sh.desktop_pointer((r.x+r.w/2) as f32,r.y as f32+1.,false,1300.);
  assert!(sh.desk.as_ref().unwrap().dock_hide_at.is_none());
  sh.dock_tick(2500.);assert!(sh.dock_is_visible());
 }

 #[test]
 fn dock_changes_resize_all_filled_windows_and_keep_floating_sizes(){
  let mut sh=desktop(1280,800);let m=sh.desk_metrics();
  let d=sh.desk.as_mut().unwrap();let a=d.open(&m,AppId::Settings,(600,400));d.toggle_zoom(&m,a);
  let b=d.open(&m,AppId::Terminal,(640,420));d.toggle_zoom(&m,b);
  let c=d.open(&m,AppId::AppStore,(500,350));let floating=d.wins[c].r;
  sh.dock_change(Change::Position(Position::Left),1000.);let area=super::super::desk::Desk::area(&sh.desk_metrics());
  let d=sh.desk.as_ref().unwrap();assert_eq!(d.wins[a].r,area);assert_eq!(d.wins[b].r,area);assert_eq!(d.wins[c].r,floating);
  sh.dock_change(Change::Autohide,2000.);let area=super::super::desk::Desk::area(&sh.desk_metrics());
  assert_eq!(area.x,0);assert_eq!(area.w,sh.w);assert_eq!(area.y+area.h,sh.h);
 }

 #[test]fn dock_choices_survive_restart(){let path=std::env::temp_dir().join(format!("bifrost-dock-{}",std::process::id()));std::fs::create_dir_all(&path).unwrap();let p=Preferences{position:Position::Right,size:72.,autohide:true,magnification:false,magnified:96.,pinned:vec!["bifrost.settings".into(),"linux:firefox".into()]};p.save(&path);let q=Preferences::load(&path);assert_eq!(q.position,p.position);assert_eq!(q.size,72.);assert!(q.autohide);assert!(!q.magnification);assert_eq!(q.magnified,96.);assert_eq!(q.pinned,p.pinned);std::fs::remove_dir_all(path).unwrap();}
 #[test]fn magnify_falls_off_with_a_cosine(){assert_eq!(magnify_factor(0.,100.),1.);assert!((magnify_factor(50.,100.)-0.5).abs()<1e-5);assert_eq!(magnify_factor(100.,100.),0.);assert_eq!(magnify_factor(500.,100.),0.);}
 #[test]fn magnified_row_grows_around_the_pointer_and_stays_centred(){
  let row:Vec<Rect>=(0..5).map(|i|Rect{x:100+i*58,y:700,w:48,h:48}).collect();
  let out=magnified_rects(&row,false,true,row[2].x as f32+24.,96,150.,1.);
  assert_eq!(out[2].w,96);assert!(out[2].w>out[1].w&&out[1].w>out[0].w);
  assert_eq!(out[2].y+out[2].h,row[2].y+row[2].h,"icons keep the bottom edge");
  let (a,b)=(row[0].x+row[4].x+row[4].w,out[0].x+out[4].x+out[4].w);assert!((a-b).abs()<=2,"centred");
  assert_eq!(magnified_rects(&row,false,true,0.,96,150.,0.),row);
 }
 /// E2E N5: the row grows about the pointer, so the icon (or gap) under the pointer in the resting row is the icon (or
 /// gap) drawn under it at every amount, for a pointer anywhere along the row; a pointer past an end keeps that end.
 #[test]fn the_magnified_row_keeps_the_pointed_icon_under_the_pointer(){
  let row:Vec<Rect>=(0..12).map(|i|Rect{x:300+i*58,y:700,w:48,h:48}).collect();
  let (first,end)=(row[0].x,row[11].x+row[11].w);
  for amount in[0.2,0.5,1.]{for p in (first-30)..(end+30){
   let ptr=p as f32+0.5;
   let out=magnified_rects(&row,false,true,ptr,96,150.,amount);
   if p<first{assert_eq!(out[0].x,first,"a pointer before the row keeps its start ({p}, {amount})");continue;}
   if p>=end{assert_eq!(out[11].x+out[11].w,end,"a pointer past the row keeps its end ({p}, {amount})");continue;}
   let i=row.iter().position(|r|p<r.x+r.w).unwrap();
   if p>=row[i].x{
    assert!(out[i].contains(ptr,724.),"pointer {p} on resting icon {i} is on drawn icon {i} ({amount}): {:?}",out[i]);
   }else{
    // in the gap before icon i: still between the same two drawn icons
    assert!(ptr>=(out[i-1].x+out[i-1].w) as f32-1.&&ptr<=out[i].x as f32+1.,"pointer {p} in gap {}..{i} stays in it ({amount})",i-1);
   }
   assert!(out[0].x<=first&&out[11].x+out[11].w>=end,"both ends grow away from the pointer");
  }}
  // the growth is shared about the pointer: on the middle of the row both ends move out alike
  let mid=(row[5].x+row[5].w+row[6].x) as f32/2.;let out=magnified_rects(&row,false,true,mid,96,150.,1.);
  assert!(((first-out[0].x)-(out[11].x+out[11].w-end)).abs()<=2,"symmetric about the pointer");
 }

 /// The pointer on each icon's centre, in each gap and at both ends, along the whole Dock: the hovered slot (its label)
 /// is the slot drawn under the pointer in that frame, while the row grows and once it has grown, and a press there
 /// resolves the same slot, for every Dock position; on a Bottom Dock with room to grow it is the icon that rested there.
 #[test]fn the_hovered_dock_icon_is_the_one_drawn_under_the_pointer_across_the_dock(){
  for(w,h,positions)in[(1920,1080,vec![Position::Bottom]),(1280,800,vec![Position::Bottom,Position::Left,Position::Right])]{for position in positions{
   let probe={let mut sh=desktop(w,h);sh.desk.as_mut().unwrap().dock_preferences.position=position;sh};
   let m=probe.desk_metrics();let(_,bar,rest)=probe.desk_dock(&m);let vertical=position!=Position::Bottom;
   let at=|along:f32,r:&Rect|if vertical{((r.x+r.w/2) as f32,along)}else{(along,(r.y+r.h/2) as f32)};
   let (s0,l0)=(|r:&Rect|if vertical{r.y}else{r.x},|r:&Rect|if vertical{r.h}else{r.w});
   let mut points=vec![];
   for k in 0..rest.len(){
    points.push((at(s0(&rest[k]) as f32+l0(&rest[k]) as f32/2.,&rest[k]),Some(k)));
    if let Some(n)=rest.get(k+1){let g=s0(n)-s0(&rest[k])-l0(&rest[k]);if g>=2{points.push((at((s0(&rest[k])+l0(&rest[k])) as f32+(g/2) as f32-0.5,&rest[k]),None));}}
   }
   // the row's two ends, on the plate's padding
   points.push((at(s0(&rest[0]) as f32-1.,&rest[0]),None));
   let last=rest[rest.len()-1];points.push((at((s0(&last)+l0(&last)) as f32,&last),None));
   for((x,y),expect)in points{
    let mut sh=desktop(w,h);sh.desk.as_mut().unwrap().dock_preferences.position=position;
    sh.desktop_pointer(x,y,false,1000.);
    for t in[1050.,1300.]{
     sh.dock_tick(t);
     let drawn=sh.dock_hit(bar,&rest,x,y,t/1000.);
     assert_eq!(sh.desk.as_ref().unwrap().dock_hover,drawn,"{position:?} {w}x{h} at ({x},{y}), {t} ms: the label names the drawn icon");
     if let(1920,Some(k))=(w,expect){
      assert_eq!(drawn,Some(k),"at ({x},{y}), {t} ms: the icon that rested under the pointer stays under it");
      assert!(sh.dock_live(bar,&rest,t/1000.).1[k].contains(x,y),"the pointer is on the drawn icon itself");
     }
    }
    let hov=sh.desk.as_ref().unwrap().dock_hover;
    let press=sh.dock_press(&sh.dock_live(bar,&rest,1.3).1);
    assert_eq!(press.iter().position(|r|r.contains(x,y)),hov,"a press resolves the slot the label names");
   }
  }}
 }
 /// W2-24: the plate grows along the axis with the magnified row and keeps the end padding; at rest it is the capsule.
 #[test]fn the_plate_grows_with_the_magnified_row(){
  let row:Vec<Rect>=(0..5).map(|i|Rect{x:110+i*58,y:710,w:48,h:48}).collect();
  let bar=Rect{x:100,y:700,w:5*48+4*10+20,h:68};
  assert_eq!(dock_plate(bar,&row,&row,false),bar,"at rest the plate is the capsule");
  let live=magnified_rects(&row,false,true,row[0].x as f32+24.,96,150.,1.);
  let plate=dock_plate(bar,&row,&live,false);
  assert!(plate.w>bar.w,"it grows");
  assert_eq!((plate.y,plate.h),(bar.y,bar.h),"its depth stays the capsule's");
  assert_eq!(live[0].x-plate.x,10,"the first icon keeps its padding");
  assert_eq!(plate.x+plate.w-(live[4].x+live[4].w),10,"and so does the last");
  assert!(live.iter().all(|r|r.x>=plate.x&&r.x+r.w<=plate.x+plate.w),"every icon is on the plate");
  // a side Dock grows along y
  let col:Vec<Rect>=row.iter().map(|r|Rect{x:r.y,y:r.x,w:r.h,h:r.w}).collect();
  let side=Rect{x:bar.y,y:bar.x,w:bar.h,h:bar.w};
  let live=magnified_rects(&col,true,false,col[2].y as f32+24.,96,150.,1.);
  let plate=dock_plate(side,&col,&live,true);
  assert!(plate.h>side.h&&plate.x==side.x&&plate.w==side.w);
  assert!(live.iter().all(|r|r.y>=plate.y&&r.y+r.h<=plate.y+plate.h));
 }
 #[test]fn minimized_entries_decode_to_window_indices(){assert_eq!(dock_min_index(DOCK_MIN_BASE),Some(0));assert_eq!(dock_min_index(DOCK_MIN_BASE-3),Some(3));assert_eq!(dock_min_index(DOCK_TRASH),None);assert_eq!(dock_min_index(4),None);}
 #[test]fn fade_scales_all_channels(){assert_eq!(fade(0xff80_4020,0.5)&0xff00_0000,0x8000_0000);assert_eq!(fade(0xffff_ffff,0.),0);assert_eq!(fade(0x1234_5678,1.),0x1234_5678);}
 #[test]fn attention_bounce_hops_then_stops(){
  assert_eq!(bounce_lift(0.,0.,3.,0.5,48),0,"no lift at the start of a hop");
  assert_eq!(bounce_lift(0.25,0.,3.,0.5,48),24,"the peak of a hop lifts half the icon");
  assert_eq!(bounce_lift(3.,0.,3.,0.5,48),0,"stops at the end of the window");
  assert_eq!(bounce_lift(1.,2.,3.,0.5,48),0,"not before the window opens");
  assert_eq!(bounce_lift(1.,0.,3.,0.,48),0,"no bounce under Reduce Motion");
 }

 #[test]
 fn dock_drag_off_needs_the_reach_past_the_capsule() {
  let bar=Rect{x:100,y:500,w:200,h:60};
  assert!(!dock_drag_off(bar,150.,520.,40.),"inside the capsule stays in the Dock");
  assert!(!dock_drag_off(bar,150.,470.,40.),"30 px above is still near the Dock");
  assert!(dock_drag_off(bar,150.,400.,40.),"well above the Dock leaves it");
  assert!(dock_drag_off(bar,40.,520.,40.),"60 px left of the capsule leaves it");
  assert!(!dock_drag_off(bar,80.,520.,40.),"20 px left is still near the Dock");
  assert!(!dock_drag_off(bar,300.,520.,40.),"exactly at the right edge is inside");
 }
}
