//! The System Information tree and its content: which categories exist, what each one lists, and how the numbers read.
//! Pure functions over `bifrost_core::sysinfo::Report` and the extra detail in `Extras`, so the rows are tested on
//! the host without a window.
//!
//! A category shows either a card of key and value rows (the processor, the operating system) or a list of items,
//! one per device, disk or app, each with a detail card of its own. `Content` holds both shapes.

use crate::extras::{split_locale, uptime_label, Extras, ScreenFacts};
use crate::probe::{self, day_label, usb_parent, usb_position, usb_tree, NetDetail};
use bifrost_core::sysinfo::{clock_label, size_label, usb_speed, Disk, Display, InstalledApp, NetInterface, PciDevice, PowerSource, Report, Summary, UsbDevice};

/// One key and value line of a detail card.
pub type Row = (String, String);

pub fn kv(key: impl Into<String>, value: impl Into<String>) -> Row {
    (key.into(), value.into())
}

/// The three parts of the sidebar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Hardware,
    Network,
    Software,
}

pub const SECTIONS: [Section; 3] = [Section::Hardware, Section::Network, Section::Software];

impl Section {
    pub fn title(self) -> &'static str {
        match self {
            Section::Hardware => "Hardware",
            Section::Network => "Network",
            Section::Software => "Software",
        }
    }

    pub fn index(self) -> usize {
        SECTIONS.iter().position(|s| *s == self).unwrap_or(0)
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
    Audio,
    Bluetooth,
    Camera,
    CardReader,
    Controller,
    Printers,
    Thunderbolt,
    Power,
    Storage,
    Nvme,
    Sata,
    Sas,
    FibreChannel,
    DiscBurning,
    Diagnostics,
    Wifi,
    Ethernet,
    Wwan,
    Firewall,
    Volumes,
    System,
    Apps,
    Extensions,
    StartupItems,
    Fonts,
    Logs,
    Language,
}

pub const ALL: [Category; 34] = [
    Category::Overview,
    Category::Processor,
    Category::Memory,
    Category::Graphics,
    Category::Pci,
    Category::Usb,
    Category::Displays,
    Category::Audio,
    Category::Bluetooth,
    Category::Camera,
    Category::CardReader,
    Category::Controller,
    Category::Printers,
    Category::Thunderbolt,
    Category::Power,
    Category::Storage,
    Category::Nvme,
    Category::Sata,
    Category::Sas,
    Category::FibreChannel,
    Category::DiscBurning,
    Category::Diagnostics,
    Category::Wifi,
    Category::Ethernet,
    Category::Wwan,
    Category::Firewall,
    Category::Volumes,
    Category::System,
    Category::Apps,
    Category::Extensions,
    Category::StartupItems,
    Category::Fonts,
    Category::Logs,
    Category::Language,
];

impl Category {
    pub fn index(self) -> usize {
        ALL.iter().position(|c| *c == self).unwrap_or(0)
    }

    pub fn from_index(index: usize) -> Option<Category> {
        ALL.get(index).copied()
    }

    /// The category with this sidebar title, for reading a saved report.
    pub fn from_title(title: &str) -> Option<Category> {
        ALL.iter().copied().find(|c| c.title() == title)
    }

    pub fn section(self) -> Section {
        match self {
            Category::Wifi | Category::Ethernet | Category::Wwan | Category::Firewall | Category::Volumes => Section::Network,
            Category::System | Category::Apps | Category::Extensions | Category::StartupItems | Category::Fonts | Category::Logs | Category::Language => Section::Software,
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
            Category::Audio => "Audio",
            Category::Bluetooth => "Bluetooth",
            Category::Camera => "Camera",
            Category::CardReader => "Card Reader",
            Category::Controller => "Controller",
            Category::Printers => "Printers",
            Category::Thunderbolt => "Thunderbolt",
            Category::Power => "Power",
            Category::Storage => "Storage",
            Category::Nvme => "NVMe",
            Category::Sata => "SATA",
            Category::Sas => "SAS",
            Category::FibreChannel => "Fibre Channel",
            Category::DiscBurning => "Disc Burning",
            Category::Diagnostics => "Diagnostics",
            Category::Wifi => "Wi-Fi",
            Category::Ethernet => "Ethernet",
            Category::Wwan => "WWAN",
            Category::Firewall => "Firewall",
            Category::Volumes => "Volumes",
            Category::System => "Operating System",
            Category::Apps => "Installed Apps",
            Category::Extensions => "Extensions",
            Category::StartupItems => "Startup Items",
            Category::Fonts => "Fonts",
            Category::Logs => "Logs",
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
            Category::Audio => "speaker.wave.2",
            Category::Bluetooth => "antenna.radiowaves.left.and.right",
            Category::Camera => "camera",
            Category::CardReader => "sdcard",
            Category::Controller => "gearshape.2",
            Category::Printers => "printer",
            Category::Thunderbolt => "bolt",
            Category::Power => "battery.100",
            Category::Storage => "internaldrive",
            Category::Nvme => "internaldrive",
            Category::Sata => "internaldrive",
            Category::Sas => "server.rack",
            Category::FibreChannel => "point.3.connected.trianglepath.dotted",
            Category::DiscBurning => "opticaldisc",
            Category::Diagnostics => "stethoscope",
            Category::Wifi => "wifi",
            Category::Ethernet => "cable.connector",
            Category::Wwan => "antenna.radiowaves.left.and.right",
            Category::Firewall => "lock.shield",
            Category::Volumes => "externaldrive",
            Category::System => "gearshape",
            Category::Apps => "app.badge",
            Category::Extensions => "puzzlepiece.extension",
            Category::StartupItems => "power",
            Category::Fonts => "textformat",
            Category::Logs => "doc.text",
            Category::Language => "globe",
        }
    }

    /// One line under the title, in the window's detail view.
    pub fn blurb(self) -> &'static str {
        match self {
            Category::Overview => "The main facts about this computer.",
            Category::Processor => "The processor, its cores, caches and clock.",
            Category::Memory => "The memory the system sees and the modules in its slots.",
            Category::Graphics => "The graphics devices on the PCI bus.",
            Category::Pci => "Every device on the PCI bus, with the driver that serves it.",
            Category::Usb => "The USB buses and the devices on them, hub by hub.",
            Category::Displays => "The display connectors and their preferred modes.",
            Category::Audio => "The sound cards the system sees.",
            Category::Bluetooth => "The Bluetooth controllers and their state.",
            Category::Camera => "The video capture devices.",
            Category::CardReader => "The memory card slots and the card in each.",
            Category::Controller => "The storage host adapters that are not Fibre Channel or SAS.",
            Category::Printers => "The printers set up on this computer and those on USB.",
            Category::Thunderbolt => "The devices on the Thunderbolt bus.",
            Category::Power => "Batteries and power supplies, with their health.",
            Category::Storage => "The disks, their partitions and where they are mounted.",
            Category::Nvme => "The NVMe controllers.",
            Category::Sata => "The SATA ports and the speed each link negotiated.",
            Category::Sas => "The SAS links, their rates and the devices on them.",
            Category::FibreChannel => "The Fibre Channel ports and their links.",
            Category::DiscBurning => "The optical drives and whether a disc is in them.",
            Category::Diagnostics => "Crash reports and core dumps saved on this computer.",
            Category::Wifi => "The wireless interfaces, their addresses and signal.",
            Category::Ethernet => "The wired and virtual interfaces, their addresses and speed.",
            Category::Wwan => "The cellular modem interfaces.",
            Category::Firewall => "The packet filter the kernel has loaded.",
            Category::Volumes => "The mounted volumes and the space left on them.",
            Category::System => "The operating system and the kernel it runs.",
            Category::Apps => "The apps installed on this computer, with their newest version.",
            Category::Extensions => "The kernel modules that are loaded now.",
            Category::StartupItems => "The apps and helpers that start when you sign in.",
            Category::Fonts => "The font families the desktop can draw.",
            Category::Logs => "The log files and how much space each takes.",
            Category::Language => "The language and region this computer uses.",
        }
    }
}

