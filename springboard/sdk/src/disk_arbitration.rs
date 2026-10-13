//! Disk Arbitration for apps: a [`DiskSession`] that hears about disks appearing, disappearing and changing, and asks
//! to unmount or eject them. The shell side is the `disks` service (`springboard/src/shell/services/disks.rs`).
//!
//! ```ignore
//! let mut session = DiskSession::create();                // DASessionCreate
//! session.set_dispatch_queue(DispatchQueue::main());      // DASessionSetDispatchQueue
//! session.register_disk_appeared_callback(|disk| { /* add the volume to the sidebar */ });
//! session.register_disk_disappeared_callback(|bsd_name| { /* remove it */ });
//! let id = session.list(ctx);                             // App::service_reply: session.on_list_reply(&result)
//! // App::service_event: session.handle_event(service, event, body)
//! ```
//! The callbacks run on the session's queue, in registration order, or on the caller's thread when no queue is set.
//! Like the native framework, disks that are already connected when the list arrives are reported as appeared.
//!
//! Differences from the native API: there is no run loop, so `schedule_with_run_loop` means the main queue (the app
//! loop); no peek callback; mount approval callbacks stay in `bifrost_core::disk_events::Approvals` on the shell side.
//! `plan_unmount` and `plan_eject` return the validated plan; running it needs the privileged helper and a real USB
//! disk on Midget, which has not been tried.
use crate::dispatch::DispatchQueue;
use crate::runtime::Ctx;
use crate::service;
use bifrost_core::disk_events::{self, Disk, DiskEvent};
use bifrost_core::disk_wire;
use bifrost_core::json::Json;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

/// The disk's BSD name (`kDADiskDescriptionMediaBSDNameKey`).
pub const DESCRIPTION_MEDIA_BSD_NAME_KEY: &str = "bsd_name";
/// The size in bytes (`kDADiskDescriptionMediaSizeKey`).
pub const DESCRIPTION_MEDIA_SIZE_KEY: &str = "media_size";
/// Whether the media can be removed (`kDADiskDescriptionMediaRemovableKey`).
pub const DESCRIPTION_MEDIA_REMOVABLE_KEY: &str = "media_removable";
/// Whether the media can be written (`kDADiskDescriptionMediaWritableKey`).
pub const DESCRIPTION_MEDIA_WRITABLE_KEY: &str = "media_writable";
/// The device model (`kDADiskDescriptionDeviceModelKey`).
pub const DESCRIPTION_DEVICE_MODEL_KEY: &str = "device_model";
/// Where the volume is mounted (`kDADiskDescriptionVolumePathKey`); absent when it is not mounted.
pub const DESCRIPTION_VOLUME_PATH_KEY: &str = "volume_path";

type Shared<F> = Arc<Mutex<F>>;
type DiskCallback = Shared<dyn FnMut(&Disk) + Send>;
type NameCallback = Shared<dyn FnMut(&str) + Send>;
type ChangeCallback = Shared<dyn FnMut(&Disk, &Disk) + Send>;

/// One command of a plan, as the shell reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlannedCommand {
    pub program: String,
    pub args: Vec<String>,
}

#[derive(Default)]
pub struct DiskSession {
    queue: Option<DispatchQueue>,
    disks: Vec<Disk>,
    appeared: Vec<DiskCallback>,
    disappeared: Vec<NameCallback>,
    changed: Vec<ChangeCallback>,
}

impl DiskSession {
    /// `DASessionCreate`.
    pub fn create() -> Self {
        Self::default()
    }

    /// `DASessionSetDispatchQueue`: callbacks run on `queue`.
    pub fn set_dispatch_queue(&mut self, queue: DispatchQueue) {
        self.queue = Some(queue);
    }

    /// `DASessionScheduleWithRunLoop`: callbacks run on the app loop (the main queue).
    pub fn schedule_with_run_loop(&mut self) {
        self.queue = Some(DispatchQueue::main());
    }

    /// `DARegisterDiskAppearedCallback`.
    pub fn register_disk_appeared_callback(&mut self, f: impl FnMut(&Disk) + Send + 'static) {
        self.appeared.push(Arc::new(Mutex::new(f)));
    }

    /// `DARegisterDiskDisappearedCallback`; the callback gets the BSD name.
    pub fn register_disk_disappeared_callback(&mut self, f: impl FnMut(&str) + Send + 'static) {
        self.disappeared.push(Arc::new(Mutex::new(f)));
    }

    /// `DARegisterDiskDescriptionChangedCallback`; the callback gets the disk before and after.
    pub fn register_disk_description_changed_callback(&mut self, f: impl FnMut(&Disk, &Disk) + Send + 'static) {
        self.changed.push(Arc::new(Mutex::new(f)));
    }

    /// The disks the session knows.
    pub fn disks(&self) -> &[Disk] {
        &self.disks
    }

