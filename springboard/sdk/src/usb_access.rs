//! Typed client for the `usb` shell service: raw access to one USB or raw HID device, with the person's consent.
//!
//! An app lists the `usb` (or `hid`) permission in its manifest, then:
//!
//! ```ignore
//! // 1. ask: the shell shows a sheet naming the device; only the app in front may ask
//! usb_access::request(ctx, DeviceKind::Usb, &[Filter { vendor_id: Some(0x0403), ..Default::default() }]);
//! // 2. App::service_event: usb_access::event(service, event, body) gives Event::Access { id, granted, device }
//! // 3. open the granted device; the reply parses with parse_opened
//! usb_access::open(ctx, &id);
//! // 4. claim an interface, then read and write; every request returns a token, and Event::Done carries it back
//! usb_access::claim(ctx, handle, 0);
//! usb_access::write(ctx, handle, 0x02, &bytes, 1000);
//! ```
//! The shell opens the device node itself and applies the rules of `bifrost_core::usb_session`: interfaces the system
//! drives (audio, HID, storage, video, hubs, smart cards) cannot be claimed, endpoints must belong to a claimed
//! interface, control requests are vendor or class requests, and one call moves at most [`MAX_BYTES`]. An app sees only
//! the devices it was granted: [`devices`] lists nothing else, and a plug or unplug of another device is silent.
//! Keyboards, keypads and security keys are never offered as raw HID devices.
//!
//! [`filters_from_criteria`] turns the matching criteria of [`crate::accessory_access`] into the filters of [`request`],
//! so code written against the listener model asks for the same devices.
use crate::runtime::Ctx;
use crate::service;
use bifrost_core::json::Json;
use bifrost_core::plist::{hex_decode, hex_encode};
pub use bifrost_core::usb_descriptors::{Direction, Recipient, RequestKind};
use bifrost_core::usb_descriptors::MatchingCriteria;
pub use bifrost_core::usb_grants::Filter;

/// The shell service name.
pub const SERVICE: &str = "usb";
/// The most bytes one transfer or control call moves.
pub const MAX_BYTES: usize = bifrost_core::usb_session::MAX_BROKER_BYTES;

/// Which bus a call is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceKind {
    Usb,
    Hid,
}

impl DeviceKind {
    fn name(self) -> &'static str {
        match self {
            DeviceKind::Usb => "usb",
            DeviceKind::Hid => "hid",
        }
    }
}

/// A device the app holds a grant for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    /// The id to pass to [`open`] and [`revoke`]: stable across ports while the device keeps its serial number.
    pub id: String,
    pub name: String,
    pub vendor_id: u16,
    pub product_id: u16,
    pub kind: DeviceKind,
    /// USB only.
    pub device_class: u8,
    /// USB only: the class of every interface.
    pub interface_classes: Vec<u8>,
    /// HID only: `(usage page, usage)` of each top-level collection.
    pub usages: Vec<(u16, u16)>,
}

/// One endpoint of an interface, as the shell read it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoint {
    pub address: u8,
    /// `bulk`, `interrupt`, `isochronous` or `control`.
    pub transfer: String,
    pub is_in: bool,
    pub max_packet_size: u16,
    pub interval: u8,
}

/// One interface (alternate setting) of the open device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Interface {
    pub number: u8,
    pub alternate: u8,
    pub class: u8,
    pub subclass: u8,
    pub protocol: u8,
    /// Whether [`claim`] will accept it: false for classes the system drives.
    pub claimable: bool,
    pub endpoints: Vec<Endpoint>,
}

/// What opening a device returns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Opened {
    Usb { handle: u32, usb_version: u16, device_class: u8, configuration: u8, max_power_ma: u16, interfaces: Vec<Interface> },
    Hid { handle: u32, descriptor: Vec<u8>, usages: Vec<(u16, u16)>, bus: String },
}

impl Opened {
    pub fn handle(&self) -> u32 {
        match self {
            Opened::Usb { handle, .. } | Opened::Hid { handle, .. } => *handle,
        }
    }
}