pub fn in_section(section: Section) -> Vec<Category> {
    ALL.iter().copied().filter(|c| c.section() == section).collect()
}

// ---------------------------------------------------------------- content

/// One entry of a list: its line in the list and the card of facts under it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Item {
    pub title: String,
    pub summary: String,
    pub detail: Vec<Row>,
}

/// What a category shows. A card (`list` false) has one item whose detail is the card; a list has one item per entry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Content {
    pub list: bool,
    /// Nothing was found for this category; the single row says so.
    pub empty: bool,
    pub items: Vec<Item>,
}

const NOTHING: &str = "Nothing was detected";

impl Content {
    /// A card of rows; no rows reads as an empty category.
    pub fn card(rows: Vec<Row>) -> Content {
        if rows.is_empty() {
            return Content { list: false, empty: true, items: vec![Item { detail: vec![kv("Status", NOTHING)], ..Item::default() }] };
        }
        Content { list: false, empty: false, items: vec![Item { detail: rows, ..Item::default() }] }
    }

    /// A list of items; no items reads as an empty category.
    pub fn list(items: Vec<Item>) -> Content {
        if items.is_empty() {
            return Content::card(Vec::new());
        }
        Content { list: true, empty: false, items }
    }

    /// A card with one row that is a message, not a fact (a find with no hit, an absent part of a saved report).
    pub fn message(key: &str, value: &str) -> Content {
        Content { list: false, empty: false, items: vec![Item { detail: vec![kv(key, value)], ..Item::default() }] }
    }

    /// The rows of the card, or the item lines of the list.
    pub fn summary_rows(&self) -> Vec<Row> {
        if self.list {
            self.items.iter().map(|i| kv(i.title.clone(), i.summary.clone())).collect()
        } else {
            self.items.first().map(|i| i.detail.clone()).unwrap_or_default()
        }
    }

    /// The facts of item `index` (the card for a card, which has one item).
    pub fn detail_of(&self, index: usize) -> &[Row] {
        let at = if self.list { index } else { 0 };
        match self.items.get(at) {
            Some(i) => &i.detail[..],
            None => &[],
        }
    }

    /// Whether anything in the content contains `query`, ignoring case. An empty category matches nothing.
    pub fn matches(&self, query: &str) -> bool {
        !self.empty && self.items.iter().any(|i| item_matches(i, query))
    }

    /// The content that survives a find: the items (or card rows) that contain `query`. A find with no hit leaves one
    /// line that says so. An empty query keeps everything.
    pub fn filtered(&self, query: &str) -> Content {
        if query.trim().is_empty() || self.empty {
            return self.clone();
        }
        let kept: Vec<Item> = if self.list {
            self.items.iter().filter(|i| item_matches(i, query)).cloned().collect()
        } else {
            match self.items.first() {
                Some(card) => {
                    let rows: Vec<Row> = card.detail.iter().filter(|r| row_matches(r, query)).cloned().collect();
                    if rows.is_empty() {
                        Vec::new()
                    } else {
                        vec![Item { detail: rows, ..card.clone() }]
                    }
                }
                None => Vec::new(),
            }
        };
        if kept.is_empty() {
            return Content::message("Find", "No matches in this category");
        }
        Content { list: self.list, empty: false, items: kept }
    }
}

fn item_matches(item: &Item, query: &str) -> bool {
    text_matches(&item.title, query) || text_matches(&item.summary, query) || item.detail.iter().any(|r| row_matches(r, query))
}

fn text_matches(text: &str, query: &str) -> bool {
    let q = query.trim().to_lowercase();
    q.is_empty() || text.to_lowercase().contains(&q)
}

/// Whether a row contains `query`, ignoring case. An empty query matches every row.
pub fn row_matches(row: &Row, query: &str) -> bool {
    text_matches(&row.0, query) || text_matches(&row.1, query)
}

fn item(title: impl Into<String>, summary: impl Into<String>, detail: Vec<Row>) -> Item {
    Item { title: title.into(), summary: summary.into(), detail }
}

/// The content of a category. A category with nothing in it shows one line that says so.
pub fn content(category: Category, r: &Report, x: &Extras) -> Content {
    match category {
        Category::Overview => Content::card(overview(r)),
        Category::Processor => Content::card(processor_rows(r, x)),
        Category::Memory => Content::card(memory_rows(r, x)),
        Category::Graphics => Content::list(r.graphics().into_iter().enumerate().map(|(i, d)| graphics_item(i, d, x)).collect()),
        Category::Pci => Content::list(r.pci.iter().map(pci_item).collect()),
        Category::Usb => Content::list(usb_tree(&r.usb).into_iter().map(|(depth, d)| usb_item(depth, d, x)).collect()),
        Category::Displays => Content::list(r.displays.iter().map(|d| display_item(d, &x.screens)).collect()),
        Category::Audio => Content::list(x.hw.audio.iter().map(audio_item).collect()),
        Category::Bluetooth => Content::list(x.hw.bluetooth.iter().map(bluetooth_item).collect()),
        Category::Camera => Content::list(x.hw.cameras.iter().map(camera_item).collect()),
        Category::CardReader => Content::list(x.hw.card_readers.iter().map(card_reader_item).collect()),
        Category::Controller => Content::list(x.hw.scsi.iter().filter(|h| h.transport.is_empty()).map(scsi_item).collect()),
        Category::Sas => Content::list(x.hw.sas_phys.iter().map(sas_item).collect()),
        Category::FibreChannel => Content::list(x.hw.fc_ports.iter().map(fc_item).collect()),
        Category::DiscBurning => Content::list(x.hw.optical.iter().map(optical_item).collect()),
        Category::Diagnostics => Content::list(x.hw.diagnostics.iter().map(diagnostic_item).collect()),
        Category::Printers => Content::list(x.hw.printers.iter().map(printer_item).collect()),
        Category::Thunderbolt => Content::list(x.hw.thunderbolt.iter().map(thunderbolt_item).collect()),
        Category::Power => Content::list(r.power.iter().map(power_item).collect()),
        Category::Storage => Content::list(r.disks.iter().map(|d| disk_item(d, x)).collect()),
        Category::Nvme => Content::list(x.hw.nvme.iter().map(nvme_item).collect()),
        Category::Sata => Content::list(x.hw.sata.iter().map(sata_item).collect()),
        Category::Wifi => Content::list(net_items(NetKind::Wifi, r, x)),
        Category::Ethernet => Content::list(net_items(NetKind::Ethernet, r, x)),
        Category::Wwan => Content::list(net_items(NetKind::Wwan, r, x)),
        Category::Firewall => Content::card(firewall_rows(x)),
        Category::Volumes => Content::list(x.hw.volumes.iter().map(volume_item).collect()),
        Category::System => Content::card(system_rows(r, x)),
        Category::Apps => Content::list(r.apps.iter().map(|a| app_item(a, x)).collect()),
        Category::Extensions => Content::card(x.modules.iter().map(|m| kv(m.name.clone(), format!("{} · used by {}", size_label(m.size_bytes, 1 << 10, "KB"), m.used_by))).collect()),
        Category::StartupItems => Content::card(x.startup.iter().map(|n| kv(n.clone(), "Opens at sign-in")).collect()),
        Category::Fonts => Content::card(x.hw.fonts.iter().map(|f| kv(f.name.clone(), font_line(f))).collect()),
        Category::Logs => Content::card(x.hw.logs.iter().map(|l| kv(l.name.clone(), bytes_label(l.bytes))).collect()),
        Category::Language => Content::card(language_rows(&x.locale)),
    }
}

