mod nearby;
mod network;
mod services;
fn account()->String{std::fs::read_to_string("/var/lib/springboard/account").unwrap_or_default().lines().find_map(|l|l.strip_prefix("username=").map(str::to_owned)).unwrap_or_else(||std::env::var("USER").unwrap_or_default())}
use bifrost_app::{App,Ctx,Input,TouchKind};
use bifrost_app::kit::{Damage,Surface};
use bifrost_app::layout::Layout;
use bifrost_app::network_path::Path;
use bifrost_app::widgets;
use springboard::shell::ui::{Weight,WHITE,TINT,DETAIL_BG,LABEL,LABEL_2,FIELD_BG,MAC_TITLE2_PT,MAC_TITLE3_PT,MAC_BODY_PT};
use nearby::Nearby;
use std::{path::PathBuf,process::Command};
/// Nearby list: at most this many rows are drawn, each `ROW_H` points tall, starting at `ROWS_Y`.
const MAX_ROWS:usize=4;
const ROWS_Y:f32=506.;
const ROW_H:f32=44.;
const ROW_W:f32=360.;
/// How often the network path is read again, in milliseconds.
const NET_EVERY_MS:f64=5000.;
struct Remote { data:PathBuf,status:String,password:String,error:String,last:f64,address:String,col:f32,
 network:String,path:Path,net_at:f64,nearby:Nearby,selected:Option<usize>,
 /// Shell service state: whether `network` answers (None until its first reply), the intro line from `screens`,
 /// the notice after a copy, and the request ids this window is waiting for.
 path_service:Option<bool>,intro:String,notice:String,started:bool,path_req:Option<u32>,screens_req:Option<u32>,copy_req:Option<u32> }
impl Remote {
 fn service(&self,ctx:&Ctx,start:bool){ctx.remote_sharing(start);}
 /// Stores a path from the shell service or the `ip` fallback. True when the words shown changed.
 fn set_path(&mut self,p:Path)->bool{let text=network::status_text(&p);let changed=text!=self.network;self.path=p;self.network=text;changed}
 /// Stores the screen list: the main screen's size is the desktop this window shares. True when the line changed.
 fn set_screens(&mut self,s:&bifrost_app::screens::Screens)->bool{let text=services::intro(s);let changed=text!=self.intro;self.intro=text;changed}
 /// Writes `text` to the shared clipboard (one change). The reply sets the notice line.
 fn copy(&mut self,ctx:&Ctx,text:String)->bool{self.copy_req=Some(services::copy_text(ctx,&text));self.notice.clear();true}
 fn refresh(&mut self){if let Ok(s)=std::fs::read_to_string(self.data.join("status.json")){if let Ok(v)=serde_json::from_str::<serde_json::Value>(&s){self.status=v["status"].as_str().unwrap_or("Stopped").into();self.password=v["password"].as_str().unwrap_or("").into();self.error=v["error"].as_str().unwrap_or("").into();}}}
 /// Index of the nearby row under a point in points, when that row is drawn.
 fn row_at(&self,x:f32,y:f32)->Option<usize>{
 (0..MAX_ROWS.min(self.nearby.rows.len())).find(|&i|{let top=ROWS_Y+i as f32*ROW_H;(self.col..self.col+ROW_W).contains(&x)&&(top..top+ROW_H-4.).contains(&y)})
 }
}
impl App for Remote {
 fn id(&self)->&str{"is.olibuijr.remote.desktop"}
 fn configure(&mut self,ctx:&Ctx){
 // Nothing here is unsaved: the sharing service runs on its own, so the shell may stop this app at once on logout.
 ctx.enable_sudden_termination();
 // The shell services are asked once, at start; after that the shell pushes changes (no polling).
 if !self.started{self.started=true;self.path_req=Some(services::request_path(ctx));self.screens_req=Some(services::request_screens(ctx));}
 self.data=std::fs::read_to_string("/etc/passwd").unwrap_or_default().lines().find_map(|l|{let f:Vec<_>=l.split(':').collect();(f.len()>5&&f[0]==account()).then(||PathBuf::from(f[5]).join(".local/share/bifrost-remote"))}).unwrap_or_else(||ctx.data_dir.clone());self.refresh();
 if let Ok(out)=Command::new("ip").args(["-j","-4","address","show","scope","global"]).output(){if let Ok(v)=serde_json::from_slice::<serde_json::Value>(&out.stdout){if let Some(ip)=v.as_array().and_then(|a|a.iter().find_map(|i|i["addr_info"].as_array().and_then(|a|a.iter().find_map(|i|i["local"].as_str())))){self.address=format!("http://{ip}:12000");}}}}

