//! The window's cloud side. At start the window asks the shared keychain group for the BifrOSt Account session; with a
//! session it starts the cloud worker (`bifrost_locate::cloud`), which registers this device. Events from the worker
//! come back through `tick`, when the worker's pipe is readable: device positions are opened with the account key,
//! pending devices and shared positions are kept, a key from an approving device is stored in the keychain, and feed
//! events become shell notifications.

use crate::window::LocateWindow;
use bifrost_app::cloud_findmy::{CloudFix, DeviceList};
use bifrost_app::core_location::{AuthorizationStatus, Delegate, Location, LocationError};
use bifrost_app::power_sources;
use bifrost_app::controls::x25519::PrivateKey;
use bifrost_app::keychain_groups;
use bifrost_app::usernotif;
use bifrost_app::{entropy, Ctx};
use bifrost_core::json::Json;
use bifrost_locate::cloud::{Event, Job, Link};
use bifrost_locate::pairing::PollAnswer;
use bifrost_locate::reporting::Action;
use bifrost_locate::sealing::{self, Report, Sealed};
use std::time::{SystemTime, UNIX_EPOCH};

/// The keychain group and item the BifrOSt Account session is read from. The account sign-in stores it there.
pub const SESSION_GROUP: &str = "is.bifrost.account";
pub const SESSION_SERVICE: &str = "bc_session";
pub const SESSION_ACCOUNT: &str = "session";

/// The window's location delegate: the newest fix delivered, whether a request failed, and the authorization.
#[derive(Default)]
pub struct Fixes {
    pub latest: Option<Location>,
    pub failed: bool,
    pub status: Option<AuthorizationStatus>,
}

impl Delegate for Fixes {
    fn did_update_locations(&mut self, locations: &[Location]) {
        if let Some(last) = locations.last() {
            self.latest = Some(last.clone());
        }
    }

    fn did_fail_with_error(&mut self, _error: LocationError) {
        self.failed = true;
    }

    fn did_change_authorization(&mut self, status: AuthorizationStatus) {
        self.status = Some(status);
    }
}

/// How often a sharing device checks whether a report is due, in milliseconds.
const REPORT_TICK_MS: f64 = 60_000.0;

fn unix_now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// The device's name on the account: the host name, or a fixed name when there is none.
pub fn device_name() -> String {
    std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "BifrOSt computer".into())
}

impl LocateWindow {
    /// Asks the keychain group for the session. Called once at start.
    pub fn start_session(&mut self, ctx: &Ctx) {
        self.session_request = Some(keychain_groups::get(ctx, SESSION_GROUP, SESSION_SERVICE, SESSION_ACCOUNT));
    }

    /// A service reply: the session starts the worker, and a location reply goes to the location manager. Returns
    /// whether the reply was one of ours.
    pub fn service_reply(&mut self, ctx: &Ctx, request: u32, result: &Result<Json, String>) -> bool {
        if self.session_request == Some(request) {
            self.session_request = None;
            match keychain_groups::decode(result) {
                Ok(bytes) => match String::from_utf8(bytes) {
                    Ok(token) if !token.trim().is_empty() => self.connect(ctx, token.trim().to_string()),
                    _ => ctx.log("locate: the account session is empty"),
                },
                Err(_) => ctx.log("locate: no BifrOSt Account session; the cloud is off"),
            }
            return true;
        }
        if self.location.handle_reply(result, &mut self.fixes) {
            self.after_location(ctx);
            return true;
        }
        false
    }

    /// Acts on what the location manager delivered: a report waiting for a fix is sent, a failure is logged.
    fn after_location(&mut self, ctx: &Ctx) {
        if self.fixes.failed {
            self.fixes.failed = false;
            self.want_report = false;
            ctx.log("locate: no location to report");
        }
        if let Some(fix) = self.fixes.latest.take() {
            self.panel.location_status = Some(format!("Position known (within {} m)", fix.horizontal_accuracy.round() as i64));
            if std::mem::take(&mut self.want_report) {
                self.send_report(ctx, &fix);
            }
        }
    }

    /// Seals a report of this position to the account key and hands it to the worker. Needs the account key and the
    /// device id the cloud gave at registration; without them nothing is sent.
    fn send_report(&mut self, ctx: &Ctx, fix: &Location) {
        let (Some(link), Some(public), Some(device)) = (self.link.as_ref(), self.keys.public(), self.device_id.clone()) else {
            ctx.log("locate: report not sent (no account key or device id yet)");
            return;
        };
        let Ok(random) = entropy::bytes(32) else { return };
        let report = Report {
            lat: fix.coordinate.latitude,
            lon: fix.coordinate.longitude,
            accuracy_m: Some(fix.horizontal_accuracy).filter(|a| *a >= 0.0),
            at_unix: fix.timestamp as i64,
            battery: power_sources::battery().and_then(|b| b.percent),
        };
        match sealing::seal_report(&public, &device, &report, &random) {
            Ok(own) => link.send(Job::Report { own, copies: Vec::new() }),
            Err(e) => ctx.log(&format!("locate: report not sealed: {e:?}")),
        }
    }

    fn connect(&mut self, ctx: &Ctx, token: String) {
        match Link::start(token, device_name()) {
            Ok(link) => {
                self.link = Some(link);
                self.publish_key();
            }
            Err(e) => ctx.log(&format!("locate: cloud worker: {e}")),
        }
    }

