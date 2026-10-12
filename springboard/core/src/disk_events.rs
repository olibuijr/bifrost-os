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

/// The description of a device as name and value pairs in a fixed order. `mount_point` is left out while the device
/// is not mounted, so a missing key always means "no such property" rather than an empty value.
pub fn description(disk: &Disk) -> Vec<(&'static str, String)> {
    let mut out = vec![
        ("bsd_name", disk.bsd_name.clone()),
        ("size", disk.size.to_string()),
        ("removable", disk.removable.to_string()),
        ("writable", (!disk.read_only).to_string()),
        ("model", disk.model.clone()),
    ];
    if let Some(mount) = &disk.mount_point {
        out.push(("mount_point", mount.clone()));
    }
    out
}

/// The device with this name (for example `sdb` or `nvme0n1p1`), when the snapshot holds it.
pub fn lookup_bsd_name<'a>(disks: &'a [Disk], name: &str) -> Option<&'a Disk> {
    disks.iter().find(|d| d.bsd_name == name)
}

/// The mounted device whose mount point is the deepest directory that contains `path`. Containment is by path
/// component, so `/media/stick` does not contain `/media/sticky`. A root mount (`/`) contains every absolute path.
pub fn lookup_volume_path<'a>(disks: &'a [Disk], path: &str) -> Option<&'a Disk> {
    disks
        .iter()
        .filter_map(|d| {
            let mount = d.mount_point.as_deref()?.trim_end_matches('/');
            contains(mount, path).then_some((mount.len(), d))
        })
        .max_by_key(|(depth, _)| *depth)
        .map(|(_, d)| d)
}

/// Whether the directory `dir` (already without a trailing slash, empty for the root) contains `path`.
fn contains(dir: &str, path: &str) -> bool {
    if dir.is_empty() {
        return path.starts_with('/');
    }
    path == dir || path.strip_prefix(dir).is_some_and(|rest| rest.starts_with('/'))
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
    fn description_lists_fixed_keys_and_omits_an_unmounted_mount_point() {
        let stick = disk("sdb", None);
        let keys: Vec<&str> = description(&stick).iter().map(|(k, _)| *k).collect();
        assert_eq!(keys, vec!["bsd_name", "size", "removable", "writable", "model"]);
        let values = description(&disk("sdb", Some("/media/stick")));
        assert_eq!(values.last(), Some(&("mount_point", "/media/stick".to_string())));
    }

    #[test]
    fn description_reports_writable_as_the_negation_of_read_only() {
        let mut ro = disk("sr0", None);
        ro.read_only = true;
        let writable = description(&ro).into_iter().find(|(k, _)| *k == "writable").unwrap();
        assert_eq!(writable.1, "false");
        assert_eq!(description(&disk("sdb", None))[3].1, "true");
    }

    #[test]
    fn lookup_by_bsd_name_finds_the_exact_name_only() {
        let snap = vec![disk("sda", Some("/")), disk("sdb1", None)];
        assert_eq!(lookup_bsd_name(&snap, "sdb1").map(|d| d.bsd_name.as_str()), Some("sdb1"));
        assert!(lookup_bsd_name(&snap, "sdb").is_none());
    }

    #[test]
    fn lookup_by_volume_path_takes_the_deepest_containing_mount() {
        let snap = vec![
            disk("sda1", Some("/")),
            disk("sdb1", Some("/media/stick")),
            disk("sdc1", Some("/media/stick/photos")),
        ];
        let found = lookup_volume_path(&snap, "/media/stick/photos/2026/a.jpg");
        assert_eq!(found.map(|d| d.bsd_name.as_str()), Some("sdc1"));
        let found = lookup_volume_path(&snap, "/media/stick/notes.txt");
        assert_eq!(found.map(|d| d.bsd_name.as_str()), Some("sdb1"));
    }

    #[test]
    fn lookup_by_volume_path_matches_whole_components_only() {
        let snap = vec![disk("sdb1", Some("/media/stick"))];
        assert!(lookup_volume_path(&snap, "/media/sticky/file").is_none());
        assert_eq!(lookup_volume_path(&snap, "/media/stick/").map(|d| d.bsd_name.as_str()), Some("sdb1"));
    }

    #[test]
    fn a_root_mount_contains_every_absolute_path_and_relative_paths_match_nothing() {
        let snap = vec![disk("sda1", Some("/"))];
        assert_eq!(lookup_volume_path(&snap, "/home/user").map(|d| d.bsd_name.as_str()), Some("sda1"));
        assert!(lookup_volume_path(&snap, "home/user").is_none());
    }
}