 fn event(&mut self,ctx:&Ctx,ev:Input)->bool {if let Input::Touch{kind:TouchKind::Up,x,y,..}=ev {let x=x/ctx.ui.s;let y=y/ctx.ui.s;
 if (self.col..self.col+218.).contains(&x)&&(296. ..344.).contains(&y){self.service(ctx,self.status!="Serving");return true}
 // Tapping the address (ours, or the selected computer's line below the list) copies it to the shared clipboard.
 if (self.col..self.col+360.).contains(&x)&&(214. ..244.).contains(&y)&&self.address.starts_with("http"){return self.copy(ctx,self.address.clone())}
 if let Some(row)=self.selected.and_then(|i|self.nearby.rows.get(i)){
 let line=ROWS_Y+MAX_ROWS as f32*ROW_H+12.;
 if (self.col..self.col+360.).contains(&x)&&(line-4. ..line+22.).contains(&y)&&row.detail.starts_with("http"){let text=row.detail.clone();return self.copy(ctx,text)}
 }
 // Tapping a nearby computer selects it (tap again to clear); the window then shows its address.
 if let Some(i)=self.row_at(x,y){self.selected=if self.selected==Some(i){None}else{Some(i)};return true}
 }false}
 fn tick(&mut self,_:&Ctx,t:f64)->bool{
 let mut changed=false;
 if t-self.last>1000. {self.last=t;let before=(self.status.clone(),self.password.clone(),self.error.clone());self.refresh();changed|=before!=(self.status.clone(),self.password.clone(),self.error.clone());}
 // The shell pushes path changes. Only when the shell has no `network` service (an older shell) is the path read here.
 if self.path_service==Some(false)&&t-self.net_at>NET_EVERY_MS {self.net_at=t;changed|=self.set_path(network::read());}
 changed|=self.nearby.poll(t);
 changed
 }
 fn next_wake(&self,_:f64)->Option<f64>{Some(self.last+1000.)}
 fn service_reply(&mut self,_:&Ctx,request:u32,result:&Result<bifrost_core::json::Json,String>)->bool{
 if self.path_req==Some(request){self.path_req=None;
 match services::path_reply(result){Ok(p)=>{self.path_service=Some(true);return self.set_path(p)}
 // An older shell answers "no service network": fall back to reading the link with `ip` on the next tick.
 Err(_)=>{self.path_service=Some(false);return true}}}
 if self.screens_req==Some(request){self.screens_req=None;return match bifrost_app::screens::from_reply(result){Ok(s)=>self.set_screens(&s),Err(_)=>true}}
 if self.copy_req==Some(request){self.copy_req=None;self.notice=services::copy_notice(result);return true}
 false
 }
 fn service_event(&mut self,_:&Ctx,service:&str,event:&str,body:&bifrost_core::json::Json)->bool{
 if let Some(Ok(p))=bifrost_app::network_path::path_event(service,event,body){self.path_service=Some(true);return self.set_path(p)}
 if let Some(Ok(s))=bifrost_app::screens::changed(service,event,body){return self.set_screens(&s)}
 false
 }
 fn draw(&mut self,ctx:&Ctx,s:&mut Surface,_:f64)->Damage {
 let u=&ctx.ui;s.fill_rect(0,0,s.w,s.h,DETAIL_BG.now());
 // Full-size window: the top 52 pt band holds the traffic lights, so the large title starts below it, on the
 // leading edge of the readable column (the content below uses the same x).
 let l=Layout::from_points(s.w as f32/u.pt(1.),s.h as f32/u.pt(1.),0.,0.);
 let (x,_)=l.readable();
 self.col=x;
 widgets::large_title(s,u,&l,"Remote",64.);
 for (label,size,color,y) in [(self.intro.as_str(),MAC_TITLE2_PT,LABEL_2.now(),110.),(self.status.as_str(),MAC_TITLE2_PT,LABEL.now(),150.),("Browser address: this computer’s IP, port 12000",MAC_TITLE3_PT,LABEL_2.now(),186.),(self.address.as_str(),MAC_TITLE2_PT,TINT,218.),("Access password",MAC_BODY_PT,LABEL_2.now(),254.)]{s.blit(&u.text(label,size,Weight::Regular,color),u.pt(x) as i32,u.pt(y) as i32,255,255);}
 s.blit(&u.text(&self.password,MAC_TITLE2_PT,Weight::Regular,LABEL.now()),u.pt(x+148.) as i32,u.pt(250.) as i32,255,255);
 let serving=self.status=="Serving";
 // Start is the one accent button; Stop is a quiet secondary fill, not red, because stopping is not destructive.
 s.fill_rrect(u.pt(x),u.pt(296.),u.pt(218.),u.pt(48.),u.pt(10.),if serving{FIELD_BG.now()}else{TINT});
 s.blit(&u.text(if serving{"Stop Sharing"}else{"Start Sharing"},MAC_TITLE2_PT,Weight::Regular,if serving{LABEL.now()}else{WHITE}),u.pt(x+20.) as i32,u.pt(310.) as i32,255,255);
 // The line at 370 pt shows a sharing error, or else the result of the last copy.
 let msg=if self.error.is_empty(){self.notice.as_str()}else{self.error.as_str()};
 s.blit(&u.text(msg,MAC_BODY_PT,Weight::Regular,LABEL_2.now()),u.pt(x) as i32,u.pt(370.) as i32,255,255);
 // Network: the path the system reports, in words (satisfied, getting an address, or none).
 widgets::section_label(s,u,&l,"Network",410.);
 s.blit(&u.text(&self.network,MAC_BODY_PT,Weight::Regular,LABEL.now()),u.pt(x) as i32,u.pt(434.) as i32,255,255);
 // Nearby computers: the DNS-SD browse for remote-sharing services on the local network.
 widgets::section_label(s,u,&l,"Nearby computers",474.);
 if self.nearby.rows.is_empty(){s.blit(&u.text("No remote computers found on this network.",MAC_BODY_PT,Weight::Regular,LABEL_2.now()),u.pt(x) as i32,u.pt(ROWS_Y) as i32,255,255);}
 for (i,row) in self.nearby.rows.iter().take(MAX_ROWS).enumerate(){
 let ry=ROWS_Y+i as f32*ROW_H;
 if self.selected==Some(i){s.fill_rrect(u.pt(x),u.pt(ry),u.pt(ROW_W),u.pt(ROW_H-4.),u.pt(9.),FIELD_BG.now());}
 s.blit(&u.text(&row.title,MAC_BODY_PT,Weight::Regular,LABEL.now()),u.pt(x+12.) as i32,u.pt(ry+4.) as i32,255,255);
 s.blit(&u.text(&row.detail,MAC_BODY_PT,Weight::Regular,LABEL_2.now()),u.pt(x+12.) as i32,u.pt(ry+22.) as i32,255,255);
 }
 if let Some(row)=self.selected.and_then(|i|self.nearby.rows.get(i)){
 s.blit(&u.text(&format!("Address: {}",row.detail),MAC_BODY_PT,Weight::Regular,LABEL.now()),u.pt(x) as i32,u.pt(ROWS_Y+MAX_ROWS as f32*ROW_H+12.) as i32,255,255);
 }
 let mut d=Damage::new(s.w,s.h);d.all();d
 }
}
fn main(){bifrost_app::run(Remote{data:PathBuf::new(),status:"Stopped".into(),password:String::new(),error:String::new(),last:0.,address:"Port 12000".into(),col:32.,
 network:String::new(),path:Path::offline(),net_at:f64::NEG_INFINITY,nearby:Nearby::new(),selected:None,
 path_service:None,intro:services::INTRO.into(),notice:String::new(),started:false,path_req:None,screens_req:None,copy_req:None});}
