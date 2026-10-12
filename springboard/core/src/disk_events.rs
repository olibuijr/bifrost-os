//! Block-device arrival, removal and description changes, computed as the difference between two snapshots, and an
//! approval chain for mount, unmount and eject requests. Callers supply the snapshots and make the requests; this
//! module reads no hardware and runs no timer.

use std::collections::BTreeMap;

/// One block device as the caller last saw it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Disk {
    pub bsd_name: String,
    pub size: u64,
    pub removable: bool,
    pub read_only: bool,
    pub model: String,
    pub mount_point: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiskEvent {
    Appeared(Disk),
    Disappeared(String),
    DescriptionChanged { before: Disk, after: Disk },
}

/// The events that turn `prev` into `next`. The order is fixed: removals, then arrivals, then changes, each sorted
/// by device name. A device counts as changed when any field differs, including its mount point.
pub fn diff(prev: &[Disk], next: &[Disk]) -> Vec<DiskEvent> {
    let before: BTreeMap<&str, &Disk> = prev.iter().map(|d| (d.bsd_name.as_str(), d)).collect();
    let after: BTreeMap<&str, &Disk> = next.iter().map(|d| (d.bsd_name.as_str(), d)).collect();
    let mut out = Vec::new();
    for &name in before.keys() {
        if !after.contains_key(name) {
            out.push(DiskEvent::Disappeared(name.to_string()));
        }
    }
    for (&name, &disk) in &after {
        match before.get(name) {
            None => out.push(DiskEvent::Appeared(disk.clone())),
            Some(&old) if old != disk => {
                out.push(DiskEvent::DescriptionChanged { before: old.clone(), after: disk.clone() })
            }
            Some(_) => {}
        }
    }
    out
}

/// The disk with this BSD device name, if the snapshot has one.
pub fn find_by_bsd_name<'a>(disks: &'a [Disk], bsd_name: &str) -> Option<&'a Disk> {
    disks.iter().find(|d| d.bsd_name == bsd_name)
}

/// The disk whose mounted volume sits at `path`. A trailing slash is ignored, except that the root stays `/`. An
/// empty path matches nothing, and a disk that is not mounted never matches.
pub fn find_by_volume_path<'a>(disks: &'a [Disk], path: &str) -> Option<&'a Disk> {
    let wanted = normalize_path(path)?;
    disks.iter().find(|d| d.mount_point.as_deref().and_then(normalize_path) == Some(wanted))
}

fn normalize_path(path: &str) -> Option<&str> {
    if path.is_empty() {
        return None;
    }
    if path == "/" {
        return Some("/");
    }
    Some(path.trim_end_matches('/'))
}

/// The facts about a disk as name and value pairs. Values are text: the size in bytes, `true` or `false` for the
/// flags, the model with spaces trimmed. A blank model and an unmounted disk leave their entries out.
pub fn description(disk: &Disk) -> BTreeMap<&'static str, String> {
    let mut out = BTreeMap::new();
    out.insert("bsd_name", disk.bsd_name.clone());
    out.insert("media_size", disk.size.to_string());
    out.insert("media_removable", disk.removable.to_string());
    out.insert("media_writable", (!disk.read_only).to_string());
    let model = disk.model.trim();
    if !model.is_empty() {
        out.insert("device_model", model.to_string());
    }
    if let Some(path) = &disk.mount_point {
        out.insert("volume_path", path.clone());
    }
    out
}

/// The requests that can be vetoed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    Mount,
    Unmount,
    Eject,
}

/// Why a request was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dissent {
    pub reason: String,
}

type Gate = Box<dyn Fn(Operation, &Disk) -> Result<(), String>>;

/// Gates run in registration order. Each one allows a request with `Ok` or refuses it with `Err(reason)`.
#[derive(Default)]
pub struct Approvals {
    gates: Vec<Gate>,
}

impl Approvals {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a gate. It sees every request and decides only the ones it cares about.
    pub fn register(&mut self, gate: impl Fn(Operation, &Disk) -> Result<(), String> + 'static) {
        self.gates.push(Box::new(gate));
    }