/// The summary rows of a category: the card, or the item lines. Kept for callers that need the flat view.
pub fn rows(category: Category, r: &Report, x: &Extras) -> Vec<Row> {
    content(category, r, x).summary_rows()
}

/// Every category's content, in sidebar order. `mini` keeps only the Overview, as Show Less Information does.
pub fn contents(r: &Report, x: &Extras, mini: bool) -> Vec<(Category, Content)> {
    ALL.iter().copied().filter(|c| !mini || *c == Category::Overview).map(|c| (c, content(c, r, x))).collect()
}

/// The next category after `from` (wrapping, `forward` or back) that has something matching `query`; `from` itself last.
pub fn next_match(from: Category, query: &str, forward: bool, r: &Report, x: &Extras) -> Option<Category> {
    let n = ALL.len();
    (1..=n)
        .map(|step| if forward { (from.index() + step) % n } else { (from.index() + n - step % n) % n })
        .filter_map(Category::from_index)
        .find(|c| content(*c, r, x).matches(query))
}

/// A report read back from a file: the contents of the categories it holds, replacing the live ones in the window.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stored {
    /// The file's name, shown in the window.
    pub name: String,
    pub contents: Vec<(Category, Content)>,
}

impl Stored {
    pub fn of(name: &str, contents: Vec<(Category, Content)>) -> Stored {
        Stored { name: name.to_string(), contents }
    }

    pub fn has(&self, category: Category) -> bool {
        self.contents.iter().any(|(c, _)| *c == category)
    }

    pub fn content(&self, category: Category) -> Content {
        self.contents.iter().find(|(c, _)| *c == category).map_or_else(|| Content::message("Status", "Not in this report"), |(_, content)| content.clone())
    }
}

/// The report as plain text: each category under its title, one `key: value` line per row; a list's items follow their
/// line with their own facts indented below it.
pub fn text_of(contents: &[(Category, Content)]) -> String {
    let mut out = String::from("System Information Report\n");
    for (c, content) in contents {
        out.push_str(&format!("\n{} ({})\n", c.title(), c.section().title()));
        if content.list {
            for item in &content.items {
                out.push_str(&format!("  {}: {}\n", item.title.trim_start(), item.summary));
                for (k, v) in &item.detail {
                    out.push_str(&format!("      {k}: {v}\n"));
                }
            }
        } else {
            for (k, v) in content.summary_rows() {
                out.push_str(&format!("  {k}: {v}\n"));
            }
        }
    }
    out
}

/// The live report as plain text. `mini` keeps only the Overview.
pub fn report_text(r: &Report, x: &Extras, mini: bool) -> String {
    text_of(&contents(r, x, mini))
}

// ---------------------------------------------------------------- card categories

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

fn processor_rows(r: &Report, x: &Extras) -> Vec<Row> {
    let cpu = &x.hw.cpu;
    let mut out = vec![
        kv("Name", or_dash(&r.cpu.model)),
        kv("Vendor", or_dash(&r.cpu.vendor)),
        kv("Physical cores", if cpu.cores == 0 { "—".to_string() } else { cpu.cores.to_string() }),
        kv("Logical processors", r.cpu.logical.to_string()),
    ];
    if cpu.packages > 1 {
        out.push(kv("Packages", cpu.packages.to_string()));
    }
    if let Some((performance, efficiency)) = cpu.core_kinds {
        out.push(kv("Performance cores", performance.to_string()));
        out.push(kv("Efficiency cores", efficiency.to_string()));
    }
    out.push(kv("Clock", clock_label(r.cpu.mhz)));
    if cpu.now_max_mhz > 0 && cpu.now_min_mhz != cpu.now_max_mhz {
        out.push(kv("Clock range now", format!("{} to {}", clock_label(cpu.now_min_mhz as f32), clock_label(cpu.now_max_mhz as f32))));
    }
    for (processor, mhz) in &cpu.core_clocks_mhz {
        out.push(kv(format!("Processor {processor} clock"), clock_label(*mhz as f32)));
    }
    if cpu.rated_max_mhz > 0 {
        out.push(kv("Maximum clock", clock_label(cpu.rated_max_mhz as f32)));
    }
    if !cpu.caches.is_empty() {
        out.push(kv("Caches", cpu.caches.iter().map(probe::CacheInfo::label).collect::<Vec<_>>().join(", ")));
    }
    out
}

fn memory_rows(r: &Report, x: &Extras) -> Vec<Row> {
    let m = &x.hw.memory;
    let mut out = vec![kv("Installed", size_label(r.memory_bytes, 1 << 30, "GB"))];
    if m.available > 0 {
        out.push(kv("Available", size_label(m.available, 1 << 30, "GB")));
    }
    out.push(kv(
        "Swap",
        if m.swap_total == 0 { "None".to_string() } else { format!("{} ({} free)", size_label(m.swap_total, 1 << 30, "GB"), size_label(m.swap_free, 1 << 30, "GB")) },
    ));
    if m.dimms.is_empty() {
        out.push(kv("Memory slots", "Not reported (the table may need administrator access)"));
    } else {
        let filled = m.dimms.iter().filter(|d| d.size_mb > 0).count();
        out.push(kv("Memory slots", format!("{filled} of {} in use", m.dimms.len())));
        for (i, d) in m.dimms.iter().enumerate() {
            out.push(kv(if d.locator.is_empty() { format!("Slot {}", i + 1) } else { d.locator.clone() }, dimm_line(d)));
        }
    }
    out
}

fn dimm_line(d: &probe::Dimm) -> String {
    if d.size_mb == 0 {
        return "Empty".to_string();
    }
    let mut parts = vec![mb_label(d.size_mb)];
    if !d.kind.is_empty() {
        parts.push(d.kind.clone());
    }
    let speed = if d.configured_mts > 0 { d.configured_mts } else { d.speed_mts };
    if speed > 0 {
        parts.push(format!("{speed} MT/s"));
    }
    for text in [&d.manufacturer, &d.part] {
        if !text.is_empty() && !text.eq_ignore_ascii_case("not specified") {
            parts.push(text.clone());
        }
    }
    parts.join(" · ")
}

fn system_rows(r: &Report, x: &Extras) -> Vec<Row> {
    let mut out = vec![kv("Name", or_dash(&r.os_name)), kv("Version", or_dash(&r.os_version))];
    if !x.hw.build_id.is_empty() {
        out.push(kv("Build", x.hw.build_id.clone()));
    }
    out.extend([
        kv("Kernel", or_dash(&r.kernel)),
        kv("Computer Name", or_dash(&x.hostname)),
        kv("Time since boot", uptime_label(x.uptime_secs)),
        kv("Boot volume", or_dash(&x.boot_volume)),
        kv("Boot mode", or_dash(&x.boot_mode)),
        kv("Secure boot", or_dash(&x.hw.secure_boot)),
    ]);
    out
}

fn firewall_rows(x: &Extras) -> Vec<Row> {
    let f = &x.hw.firewall;
    if f.backend.is_empty() && f.configs.is_empty() {
        return Vec::new();
    }
    let list = |v: &[String], none: &str| if v.is_empty() { none.to_string() } else { v.join(", ") };
    vec![
        kv("Packet filter", if f.backend.is_empty() { "None loaded".to_string() } else { f.backend.clone() }),
        kv("iptables tables", list(&f.tables, "—")),
        kv("Configuration found", list(&f.configs, "None")),
        kv("Rules", "Listing the rules needs administrator rights"),
    ]
}