/// The result of one finished request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Done {
    pub handle: u32,
    /// The token the call returned.
    pub token: u64,
    /// The request this finishes: `claim`, `read`, `write`, `control`, `hid_write`, ...
    pub op: String,
    /// The bytes a read, control-in or get-feature returned.
    pub data: Option<Vec<u8>>,
    /// The bytes a write or set-feature moved.
    pub count: Option<usize>,
    /// Why it failed; `None` when it succeeded.
    pub error: Option<String>,
    /// The class of the failure: `denied` (the shell's rules refused it), `timeout`, `stall`, `disconnected`, `busy`,
    /// `overflow` or `other`.
    pub failure: Option<String>,
}

impl Done {
    pub fn is_ok(&self) -> bool {
        self.error.is_none()
    }
}

/// A service event, decoded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// The person answered a [`request`]. `device` is set when it was granted and is connected.
    Access { id: String, granted: bool, device: Option<Device> },
    /// A device the app holds a grant for was plugged in.
    Connected(Device),
    /// A device the app held a grant for was unplugged.
    Disconnected { id: String },
    Done(Done),
    /// An input report of an open HID device, report ID first when the device uses IDs.
    Report { handle: u32, data: Vec<u8> },
    /// A handle was closed by the shell: the device went away, the grant was revoked, or the read failed.
    Closed { handle: u32, reason: String },
}

fn num(n: impl Into<f64>) -> Json {
    Json::Num(n.into())
}

fn handle_body(handle: u32, mut more: Vec<(String, Json)>) -> Json {
    let mut pairs = vec![("handle".to_string(), num(handle))];
    pairs.append(&mut more);
    Json::Obj(pairs)
}

fn field(name: &str, v: Json) -> (String, Json) {
    (name.to_string(), v)
}

/// The body of a `devices` call.
pub fn devices_body(kind: DeviceKind) -> Json {
    Json::Obj(vec![field("kind", Json::Str(kind.name().into()))])
}

/// The body of a `request` call.
pub fn request_body(kind: DeviceKind, filters: &[Filter]) -> Json {
    let list = filters
        .iter()
        .map(|f| {
            let mut pairs = Vec::new();
            if let Some(v) = f.vendor_id {
                pairs.push(field("vendor_id", num(v)));
            }
            if let Some(v) = f.product_id {
                pairs.push(field("product_id", num(v)));
            }
            if let Some(v) = f.class {
                pairs.push(field("class", num(v)));
            }
            if let Some(v) = f.usage_page {
                pairs.push(field("usage_page", num(v)));
            }
            if let Some(v) = f.usage {
                pairs.push(field("usage", num(v)));
            }
            Json::Obj(pairs)
        })
        .collect();
    Json::Obj(vec![field("kind", Json::Str(kind.name().into())), field("filters", Json::Arr(list))])
}

/// A control request to send with [`control`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ControlRequest {
    pub direction: Direction,
    /// `Class` or `Vendor`; the shell refuses `Standard` requests.
    pub kind: RequestKind,
    pub recipient: Recipient,
    pub request: u8,
    pub value: u16,
    pub index: u16,
    /// The bytes to read (device to host) or the length of `data` (host to device).
    pub length: u16,
}

/// The body of a `control` call.
pub fn control_body(handle: u32, c: &ControlRequest, data: &[u8], timeout_ms: u32) -> Json {
    handle_body(
        handle,
        vec![
            field("direction", Json::Str(if c.direction == Direction::DeviceToHost { "in" } else { "out" }.into())),
            field(
                "kind",
                Json::Str(
                    match c.kind {
                        RequestKind::Standard => "standard",
                        RequestKind::Class => "class",
                        RequestKind::Vendor => "vendor",
                    }
                    .into(),
                ),
            ),
            field(
                "recipient",
                Json::Str(
                    match c.recipient {
                        Recipient::Device => "device",
                        Recipient::Interface => "interface",
                        Recipient::Endpoint => "endpoint",
                    }
                    .into(),
                ),
            ),
            field("request", num(c.request)),
            field("value", num(c.value)),
            field("index", num(c.index)),
            field("length", num(c.length)),
            field("data", Json::Str(hex_encode(data))),
            field("timeout", num(timeout_ms)),
        ],
    )
}

