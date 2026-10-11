fn account()->String{std::fs::read_to_string("/var/lib/springboard/account").unwrap_or_default().lines().find_map(|l|l.strip_prefix("username=").map(str::to_owned)).unwrap_or_else(||std::env::var("USER").unwrap_or_default())}
use bifrost_app::{App,Ctx,Input,TouchKind};
use bifrost_app::kit::{Damage,Surface};
use bifrost_app::layout::Layout;
use bifrost_app::widgets;
use springboard::shell::ui::{Weight,WHITE,TINT,DETAIL_BG,LABEL,LABEL_2,FIELD_BG,MAC_TITLE2_PT,MAC_TITLE3_PT,MAC_BODY_PT};
use std::{path::PathBuf,process::Command};
struct Remote { data:PathBuf,status:String,password:String,error:String,last:f64,address:String,col:f32 }
impl Remote {
 fn service(&self,ctx:&Ctx,start:bool){ctx.remote_sharing(start);}
 fn refresh(&mut self){if let Ok(s)=std::fs::read_to_string(self.data.join("status.json")){if let Ok(v)=serde_json::from_str::<serde_json::Value>(&s){self.status=v["status"].as_str().unwrap_or("Stopped").into();self.password=v["password"].as_str().unwrap_or("").into();self.error=v["error"].as_str().unwrap_or("").into();}}}
}
impl App for Remote {
 fn id(&self)->&str{"is.olibuijr.remote.desktop"}
 fn configure(&mut self,ctx:&Ctx){
 // Nothing here is unsaved: the sharing service runs on its own, so the shell may stop this app at once on logout.
 ctx.enable_sudden_termination();
 self.data=std::fs::read_to_string("/etc/passwd").unwrap_or_default().lines().find_map(|l|{let f:Vec<_>=l.split(':').collect();(f.len()>5&&f[0]==account()).then(||PathBuf::from(f[5]).join(".local/share/bifrost-remote"))}).unwrap_or_else(||ctx.data_dir.clone());self.refresh();
 if let Ok(out)=Command::new("ip").args(["-j","-4","address","show","scope","global"]).output(){if let Ok(v)=serde_json::from_slice::<serde_json::Value>(&out.stdout){if let Some(ip)=v.as_array().and_then(|a|a.iter().find_map(|i|i["addr_info"].as_array().and_then(|a|a.iter().find_map(|i|i["local"].as_str())))){self.address=format!("http://{ip}:12000");}}}}

 fn event(&mut self,ctx:&Ctx,ev:Input)->bool {if let Input::Touch{kind:TouchKind::Up,x,y,..}=ev {let x=x/ctx.ui.s;let y=y/ctx.ui.s;if (self.col..self.col+218.).contains(&x)&&(296. ..344.).contains(&y){self.service(ctx,self.status!="Serving");return true}}false}
 fn tick(&mut self,_:&Ctx,t:f64)->bool{if t-self.last>1000. {self.last=t;let before=(self.status.clone(),self.password.clone(),self.error.clone());self.refresh();before!=(self.status.clone(),self.password.clone(),self.error.clone())}else{false}}
 fn next_wake(&self,_:f64)->Option<f64>{Some(self.last+1000.)}
 fn draw(&mut self,ctx:&Ctx,s:&mut Surface,_:f64)->Damage {
 let u=&ctx.ui;s.fill_rect(0,0,s.w,s.h,DETAIL_BG.now());
 // Full-size window: the top 52 pt band holds the traffic lights, so the large title starts below it, on the
 // leading edge of the readable column (the content below uses the same x).
 let l=Layout::from_points(s.w as f32/u.pt(1.),s.h as f32/u.pt(1.),0.,0.);
 let (x,_)=l.readable();
 self.col=x;
 widgets::large_title(s,u,&l,"Remote",64.);
 for (label,size,color,y) in [("Share this desktop in your browser.",MAC_TITLE2_PT,LABEL_2.now(),110.),(self.status.as_str(),MAC_TITLE2_PT,LABEL.now(),150.),("Browser address: this computer’s IP, port 12000",MAC_TITLE3_PT,LABEL_2.now(),186.),(self.address.as_str(),MAC_TITLE2_PT,TINT,218.),("Access password",MAC_BODY_PT,LABEL_2.now(),254.)]{s.blit(&u.text(label,size,Weight::Regular,color),u.pt(x) as i32,u.pt(y) as i32,255,255);}
 s.blit(&u.text(&self.password,MAC_TITLE2_PT,Weight::Regular,LABEL.now()),u.pt(x+148.) as i32,u.pt(250.) as i32,255,255);
 let serving=self.status=="Serving";
 // Start is the one accent button; Stop is a quiet secondary fill, not red, because stopping is not destructive.
 s.fill_rrect(u.pt(x),u.pt(296.),u.pt(218.),u.pt(48.),u.pt(10.),if serving{FIELD_BG.now()}else{TINT});
 s.blit(&u.text(if serving{"Stop Sharing"}else{"Start Sharing"},MAC_TITLE2_PT,Weight::Regular,if serving{LABEL.now()}else{WHITE}),u.pt(x+20.) as i32,u.pt(310.) as i32,255,255);
 s.blit(&u.text(&self.error,MAC_BODY_PT,Weight::Regular,LABEL_2.now()),u.pt(x) as i32,u.pt(370.) as i32,255,255);let mut d=Damage::new(s.w,s.h);d.all();d
 }
}
fn main(){bifrost_app::run(Remote{data:PathBuf::new(),status:"Stopped".into(),password:String::new(),error:String::new(),last:0.,address:"Port 12000".into(),col:32.});}