fn language_rows(locale: &str) -> Vec<Row> {
    if locale.trim().is_empty() {
        return Vec::new();
    }
    let (language, region, encoding) = split_locale(locale);
    vec![kv("Locale", locale.trim()), kv("Language", or_dash(&language)), kv("Region", or_dash(&region)), kv("Encoding", or_dash(&encoding))]
}

fn font_line(f: &probe::FontFamily) -> String {
    let mut parts = vec![if f.mono { "Monospaced" } else { "Proportional" }];
    if f.variable {
        parts.push("Variable");
    }
    parts.join(" · ")
}

// ---------------------------------------------------------------- list categories

fn graphics_item(i: usize, d: &PciDevice, x: &Extras) -> Item {
    let mut detail = vec![kv("Vendor", d.vendor_name()), kv("Vendor ID", format!("{:04x}", d.vendor_id)), kv("Device ID", format!("{:04x}", d.device_id)), kv("Bus address", d.address.clone()), kv("Driver", driver_label(&d.driver))];
    if let Some(g) = x.hw.gpu_memory.iter().find(|g| g.address == d.address) {
        detail.push(kv("Video memory", memory_label(g.vram_bytes)));
    }
    item(format!("Graphics {}", i + 1), pci_line(d), detail)
}

fn pci_item(d: &PciDevice) -> Item {
    item(
        d.address.clone(),
        format!("{} · {} · {}", d.vendor_name(), d.class_name(), driver_label(&d.driver)),
        vec![kv("Vendor", d.vendor_name()), kv("Vendor ID", format!("{:04x}", d.vendor_id)), kv("Device ID", format!("{:04x}", d.device_id)), kv("Class", d.class_name()), kv("Class code", format!("{:06x}", d.class)), kv("Driver", driver_label(&d.driver))],
    )
}

fn usb_item(depth: usize, d: &UsbDevice, x: &Extras) -> Item {
    let name = if d.product.is_empty() { format!("Device {:04x}:{:04x}", d.vendor_id, d.product_id) } else { d.product.clone() };
    let power = x.hw.usb_power.iter().find(|(port, _)| *port == d.port).map(|(_, p)| p.clone());
    let mut summary = format!("{} · {} · port {}", or_dash(&d.manufacturer), usb_speed(d.speed_mbps), d.port);
    if let Some(p) = &power {
        summary.push_str(&format!(" · {p}"));
    }
    let bus = usb_position(&d.port).0;
    let parent = usb_parent(&d.port);
    let detail = vec![
        kv("Product", name.clone()),
        kv("Manufacturer", or_dash(&d.manufacturer)),
        kv("Vendor ID", format!("{:04x}", d.vendor_id)),
        kv("Product ID", format!("{:04x}", d.product_id)),
        kv("Speed", usb_speed(d.speed_mbps)),
        kv("Port", d.port.clone()),
        kv("Bus", if bus == u32::MAX { "—".to_string() } else { bus.to_string() }),
        kv("Connected to", if parent.is_empty() { "Host controller".to_string() } else { parent }),
        kv("Maximum power", power.unwrap_or_else(|| "—".to_string())),
    ];
    item(format!("{}{name}", "    ".repeat(depth)), summary, detail)
}

fn display_item(d: &Display, screens: &[ScreenFacts]) -> Item {
    let state = match (d.connected, d.mode.is_empty()) {
        (false, _) => "Not connected".to_string(),
        (true, true) => "Connected".to_string(),
        (true, false) => format!("Connected · {}", d.mode),
    };
    let mut detail = vec![kv("Connector", d.connector.clone()), kv("Status", if d.connected { "Connected" } else { "Not connected" }), kv("Mode", or_dash(&d.mode))];
    if let Some(screen) = screen_for(&d.connector, screens) {
        if screen.refresh_hz > 0 {
            detail.push(kv("Refresh rate", format!("{} Hz", screen.refresh_hz)));
        }
        if screen.scale_percent > 0 {
            detail.push(kv("Scale", scale_label(screen.scale_percent)));
        }
    }
    item(d.connector.clone(), state, detail)
}

/// The display service's entry for a connector. The service names a display by its own label, so the match is by name
/// either way round (the connector "eDP-1" holds the label "eDP-1", or the label "Built-in" is part of it); a label
/// that is empty matches nothing.
pub fn screen_for<'a>(connector: &str, screens: &'a [ScreenFacts]) -> Option<&'a ScreenFacts> {
    let connector = connector.to_lowercase();
    screens.iter().find(|s| {
        let name = s.name.trim().to_lowercase();
        !name.is_empty() && (name == connector || connector.contains(&name) || name.contains(&connector))
    })
}

/// A scale in percent as the desktop writes it: 200 is "2x", 125 is "1.25x", 150 is "1.5x".
pub fn scale_label(percent: u32) -> String {
    let (whole, part) = (percent / 100, percent % 100);
    if part == 0 {
        format!("{whole}x")
    } else if part % 10 == 0 {
        format!("{whole}.{}x", part / 10)
    } else {
        format!("{whole}.{part:02}x")
    }
}

fn audio_item(c: &probe::AudioCard) -> Item {
    let title = if c.name.is_empty() { c.id.clone() } else { c.name.clone() };
    item(
        title,
        format!("{} · card {}", or_dash(&c.driver), c.index),
        vec![kv("Name", or_dash(&c.name)), kv("Card", c.index.to_string()), kv("Identifier", or_dash(&c.id)), kv("Driver", or_dash(&c.driver)), kv("Description", or_dash(&c.detail)), kv("Codec", or_dash(&c.codec))],
    )
}

fn bluetooth_item(c: &probe::BluetoothController) -> Item {
    let radio = match c.blocked {
        Some(true) => "Off",
        Some(false) => "On",
        None => "—",
    };
    let connections = if c.links == 1 { "1 connection".to_string() } else { format!("{} connections", c.links) };
    item(
        c.name.clone(),
        format!("{} · {} · {radio} · {connections}", or_dash(&c.bus), or_dash(&c.driver)),
        vec![kv("Controller", c.name.clone()), kv("Bus", or_dash(&c.bus)), kv("Driver", or_dash(&c.driver)), kv("Radio", radio), kv("Connections", c.links.to_string())],
    )
}

fn camera_item(c: &probe::Camera) -> Item {
    let title = if c.name.is_empty() { c.node.clone() } else { c.name.clone() };
    item(title, format!("{} · {}", c.node, or_dash(&c.driver)), vec![kv("Name", or_dash(&c.name)), kv("Device node", c.node.clone()), kv("Driver", or_dash(&c.driver))])
}

fn card_reader_item(c: &probe::CardSlot) -> Item {
    let title = if c.card.is_empty() { c.host.clone() } else { c.card.clone() };
    let state = if c.card.is_empty() { "Empty slot" } else { "Card inserted" };
    item(
        title,
        format!("{} · {state}", or_dash(&c.driver)),
        vec![kv("Host", c.host.clone()), kv("Driver", or_dash(&c.driver)), kv("Card", or_dash(&c.card)), kv("Card type", or_dash(&c.kind)), kv("Manufacturer ID", or_dash(&c.manufacturer))],
    )
}

fn scsi_item(h: &probe::ScsiHost) -> Item {
    let title = if h.proc_name.is_empty() { h.name.clone() } else { h.proc_name.clone() };
    item(title, format!("{} · {}", h.name, or_dash(&h.driver)), vec![kv("Host", h.name.clone()), kv("Driver", or_dash(&h.driver)), kv("Name", or_dash(&h.proc_name))])
}

fn sas_item(p: &probe::SasPhy) -> Item {
    let link = if p.link_rate.is_empty() { "No link" } else { p.link_rate.as_str() };
    item(p.name.clone(), format!("{link} · {}", or_dash(&p.device)), vec![kv("Phy", p.name.clone()), kv("Link rate", link), kv("Device", or_dash(&p.device)), kv("SAS address", or_dash(&p.address))])
}

