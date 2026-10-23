//! The JSON shape of a disk on the wire between the shell's `disks` service and the SDK's disk session, and of the
//! events the service pushes. One codec for both sides, so they cannot drift apart.
use crate::disk_events::{Disk, DiskEvent};
use crate::disk_scan::Volume;
use crate::json::Json;

/// The service name, as the shell registers it.
pub const SERVICE: &str = "disks";
/// Event: a disk appeared. Body is a disk.
pub const APPEARED: &str = "disk_appeared";
/// Event: a disk went away. Body is `{bsd_name}`.
pub const DISAPPEARED: &str = "disk_disappeared";
/// Event: a disk's description changed. Body is `{before, after}`.
pub const DESCRIPTION_CHANGED: &str = "disk_description_changed";

/// A disk as JSON.
pub fn encode_disk(d: &Disk) -> Json {
    let mut f = vec![
        ("bsd_name".to_string(), Json::Str(d.bsd_name.clone())),
        ("size".to_string(), Json::Num(d.size as f64)),
        ("removable".to_string(), Json::Bool(d.removable)),
        ("read_only".to_string(), Json::Bool(d.read_only)),
        ("model".to_string(), Json::Str(d.model.clone())),
    ];
    if let Some(p) = &d.mount_point {
        f.push(("mount_point".to_string(), Json::Str(p.clone())));
    }
    Json::Obj(f)
}

/// A disk from JSON. None when the name is missing or empty, or a flag or the size has the wrong type.
pub fn decode_disk(j: &Json) -> Option<Disk> {
    let bsd_name = j.str_field("bsd_name").filter(|n| !n.is_empty())?;
    let flag = |k: &str| match j.get(k) {
        Some(Json::Bool(b)) => Some(*b),
        _ => None,
    };
    let size = j.get("size").and_then(Json::number).filter(|n| *n >= 0.0 && n.fract() == 0.0)? as u64;
    Some(Disk {
        bsd_name,
        size,
        removable: flag("removable")?,
        read_only: flag("read_only")?,
        model: j.str_field("model").unwrap_or_default(),
        mount_point: j.str_field("mount_point"),
    })
}

/// A disk or partition with its parent, file system type and label as JSON (the `volumes` call). The fields of a disk
/// come first, so [`decode_disk`] reads a volume too.
pub fn encode_volume(v: &Volume) -> Json {
    let mut j = encode_disk(&v.disk);
    if let Json::Obj(f) = &mut j {
        for (key, value) in [("parent", &v.parent), ("fs_type", &v.fs_type), ("label", &v.label)] {
            if let Some(value) = value {
                f.push((key.to_string(), Json::Str(value.clone())));
            }
        }
    }
    j
}

/// The space on a mounted volume, measured by the shell (`statvfs` on the host mount point, which an app sandbox
/// cannot see): an entry of the `volumes` answer's `usage` list. `free` is what an unprivileged write can still use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VolumeUsage {
    pub bsd_name: String,
    pub total: u64,
    pub used: u64,
    pub free: u64,
}

pub fn encode_usage(u: &VolumeUsage) -> Json {
    Json::Obj(vec![
        ("bsd_name".into(), Json::Str(u.bsd_name.clone())),
        ("total".into(), Json::Num(u.total as f64)),
        ("used".into(), Json::Num(u.used as f64)),
        ("free".into(), Json::Num(u.free as f64)),
    ])
}

/// A usage entry from JSON. None when the name is missing or a number is not a whole, non-negative one.
pub fn decode_usage(j: &Json) -> Option<VolumeUsage> {
    let n = |k: &str| j.get(k).and_then(Json::number).filter(|n| *n >= 0.0 && n.fract() == 0.0).map(|n| n as u64);
    Some(VolumeUsage { bsd_name: j.str_field("bsd_name").filter(|s| !s.is_empty())?, total: n("total")?, used: n("used")?, free: n("free")? })
}

/// A volume from JSON. None when its disk part is malformed.
pub fn decode_volume(j: &Json) -> Option<Volume> {
    Some(Volume { disk: decode_disk(j)?, parent: j.str_field("parent"), fs_type: j.str_field("fs_type"), label: j.str_field("label") })
}