    /// Publishes the account public key, once it is held and the worker is running.
    pub fn publish_key(&self) {
        if let (Some(link), Some(public)) = (self.link.as_ref(), self.keys.public()) {
            link.send(Job::PublishKey(public));
        }
    }

    /// The asks a joining device makes: a key pair for this device, and a join request under this computer's name.
    /// Only a device without an account key asks, and only once the worker runs.
    pub fn start_enrollment(&mut self, ctx: &Ctx) {
        let Some(link) = self.link.as_ref() else {
            ctx.log("locate: sign in to your BifrOSt Account first");
            return;
        };
        if self.keys.private().is_some() || self.device_key.is_some() {
            return;
        }
        let Ok(random) = entropy::bytes(32) else { return };
        let Ok(raw) = <[u8; 32]>::try_from(random.as_slice()) else { return };
        let Ok(key) = PrivateKey::from_raw(&raw) else { return };
        self.device_key = Some(raw);
        link.send(Job::Enroll { device_public: key.public_key().raw(), name: device_name() });
    }

    /// The periodic step: while the worker runs, a sharing device reports when a report is due or a locate was asked.
    pub fn next_wake(&self, t: f64) -> Option<f64> {
        (self.panel.sharing() && self.link.is_some()).then_some(t + REPORT_TICK_MS)
    }

    /// Handles the worker's events and the report step. Returns whether a frame is needed.
    pub fn tick(&mut self, ctx: &Ctx, _t: f64) -> bool {
        let mut dirty = self.drain_events(ctx);
        dirty |= self.step_report(ctx);
        dirty
    }

    fn drain_events(&mut self, ctx: &Ctx) -> bool {
        let events = match self.link.as_mut() {
            Some(link) => link.drain(),
            None => return false,
        };
        let any = !events.is_empty();
        for event in events {
            self.handle(ctx, event);
        }
        any
    }

    fn handle(&mut self, ctx: &Ctx, event: Event) {
        match event {
            Event::Registered(id) => {
                self.device_id = Some(id);
                ctx.log("locate: device registered");
            }
            Event::Locate => self.locate_asked = true,
            Event::PlaySound => {}
            Event::Devices(mut list) => {
                self.open_devices(&mut list);
                self.panel.apply_list(&list);
            }
            Event::Pending(list) => self.approvals.sync(list),
            Event::Shared(list) => self.shared = list,
            Event::Enrolled(id) => {
                self.enrollment.start(&id);
            }
            Event::SealedKey(answer) => self.finish_enrollment(ctx, answer),
            Event::Notify { title, body } => self.notify(ctx, &title, &body),
            Event::Failed(text) => ctx.log(&format!("locate: {text}")),
        }
    }

    /// Opens the sealed position of each device with the account key, when this device holds it.
    fn open_devices(&self, list: &mut DeviceList) {
        let Some(private) = self.keys.private() else { return };
        for device in list.devices.iter_mut() {
            let Some(blob) = &device.sealed_location else { continue };
            let sealed = Sealed { sealed: blob.sealed.clone(), enc: blob.enc.clone() };
            if let Ok(report) = sealing::open_report(private, &device.id, &sealed) {
                device.fix = Some(CloudFix {
                    latitude: report.lat,
                    longitude: report.lon,
                    accuracy_m: report.accuracy_m,
                    recorded_at: report.at_unix,
                });
                if let Some(battery) = report.battery {
                    device.battery_percent = Some(battery);
                }
            }
        }
    }

    /// The sealed-key poll of a joining device: a sealed key is opened with this device's key and stored in the keychain.
    fn finish_enrollment(&mut self, ctx: &Ctx, answer: PollAnswer) {
        let pending = self.enrollment.pending_id().map(str::to_string);
        let Some(blob) = self.enrollment.on_poll(answer) else { return };
        let (Some(pending), Some(device)) = (pending, self.device_key) else { return };
        let Some(sealed) = sealing::unpack(&blob) else {
            ctx.log("locate: the sealed key is malformed");
            return;
        };
        match sealing::open_account_key(&device, &pending, &sealed) {
            Ok(key) => {
                self.keys.adopt(ctx, key.raw());
                self.device_key = None;
            }
            Err(e) => ctx.log(&format!("locate: account key: {e:?}")),
        }
    }

    fn notify(&mut self, ctx: &Ctx, title: &str, body: &str) {
        self.notice_seq = self.notice_seq.wrapping_add(1);
        let id = format!("is.olibuijr.locate.desktop.{}", self.notice_seq);
        usernotif::add(ctx, usernotif::Request::new(&id, title).body(body));
    }

    /// Runs the reporter for this tick. The clear and the report go to the worker; the report itself is built in
    /// `report`, which reads the position.
    fn step_report(&mut self, ctx: &Ctx) -> bool {
        if self.link.is_none() {
            return false;
        }
        let asked = std::mem::take(&mut self.locate_asked);
        match self.reporter.step(self.panel.sharing(), asked, unix_now()) {
            Action::Nothing => false,
            Action::Clear => {
                if let Some(link) = self.link.as_ref() {
                    link.send(Job::Clear);
                }
                true
            }
            Action::Report => {
                self.want_report = true;
                self.location.request_location(ctx);
                false
            }
        }
    }
}