fn fc_item(p: &probe::FcPort) -> Item {
    let title = if p.symbolic.is_empty() { p.host.clone() } else { p.symbolic.clone() };
    item(
        title,
        format!("{} · {}", or_dash(&p.state), or_dash(&p.speed)),
        vec![
            kv("Host", p.host.clone()),
            kv("Port name", or_dash(&p.port_name)),
            kv("Node name", or_dash(&p.node_name)),
            kv("State", or_dash(&p.state)),
            kv("Port type", or_dash(&p.port_type)),
            kv("Speed", or_dash(&p.speed)),
        ],
    )
}

fn optical_item(o: &probe::OpticalDrive) -> Item {
    let title = if o.model.is_empty() { o.device.clone() } else { o.model.clone() };
    let media = if o.media_bytes == 0 { "No disc" } else { "Disc inserted" };
    let size = if o.media_bytes == 0 { "—".to_string() } else { gb_label(o.media_bytes) };
    item(
        title,
        format!("{} · {media}", o.device),
        vec![kv("Device", format!("/dev/{}", o.device)), kv("Vendor", or_dash(&o.vendor)), kv("Model", or_dash(&o.model)), kv("Revision", or_dash(&o.revision)), kv("Media", media), kv("Disc size", size)],
    )
}

fn diagnostic_item(d: &probe::DiagnosticReport) -> Item {
    item(d.name.clone(), format!("{} · {}", bytes_label(d.bytes), day_label(d.day)), vec![kv("Name", d.name.clone()), kv("Size", bytes_label(d.bytes)), kv("Date", day_label(d.day))])
}

fn printer_item(p: &probe::Printer) -> Item {
    let state = if p.state.is_empty() { "—".to_string() } else { p.state.clone() };
    item(p.name.clone(), format!("{state} · {}", or_dash(&p.uri)), vec![kv("Name", p.name.clone()), kv("Status", state), kv("Connection", or_dash(&p.uri))])
}

fn thunderbolt_item(t: &probe::ThunderboltDevice) -> Item {
    item(
        t.name.clone(),
        format!("{} · {}", or_dash(&t.vendor), or_dash(&t.authorized)),
        vec![kv("Name", t.name.clone()), kv("Vendor", or_dash(&t.vendor)), kv("Authorization", or_dash(&t.authorized)), kv("Generation", or_dash(&t.generation))],
    )
}

fn power_item(p: &PowerSource) -> Item {
    let (title, summary) = power_row(p);
    let wh = |v: f32| if v > 0.0 { format!("{v:.1} Wh") } else { "—".to_string() };
    let detail = vec![
        kv("Name", p.name.clone()),
        kv("Type", or_dash(&p.kind)),
        kv("Model", or_dash(&p.model)),
        kv("Charge", p.capacity_percent.map_or("—".to_string(), |c| format!("{c}%"))),
        kv("Status", or_dash(&p.status)),
        kv("Health", p.health_percent().map_or("—".to_string(), |h| format!("{h}%"))),
        kv("Cycle count", p.cycles.map_or("—".to_string(), |c| c.to_string())),
        kv("Full charge capacity", wh(p.full_wh)),
        kv("Design capacity", wh(p.design_wh)),
    ];
    item(title, summary, detail)
}

fn disk_item(d: &Disk, x: &Extras) -> Item {
    let (title, summary) = disk_row(d);
    let mut detail = vec![
        kv("Device", format!("/dev/{}", d.name)),
        kv("Model", or_dash(&d.model)),
        kv("Capacity", size_label(d.size_bytes, 1_000_000_000, "GB")),
        kv("Type", if d.rotational { "HDD" } else { "SSD" }),
        kv("Location", if d.removable { "Removable" } else { "Internal" }),
        kv("Partitions", d.partitions.len().to_string()),
    ];
    for part in &d.partitions {
        let device = format!("/dev/{part}");
        let line = match x.hw.volumes.iter().find(|v| v.device == device) {
            Some(v) => volume_line(v),
            None => "Not mounted".to_string(),
        };
        detail.push(kv(part.clone(), line));
    }
    item(title, summary, detail)
}

fn nvme_item(n: &probe::NvmeDrive) -> Item {
    item(
        n.name.clone(),
        format!("{} · {}", or_dash(&n.model), or_dash(&n.state)),
        vec![kv("Controller", n.name.clone()), kv("Model", or_dash(&n.model)), kv("Firmware", or_dash(&n.firmware)), kv("State", or_dash(&n.state)), kv("Transport", or_dash(&n.transport))],
    )
}

fn sata_item(p: &probe::SataPort) -> Item {
    let speed = if p.speed.is_empty() { "Speed unknown".to_string() } else { p.speed.clone() };
    let device = if p.attached { "Device attached" } else { "No device" };
    item(p.port.clone(), format!("{speed} · {device}"), vec![kv("Port", p.port.clone()), kv("Link speed", speed), kv("Device", device)])
}

fn volume_line(v: &probe::Volume) -> String {
    format!("{} · {} · {} available of {}", or_dash(&v.fs), v.mount, gb_label(v.free), gb_label(v.total))
}

fn volume_item(v: &probe::Volume) -> Item {
    let used = if v.total == 0 { "—".to_string() } else { format!("{}%", (v.total.saturating_sub(v.free) as f64 / v.total as f64 * 100.0).round()) };
    item(
        v.mount.clone(),
        format!("{} · {} available of {}", or_dash(&v.fs), gb_label(v.free), gb_label(v.total)),
        vec![
            kv("Mount point", v.mount.clone()),
            kv("Device", or_dash(&v.device)),
            kv("File system", or_dash(&v.fs)),
            kv("Capacity", gb_label(v.total)),
            kv("Available", gb_label(v.free)),
            kv("Used", used),
        ],
    )
}

fn app_item(a: &InstalledApp, x: &Extras) -> Item {
    let known = x.hw.apps.iter().find(|k| k.id == a.id);
    let name = known.map(|k| k.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| a.id.clone());
    let kind = known.map_or("—", |k| k.kind_label());
    let installed = known.filter(|k| k.installed_day > 0).map_or("—".to_string(), |k| day_label(k.installed_day));
    let package = if known.is_some() { "Registered; checksum verified at install" } else { "Not in the app registry" };
    item(
        name.clone(),
        format!("{} · {kind}", a.version),
        vec![kv("Name", name), kv("Identifier", a.id.clone()), kv("Version", a.version.clone()), kv("Kind", kind), kv("Installed", installed), kv("Package", package)],
    )
}

/// Which sidebar category an interface belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetKind {
    Wifi,
    Ethernet,
    Wwan,
}

pub fn net_kind(n: &NetInterface) -> NetKind {
    if n.wireless {
        NetKind::Wifi
    } else if n.name.starts_with("ww") {
        NetKind::Wwan
    } else {
        NetKind::Ethernet
    }
}

fn net_items(kind: NetKind, r: &Report, x: &Extras) -> Vec<Item> {
    r.net.iter().filter(|n| net_kind(n) == kind).map(|n| net_item(n, kind, x.hw.net.iter().find(|d| d.name == n.name), &x.hw.dns)).collect()
}