    /// The first refusal wins. With no gates, or none that refuses, the request is allowed.
    pub fn approve(&self, op: Operation, disk: &Disk) -> Result<(), Dissent> {
        for gate in &self.gates {
            gate(op, disk).map_err(|reason| Dissent { reason })?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn disk(name: &str, mount: Option<&str>) -> Disk {
        Disk {
            bsd_name: name.to_string(),
            size: 1 << 30,
            removable: true,
            read_only: false,
            model: "Stick".to_string(),
            mount_point: mount.map(str::to_string),
        }
    }

    #[test]
    fn identical_snapshots_produce_no_events() {
        let snap = vec![disk("sda", Some("/"))];
        assert!(diff(&snap, &snap).is_empty());
    }

    #[test]
    fn arrival_and_removal_are_reported() {
        let events = diff(&[disk("sda", None)], &[disk("sdb", None)]);
        assert_eq!(events, vec![DiskEvent::Disappeared("sda".into()), DiskEvent::Appeared(disk("sdb", None))]);
    }

    #[test]
    fn a_mount_point_change_is_a_description_change() {
        let events = diff(&[disk("sdb", None)], &[disk("sdb", Some("/media/stick"))]);
        assert_eq!(
            events,
            vec![DiskEvent::DescriptionChanged {
                before: disk("sdb", None),
                after: disk("sdb", Some("/media/stick")),
            }]
        );
    }

    #[test]
    fn arrivals_are_sorted_by_name() {
        let events = diff(&[], &[disk("sdc", None), disk("sdb", None)]);
        let names: Vec<&str> = events
            .iter()
            .map(|e| match e {
                DiskEvent::Appeared(d) => d.bsd_name.as_str(),
                _ => "?",
            })
            .collect();
        assert_eq!(names, vec!["sdb", "sdc"]);
    }

    #[test]
    fn approve_allows_when_no_gate_refuses() {
        let approvals = Approvals::new();
        assert!(approvals.approve(Operation::Mount, &disk("sdb", None)).is_ok());
    }

    #[test]
    fn a_gate_refuses_only_the_operation_it_names() {
        let mut approvals = Approvals::new();
        approvals.register(|op, _| match op {
            Operation::Mount => Err("mounts are off".to_string()),
            _ => Ok(()),
        });
        let stick = disk("sdb", None);
        assert_eq!(
            approvals.approve(Operation::Mount, &stick),
            Err(Dissent { reason: "mounts are off".into() })
        );
        assert!(approvals.approve(Operation::Unmount, &stick).is_ok());
        assert!(approvals.approve(Operation::Eject, &stick).is_ok());
    }

    #[test]
    fn the_first_refusal_wins() {
        let mut approvals = Approvals::new();
        approvals.register(|_, d| if d.removable { Err("removable".to_string()) } else { Ok(()) });
        approvals.register(|_, _| Err("second".to_string()));
        let err = approvals.approve(Operation::Eject, &disk("sdb", None)).unwrap_err();
        assert_eq!(err.reason, "removable");
    }

    #[test]
    fn a_disk_is_found_by_its_bsd_name() {
        let snap = vec![disk("sdb", None), disk("sdc", Some("/media/card"))];
        assert_eq!(find_by_bsd_name(&snap, "sdc").map(|d| d.size), Some(1 << 30));
        assert!(find_by_bsd_name(&snap, "sdd").is_none());
    }

    #[test]
    fn a_volume_path_matches_its_mount_point_with_or_without_a_trailing_slash() {
        let snap = vec![disk("sdb", None), disk("sdc", Some("/media/stick"))];
        assert_eq!(find_by_volume_path(&snap, "/media/stick").map(|d| d.bsd_name.as_str()), Some("sdc"));
        assert_eq!(find_by_volume_path(&snap, "/media/stick/").map(|d| d.bsd_name.as_str()), Some("sdc"));
        assert!(find_by_volume_path(&snap, "/media").is_none());
        assert!(find_by_volume_path(&snap, "").is_none());
    }

    #[test]
    fn the_root_volume_path_matches_only_a_root_mount() {
        let root = vec![disk("sda", Some("/"))];
        assert_eq!(find_by_volume_path(&root, "/").map(|d| d.bsd_name.as_str()), Some("sda"));
        let unmounted = vec![disk("sdb", None)];
        assert!(find_by_volume_path(&unmounted, "/").is_none());
    }

    #[test]
    fn a_description_lists_size_flags_model_and_volume_path() {
        let mut stick = disk("sdb", Some("/media/stick"));
        stick.model = "  Stick  ".to_string();
        stick.read_only = true;
        let d = description(&stick);
        assert_eq!(d.get("bsd_name").map(String::as_str), Some("sdb"));
        assert_eq!(d.get("media_size").map(String::as_str), Some("1073741824"));
        assert_eq!(d.get("media_removable").map(String::as_str), Some("true"));
        assert_eq!(d.get("media_writable").map(String::as_str), Some("false"));
        assert_eq!(d.get("device_model").map(String::as_str), Some("Stick"));
        assert_eq!(d.get("volume_path").map(String::as_str), Some("/media/stick"));
        assert_eq!(d.len(), 6);
    }

    #[test]
    fn a_blank_model_and_an_unmounted_disk_leave_their_entries_out() {
        let mut bare = disk("sdb", None);
        bare.model = "   ".to_string();
        let d = description(&bare);
        assert!(!d.contains_key("device_model"));
        assert!(!d.contains_key("volume_path"));
        assert_eq!(d.len(), 4);
    }
}