/// The filters that select the same devices as `criteria` (the listener model of [`crate::accessory_access`]): the
/// device match gives the vendor, product and device class, and each interface criterion becomes one filter that adds
/// its interface class. Subclass and protocol cannot be expressed in a filter; the shell checks them when it opens the
/// device, and a caller that needs them reads the interface list from [`parse_opened`].
pub fn filters_from_criteria(criteria: &MatchingCriteria) -> Vec<Filter> {
    let base = Filter { vendor_id: criteria.device.vendor_id, product_id: criteria.device.product_id, class: criteria.device.class, ..Default::default() };
    if criteria.interfaces.is_empty() {
        return vec![base];
    }
    criteria.interfaces.iter().map(|i| Filter { class: i.class.or(base.class), ..base }).collect()
}

/// Asks the person to grant a device. Only the app in front may ask. The reply is `{offered, asked}`; the answer arrives
/// as [`Event::Access`].
pub fn request(ctx: &Ctx, kind: DeviceKind, filters: &[Filter]) -> u32 {
    service::call(ctx, SERVICE, "request", &request_body(kind, filters))
}

/// Asks for the devices the app holds a grant for. The reply is parsed with [`parse_devices`].
pub fn devices(ctx: &Ctx, kind: DeviceKind) -> u32 {
    service::call(ctx, SERVICE, "devices", &devices_body(kind))
}

/// Takes a grant back and closes the device.
pub fn revoke(ctx: &Ctx, id: &str) -> u32 {
    service::call(ctx, SERVICE, "revoke", &Json::Obj(vec![field("id", Json::Str(id.into()))]))
}

/// Opens a granted device. The reply is parsed with [`parse_opened`].
pub fn open(ctx: &Ctx, id: &str) -> u32 {
    service::call(ctx, SERVICE, "open", &Json::Obj(vec![field("id", Json::Str(id.into()))]))
}

/// Closes an open device.
pub fn close(ctx: &Ctx, handle: u32) -> u32 {
    service::call(ctx, SERVICE, "close", &handle_body(handle, Vec::new()))
}

/// Claims an interface of an open USB device. Finishes as [`Event::Done`].
pub fn claim(ctx: &Ctx, handle: u32, interface: u8) -> u32 {
    service::call(ctx, SERVICE, "claim", &handle_body(handle, vec![field("interface", num(interface))]))
}

/// Gives an interface back.
pub fn release(ctx: &Ctx, handle: u32, interface: u8) -> u32 {
    service::call(ctx, SERVICE, "release", &handle_body(handle, vec![field("interface", num(interface))]))
}

/// Selects an alternate setting of a claimed interface.
pub fn alternate(ctx: &Ctx, handle: u32, interface: u8, alternate: u8) -> u32 {
    service::call(ctx, SERVICE, "alternate", &handle_body(handle, vec![field("interface", num(interface)), field("alternate", num(alternate))]))
}

/// Reads up to `length` bytes from a bulk or interrupt IN endpoint (`endpoint` includes the direction bit, `0x81`).
/// `timeout_ms` of 0 uses the default, and the shell caps it.
pub fn read(ctx: &Ctx, handle: u32, endpoint: u8, length: usize, timeout_ms: u32) -> u32 {
    service::call(ctx, SERVICE, "read", &handle_body(handle, vec![field("endpoint", num(endpoint)), field("length", num(length as f64)), field("timeout", num(timeout_ms))]))
}

