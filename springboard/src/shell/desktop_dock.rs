//! Persistent desktop dock choices; app identities survive registry changes.
//! It also draws what an app sets on its Dock tile through the `dock` service: a progress bar and an attention bounce.
use super::{apps,Shell,Effect,AppId,desktop_bar::Menu,desk::{Rect,DOCK_TRASH},dock_stack::DOCK_STACK};
use super::ui::{FILL_GRAY,LINK};
use crate::gfx::Surface;
use bifrost_ui::values::Progress;
use bifrost_render::motion::{self,Anim,Curve,Kind};
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
  self.dock_preferences().save(&self.data_dir);self.dirty=true;vec![]
 }
 pub(super) fn dock_app_key(&self,index:usize)->Option<String>{if let Some(a)=apps().get(index){return Some(a.icon.into());} #[cfg(feature="desktop-wayland")]if index>apps().len(){return self.desk.as_ref()?.linux_apps.get(index-apps().len()-1).map(|a|format!("linux:{}",a.app_id));}None}
#[cfg(feature = "desktop-wayland")]
 pub(super) fn dock_popup_rect(&self)->Option<(usize,usize,usize,usize)>{let d=self.desk.as_ref()?;let (x,y)=d.dock_menu_anchor;let width=self.ui.pt(260.) as usize;let items=d.dock_menu_target.and_then(|i|apps().get(i).map(|a|a.id)).and_then(|id|if let AppId::External(slot)=id{self.ext.get(slot as usize).and_then(Option::as_ref).map(|h|h.dock_menu.len())}else{None}).unwrap_or(0);let base=if d.dock_menu_target.is_some(){404.}else{260.};let extra=if items>0{items as f32*24.+10.}else{0.};let height=self.ui.pt(base+extra) as usize;Some(((x as usize).min((self.w as usize).saturating_sub(width)),(y as usize).min((self.h as usize).saturating_sub(height)).max(self.desktop_bar_height()),width,height))}
 pub(super) fn dock_pointer(&mut self,event:Pointer,t:f64)->Option<Vec<Effect>> {
  if !self.desk_active()||self.is_locked(){return None;}
  if let Pointer::Hover(x,y)=event {
   let m=self.desk_metrics();let (_,bar,rects)=self.desk_dock(&m);let now=t/1000.;
   let (_,live)=self.dock_live(bar,&rects,now);
   let zone=self.dock_zone(bar);
   let p=self.dock_preferences();
   let edge=match p.position{Position::Bottom=>y>=self.h as f32-self.ui.pt(4.),Position::Left=>x<=self.ui.pt(4.),Position::Right=>x>=self.w as f32-self.ui.pt(4.)};
   let inside=bar.contains(x,y);
   let in_zone=self.dock_is_visible()&&(inside||zone.contains(x,y));
   let hov=if in_zone{live.iter().position(|r|r.contains(x,y))}else{None};
   let magnify=in_zone&&p.magnification;
   let menu=self.desktop_menu==Some(Menu::Dock);
   let cur=self.dock_mag_amount(now);
   let d=self.desk.as_mut().unwrap();
   if d.dock_hover!=hov{d.dock_label_last=d.dock_hover;d.dock_label_ms=t;d.dock_hover=hov;self.dirty=true;}
   if magnify!=d.dock_ptr.is_some(){d.dock_mag=Some(Anim{from:cur,to:if magnify{1.}else{0.},start:now,kind:Kind::DockMagnify,curve:Curve::EaseOut});self.dirty=true;}
   let moved=d.dock_ptr!=magnify.then_some((x,y));d.dock_ptr=magnify.then_some((x,y));if moved&&magnify{self.dirty=true;}
   if edge||inside||menu {d.dock_hide_at=None;dock_reveal(d,now);if !d.dock_visible{self.dirty=true;}}
   else if d.dock_autohides()&&d.dock_hide_at.is_none(){d.dock_hide_at=Some(t+550.);}
   return Some(vec![]);
  }
  if let Pointer::Context(x,y)=event {let m=self.desk_metrics();let (list,bar,rects)=self.desk_dock(&m);if self.desk.as_ref().unwrap().dock_visible&&bar.contains(x,y){let target=rects.iter().position(|r|r.contains(x,y)).map(|i|list[i]).filter(|i|*i!=apps().len()&&*i!=super::desk::DOCK_TRASH&&*i!=DOCK_STACK&&dock_min_index(*i).is_none());let d=self.desk.as_mut().unwrap();d.dock_menu_target=target;d.dock_menu_anchor=(x,y);self.desktop_menu=Some(Menu::Dock);self.dirty=true;return Some(vec![]);}}
  None
 }
 pub fn desktop_pointer(&mut self,x:f32,y:f32,context:bool,t:f64){if !context{self.pointer_hover(x,y);self.note_pointer_y(y);}#[cfg(feature="desktop-wayland")] if !context&&self.desk_active()&&!self.is_locked(){self.arrange_hover(x,y,t);self.desktop_menu_hover(x,y);}if !context&&self.desk_active()&&!self.is_locked(){let m=self.desk_metrics();let d=self.desk.as_mut().unwrap();let over=match d.pick(&m,x,y){Some((i,super::desk::Hit::Light(_)))=>Some(d.wins[i].app),_=>None};if d.lights_hover!=over{d.lights_hover=over;self.dirty=true;}}if self.dock_pointer(if context{Pointer::Context(x,y)}else{Pointer::Hover(x,y)},t).is_some(){return;}if context&&self.desk_active()&&!self.is_locked(){let m=self.desk_metrics();if let Some((i,super::desk::Hit::Content))=self.desk.as_ref().unwrap().pick(&m,x,y){let w=self.desk.as_ref().unwrap().wins[i];let b=w.body(&m);if let AppId::External(slot)=w.app {if let Some(Some(host))=self.ext.get_mut(slot as usize){host.send(&bifrost_proto::ToApp::Context{x:x-b.x as f32,y:y-b.y as f32});}}}}}
 pub(super) fn dock_tick(&mut self,t:f64){
  let now=t/1000.;let menu_open=self.desktop_menu==Some(Menu::Dock);
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
 }
 /// Whether a dock list entry is an app the user keeps in the Dock (not the Apps icon, Trash, the Downloads stack, a minimized window or a recent).
 pub(super) fn dock_removable(&self,entry:usize)->bool{entry!=apps().len()&&entry!=DOCK_TRASH&&dock_min_index(entry).is_none()&&self.dock_section(entry)==0}
 pub(super) fn dock_reorder(&mut self,from:usize,to:usize,after:bool){let Some(key)=self.dock_app_key(from)else{return;};let target=self.dock_app_key(to);let d=self.desk.as_mut().unwrap();d.dock_preferences.pinned.retain(|s|s!=&key);let at=target.and_then(|key|d.dock_preferences.pinned.iter().position(|s|s==&key)).map(|at|at+after as usize).unwrap_or(d.dock_preferences.pinned.len());d.dock_preferences.pinned.insert(at.min(d.dock_preferences.pinned.len()),key);d.dock_preferences.save(&self.data_dir);self.dirty=true;}
 pub(super) fn dock_is_visible(&self)->bool{self.desk.as_ref().is_some_and(|d|!d.dock_autohides()||d.dock_visible)}
 pub(super) fn dock_overlay_rect(&self)->Rect {let m=self.desk_metrics();let (_,bar,_)=self.desk_dock(&m);if self.dock_is_visible(){bar}else{let p=self.ui.pt(3.) as i32;match self.dock_preferences().position {Position::Bottom=>Rect{x:0,y:self.h-p,w:self.w,h:p},Position::Left=>Rect{x:0,y:m.menubar,w:p,h:self.h-m.menubar},Position::Right=>Rect{x:self.w-p,y:m.menubar,w:p,h:self.h-m.menubar}}}}
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
/// Pointer magnification: 1 at the pointer, 0 at `radius` and beyond, a cosine in between.
pub fn magnify_factor(dist:f32,radius:f32)->f32{if radius<=0.||dist>=radius{0.}else{0.5+0.5*(std::f32::consts::PI*dist.max(0.)/radius).cos()}}
/// Scales a premultiplied colour (all four channels) by `k`, for fades.
pub(super) fn fade(c:u32,k:f32)->u32{let k=k.clamp(0.,1.);let m=|shift:u32|((((c>>shift)&255) as f32*k).round() as u32)<<shift;m(0)|m(8)|m(16)|m(24)}
/// Magnifies a row of equal square icon `rects` around `ptr` (a coordinate along the dock axis: x unless `vertical`).
/// Sizes follow `magnify_factor` between the base size and `max`, quantised to 2 px so scaled icons cache well; gaps are
/// kept, the row stays centred where it was, and icons keep their far edge (`far`: bottom or right) or near edge, so they
/// grow into the screen.
pub fn magnified_rects(rects:&[Rect],vertical:bool,far:bool,ptr:f32,max:i32,radius:f32,amount:f32)->Vec<Rect>{
 if rects.is_empty()||amount<=0.{return rects.to_vec();}
 let (start,len):(fn(&Rect)->i32,fn(&Rect)->i32)=if vertical{(|r|r.y,|r|r.h)}else{(|r|r.x,|r|r.w)};
 let base=len(&rects[0]);
 let sizes:Vec<i32>=rects.iter().map(|r|{let c=start(r) as f32+len(r) as f32/2.;let f=magnify_factor((c-ptr).abs(),radius)*amount.min(1.);base+(((max-base).max(0) as f32*f/2.).round() as i32)*2}).collect();
 let last=&rects[rects.len()-1];
 let old_span=start(last)+len(last)-start(&rects[0]);
 let gaps:Vec<i32>=rects.windows(2).map(|w|start(&w[1])-start(&w[0])-len(&w[0])).collect();
 let new_span:i32=sizes.iter().sum::<i32>()+gaps.iter().sum::<i32>();
 let mut at=start(&rects[0])-(new_span-old_span)/2;
 let mut out=Vec::with_capacity(rects.len());
 for (i,r) in rects.iter().enumerate(){
  let n=sizes[i];
  out.push(if vertical{let x=if far{r.x+r.w-n}else{r.x};Rect{x,y:at,w:n,h:n}}else{let y=if far{r.y+r.h-n}else{r.y};Rect{x:at,y,w:n,h:n}});
  at+=n+gaps.get(i).copied().unwrap_or(0);
 }
 out
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
 /// Whether the app of built-in or installed entry `entry` (an index into `apps()`) itself has a window.
 pub(super) fn dock_entry_running(&self,entry:usize)->bool{
  let Some(d)=self.desk.as_ref()else{return false;};
  let Some(app)=apps().get(entry)else{return false;};
  #[allow(unused_mut)] let mut running=d.index_of(app.id).is_some();
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
 pub(super) fn dock_zone(&self,bar:Rect)->Rect{let p=self.dock_preferences();if !p.magnification{return bar;}let grow=(self.ui.pt(p.magnified)-self.ui.pt(p.size)).max(0.) as i32;Rect{x:bar.x-grow,y:bar.y-grow,w:bar.w+2*grow,h:bar.h+2*grow}}
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
 /// Icon slots as drawn and hit-tested now: slid, then magnified around the pointer.
 pub(super) fn dock_live(&self,bar:Rect,rects:&[Rect],now:f64)->(Rect,Vec<Rect>){let (bar,slid)=self.dock_slid(bar,rects,now);let live=self.dock_magnified(&slid,now);(bar,live)}
 pub(super) fn dock_magnified(&self,rects:&[Rect],now:f64)->Vec<Rect>{
  let Some(d)=self.desk.as_ref()else{return rects.to_vec();};
  let p=&d.dock_preferences;
  let Some((px,py))=d.dock_ptr.filter(|_|p.magnification&&d.grab_idle())else{return rects.to_vec();};
  let max=self.ui.pt(p.magnified).round() as i32;
  let (vertical,far,ptr)=match p.position{Position::Bottom=>(false,true,px),Position::Left=>(true,false,py),Position::Right=>(true,true,py)};
  magnified_rects(rects,vertical,far,ptr,max,max as f32*1.6,self.dock_mag_amount(now))
 }
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
/// Makes the dock visible again, sliding it in from wherever a slide-out had got to.
fn dock_reveal(d:&mut super::desk::Desk,now:f64){
 let hiding=d.dock_vis_anim.as_ref().is_some_and(|a|a.to<0.5);
 if !d.dock_visible||hiding{let cur=d.dock_vis_anim.as_ref().map_or(0.,|a|a.value(now));d.dock_visible=true;d.dock_vis_anim=Some(Anim{from:cur,to:1.,start:now,kind:Kind::PanelReveal,curve:Curve::EaseOut});}
}

#[cfg(test)]mod tests {
 use super::*;
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
