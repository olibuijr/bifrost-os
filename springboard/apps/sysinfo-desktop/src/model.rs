//! The System Information tree and its rows: which categories exist, what each one lists, and how the numbers read.
//! Pure functions over `bifrost_core::sysinfo::Report`, so the rows are tested on the host without a window.

use crate::extras::{split_locale, uptime_label, Extras};
use bifrost_core::sysinfo::{clock_label, size_label, usb_speed, Disk, Display, InstalledApp, NetInterface, PciDevice, PowerSource, Report, Summary, UsbDevice};

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
    Extensions,
    StartupItems,
    Language,
}

pub const ALL: [Category; 15] = [
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
    Category::Extensions,
    Category::StartupItems,
    Category::Language,
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
            Category::System | Category::Apps | Category::Extensions | Category::StartupItems | Category::Language => Section::Software,
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
            Category::Extensions => "Extensions",
            Category::StartupItems => "Startup Items",
            Category::Language => "Language & Region",
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
            Category::Extensions => "puzzlepiece.extension",
            Category::StartupItems => "power",
            Category::Language => "globe",
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
            Category::Extensions => "The kernel modules that are loaded now.",
            Category::StartupItems => "The apps and helpers that start when you sign in.",
            Category::Language => "The language and region this computer uses.",
        }
    }
}

pub fn in_section(section: Section) -> Vec<Category> {
    ALL.iter().copied().filter(|c| c.section() == section).collect()
}

/// The detail rows for a category. A category with nothing in it shows one line that says so.
pub fn rows(category: Category, r: &Report, x: &Extras) -> Vec<Row> {
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
        Category::System => vec![
            kv("Name", or_dash(&r.os_name)),
            kv("Version", or_dash(&r.os_version)),
            kv("Kernel", or_dash(&r.kernel)),
            kv("Computer Name", or_dash(&x.hostname)),
            kv("Time since boot", uptime_label(x.uptime_secs)),
            kv("Boot volume", or_dash(&x.boot_volume)),
            kv("Boot mode", or_dash(&x.boot_mode)),
        ],
        Category::Apps => r.apps.iter().map(app_row).collect(),
        Category::Extensions => x.modules.iter().map(|m| kv(m.name.clone(), format!("{} · used by {}", size_label(m.size_bytes, 1 << 10, "KB"), m.used_by))).collect(),
        Category::StartupItems => x.startup.iter().map(|n| kv(n.clone(), "Opens at sign-in")).collect(),
        Category::Language => language_rows(&x.locale),
    };
    if out.is_empty() {
        vec![kv("Status", "Nothing was detected")]
    } else {
        out
    }
}

fn language_rows(locale: &str) -> Vec<Row> {
    if locale.trim().is_empty() {
        return Vec::new();
    }
    let (language, region, encoding) = split_locale(locale);
    vec![kv("Locale", locale.trim()), kv("Language", or_dash(&language)), kv("Region", or_dash(&region)), kv("Encoding", or_dash(&encoding))]
}

/// Whether a row contains `query`, ignoring case. An empty query matches every row.
pub fn row_matches(row: &Row, query: &str) -> bool {
    let q = query.trim().to_lowercase();
    q.is_empty() || row.0.to_lowercase().contains(&q) || row.1.to_lowercase().contains(&q)
}

/// The next category after `from` (wrapping, `forward` or back) that has a row matching `query`; `from` itself last.
pub fn next_match(from: Category, query: &str, forward: bool, r: &Report, x: &Extras) -> Option<Category> {
    let n = ALL.len();
    (1..=n)
        .map(|step| if forward { (from.index() + step) % n } else { (from.index() + n - step % n) % n })
        .filter_map(Category::from_index)
        .find(|c| {
            let found = rows(*c, r, x);
            found != vec![kv("Status", "Nothing was detected")] && found.iter().any(|row| row_matches(row, query))
        })
}