/// Writes to a bulk or interrupt OUT endpoint (`0x02`).
pub fn write(ctx: &Ctx, handle: u32, endpoint: u8, data: &[u8], timeout_ms: u32) -> u32 {
    service::call(ctx, SERVICE, "write", &handle_body(handle, vec![field("endpoint", num(endpoint)), field("data", Json::Str(hex_encode(data))), field("timeout", num(timeout_ms))]))
}

/// Sends a class or vendor control request.
pub fn control(ctx: &Ctx, handle: u32, request: &ControlRequest, data: &[u8], timeout_ms: u32) -> u32 {
    service::call(ctx, SERVICE, "control", &control_body(handle, request, data, timeout_ms))
}

/// Clears a halted endpoint of a claimed interface.
pub fn clear_halt(ctx: &Ctx, handle: u32, endpoint: u8) -> u32 {
    service::call(ctx, SERVICE, "clear_halt", &handle_body(handle, vec![field("endpoint", num(endpoint))]))
}

/// Resets the device.
pub fn reset(ctx: &Ctx, handle: u32) -> u32 {
    service::call(ctx, SERVICE, "reset", &handle_body(handle, Vec::new()))
}

/// Writes an output report to an open HID device (the report ID first, or 0 when the device has none).
pub fn hid_write(ctx: &Ctx, handle: u32, report: &[u8]) -> u32 {
    service::call(ctx, SERVICE, "hid_write", &handle_body(handle, vec![field("data", Json::Str(hex_encode(report)))]))
}

/// Reads a feature report. `length` counts the report ID byte; the answer's first byte is the report ID.
pub fn hid_get_feature(ctx: &Ctx, handle: u32, report_id: u8, length: usize) -> u32 {
    service::call(ctx, SERVICE, "hid_get_feature", &handle_body(handle, vec![field("report_id", num(report_id)), field("length", num(length as f64))]))
}

/// Sends a feature report (the report ID first, or 0 when the device has none).
pub fn hid_set_feature(ctx: &Ctx, handle: u32, report: &[u8]) -> u32 {
    service::call(ctx, SERVICE, "hid_set_feature", &handle_body(handle, vec![field("data", Json::Str(hex_encode(report)))]))
}

fn number(v: &Json, key: &str) -> Option<f64> {
    v.get(key)?.number()
}

fn pairs(v: &Json, key: &str) -> Vec<(u16, u16)> {
    v.get(key)
        .and_then(Json::as_array)
        .unwrap_or(&[])
        .iter()
        .filter_map(|p| {
            let p = p.as_array()?;
            Some((p.first()?.number()? as u16, p.get(1)?.number()? as u16))
        })
        .collect()
}

/// A device as the service describes it. `None` for anything malformed.
pub fn parse_device(v: &Json) -> Option<Device> {
    let kind = match v.get("kind")?.as_str()? {
        "usb" => DeviceKind::Usb,
        "hid" => DeviceKind::Hid,
        _ => return None,
    };
    Some(Device {
        id: v.get("id")?.as_str()?.to_string(),
        name: v.get("name")?.as_str()?.to_string(),
        vendor_id: number(v, "vendor_id")? as u16,
        product_id: number(v, "product_id")? as u16,
        kind,
        device_class: number(v, "device_class").unwrap_or(0.0) as u8,
        interface_classes: v.get("interface_classes").and_then(Json::as_array).unwrap_or(&[]).iter().filter_map(|k| k.number().map(|k| k as u8)).collect(),
        usages: pairs(v, "usages"),
    })
}

/// The devices in a `devices` reply. A malformed entry is skipped.
pub fn parse_devices(reply: &Json) -> Vec<Device> {
    reply.as_array().unwrap_or(&[]).iter().filter_map(parse_device).collect()
}

/// The `(offered, asked)` of a `request` reply: how many devices matched, and whether a sheet was queued.
pub fn parse_request(reply: &Json) -> Option<(usize, bool)> {
    Some((reply.get("offered")?.as_u64()? as usize, matches!(reply.get("asked"), Some(Json::Bool(true)))))
}