    /// `DADiskCreateFromBSDName`.
    pub fn disk_from_bsd_name(&self, bsd_name: &str) -> Option<&Disk> {
        disk_events::find_by_bsd_name(&self.disks, bsd_name)
    }

    /// `DADiskCreateFromVolumePath`.
    pub fn disk_from_volume_path(&self, path: &str) -> Option<&Disk> {
        disk_events::find_by_volume_path(&self.disks, path)
    }

    /// `DADiskCopyDescription`, with the `DESCRIPTION_*_KEY` names.
    pub fn description(disk: &Disk) -> BTreeMap<&'static str, String> {
        disk_events::description(disk)
    }

    /// Asks the shell for the connected disks. The reply goes to [`DiskSession::on_list_reply`].
    pub fn list(&self, ctx: &Ctx) -> u32 {
        service::call(ctx, disk_wire::SERVICE, "list", &Json::Null)
    }

    /// Asks for the plan to unmount `bsd_name`. The reply is read with [`plan_from_reply`].
    pub fn plan_unmount(&self, ctx: &Ctx, bsd_name: &str) -> u32 {
        service::call(ctx, disk_wire::SERVICE, "plan_unmount", &named(bsd_name))
    }

    /// Asks for the plan to eject `bsd_name`. The reply is read with [`plan_from_reply`].
    pub fn plan_eject(&self, ctx: &Ctx, bsd_name: &str) -> u32 {
        service::call(ctx, disk_wire::SERVICE, "plan_eject", &named(bsd_name))
    }

    /// Takes the reply to [`DiskSession::list`]: disks not known yet are reported as appeared, known ones that
    /// differ as changed, and known ones the list lacks as disappeared. False for a reply that is not a disk list.
    pub fn on_list_reply(&mut self, result: &Result<Json, String>) -> bool {
        let Some(list) = result.as_ref().ok().and_then(|b| b.get("disks")).and_then(Json::as_array) else { return false };
        let next: Option<Vec<Disk>> = list.iter().map(disk_wire::decode_disk).collect();
        let Some(next) = next else { return false };
        for event in disk_events::diff(&self.disks, &next) {
            self.apply(event);
        }
        true
    }

    /// Takes a shell event. True when it was a disk event (whether or not it changed anything).
    pub fn handle_event(&mut self, service: &str, event: &str, body: &Json) -> bool {
        match disk_wire::decode_event(service, event, body) {
            Some(e) => {
                self.apply(e);
                true
            }
            None => false,
        }
    }

    /// Updates the known disks and runs the callbacks for one change. A repeat of a change already known is dropped.
    fn apply(&mut self, event: DiskEvent) {
        match event {
            DiskEvent::Appeared(disk) => {
                if self.disks.iter().any(|d| d.bsd_name == disk.bsd_name) {
                    return;
                }
                self.disks.push(disk.clone());
                for cb in self.appeared.clone() {
                    let d = disk.clone();
                    self.run(move || (*lock(&cb))(&d));
                }
            }
            DiskEvent::Disappeared(name) => {
                let before = self.disks.len();
                self.disks.retain(|d| d.bsd_name != name);
                if self.disks.len() == before {
                    return;
                }
                for cb in self.disappeared.clone() {
                    let n = name.clone();
                    self.run(move || (*lock(&cb))(n.as_str()));
                }
            }
            DiskEvent::DescriptionChanged { before, after } => {
                match self.disks.iter_mut().find(|d| d.bsd_name == after.bsd_name) {
                    Some(slot) if *slot != after => *slot = after.clone(),
                    Some(_) => return,
                    None => self.disks.push(after.clone()),
                }
                for cb in self.changed.clone() {
                    let (b, a) = (before.clone(), after.clone());
                    self.run(move || (*lock(&cb))(&b, &a));
                }
            }
        }
    }

    fn run(&self, f: impl FnOnce() + Send + 'static) {
        match &self.queue {
            Some(q) => q.async_(f),
            None => f(),
        }
    }
}

/// The guard of a callback, also after a callback panicked.
fn lock<T: ?Sized>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn named(bsd_name: &str) -> Json {
    Json::Obj(vec![("bsd_name".to_string(), Json::Str(bsd_name.to_string()))])
}

