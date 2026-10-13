//! The System Information tree and its rows: which categories exist, what each one lists, and how the numbers read.
//! Pure functions over `bifrost_core::sysinfo::Report`, so the rows are tested on the host without a window.

use bifrost_core::sysinfo::{Disk, Display, InstalledApp, NetInterface, PciDevice, PowerSource, Report, Summary, UsbDevice};

/// One key and value line of the detail card.
pub type Row = (String, String);

pub fn kv(key: impl Into<String>, value: impl Into<String>) -> Row {
    (key.into(), value.into())
}

/// The two halves of the sidebar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Hardware,
    Software,
}

impl Section {
    pub fn title(self) -> &'static str {
        match self {
            Section::Hardware => "Hardware",
            Section::Software => "Software",
        }
    }
}

/// A sidebar category. The order here is the order of the sidebar and the sidebar's row ids.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Overview,
    Processor,
    Memory,
    Graphics,
    Pci,
    Usb,
    Displays,
    Network,
    Power,
    Storage,
    System,
    Apps,
}

pub const ALL: [Category; 12] = [
    Category::Overview,
    Category::Processor,
    Category::Memory,
    Category::Graphics,
    Category::Pci,
    Category::Usb,
    Category::Displays,
    Category::Network,
    Category::Power,
    Category::Storage,
    Category::System,
    Category::Apps,
];

impl Category {
    pub fn index(self) -> usize {
        ALL.iter().position(|c| *c == self).unwrap_or(0)
    }

    pub fn from_index(index: usize) -> Option<Category> {
        ALL.get(index).copied()
    }

    pub fn section(self) -> Section {
        match self {
            Category::System | Category::Apps => Section::Software,
            _ => Section::Hardware,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Category::Overview => "Overview",
            Category::Processor => "Processor",
            Category::Memory => "Memory",
            Category::Graphics => "Graphics",
            Category::Pci => "PCI Devices",
            Category::Usb => "USB",
            Category::Displays => "Displays",
            Category::Network => "Network",
            Category::Power => "Power",
            Category::Storage => "Storage",
            Category::System => "Operating System",
            Category::Apps => "Installed Apps",
        }
    }

    /// The sidebar symbol (a generic symbol name, not a product mark).
    pub fn symbol(self) -> &'static str {
        match self {
            Category::Overview => "desktopcomputer",
            Category::Processor => "cpu",
            Category::Memory => "memorychip",
            Category::Graphics => "display",
            Category::Pci => "square.stack.3d.up",
            Category::Usb => "cable.connector",
            Category::Displays => "display.2",
            Category::Network => "wifi",
            Category::Power => "battery.100",
            Category::Storage => "internaldrive",
            Category::System => "gearshape",
            Category::Apps => "app.badge",
        }
    }

    /// One line under the title, in the window's detail view.
    pub fn blurb(self) -> &'static str {
        match self {
            Category::Overview => "The main facts about this computer.",
            Category::Processor => "The processor, its logical cores and its clock.",
            Category::Memory => "The memory the system sees.",
            Category::Graphics => "The graphics devices on the PCI bus.",
            Category::Pci => "Every device on the PCI bus, with the driver that serves it.",
            Category::Usb => "The devices on the USB buses.",
            Category::Displays => "The display connectors and their preferred modes.",
            Category::Network => "The network interfaces and their state.",
            Category::Power => "Batteries and power supplies, with their health.",
            Category::Storage => "The disks and the partitions on them.",
            Category::System => "The operating system and the kernel it runs.",
            Category::Apps => "The apps installed on this computer, with their newest version.",
        }
    }
}

pub fn in_section(section: Section) -> Vec<Category> {
    ALL.iter().copied().filter(|c| c.section() == section).collect()
}