/// The token of a request that finishes later, from the reply to [`claim`], [`read`] and the other handle calls.
pub fn parse_token(reply: &Json) -> Option<u64> {
    reply.get("token")?.as_u64()
}

/// What an `open` reply says, for a USB device or a HID device.
pub fn parse_opened(reply: &Json) -> Option<Opened> {
    let handle = number(reply, "handle")? as u32;
    if let Some(descriptor) = reply.get("descriptor") {
        return Some(Opened::Hid {
            handle,
            descriptor: hex_decode(descriptor.as_str()?)?,
            usages: pairs(reply, "usages"),
            bus: reply.get("bus").and_then(Json::as_str).unwrap_or("").to_string(),
        });
    }
    let config = reply.get("configuration")?;
    let interfaces = config
        .get("interfaces")?
        .as_array()?
        .iter()
        .filter_map(|i| {
            let endpoints = i
                .get("endpoints")
                .and_then(Json::as_array)
                .unwrap_or(&[])
                .iter()
                .filter_map(|e| {
                    Some(Endpoint {
                        address: number(e, "address")? as u8,
                        transfer: e.get("type")?.as_str()?.to_string(),
                        is_in: e.get("direction")?.as_str()? == "in",
                        max_packet_size: number(e, "max_packet_size")? as u16,
                        interval: number(e, "interval").unwrap_or(0.0) as u8,
                    })
                })
                .collect();
            Some(Interface {
                number: number(i, "number")? as u8,
                alternate: number(i, "alternate")? as u8,
                class: number(i, "class")? as u8,
                subclass: number(i, "subclass").unwrap_or(0.0) as u8,
                protocol: number(i, "protocol").unwrap_or(0.0) as u8,
                claimable: matches!(i.get("claimable"), Some(Json::Bool(true))),
                endpoints,
            })
        })
        .collect();
    Some(Opened::Usb {
        handle,
        usb_version: number(reply, "usb_version")? as u16,
        device_class: number(reply, "device_class").unwrap_or(0.0) as u8,
        configuration: number(config, "value")? as u8,
        max_power_ma: number(config, "max_power_ma").unwrap_or(0.0) as u16,
        interfaces,
    })
}

/// The parsed report descriptor of an opened HID device, with its elements and collections
/// (`bifrost_ui::hid_report`), so an app reads fields out of the reports of [`Event::Report`] by usage. `None` for a USB
/// open or a descriptor that does not parse.
pub fn hid_descriptor(opened: &Opened) -> Option<bifrost_ui::hid_report::ReportDescriptor> {
    match opened {
        Opened::Hid { descriptor, .. } => bifrost_ui::hid_report::ReportDescriptor::parse(descriptor).ok(),
        Opened::Usb { .. } => None,
    }
}