/// Reads the commands from a `plan_unmount` or `plan_eject` reply, or the shell's refusal sentence.
pub fn plan_from_reply(result: &Result<Json, String>) -> Result<Vec<PlannedCommand>, String> {
    let body = result.as_ref().map_err(|e| e.clone())?;
    let list = body.get("commands").and_then(Json::as_array).ok_or_else(|| "disks: the reply has no commands".to_string())?;
    list.iter()
        .map(|c| {
            let program = c.str_field("program").ok_or_else(|| "disks: a command has no program".to_string())?;
            let args = c
                .get("args")
                .and_then(Json::as_array)
                .ok_or_else(|| "disks: a command has no args".to_string())?
                .iter()
                .map(|a| a.as_str().map(str::to_string).ok_or_else(|| "disks: an argument is not text".to_string()))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(PlannedCommand { program, args })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usb(mount: Option<&str>) -> Disk {
        Disk { bsd_name: "sdb1".into(), size: 1024, removable: true, read_only: false, model: String::new(), mount_point: mount.map(String::from) }
    }

    /// A session whose callbacks append a line to a shared log.
    fn logged() -> (DiskSession, Arc<Mutex<Vec<String>>>) {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut s = DiskSession::create();
        let l = log.clone();
        s.register_disk_appeared_callback(move |d| l.lock().unwrap().push(format!("appeared {}", d.bsd_name)));
        let l = log.clone();
        s.register_disk_disappeared_callback(move |n| l.lock().unwrap().push(format!("disappeared {n}")));
        let l = log.clone();
        s.register_disk_description_changed_callback(move |b, a| {
            l.lock().unwrap().push(format!("changed {:?} -> {:?}", b.mount_point, a.mount_point))
        });
        (s, log)
    }

    fn feed(s: &mut DiskSession, e: &DiskEvent) -> bool {
        let (name, body) = disk_wire::encode_event(e);
        s.handle_event(disk_wire::SERVICE, name, &body)
    }

    #[test]
    fn events_run_the_callbacks_in_order() {
        let (mut s, log) = logged();
        assert!(feed(&mut s, &DiskEvent::Appeared(usb(None))));
        assert!(feed(&mut s, &DiskEvent::DescriptionChanged { before: usb(None), after: usb(Some("/media/u")) }));
        assert!(feed(&mut s, &DiskEvent::Disappeared("sdb1".into())));
        assert_eq!(*log.lock().unwrap(), vec!["appeared sdb1", "changed None -> Some(\"/media/u\")", "disappeared sdb1"]);
        assert!(s.disks().is_empty());
    }

    #[test]
    fn repeats_and_foreign_events_do_nothing() {
        let (mut s, log) = logged();
        feed(&mut s, &DiskEvent::Appeared(usb(None)));
        feed(&mut s, &DiskEvent::Appeared(usb(None)));
        feed(&mut s, &DiskEvent::Disappeared("sdz".into()));
        assert!(!s.handle_event("notify_bus", disk_wire::APPEARED, &Json::Null));
        assert_eq!(log.lock().unwrap().len(), 1);
    }

    #[test]
    fn the_list_reply_reports_disks_already_connected() {
        let (mut s, log) = logged();
        let reply = Ok(Json::Obj(vec![("disks".into(), Json::Arr(vec![disk_wire::encode_disk(&usb(None))]))]));
        assert!(s.on_list_reply(&reply));
        assert_eq!(*log.lock().unwrap(), vec!["appeared sdb1"]);
        assert!(s.on_list_reply(&reply));
        assert_eq!(log.lock().unwrap().len(), 1);
        assert!(!s.on_list_reply(&Err("no".into())));
        assert!(!s.on_list_reply(&Ok(Json::Null)));
    }

    #[test]
    fn lookups_use_the_known_disks() {
        let (mut s, _) = logged();
        feed(&mut s, &DiskEvent::Appeared(usb(Some("/media/u/"))));
        assert!(s.disk_from_bsd_name("sdb1").is_some());
        assert!(s.disk_from_volume_path("/media/u").is_some());
        assert!(s.disk_from_bsd_name("sdc").is_none());
        let d = DiskSession::description(s.disk_from_bsd_name("sdb1").unwrap());
        assert_eq!(d.get(DESCRIPTION_VOLUME_PATH_KEY).map(String::as_str), Some("/media/u/"));
        assert_eq!(d.get(DESCRIPTION_MEDIA_REMOVABLE_KEY).map(String::as_str), Some("true"));
    }

    #[test]
    fn a_dispatch_queue_receives_the_callbacks() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut s = DiskSession::create();
        s.set_dispatch_queue(DispatchQueue::serial("test.disks"));
        s.register_disk_appeared_callback(move |d| tx.send(d.bsd_name.clone()).unwrap());
        feed(&mut s, &DiskEvent::Appeared(usb(None)));
        assert_eq!(rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap(), "sdb1");
    }

    #[test]
    fn a_plan_reply_decodes() {
        let body = bifrost_core::json::parse(r#"{"commands":[{"program":"udisksctl","args":["unmount","-b","/dev/sdb1"]}]}"#).unwrap();
        let plan = plan_from_reply(&Ok(body)).unwrap();
        assert_eq!(plan, vec![PlannedCommand { program: "udisksctl".into(), args: vec!["unmount".into(), "-b".into(), "/dev/sdb1".into()] }]);
        assert_eq!(plan_from_reply(&Err("sdb1 is in use by x.".into())), Err("sdb1 is in use by x.".into()));
        assert!(plan_from_reply(&Ok(Json::Null)).is_err());
    }
}