/// The detail rows for a category. A category with nothing in it shows one line that says so.
pub fn rows(category: Category, r: &Report) -> Vec<Row> {
    let out: Vec<Row> = match category {
        Category::Overview => overview(r),
        Category::Processor => vec![
            kv("Name", or_dash(&r.cpu.model)),
            kv("Vendor", or_dash(&r.cpu.vendor)),
            kv("Logical processors", r.cpu.logical.to_string()),
            kv("Clock", clock_label(r.cpu.mhz)),
        ],
        Category::Memory => vec![kv("Installed", size_label(r.memory_bytes, 1 << 30, "GB"))],
        Category::Graphics => r.graphics().iter().enumerate().map(|(i, d)| kv(format!("Graphics {}", i + 1), pci_line(d))).collect(),
        Category::Pci => r.pci.iter().map(|d| kv(d.address.clone(), format!("{} · {} · {}", d.vendor_name(), d.class_name(), driver_label(&d.driver)))).collect(),
        Category::Usb => r.usb.iter().map(usb_row).collect(),
        Category::Displays => r.displays.iter().map(display_row).collect(),
        Category::Network => r.net.iter().map(net_row).collect(),
        Category::Power => r.power.iter().map(power_row).collect(),
        Category::Storage => r.disks.iter().map(disk_row).collect(),
        Category::System => vec![kv("Name", or_dash(&r.os_name)), kv("Version", or_dash(&r.os_version)), kv("Kernel", or_dash(&r.kernel))],
        Category::Apps => r.apps.iter().map(app_row).collect(),
    };
    if out.is_empty() {
        vec![kv("Status", "Nothing was detected")]
    } else {
        out
    }
}

fn overview(r: &Report) -> Vec<Row> {
    let s = Summary::from_report(r);
    vec![
        kv("Model", or_dash(&s.model)),
        kv("Vendor", or_dash(&r.firmware.vendor)),
        kv("Form factor", or_dash(&r.firmware.chassis)),
        kv("Processor", or_dash(&s.processor)),
        kv("Memory", size_label(r.memory_bytes, 1 << 30, "GB")),
        kv("Graphics", if s.graphics.is_empty() { "—".to_string() } else { s.graphics.join("; ") }),
        kv("Serial Number", or_dash(&s.serial)),
        kv("Operating System", or_dash(&format!("{} {}", s.os_name, s.os_version).trim())),
        kv("Kernel", or_dash(&s.kernel)),
        kv("Firmware", or_dash(&r.firmware.bios)),
    ]
}

fn usb_row(d: &UsbDevice) -> Row {
    let name = if d.product.is_empty() { format!("Device {:04x}:{:04x}", d.vendor_id, d.product_id) } else { d.product.clone() };
    (name, format!("{} · {} · port {}", or_dash(&d.manufacturer), usb_speed(d.speed_mbps), d.port))
}

fn display_row(d: &Display) -> Row {
    let state = match (d.connected, d.mode.is_empty()) {
        (false, _) => "Not connected".to_string(),
        (true, true) => "Connected".to_string(),
        (true, false) => format!("Connected · {}", d.mode),
    };
    (d.connector.clone(), state)
}

fn net_row(n: &NetInterface) -> Row {
    let kind = if n.wireless { "Wi-Fi" } else { "Ethernet" };
    (n.name.clone(), format!("{kind} · {} · {}", capitalize(&n.state), or_dash(&n.mac)))
}

fn power_row(p: &PowerSource) -> Row {
    let mut parts = vec![if p.kind.is_empty() { "Power".to_string() } else { p.kind.clone() }];
    if let Some(c) = p.capacity_percent {
        parts.push(format!("{c}%"));
    }
    if !p.status.is_empty() {
        parts.push(p.status.clone());
    }
    if let Some(h) = p.health_percent() {
        parts.push(format!("Health {h}%"));
    }
    if let Some(c) = p.cycles {
        parts.push(format!("{c} cycles"));
    }
    let name = if p.model.is_empty() { p.name.clone() } else { p.model.clone() };
    (name, parts.join(" · "))
}

fn disk_row(d: &Disk) -> Row {
    let mut parts = vec![d.name.clone(), size_label(d.size_bytes, 1_000_000_000, "GB")];
    parts.push(if d.removable { "Removable".to_string() } else { "Internal".to_string() });
    parts.push(if d.rotational { "HDD".to_string() } else { "SSD".to_string() });
    if !d.partitions.is_empty() {
        let n = d.partitions.len();
        parts.push(if n == 1 { "1 partition".to_string() } else { format!("{n} partitions") });
    }
    let name = if d.model.is_empty() { d.name.clone() } else { d.model.clone() };
    (name, parts.join(" · "))
}

fn app_row(a: &InstalledApp) -> Row {
    (a.id.clone(), a.version.clone())
}