/// Decodes a service event for the `usb` service. Returns `None` for another service or an unknown event, so an app can
/// pass every `App::service_event` through it.
pub fn event(service: &str, event: &str, body: &Json) -> Option<Event> {
    if service != SERVICE {
        return None;
    }
    match event {
        "access" => Some(Event::Access {
            id: body.get("id")?.as_str()?.to_string(),
            granted: matches!(body.get("granted"), Some(Json::Bool(true))),
            device: body.get("device").and_then(parse_device),
        }),
        "connected" => parse_device(body.get("device")?).map(Event::Connected),
        "disconnected" => Some(Event::Disconnected { id: body.get("id")?.as_str()?.to_string() }),
        "done" => {
            let ok = matches!(body.get("ok"), Some(Json::Bool(true)));
            let data = match body.get("data") {
                Some(d) => Some(hex_decode(d.as_str()?)?),
                None => None,
            };
            Some(Event::Done(Done {
                handle: number(body, "handle")? as u32,
                token: body.get("token")?.as_u64()?,
                op: body.get("op").and_then(Json::as_str).unwrap_or("").to_string(),
                data,
                count: body.get("count").and_then(Json::as_u64).map(|n| n as usize),
                error: if ok { None } else { Some(body.get("error").and_then(Json::as_str).unwrap_or("failed").to_string()) },
                failure: if ok { None } else { Some(body.get("failure").and_then(Json::as_str).unwrap_or("other").to_string()) },
            }))
        }
        "report" => Some(Event::Report { handle: number(body, "handle")? as u32, data: hex_decode(body.get("data")?.as_str()?)? }),
        "closed" => Some(Event::Closed { handle: number(body, "handle")? as u32, reason: body.get("reason").and_then(Json::as_str).unwrap_or("").to_string() }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_core::usb_descriptors::{DeviceMatch, InterfaceMatch};

    fn obj(fields: &[(&str, Json)]) -> Json {
        Json::Obj(fields.iter().map(|(k, v)| ((*k).to_string(), v.clone())).collect())
    }

    fn s(t: &str) -> Json {
        Json::Str(t.into())
    }

    fn n(v: f64) -> Json {
        Json::Num(v)
    }

    #[test]
    fn a_request_body_carries_only_the_filter_fields_that_are_set() {
        let body = request_body(DeviceKind::Hid, &[Filter { vendor_id: Some(0x1234), usage_page: Some(0xff00), ..Default::default() }, Filter::default()]);
        assert_eq!(body.get("kind").and_then(Json::as_str), Some("hid"));
        let list = body.get("filters").and_then(Json::as_array).unwrap();
        assert_eq!(list[0], obj(&[("vendor_id", n(f64::from(0x1234))), ("usage_page", n(f64::from(0xff00)))]));
        assert_eq!(list[1], obj(&[]));
        assert_eq!(devices_body(DeviceKind::Usb).get("kind").and_then(Json::as_str), Some("usb"));
    }

    #[test]
    fn matching_criteria_become_filters_one_per_interface_criterion() {
        let none = MatchingCriteria { device: DeviceMatch { vendor_id: Some(1), product_id: Some(2), ..Default::default() }, ..Default::default() };
        assert_eq!(filters_from_criteria(&none), vec![Filter { vendor_id: Some(1), product_id: Some(2), ..Default::default() }]);
        let two = MatchingCriteria {
            device: DeviceMatch { vendor_id: Some(1), ..Default::default() },
            interfaces: vec![InterfaceMatch { class: Some(0xff), ..Default::default() }, InterfaceMatch { class: Some(0x02), subclass: Some(2), ..Default::default() }],
            option: None,
        };
        let f = filters_from_criteria(&two);
        assert_eq!(f.len(), 2);
        assert_eq!((f[0].vendor_id, f[0].class), (Some(1), Some(0xff)));
        assert_eq!(f[1].class, Some(0x02));
        let any = filters_from_criteria(&MatchingCriteria::default());
        assert_eq!(any, vec![Filter::default()], "no criteria selects any eligible device");
    }

    #[test]
    fn a_control_body_spells_the_request_and_hex_encodes_the_data() {
        let c = ControlRequest { direction: Direction::HostToDevice, kind: RequestKind::Vendor, recipient: Recipient::Interface, request: 5, value: 1, index: 0, length: 2 };
        let body = control_body(3, &c, &[0xab, 0x01], 250);
        assert_eq!(body.get("handle").and_then(Json::as_u64), Some(3));
        assert_eq!(body.get("direction").and_then(Json::as_str), Some("out"));
        assert_eq!(body.get("kind").and_then(Json::as_str), Some("vendor"));
        assert_eq!(body.get("recipient").and_then(Json::as_str), Some("interface"));
        assert_eq!(body.get("data").and_then(Json::as_str), Some("ab01"));
        assert_eq!(body.get("timeout").and_then(Json::as_u64), Some(250));
    }

    #[test]
    fn devices_and_requests_replies_decode() {
        let usb = obj(&[("id", s("0403:6001:A1")), ("name", s("Probe")), ("vendor_id", n(1027.0)), ("product_id", n(24577.0)), ("kind", s("usb")), ("device_class", n(0.0)), ("interface_classes", Json::Arr(vec![n(255.0)]))]);
        let hid = obj(&[("id", s("hid:1234:0002:-")), ("name", s("Pad")), ("vendor_id", n(4660.0)), ("product_id", n(2.0)), ("kind", s("hid")), ("usages", Json::Arr(vec![Json::Arr(vec![n(65280.0), n(1.0)])]))]);
        let list = parse_devices(&Json::Arr(vec![usb, hid, obj(&[("kind", s("usb"))]), obj(&[("id", s("x")), ("name", s("y")), ("vendor_id", n(1.0)), ("product_id", n(1.0)), ("kind", s("scsi"))])]));
        assert_eq!(list.len(), 2, "malformed entries are skipped");
        assert_eq!((list[0].vendor_id, list[0].product_id, list[0].interface_classes.clone()), (0x0403, 0x6001, vec![0xff]));
        assert_eq!(list[1].kind, DeviceKind::Hid);
        assert_eq!(list[1].usages, vec![(0xff00, 1)]);
        assert_eq!(parse_request(&obj(&[("offered", n(2.0)), ("asked", Json::Bool(true))])), Some((2, true)));
        assert_eq!(parse_request(&obj(&[("offered", n(0.0)), ("asked", Json::Bool(false))])), Some((0, false)));
        assert_eq!(parse_request(&Json::Null), None);
        assert_eq!(parse_token(&obj(&[("token", n(9.0))])), Some(9));
        assert_eq!(parse_token(&Json::Null), None);
    }

    #[test]
    fn an_open_reply_decodes_for_usb_and_hid() {
        let usb = obj(&[
            ("handle", n(4.0)),
            ("usb_version", n(512.0)),
            ("device_class", n(0.0)),
            (
                "configuration",
                obj(&[
                    ("value", n(1.0)),
                    ("max_power_ma", n(100.0)),
                    (
                        "interfaces",
                        Json::Arr(vec![obj(&[
                            ("number", n(0.0)),
                            ("alternate", n(0.0)),
                            ("class", n(255.0)),
                            ("claimable", Json::Bool(true)),
                            ("endpoints", Json::Arr(vec![obj(&[("address", n(129.0)), ("type", s("bulk")), ("direction", s("in")), ("max_packet_size", n(64.0)), ("interval", n(0.0))])])),
                        ])]),
                    ),
                ]),
            ),
        ]);
        let Some(Opened::Usb { handle, usb_version, configuration, interfaces, .. }) = parse_opened(&usb) else { panic!("not a USB open") };
        assert_eq!((handle, usb_version, configuration), (4, 0x0200, 1));
        assert!(interfaces[0].claimable);
        assert_eq!(interfaces[0].endpoints[0], Endpoint { address: 0x81, transfer: "bulk".into(), is_in: true, max_packet_size: 64, interval: 0 });
        let hid = obj(&[("handle", n(5.0)), ("descriptor", s("0601ff")), ("usages", Json::Arr(vec![Json::Arr(vec![n(65280.0), n(1.0)])])), ("bus", s("usb"))]);
        assert_eq!(parse_opened(&hid), Some(Opened::Hid { handle: 5, descriptor: vec![6, 1, 255], usages: vec![(0xff00, 1)], bus: "usb".into() }));
        assert_eq!(parse_opened(&hid).unwrap().handle(), 5);
        assert_eq!(parse_opened(&obj(&[("handle", n(1.0)), ("descriptor", s("zz"))])), None, "bad hex is refused");
        assert_eq!(parse_opened(&Json::Null), None);
    }

    #[test]
    fn the_report_descriptor_of_an_opened_hid_device_parses_into_elements() {
        // vendor page 0xFF00, one application collection with an 8-bit input field and an 8-bit output field
        let descriptor = vec![0x06, 0x00, 0xff, 0x09, 0x01, 0xa1, 0x01, 0x09, 0x02, 0x15, 0x00, 0x26, 0xff, 0x00, 0x75, 0x08, 0x95, 0x01, 0x81, 0x02, 0x09, 0x03, 0x91, 0x02, 0xc0];
        let hid = Opened::Hid { handle: 1, descriptor, usages: vec![(0xff00, 1)], bus: "usb".into() };
        let parsed = hid_descriptor(&hid).expect("the descriptor parses");
        assert_eq!(parsed.elements.len(), 2);
        assert_eq!((parsed.elements[0].usage_page, parsed.elements[0].usage), (0xff00, 2));
        let report = [0x2a];
        assert_eq!(parsed.elements[0].value(&report), Some(42), "a field is read out of an input report by its element");
        assert_eq!(hid_descriptor(&Opened::Hid { handle: 1, descriptor: vec![0x05], usages: vec![], bus: String::new() }), None, "a cut descriptor is refused");
        let usb = Opened::Usb { handle: 1, usb_version: 0x0200, device_class: 0, configuration: 1, max_power_ma: 0, interfaces: vec![] };
        assert!(hid_descriptor(&usb).is_none());
    }

    #[test]
    fn events_decode_and_other_services_are_ignored() {
        let dev = obj(&[("id", s("k")), ("name", s("Probe")), ("vendor_id", n(1.0)), ("product_id", n(2.0)), ("kind", s("usb"))]);
        assert!(matches!(event(SERVICE, "access", &obj(&[("id", s("k")), ("granted", Json::Bool(true)), ("device", dev.clone())])), Some(Event::Access { granted: true, device: Some(_), .. })));
        assert_eq!(event(SERVICE, "access", &obj(&[("id", s("k")), ("granted", Json::Bool(false))])), Some(Event::Access { id: "k".into(), granted: false, device: None }));
        assert!(matches!(event(SERVICE, "connected", &obj(&[("device", dev)])), Some(Event::Connected(d)) if d.name == "Probe"));
        assert_eq!(event(SERVICE, "disconnected", &obj(&[("id", s("k"))])), Some(Event::Disconnected { id: "k".into() }));
        assert_eq!(event(SERVICE, "report", &obj(&[("handle", n(2.0)), ("data", s("0102"))])), Some(Event::Report { handle: 2, data: vec![1, 2] }));
        assert_eq!(event(SERVICE, "closed", &obj(&[("handle", n(2.0)), ("reason", s("gone"))])), Some(Event::Closed { handle: 2, reason: "gone".into() }));
        assert_eq!(event("gamepad", "report", &obj(&[])), None);
        assert_eq!(event(SERVICE, "unknown", &obj(&[])), None);
        assert_eq!(event(SERVICE, "report", &obj(&[("handle", n(2.0)), ("data", s("0"))])), None, "odd hex");
    }

    #[test]
    fn a_done_event_carries_data_a_count_or_an_error() {
        let ok = event(SERVICE, "done", &obj(&[("handle", n(1.0)), ("token", n(7.0)), ("ok", Json::Bool(true)), ("data", s("beef"))])).unwrap();
        assert_eq!(ok, Event::Done(Done { handle: 1, token: 7, op: String::new(), data: Some(vec![0xbe, 0xef]), count: None, error: None, failure: None }));
        let wrote = event(SERVICE, "done", &obj(&[("handle", n(1.0)), ("token", n(8.0)), ("ok", Json::Bool(true)), ("count", n(64.0))])).unwrap();
        assert!(matches!(wrote, Event::Done(Done { count: Some(64), .. })));
        let failed = event(SERVICE, "done", &obj(&[("handle", n(1.0)), ("token", n(9.0)), ("ok", Json::Bool(false)), ("op", s("read")), ("error", s("endpoint 0x81 has the other direction")), ("failure", s("denied"))])).unwrap();
        let Event::Done(d) = failed else { panic!("not done") };
        assert!(!d.is_ok());
        assert_eq!(d.error.as_deref(), Some("endpoint 0x81 has the other direction"));
        assert_eq!((d.op.as_str(), d.failure.as_deref()), ("read", Some("denied")));
    }
}
