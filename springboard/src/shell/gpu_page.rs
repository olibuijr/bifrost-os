//! Display & Brightness > Graphics: what the graphics adapters and displays are, read from sysfs and `pci.ids`.
//! Vendor neutral: Intel (i915/xe), AMD (amdgpu) and NVIDIA (nouveau/nvidia) each fill what their driver exposes.
//! Rows a driver does not expose are left out rather than shown empty.

use super::list::{Group, Item, Spec};
use super::pages::PageId;
use super::storage::human;
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Output {
    /// "DP-1", "HDMI-A-1"
    pub name: String,
    /// the preferred mode, e.g. "2560x1440"
    pub mode: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Gpu {
    pub name: String,
    pub vendor: &'static str,
    pub driver: String,
    /// current and maximum clock in MHz (Intel `gt_*_freq_mhz`)
    pub clock_mhz: Option<(u32, u32)>,
    /// dedicated video memory (AMD `mem_info_vram_total`)
    pub vram: Option<u64>,
    /// AMD `gpu_busy_percent`
    pub busy: Option<u32>,
    pub outputs: Vec<Output>,
}

pub fn vendor_name(id: u32) -> &'static str {
    match id {
        0x8086 => "Intel",
        0x1002 | 0x1022 => "AMD",
        0x10de => "NVIDIA",
        0x1af4 | 0x1234 => "Virtual",
        _ => "Unknown",
    }
}

/// The device name for PCI ids from the text of `pci.ids`, if listed.
pub fn pci_name(ids: &str, vendor: u32, device: u32) -> Option<String> {
    let (v, d) = (format!("{vendor:04x}"), format!("{device:04x}"));
    let mut in_vendor = false;
    for line in ids.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        if !line.starts_with('\t') {
            in_vendor = line.starts_with(&v) && line[v.len()..].starts_with("  ");
        } else if in_vendor && !line.starts_with("\t\t") {
            let t = &line[1..];
            if t.starts_with(&d) && t[d.len()..].starts_with("  ") {
                // "HD Graphics 530" or "Skylake GT2 [HD Graphics 530]": prefer the bracketed marketing name
                let name = t[d.len() + 2..].trim();
                return Some(match (name.find('['), name.rfind(']')) {
                    (Some(a), Some(b)) if b > a => name[a + 1..b].to_string(),
                    _ => name.to_string(),
                });
            }
        }
    }
    None
}

fn read(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

fn hex(path: &Path) -> Option<u32> {
    u32::from_str_radix(read(path)?.trim_start_matches("0x"), 16).ok()
}

/// Reads every graphics adapter under a DRM class directory (`/sys/class/drm`); `pci_ids` is the `pci.ids` text.
pub fn read_from(drm: &Path, pci_ids: &str) -> Vec<Gpu> {
    let mut cards: Vec<_> = std::fs::read_dir(drm).into_iter().flatten().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.starts_with("card") && n[4..].bytes().all(|b| b.is_ascii_digit())).collect();
    cards.sort();
    cards
        .iter()
        .filter_map(|card| {
            let dev = drm.join(card).join("device");
            let (vendor, device) = (hex(&dev.join("vendor"))?, hex(&dev.join("device"))?);
            let driver = std::fs::read_link(dev.join("driver")).ok().and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).unwrap_or_default();
            let mhz = |f: &str| read(&drm.join(card).join(f)).and_then(|s| s.parse::<u32>().ok());
            let clock_mhz = mhz("gt_cur_freq_mhz").zip(mhz("gt_max_freq_mhz"));
            let vram = read(&dev.join("mem_info_vram_total")).and_then(|s| s.parse().ok());
            let busy = read(&dev.join("gpu_busy_percent")).and_then(|s| s.parse().ok());
            let prefix = format!("{card}-");
            let mut outputs: Vec<Output> = std::fs::read_dir(drm)
                .into_iter()
                .flatten()
                .flatten()
                .filter_map(|e| {
                    let n = e.file_name().to_string_lossy().into_owned();
                    let name = n.strip_prefix(&prefix)?.to_string();
                    (read(&e.path().join("status"))? == "connected").then(|| Output { name, mode: read(&e.path().join("modes")).and_then(|m| m.lines().next().map(str::to_string)).unwrap_or_default() })
                })
                .collect();
            outputs.sort_by(|a, b| a.name.cmp(&b.name));
            let vendor_label = vendor_name(vendor);
            let name = pci_name(pci_ids, vendor, device).map(|n| if n.starts_with(vendor_label) { n } else { format!("{vendor_label} {n}") }).unwrap_or_else(|| format!("{vendor_label} Graphics ({device:04x})"));
            Some(Gpu { name, vendor: vendor_label, driver, clock_mhz, vram, busy, outputs })
        })
        .collect()
}

/// The adapters of this machine.
pub fn read_system() -> Vec<Gpu> {
    let ids = std::fs::read_to_string("/usr/share/hwdata/pci.ids").or_else(|_| std::fs::read_to_string("/usr/share/misc/pci.ids")).unwrap_or_default();
    read_from(Path::new("/sys/class/drm"), &ids)
}

/// "2560x1440" as "2560 × 1440".
fn mode_label(mode: &str) -> String {
    mode.replace('x', " \u{d7} ")
}