fn pci_line(d: &PciDevice) -> String {
    format!("{} · {:04x} · driver {}", d.vendor_name(), d.device_id, driver_label(&d.driver))
}

fn driver_label(driver: &str) -> &str {
    if driver.is_empty() {
        "none"
    } else {
        driver
    }
}

/// The name of a USB link speed in Mb/s, as the kernel reports it.
pub fn usb_speed(mbps: u32) -> &'static str {
    match mbps {
        1 => "Low speed",
        12 => "Full speed",
        480 => "High speed",
        5000 => "SuperSpeed",
        10000 => "SuperSpeed+",
        20000 => "SuperSpeed+ 20 Gb/s",
        _ => "Speed unknown",
    }
}

/// A size in gigabytes: whole numbers from 10 up, one decimal below. `base` is 2^30 for memory, 10^9 for disks.
pub fn size_label(bytes: u64, base: u64, unit: &str) -> String {
    if bytes == 0 {
        return "—".to_string();
    }
    let g = bytes as f64 / base as f64;
    if g >= 10.0 {
        format!("{:.0} {unit}", g.round())
    } else {
        format!("{g:.1} {unit}")
    }
}

pub fn clock_label(mhz: f32) -> String {
    if mhz <= 0.0 {
        "—".to_string()
    } else if mhz >= 1000.0 {
        format!("{:.2} GHz", mhz / 1000.0)
    } else {
        format!("{:.0} MHz", mhz)
    }
}

pub fn or_dash(text: &str) -> String {
    if text.trim().is_empty() {
        "—".to_string()
    } else {
        text.trim().to_string()
    }
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => "Unknown".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_rounds_to_whole_gigabytes_from_ten_up() {
        assert_eq!(size_label(16_384_000 * 1024, 1 << 30, "GB"), "16 GB");
        assert_eq!(size_label(4 << 30, 1 << 30, "GB"), "4.0 GB");
        assert_eq!(size_label(0, 1 << 30, "GB"), "—");
    }

    #[test]
    fn disks_use_decimal_gigabytes() {
        assert_eq!(size_label(500_000_000_000, 1_000_000_000, "GB"), "500 GB");
        assert_eq!(size_label(8_000_000_000, 1_000_000_000, "GB"), "8.0 GB");
    }

    #[test]
    fn clock_switches_to_gigahertz() {
        assert_eq!(clock_label(1800.0), "1.80 GHz");
        assert_eq!(clock_label(800.0), "800 MHz");
        assert_eq!(clock_label(0.0), "—");
    }

    #[test]
    fn usb_speeds_have_names() {
        assert_eq!(usb_speed(480), "High speed");
        assert_eq!(usb_speed(1), "Low speed");
        assert_eq!(usb_speed(0), "Speed unknown");
    }

    #[test]
    fn every_category_has_a_row_even_when_empty() {
        let r = Report::default();
        for c in ALL {
            assert!(!rows(c, &r).is_empty(), "{:?}", c);
        }
        assert_eq!(rows(Category::Usb, &r), vec![kv("Status", "Nothing was detected")]);
    }

    #[test]
    fn sidebar_order_round_trips_through_the_index() {
        for (i, c) in ALL.iter().enumerate() {
            assert_eq!(Category::from_index(i), Some(*c));
            assert_eq!(c.index(), i);
        }
        assert_eq!(Category::from_index(ALL.len()), None);
        assert_eq!(in_section(Section::Software), vec![Category::System, Category::Apps]);
    }

    #[test]
    fn overview_shows_the_model_and_dashes_the_missing_facts() {
        let mut r = Report::default();
        r.firmware.product = "Laptop 13".into();
        let rows = rows(Category::Overview, &r);
        assert_eq!(rows[0], kv("Model", "Laptop 13"));
        assert_eq!(rows[2], kv("Form factor", "—"));
    }

    #[test]
    fn disk_row_names_the_model_and_its_facts() {
        let d = Disk { name: "nvme0n1".into(), model: "SSD 980".into(), size_bytes: 1_000_000_000_000, partitions: vec!["nvme0n1p1".into()], ..Default::default() };
        assert_eq!(disk_row(&d), kv("SSD 980", "nvme0n1 · 1000 GB · Internal · SSD · 1 partition"));
    }
}