/// The event name and body the service pushes for one change.
pub fn encode_event(e: &DiskEvent) -> (&'static str, Json) {
    match e {
        DiskEvent::Appeared(d) => (APPEARED, encode_disk(d)),
        DiskEvent::Disappeared(n) => (DISAPPEARED, Json::Obj(vec![("bsd_name".into(), Json::Str(n.clone()))])),
        DiskEvent::DescriptionChanged { before, after } => {
            (DESCRIPTION_CHANGED, Json::Obj(vec![("before".into(), encode_disk(before)), ("after".into(), encode_disk(after))]))
        }
    }
}

/// The change an event carries. None for another service's event, an unknown name or a malformed body.
pub fn decode_event(service: &str, event: &str, body: &Json) -> Option<DiskEvent> {
    if service != SERVICE {
        return None;
    }
    match event {
        APPEARED => decode_disk(body).map(DiskEvent::Appeared),
        DISAPPEARED => body.str_field("bsd_name").filter(|n| !n.is_empty()).map(DiskEvent::Disappeared),
        DESCRIPTION_CHANGED => {
            Some(DiskEvent::DescriptionChanged { before: decode_disk(body.get("before")?)?, after: decode_disk(body.get("after")?)? })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_round_trips_and_malformed_entries_are_dropped() {
        let u = VolumeUsage { bsd_name: "nvme0n1p2".into(), total: 512_000_000_000, used: 100_000_000_000, free: 400_000_000_000 };
        assert_eq!(decode_usage(&encode_usage(&u)), Some(u));
        let bad = Json::Obj(vec![("bsd_name".into(), Json::Str("sda1".into())), ("total".into(), Json::Num(-1.0))]);
        assert_eq!(decode_usage(&bad), None);
    }

    fn usb() -> Disk {
        Disk { bsd_name: "sdb1".into(), size: 8_000_000_000, removable: true, read_only: false, model: "Flash".into(), mount_point: Some("/media/usb".into()) }
    }

    #[test]
    fn a_disk_round_trips() {
        assert_eq!(decode_disk(&encode_disk(&usb())), Some(usb()));
        let bare = Disk { mount_point: None, ..usb() };
        assert_eq!(decode_disk(&encode_disk(&bare)), Some(bare));
    }

    #[test]
    fn a_volume_round_trips_through_the_wire_text() {
        let v = Volume { disk: usb(), parent: Some("sdb".into()), fs_type: Some("vfat".into()), label: None };
        let text = crate::json::parse(&encode_volume(&v).encode()).unwrap();
        assert_eq!(decode_volume(&text), Some(v));
        assert_eq!(decode_volume(&Json::Null), None);
    }

    #[test]
    fn a_malformed_disk_is_refused() {
        assert_eq!(decode_disk(&Json::Null), None);
        assert_eq!(decode_disk(&Json::Obj(vec![("bsd_name".into(), Json::Str(String::new()))])), None);
        let mut j = encode_disk(&usb());
        if let Json::Obj(f) = &mut j {
            f.retain(|(k, _)| k != "size");
        }
        assert_eq!(decode_disk(&j), None);
    }

    #[test]
    fn events_round_trip_through_the_wire_text() {
        let events = [
            DiskEvent::Appeared(usb()),
            DiskEvent::Disappeared("sdb1".into()),
            DiskEvent::DescriptionChanged { before: Disk { mount_point: None, ..usb() }, after: usb() },
        ];
        for e in events {
            let (name, body) = encode_event(&e);
            let body = crate::json::parse(&body.encode()).unwrap();
            assert_eq!(decode_event(SERVICE, name, &body), Some(e));
        }
    }

    #[test]
    fn foreign_or_unknown_events_decode_to_nothing() {
        let (name, body) = encode_event(&DiskEvent::Appeared(usb()));
        assert_eq!(decode_event("notify_bus", name, &body), None);
        assert_eq!(decode_event(SERVICE, "other", &body), None);
        assert_eq!(decode_event(SERVICE, DISAPPEARED, &Json::Null), None);
    }
}