/// The page for the adapters read so far; `None` while the read runs (a placeholder row, not "Not detected").
pub fn spec(gpus: Option<&[Gpu]>) -> Spec {
    let Some(gpus) = gpus else {
        return Spec { id: PageId::Graphics, title: "Graphics".into(), large: false, groups: vec![Group::new(vec![Item::new("Graphics").value("Loading…").tint(super::list::Tint::Dim)])] };
    };
    let mut groups = Vec::new();
    for g in gpus {
        let mut rows = vec![Item::new("Graphics").value(g.name.clone()), Item::new("Driver").value(if g.driver.is_empty() { "None".into() } else { g.driver.clone() })];
        if let Some((cur, max)) = g.clock_mhz {
            rows.push(Item::new("Clock Speed").value(format!("{cur} of {max} MHz")));
        }
        if let Some(b) = g.busy {
            rows.push(Item::new("Utilisation").value(format!("{b}%")));
        }
        if let Some(v) = g.vram {
            rows.push(Item::new("Video Memory").value(human(v)));
        }
        let header = if gpus.len() > 1 { g.vendor.to_uppercase() } else { "GRAPHICS PROCESSOR".into() };
        groups.push(Group::new(rows).header(&header));
        if !g.outputs.is_empty() {
            groups.push(Group::new(g.outputs.iter().map(|o| Item::new(o.name.clone()).value(mode_label(&o.mode))).collect()).header("DISPLAYS"));
        }
    }
    if groups.is_empty() {
        groups.push(Group::new(vec![Item::new("Graphics").value("Not detected")]));
    }
    Spec { id: PageId::Graphics, title: "Graphics".into(), large: false, groups }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    const IDS: &str = "# comment\n8086  Intel Corporation\n\t1912  Skylake GT2 [HD Graphics 530]\n\t\t1458 d000  Subsystem\n\t1916  HD Graphics 520\n10de  NVIDIA Corporation\n\t2204  GA102 [GeForce RTX 3090]\n";

    fn fake() -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("bifrost-drm-{}-{:?}", std::process::id(), std::thread::current().id()));
        let _ = std::fs::remove_dir_all(&root);
        let dev = root.join("card1/device");
        std::fs::create_dir_all(&dev).unwrap();
        std::fs::write(dev.join("vendor"), "0x8086\n").unwrap();
        std::fs::write(dev.join("device"), "0x1912\n").unwrap();
        std::fs::create_dir_all(root.join("drivers/i915")).unwrap();
        symlink(root.join("drivers/i915"), dev.join("driver")).unwrap();
        std::fs::write(root.join("card1/gt_cur_freq_mhz"), "350\n").unwrap();
        std::fs::write(root.join("card1/gt_max_freq_mhz"), "1050\n").unwrap();
        for (n, status, modes) in [("card1-DP-1", "connected", "2560x1440\n1920x1080\n"), ("card1-DP-2", "disconnected", ""), ("card1-HDMI-A-1", "connected", "1920x1080\n")] {
            std::fs::create_dir_all(root.join(n)).unwrap();
            std::fs::write(root.join(n).join("status"), format!("{status}\n")).unwrap();
            std::fs::write(root.join(n).join("modes"), modes).unwrap();
        }
        root
    }

    #[test]
    fn pci_names_prefer_the_marketing_name() {
        assert_eq!(pci_name(IDS, 0x8086, 0x1912).as_deref(), Some("HD Graphics 530"));
        assert_eq!(pci_name(IDS, 0x8086, 0x1916).as_deref(), Some("HD Graphics 520"));
        assert_eq!(pci_name(IDS, 0x10de, 0x2204).as_deref(), Some("GeForce RTX 3090"));
        assert_eq!(pci_name(IDS, 0x8086, 0x9999), None);
        assert_eq!(pci_name(IDS, 0x1002, 0x1912), None, "another vendor's id is not matched");
    }

    #[test]
    fn an_intel_card_reads_name_driver_clock_and_connected_displays_only() {
        let root = fake();
        let g = read_from(&root, IDS);
        assert_eq!(g.len(), 1);
        let g = &g[0];
        assert_eq!((g.name.as_str(), g.vendor, g.driver.as_str()), ("Intel HD Graphics 530", "Intel", "i915"));
        assert_eq!(g.clock_mhz, Some((350, 1050)));
        assert_eq!(g.vram, None);
        assert_eq!(g.outputs, vec![Output { name: "DP-1".into(), mode: "2560x1440".into() }, Output { name: "HDMI-A-1".into(), mode: "1920x1080".into() }]);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn an_unknown_device_still_gets_a_readable_name() {
        let root = fake();
        let g = read_from(&root, "");
        assert_eq!(g[0].name, "Intel Graphics (1912)");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_page_lists_rows_the_driver_exposes_and_a_displays_group() {
        let root = fake();
        let s = spec(Some(&read_from(&root, IDS)));
        let labels: Vec<_> = s.groups.iter().flat_map(|g| g.items.iter().map(|i| (i.label.clone(), i.value.clone()))).collect();
        assert!(labels.contains(&("Clock Speed".into(), "350 of 1050 MHz".into())));
        assert!(labels.contains(&("DP-1".into(), "2560 \u{d7} 1440".into())));
        assert!(!labels.iter().any(|(l, _)| l == "Video Memory"), "no VRAM row when the driver has none");
        assert_eq!(spec(Some(&[])).groups.len(), 1);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_page_shows_a_placeholder_until_the_read_lands() {
        let s = spec(None);
        assert_eq!(s.groups.len(), 1);
        assert_eq!(s.groups[0].items[0].value, "Loading…");
    }
}