/// The whole report as plain text: each category under its title, one `key: value` line per row. `mini` keeps only the
/// Overview, as Show Less Information does.
pub fn report_text(r: &Report, x: &Extras, mini: bool) -> String {
    let mut out = String::from("System Information Report\n");
    for c in ALL.iter().filter(|c| !mini || **c == Category::Overview) {
        out.push_str(&format!("\n{} ({})\n", c.title(), c.section().title()));
        for (k, v) in rows(*c, r, x) {
            out.push_str(&format!("  {k}: {v}\n"));
        }
    }
    out
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
    let mut line = format!("{kind} · {} · {}", capitalize(&n.state), or_dash(&n.mac));
    if n.default_route {
        line.push_str(" · Default route");
    }
    if !n.addresses.is_empty() {
        line.push_str(" · ");
        line.push_str(&n.addresses.join(", "));
    }
    (n.name.clone(), line)
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
    fn every_category_has_a_row_even_when_empty() {
        let r = Report::default();
        for c in ALL {
            assert!(!rows(c, &r, &Extras::default()).is_empty(), "{:?}", c);
        }
        assert_eq!(rows(Category::Usb, &r, &Extras::default()), vec![kv("Status", "Nothing was detected")]);
    }

    #[test]
    fn sidebar_order_round_trips_through_the_index() {
        for (i, c) in ALL.iter().enumerate() {
            assert_eq!(Category::from_index(i), Some(*c));
            assert_eq!(c.index(), i);
        }
        assert_eq!(Category::from_index(ALL.len()), None);
        assert_eq!(in_section(Section::Software), vec![Category::System, Category::Apps, Category::Extensions, Category::StartupItems, Category::Language]);
    }

    #[test]
    fn overview_shows_the_model_and_dashes_the_missing_facts() {
        let mut r = Report::default();
        r.firmware.product = "Laptop 13".into();
        let rows = rows(Category::Overview, &r, &Extras::default());
        assert_eq!(rows[0], kv("Model", "Laptop 13"));
        assert_eq!(rows[2], kv("Form factor", "—"));
    }

    #[test]
    fn disk_row_names_the_model_and_its_facts() {
        let d = Disk { name: "nvme0n1".into(), model: "SSD 980".into(), size_bytes: 1_000_000_000_000, partitions: vec!["nvme0n1p1".into()], ..Default::default() };
        assert_eq!(disk_row(&d), kv("SSD 980", "nvme0n1 · 1000 GB · Internal · SSD · 1 partition"));
    }

    fn sample_extras() -> Extras {
        Extras { hostname: "midget".into(), uptime_secs: 3_700, boot_volume: "/dev/sda2 (ext4)".into(), modules: vec![crate::extras::Module { name: "snd".into(), size_bytes: 2048, used_by: 1 }], locale: "is_IS.UTF-8".into(), ..Default::default() }
    }

    #[test]
    fn system_rows_carry_host_name_uptime_and_boot_volume() {
        let rows = rows(Category::System, &Report::default(), &sample_extras());
        assert!(rows.contains(&kv("Computer Name", "midget")));
        assert!(rows.contains(&kv("Time since boot", "1 hour, 1 minute")));
        assert!(rows.contains(&kv("Boot volume", "/dev/sda2 (ext4)")));
    }

    #[test]
    fn extensions_and_language_come_from_the_extras() {
        let x = sample_extras();
        assert_eq!(rows(Category::Extensions, &Report::default(), &x), vec![kv("snd", "2.0 KB · used by 1")]);
        assert!(rows(Category::Language, &Report::default(), &x).contains(&kv("Region", "IS")));
        assert_eq!(rows(Category::StartupItems, &Report::default(), &x), vec![kv("Status", "Nothing was detected")]);
    }

    #[test]
    fn find_matches_keys_and_values_ignoring_case() {
        assert!(row_matches(&kv("Computer Name", "midget"), "MIDG"));
        assert!(row_matches(&kv("a", "b"), "  "));
        assert!(!row_matches(&kv("a", "b"), "zzz"));
    }

    #[test]
    fn find_next_wraps_and_skips_empty_categories() {
        let r = Report::default();
        let x = sample_extras();
        assert_eq!(next_match(Category::Overview, "midget", true, &r, &x), Some(Category::System));
        assert_eq!(next_match(Category::System, "midget", true, &r, &x), Some(Category::System), "only match wraps back to itself");
        assert_eq!(next_match(Category::Language, "snd", true, &r, &x), Some(Category::Extensions));
        assert_eq!(next_match(Category::Overview, "qqqq", true, &r, &x), None);
        assert_eq!(next_match(Category::Extensions, "midget", false, &r, &x), Some(Category::System));
    }

    #[test]
    fn report_text_lists_every_category_or_only_the_overview() {
        let r = Report::default();
        let x = sample_extras();
        let full = report_text(&r, &x, false);
        assert!(full.contains("Extensions (Software)"));
        assert!(full.contains("  Computer Name: midget"));
        let mini = report_text(&r, &x, true);
        assert!(mini.contains("Overview (Hardware)"));
        assert!(!mini.contains("Extensions"));
    }
}