fn net_item(n: &NetInterface, kind: NetKind, d: Option<&NetDetail>, dns: &[String]) -> Item {
    let type_name = match kind {
        NetKind::Wifi => "Wi-Fi",
        NetKind::Ethernet => "Ethernet",
        NetKind::Wwan => "Cellular",
    };
    let mut parts = vec![capitalize(&n.state), or_dash(&n.mac)];
    if n.default_route {
        parts.push("Default route".to_string());
    }
    let ipv4: Vec<String> = d.map(|d| d.ipv4.clone()).unwrap_or_default();
    let addresses: Vec<String> = ipv4.iter().chain(n.addresses.iter()).cloned().collect();
    if !addresses.is_empty() {
        parts.push(addresses.join(", "));
    }
    let list = |v: &[String]| if v.is_empty() { "—".to_string() } else { v.join(", ") };
    let mut detail = vec![
        kv("Interface", n.name.clone()),
        kv("Type", type_name),
        kv("State", capitalize(&n.state)),
        kv("Hardware", match d {
            Some(d) if d.physical => "Physical",
            Some(_) => "Virtual",
            None => "—",
        }),
        kv("MAC address", or_dash(&n.mac)),
        kv("IPv4 addresses", list(&ipv4)),
        kv("IPv6 addresses", list(&n.addresses)),
        kv("Default route", if n.default_route { "Yes" } else { "No" }),
    ];
    if let Some(d) = d.filter(|d| !d.gateway.is_empty()) {
        detail.push(kv("Gateway", d.gateway.clone()));
    }
    if n.default_route && !dns.is_empty() {
        detail.push(kv("DNS servers", dns.join(", ")));
    }
    if let Some(speed) = d.map(|d| d.speed_mbps).filter(|s| *s > 0) {
        detail.push(kv("Link speed", format!("{speed} Mb/s")));
        parts.push(format!("{speed} Mb/s"));
    }
    if let Some((quality, dbm)) = d.and_then(|d| d.signal) {
        detail.push(kv("Signal", format!("{quality}% · {dbm} dBm")));
        parts.push(format!("Signal {quality}%"));
    }
    item(n.name.clone(), parts.join(" · "), detail)
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

fn pci_line(d: &PciDevice) -> String {
    format!("{} · {:04x} · driver {}", d.vendor_name(), d.device_id, driver_label(&d.driver))
}

fn driver_label(driver: &str) -> String {
    if driver.is_empty() {
        "none".to_string()
    } else {
        driver.to_string()
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

// ---------------------------------------------------------------- how the numbers read

/// A size in decimal gigabytes, as disks are sold; "—" for zero.
pub fn gb_label(bytes: u64) -> String {
    size_label(bytes, 1_000_000_000, "GB")
}

/// A memory size in binary units: megabytes below a gigabyte.
pub fn memory_label(bytes: u64) -> String {
    if bytes >= 1 << 30 {
        size_label(bytes, 1 << 30, "GB")
    } else {
        size_label(bytes, 1 << 20, "MB")
    }
}

pub fn mb_label(mb: u64) -> String {
    if mb >= 1024 && mb % 1024 == 0 {
        format!("{} GB", mb / 1024)
    } else {
        format!("{mb} MB")
    }
}

/// A file size: bytes, kilobytes, megabytes or gigabytes (binary units).
pub fn bytes_label(bytes: u64) -> String {
    let b = bytes as f64;
    let (kb, mb, gb) = (1024.0, 1024.0 * 1024.0, 1024.0 * 1024.0 * 1024.0);
    if bytes < 1024 {
        format!("{bytes} bytes")
    } else if b < mb {
        format!("{:.0} KB", b / kb)
    } else if b < gb {
        format!("{:.1} MB", b / mb)
    } else {
        format!("{:.1} GB", b / gb)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extras::Module;
    use crate::probe::{Dimm, Hardware, NetDetail, Volume};

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
            assert_eq!(Category::from_title(c.title()), Some(*c), "titles are unique");
        }
        assert_eq!(Category::from_index(ALL.len()), None);
        assert_eq!(in_section(Section::Network), vec![Category::Wifi, Category::Ethernet, Category::Wwan, Category::Firewall, Category::Volumes]);
        assert_eq!(in_section(Section::Hardware).len(), 22, "every hardware class is listed under Hardware");
        assert_eq!(in_section(Section::Software)[0], Category::System);
        assert_eq!(in_section(Section::Software).last(), Some(&Category::Language));
    }

    #[test]
    fn a_section_is_one_run_of_the_sidebar_order() {
        let mut last = 0;
        for c in ALL {
            assert!(c.section().index() >= last, "{:?} breaks the section order", c);
            last = c.section().index();
        }
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
        Extras { hostname: "midget".into(), uptime_secs: 3_700, boot_volume: "/dev/sda2 (ext4)".into(), modules: vec![Module { name: "snd".into(), size_bytes: 2048, used_by: 1 }], locale: "is_IS.UTF-8".into(), ..Default::default() }
    }

    #[test]
    fn system_rows_carry_host_name_uptime_boot_volume_and_secure_boot() {
        let mut x = sample_extras();
        x.hw.secure_boot = "Enabled".into();
        x.hw.build_id = "2026.10".into();
        let rows = rows(Category::System, &Report::default(), &x);
        assert!(rows.contains(&kv("Computer Name", "midget")));
        assert!(rows.contains(&kv("Time since boot", "1 hour, 1 minute")));
        assert!(rows.contains(&kv("Boot volume", "/dev/sda2 (ext4)")));
        assert!(rows.contains(&kv("Secure boot", "Enabled")));
        assert!(rows.contains(&kv("Build", "2026.10")));
    }

    #[test]
    fn extensions_and_language_come_from_the_extras() {
        let x = sample_extras();
        assert_eq!(rows(Category::Extensions, &Report::default(), &x), vec![kv("snd", "2.0 KB · used by 1")]);
        assert!(rows(Category::Language, &Report::default(), &x).contains(&kv("Region", "IS")));
        assert_eq!(rows(Category::StartupItems, &Report::default(), &x), vec![kv("Status", "Nothing was detected")]);
    }

    #[test]
    fn processor_rows_add_cores_caches_and_the_clock_range() {
        let mut r = Report::default();
        r.cpu.model = "Test CPU".into();
        r.cpu.logical = 8;
        r.cpu.mhz = 1800.0;
        let mut x = Extras::default();
        x.hw.cpu = probe::CpuDetail {
            cores: 4,
            packages: 1,
            caches: vec![probe::CacheInfo { level: 1, kind: "Data".into(), size_kb: 32 }, probe::CacheInfo { level: 3, kind: "Unified".into(), size_kb: 8192 }],
            now_min_mhz: 800,
            now_max_mhz: 3200,
            rated_max_mhz: 4200,
            core_clocks_mhz: vec![(0, 3200), (1, 800)],
            core_kinds: Some((2, 2)),
        };
        let rows = rows(Category::Processor, &r, &x);
        assert!(rows.contains(&kv("Physical cores", "4")));
        assert!(rows.contains(&kv("Performance cores", "2")));
        assert!(rows.contains(&kv("Efficiency cores", "2")));
        assert!(rows.contains(&kv("Processor 1 clock", "800 MHz")));
        assert!(rows.contains(&kv("Logical processors", "8")));
        assert!(rows.contains(&kv("Clock range now", "800 MHz to 3.20 GHz")));
        assert!(rows.contains(&kv("Maximum clock", "4.20 GHz")));
        assert!(rows.contains(&kv("Caches", "L1 data 32 KB, L3 8 MB")));
        assert!(!rows.iter().any(|(k, _)| k == "Packages"), "one package is not worth a row");
        assert!(super::rows(Category::Processor, &Report::default(), &Extras::default()).contains(&kv("Physical cores", "—")));
    }

    #[test]
    fn memory_rows_list_the_slots() {
        let mut r = Report::default();
        r.memory_bytes = 16 << 30;
        let mut x = Extras::default();
        x.hw.memory.available = 8 << 30;
        x.hw.memory.dimms = vec![
            Dimm { locator: "DIMM A1".into(), size_mb: 16_384, kind: "DDR4".into(), speed_mts: 3200, configured_mts: 2933, manufacturer: "Acme".into(), part: "Not Specified".into() },
            Dimm { locator: "DIMM B1".into(), ..Default::default() },
        ];
        let rows = rows(Category::Memory, &r, &x);
        assert!(rows.contains(&kv("Memory slots", "1 of 2 in use")));
        assert!(rows.contains(&kv("DIMM A1", "16 GB · DDR4 · 2933 MT/s · Acme")));
        assert!(rows.contains(&kv("DIMM B1", "Empty")));
        assert!(rows.contains(&kv("Swap", "None")));
        let without = super::rows(Category::Memory, &r, &Extras::default());
        assert!(without.iter().any(|(k, v)| k == "Memory slots" && v.starts_with("Not reported")));
    }

    fn usb(port: &str, product: &str) -> UsbDevice {
        UsbDevice { port: port.into(), product: product.into(), manufacturer: "Acme".into(), speed_mbps: 480, vendor_id: 0x1d6b, product_id: 2, ..Default::default() }
    }

    #[test]
    fn usb_devices_list_in_tree_order_with_power() {
        let mut r = Report::default();
        r.usb = vec![usb("1-1.2", "Mouse"), usb("usb1", "Host"), usb("1-1", "Hub")];
        let mut x = Extras::default();
        x.hw.usb_power = vec![("1-1.2".to_string(), "100 mA".to_string())];
        let c = content(Category::Usb, &r, &x);
        assert!(c.list);
        let titles: Vec<&str> = c.items.iter().map(|i| i.title.as_str()).collect();
        assert_eq!(titles, vec!["Host", "    Hub", "        Mouse"]);
        let mouse = &c.items[2];
        assert!(mouse.summary.ends_with("port 1-1.2 · 100 mA"));
        assert!(mouse.detail.contains(&kv("Connected to", "1-1")));
        assert!(mouse.detail.contains(&kv("Maximum power", "100 mA")));
        assert!(c.items[0].detail.contains(&kv("Connected to", "Host controller")));
    }

    #[test]
    fn graphics_items_show_video_memory_when_known() {
        let mut r = Report::default();
        r.pci = vec![PciDevice { address: "0000:01:00.0".into(), vendor_id: 0x10de, device_id: 0x2504, class: 0x030000, driver: "nvidia".into() }];
        let mut x = Extras::default();
        assert!(!content(Category::Graphics, &r, &x).items[0].detail.iter().any(|(k, _)| k == "Video memory"));
        x.hw.gpu_memory = vec![probe::GpuMemory { address: "0000:01:00.0".into(), vram_bytes: 6 << 30 }];
        let c = content(Category::Graphics, &r, &x);
        assert_eq!(c.items[0].title, "Graphics 1");
        assert!(c.items[0].detail.contains(&kv("Video memory", "6.0 GB")));
    }

    #[test]
    fn storage_items_list_each_partition_with_its_volume() {
        let mut r = Report::default();
        r.disks = vec![Disk { name: "sda".into(), model: "Disk".into(), size_bytes: 500_000_000_000, partitions: vec!["sda1".into(), "sda2".into()], ..Default::default() }];
        let mut x = Extras::default();
        x.hw.volumes = vec![Volume { device: "/dev/sda2".into(), mount: "/".into(), fs: "ext4".into(), total: 100_000_000_000, free: 40_000_000_000 }];
        let detail = &content(Category::Storage, &r, &x).items[0].detail;
        assert!(detail.contains(&kv("sda1", "Not mounted")));
        assert!(detail.contains(&kv("sda2", "ext4 · / · 40 GB available of 100 GB")));
        assert!(detail.contains(&kv("Device", "/dev/sda")));
    }

    #[test]
    fn network_interfaces_split_by_kind_and_carry_addressing() {
        let mut r = Report::default();
        r.net = vec![
            NetInterface { name: "wlan0".into(), mac: "aa:bb".into(), state: "up".into(), wireless: true, addresses: vec!["2001:db8:0:0:0:0:0:1".into()], default_route: true },
            NetInterface { name: "enp3s0".into(), state: "down".into(), ..Default::default() },
            NetInterface { name: "wwan0".into(), state: "down".into(), ..Default::default() },
        ];
        let mut x = Extras::default();
        x.hw.dns = vec!["1.1.1.1".into()];
        x.hw.net = vec![NetDetail { name: "wlan0".into(), physical: true, speed_mbps: 0, ipv4: vec!["192.168.1.5".into()], gateway: "192.168.1.1".into(), signal: Some((80, -52)) }];
        let wifi = content(Category::Wifi, &r, &x);
        assert_eq!(wifi.items.len(), 1);
        let d = &wifi.items[0].detail;
        assert!(d.contains(&kv("IPv4 addresses", "192.168.1.5")));
        assert!(d.contains(&kv("Gateway", "192.168.1.1")));
        assert!(d.contains(&kv("DNS servers", "1.1.1.1")));
        assert!(d.contains(&kv("Signal", "80% · -52 dBm")));
        assert!(d.contains(&kv("Hardware", "Physical")));
        assert!(wifi.items[0].summary.contains("192.168.1.5, 2001:db8:0:0:0:0:0:1"));
        assert_eq!(content(Category::Ethernet, &r, &x).items[0].title, "enp3s0");
        assert_eq!(content(Category::Wwan, &r, &x).items[0].title, "wwan0");
        assert!(content(Category::Ethernet, &r, &x).items[0].detail.contains(&kv("Hardware", "—")), "no probe data: unknown");
    }

    #[test]
    fn apps_show_their_name_kind_and_install_date() {
        let mut r = Report::default();
        r.apps = vec![InstalledApp { id: "is.o.notes".into(), version: "1.2.0".into() }, InstalledApp { id: "x.loose".into(), version: "0.1".into() }];
        let mut x = Extras::default();
        x.hw.apps = vec![probe::AppDetail { id: "is.o.notes".into(), name: "Notes".into(), kind: "system-app".into(), installed_day: 20_000 }];
        let c = content(Category::Apps, &r, &x);
        assert_eq!(c.items[0].title, "Notes");
        assert_eq!(c.items[0].summary, "1.2.0 · Preinstalled");
        assert!(c.items[0].detail.contains(&kv("Installed", "2024-10-04")));
        assert!(c.items[0].detail.contains(&kv("Identifier", "is.o.notes")));
        assert_eq!(c.items[1].title, "x.loose", "an app the registry does not know keeps its id");
        assert!(c.items[1].detail.contains(&kv("Package", "Not in the app registry")));
    }

    #[test]
    fn volumes_list_free_space_and_use() {
        let mut x = Extras::default();
        x.hw.volumes = vec![Volume { device: "/dev/sda1".into(), mount: "/boot".into(), fs: "vfat".into(), total: 1_000_000_000, free: 750_000_000 }];
        let c = content(Category::Volumes, &Report::default(), &x);
        assert_eq!(c.items[0].summary, "vfat · 0.8 GB available of 1.0 GB");
        assert!(c.items[0].detail.contains(&kv("Used", "25%")));
    }

    #[test]
    fn firewall_is_empty_without_a_filter_and_notes_the_backend_otherwise() {
        assert!(content(Category::Firewall, &Report::default(), &Extras::default()).empty);
        let mut x = Extras::default();
        x.hw.firewall = probe::Firewall { backend: "nftables".into(), tables: Vec::new(), configs: vec!["ufw".into()] };
        let rows = rows(Category::Firewall, &Report::default(), &x);
        assert!(rows.contains(&kv("Packet filter", "nftables")));
        assert!(rows.contains(&kv("Configuration found", "ufw")));
    }

    #[test]
    fn find_matches_keys_and_values_ignoring_case() {
        assert!(row_matches(&kv("Computer Name", "midget"), "MIDG"));
        assert!(row_matches(&kv("a", "b"), "  "));
        assert!(!row_matches(&kv("a", "b"), "zzz"));
    }

    #[test]
    fn a_find_keeps_matching_items_and_their_details_count() {
        let mut r = Report::default();
        r.usb = vec![usb("usb1", "Host"), usb("1-1", "Keyboard")];
        let c = content(Category::Usb, &r, &Extras::default());
        assert_eq!(c.filtered("keyboard").items.len(), 1);
        assert_eq!(c.filtered("1-1").items.len(), 1, "a fact in the detail card counts");
        assert_eq!(c.filtered("zzz").summary_rows(), vec![kv("Find", "No matches in this category")]);
        assert_eq!(c.filtered("  ").items.len(), 2, "an empty query keeps everything");
        assert!(c.matches("keyboard"));
        assert!(!content(Category::Usb, &Report::default(), &Extras::default()).matches("status"), "an empty category matches nothing");
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
        assert!(full.contains("Wi-Fi (Network)"));
        let mini = report_text(&r, &x, true);
        assert!(mini.contains("Overview (Hardware)"));
        assert!(!mini.contains("Extensions"));
    }

    #[test]
    fn report_text_indents_the_facts_of_list_items() {
        let mut r = Report::default();
        r.usb = vec![usb("usb1", "Host")];
        let text = report_text(&r, &Extras::default(), false);
        assert!(text.contains("\n  Host: Acme · High speed · port usb1\n      Product: Host\n"));
    }

    #[test]
    fn stored_reports_answer_for_the_categories_they_hold() {
        let s = Stored::of("r.json", vec![(Category::Overview, Content::card(vec![kv("Model", "X")]))]);
        assert!(s.has(Category::Overview));
        assert!(!s.has(Category::Usb));
        assert_eq!(s.content(Category::Overview).summary_rows(), vec![kv("Model", "X")]);
        assert_eq!(s.content(Category::Usb).summary_rows(), vec![kv("Status", "Not in this report")]);
    }

    #[test]
    fn labels_read_in_the_right_unit() {
        assert_eq!(bytes_label(0), "0 bytes");
        assert_eq!(bytes_label(2048), "2 KB");
        assert_eq!(bytes_label(5 * 1024 * 1024), "5.0 MB");
        assert_eq!(bytes_label(3 << 30), "3.0 GB");
        assert_eq!(mb_label(8192), "8 GB");
        assert_eq!(mb_label(1536), "1536 MB");
        assert_eq!(memory_label(512 << 20), "512 MB");
        assert_eq!(memory_label(6 << 30), "6.0 GB");
    }

    #[test]
    fn displays_show_the_refresh_rate_and_scale_the_service_reports() {
        let mut r = Report::default();
        r.displays = vec![Display { connector: "eDP-1".into(), connected: true, mode: "2880x1800".into() }, Display { connector: "HDMI-A-1".into(), connected: false, mode: String::new() }];
        let mut x = Extras::default();
        x.screens = vec![ScreenFacts::new("eDP-1", 120.0, 2.0)];
        let c = content(Category::Displays, &r, &x);
        assert!(c.items[0].detail.contains(&kv("Refresh rate", "120 Hz")));
        assert!(c.items[0].detail.contains(&kv("Scale", "2x")));
        assert!(!c.items[1].detail.iter().any(|(k, _)| k == "Scale"), "a display the service did not list gets no scale");
    }

    #[test]
    fn scale_labels_read_as_the_desktop_writes_them() {
        assert_eq!(scale_label(100), "1x");
        assert_eq!(scale_label(200), "2x");
        assert_eq!(scale_label(125), "1.25x");
        assert_eq!(scale_label(150), "1.5x");
    }

    #[test]
    fn a_screen_matches_its_connector_by_name_and_an_empty_name_matches_nothing() {
        let screens = [ScreenFacts::new("", 60.0, 1.0), ScreenFacts::new("Built-in", 60.0, 1.0)];
        assert!(screen_for("eDP-1", &screens).is_none());
        let screens = [ScreenFacts::new("eDP-1", 144.0, 1.0)];
        assert_eq!(screen_for("eDP-1", &screens).map(|s| s.refresh_hz), Some(144));
        assert_eq!(screen_for("edp-1", &screens).map(|s| s.refresh_hz), Some(144), "case does not matter");
    }

    #[test]
    fn the_new_hardware_classes_list_their_devices() {
        let mut x = Extras::default();
        x.hw.card_readers = vec![probe::CardSlot { host: "mmc0".into(), driver: "sdhci".into(), card: "SD64G".into(), kind: "SD".into(), manufacturer: "0x03".into() }, probe::CardSlot { host: "mmc1".into(), ..Default::default() }];
        x.hw.scsi = vec![probe::ScsiHost { name: "host0".into(), proc_name: "ahci".into(), ..Default::default() }, probe::ScsiHost { name: "host1".into(), proc_name: "mpt3sas".into(), transport: "SAS".into(), ..Default::default() }];
        x.hw.sas_phys = vec![probe::SasPhy { name: "phy-1:0".into(), link_rate: "6.0 Gbit".into(), device: "end device".into(), address: "0x5000".into() }];
        x.hw.fc_ports = vec![probe::FcPort { host: "host2".into(), symbolic: "HBA 1".into(), state: "Online".into(), speed: "8 Gbit".into(), ..Default::default() }];
        x.hw.optical = vec![probe::OpticalDrive { device: "sr0".into(), model: "DVD-RW".into(), media_bytes: 4_700_000_000, ..Default::default() }];
        x.hw.diagnostics = vec![probe::DiagnosticReport { name: "core.app.1".into(), bytes: 2048, day: 20_000 }];

        let slots = content(Category::CardReader, &Report::default(), &x);
        assert_eq!(slots.items[0].title, "SD64G", "a card is named by the card");
        assert_eq!(slots.items[1].title, "mmc1", "an empty slot is named by its host");
        assert!(slots.items[1].summary.ends_with("Empty slot"));

        let controllers = content(Category::Controller, &Report::default(), &x);
        assert_eq!(controllers.items.len(), 1, "the SAS adapter lists under SAS, not here");
        assert_eq!(controllers.items[0].title, "ahci");
        assert_eq!(content(Category::Sas, &Report::default(), &x).items[0].summary, "6.0 Gbit · end device");
        assert_eq!(content(Category::FibreChannel, &Report::default(), &x).items[0].title, "HBA 1");

        let discs = content(Category::DiscBurning, &Report::default(), &x);
        assert!(discs.items[0].detail.contains(&kv("Media", "Disc inserted")));
        assert!(discs.items[0].detail.contains(&kv("Disc size", "4.7 GB")));

        let reports = content(Category::Diagnostics, &Report::default(), &x);
        assert_eq!(reports.items[0].title, "core.app.1");
        assert_eq!(reports.items[0].detail.last(), Some(&kv("Date", "2024-10-04")));
    }

    #[test]
    fn hardware_default_leaves_every_hardware_list_empty() {
        let x = Extras { hw: Hardware::default(), ..Default::default() };
        for c in [
            Category::Audio,
            Category::Bluetooth,
            Category::Camera,
            Category::CardReader,
            Category::Controller,
            Category::Printers,
            Category::Thunderbolt,
            Category::Nvme,
            Category::Sata,
            Category::Sas,
            Category::FibreChannel,
            Category::DiscBurning,
            Category::Diagnostics,
            Category::Volumes,
            Category::Wifi,
        ] {
            assert!(content(c, &Report::default(), &x).empty, "{:?}", c);
        }
    }
}
