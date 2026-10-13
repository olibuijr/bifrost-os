//! The report behind `system_profiler`: the hardware, software, network, storage, display, audio and USB data types as
//! trees of named items with their fields, printed as text or JSON. The facts come from `bifrost_core::sysinfo`, the same
//! readers the System Information app uses, and the audio cards from `/proc/asound/cards`.
//! The detail level decides what is listed: `mini` leaves out the serial number, addresses and the other facts that
//! identify the machine's owner; `basic` is the default; `full` adds the device lists (PCI, power, partitions, apps).

use bifrost_core::sysinfo::{clock_label, size_label, usb_speed, Report, Roots};
use std::fs;
use std::path::Path;

pub const USAGE: &str = "usage: system_profiler [-listDataTypes] [-json] [-detailLevel mini|basic|full] [-timeout seconds] [dataType ...]";
/// The sound cards the kernel lists.
pub const AUDIO_CARDS: &str = "/proc/asound/cards";

/// The data types the report has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DataType {
    Hardware,
    Software,
    Network,
    Storage,
    Displays,
    Audio,
    Usb,
}

/// Every data type, in report order.
pub const ALL: [DataType; 7] = [DataType::Hardware, DataType::Software, DataType::Network, DataType::Storage, DataType::Displays, DataType::Audio, DataType::Usb];

impl DataType {
    /// The identifier the command line and the JSON keys use.
    pub fn id(self) -> &'static str {
        match self {
            DataType::Hardware => "SPHardwareDataType",
            DataType::Software => "SPSoftwareDataType",
            DataType::Network => "SPNetworkDataType",
            DataType::Storage => "SPStorageDataType",
            DataType::Displays => "SPDisplaysDataType",
            DataType::Audio => "SPAudioDataType",
            DataType::Usb => "SPUSBDataType",
        }
    }

    /// The heading of the type in the text report.
    pub fn title(self) -> &'static str {
        match self {
            DataType::Hardware => "Hardware",
            DataType::Software => "Software",
            DataType::Network => "Network",
            DataType::Storage => "Storage",
            DataType::Displays => "Displays",
            DataType::Audio => "Audio",
            DataType::Usb => "USB",
        }
    }

    /// A data type by its identifier or its heading, ignoring case.
    pub fn parse(name: &str) -> Option<DataType> {
        ALL.iter().copied().find(|t| t.id().eq_ignore_ascii_case(name) || t.title().eq_ignore_ascii_case(name))
    }
}

/// How much the report lists.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Detail {
    Mini,
    #[default]
    Basic,
    Full,
}

impl Detail {
    pub fn parse(level: &str) -> Option<Detail> {
        match level {
            "mini" => Some(Detail::Mini),
            "basic" => Some(Detail::Basic),
            "full" => Some(Detail::Full),
            _ => None,
        }
    }
}

/// The parsed command line.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Options {
    pub help: bool,
    pub list: bool,
    pub json: bool,
    pub detail: Detail,
    /// The types asked for, in the order given. Empty means every type.
    pub types: Vec<DataType>,
}

/// Reads the arguments (without the program name). `-xml` is refused; `-timeout` is accepted and has no effect.
pub fn parse(args: &[String]) -> Result<Options, String> {
    let mut o = Options::default();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-usage" | "-help" | "--help" | "-h" => o.help = true,
            "-listDataTypes" => o.list = true,
            "-json" => o.json = true,
            "-xml" => return Err("XML reports are not supported; use -json".to_string()),
            "-detailLevel" => {
                let level = it.next().ok_or("-detailLevel needs mini, basic or full")?;
                o.detail = Detail::parse(level).ok_or_else(|| format!("unknown detail level {level}"))?;
            }
            "-timeout" => {
                let seconds = it.next().ok_or("-timeout needs a number of seconds")?;
                seconds.parse::<u64>().map_err(|_| format!("-timeout needs a number of seconds, not {seconds}"))?;
            }
            other if other.starts_with('-') => return Err(format!("unknown option {other}")),
            name => o.types.push(DataType::parse(name).ok_or_else(|| format!("unknown data type {name}"))?),
        }
    }
    Ok(o)
}

/// A sound card from `/proc/asound/cards`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AudioCard {
    pub index: u32,
    pub id: String,
    pub driver: String,
    pub name: String,
}

/// The card lines of `/proc/asound/cards`: "0 [PCH  ]: HDA-Intel - HDA Intel PCH". The indented lines under them are
/// skipped.
pub fn parse_audio_cards(text: &str) -> Vec<AudioCard> {
    let mut cards = Vec::new();
    for line in text.lines() {
        let Some((num, rest)) = line.trim_start().split_once(' ') else { continue };
        let Ok(index) = num.parse::<u32>() else { continue };
        let Some(rest) = rest.trim_start().strip_prefix('[') else { continue };
        let Some((id, after)) = rest.split_once(']') else { continue };
        let Some(after) = after.strip_prefix(": ") else { continue };
        let (driver, name) = after.split_once(" - ").unwrap_or(("", after));
        cards.push(AudioCard { index, id: id.trim().to_string(), driver: driver.trim().to_string(), name: name.trim().to_string() });
    }
    cards
}

/// What `system_profiler` reads: the system's facts and its audio cards.
pub struct Collected {
    pub report: Report,
    pub audio: Vec<AudioCard>,
}

/// Reads the facts of this machine.
pub fn collect() -> Collected {
    let roots = Roots::system();
    Collected { report: Report::collect(&roots), audio: parse_audio_cards(&fs::read_to_string(Path::new(AUDIO_CARDS)).unwrap_or_default()) }
}

/// A named item of the report with its fields and the items under it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Node {
    pub name: String,
    pub fields: Vec<(String, String)>,
    pub children: Vec<Node>,
}

impl Node {
    pub fn named(name: impl Into<String>) -> Node {
        Node { name: name.into(), ..Default::default() }
    }

    pub fn field(mut self, key: &str, value: impl Into<String>) -> Node {
        self.fields.push((key.to_string(), value.into()));
        self
    }

    pub fn with(mut self, children: Vec<Node>) -> Node {
        self.children = children;
        self
    }
}

fn yes_no(b: bool) -> &'static str {
    if b {
        "Yes"
    } else {
        "No"
    }
}

fn or_dash(text: &str) -> String {
    if text.trim().is_empty() {
        "—".to_string()
    } else {
        text.trim().to_string()
    }
}

/// The items of one data type at a detail level.
pub fn items(t: DataType, r: &Report, audio: &[AudioCard], d: Detail) -> Vec<Node> {
    match t {
        DataType::Hardware => hardware(r, d),
        DataType::Software => software(r, d),
        DataType::Network => network(r, d),
        DataType::Storage => storage(r, d),
        DataType::Displays => displays(r),
        DataType::Audio => audio.iter().map(audio_card).collect(),
        DataType::Usb => usb(r, d),
    }
}

fn hardware(r: &Report, d: Detail) -> Vec<Node> {
    let model = if r.firmware.product.is_empty() { &r.firmware.board } else { &r.firmware.product };
    let mut overview = Node::named("Hardware Overview")
        .field("Model Name", or_dash(model))
        .field("Vendor", or_dash(&r.firmware.vendor))
        .field("Form Factor", or_dash(&r.firmware.chassis))
        .field("Processor Name", or_dash(&r.cpu.model))
        .field("Logical Processors", r.cpu.logical.to_string())
        .field("Processor Speed", clock_label(r.cpu.mhz))
        .field("Memory", size_label(r.memory_bytes, 1 << 30, "GB"))
        .field("Firmware Version", or_dash(&r.firmware.bios));
    if d != Detail::Mini {
        overview = overview.field("Serial Number", or_dash(&r.firmware.serial));
    }
    let graphics = r.graphics().iter().enumerate().map(|(i, g)| Node::named(format!("Graphics {}", i + 1)).field("Vendor", g.vendor_name()).field("Driver", driver_or_none(&g.driver))).collect();
    let mut out = vec![overview, Node::named("Graphics").with(graphics)];
    if d == Detail::Full {
        let pci = r.pci.iter().map(|p| Node::named(p.address.clone()).field("Vendor", p.vendor_name()).field("Class", p.class_name()).field("Driver", driver_or_none(&p.driver))).collect();
        out.push(Node::named("PCI Devices").with(pci));
        let power = r.power.iter().map(power_source).collect();
        out.push(Node::named("Power").with(power));
    }
    out
}

fn driver_or_none(driver: &str) -> &str {
    if driver.is_empty() {
        "none"
    } else {
        driver
    }
}

fn power_source(p: &bifrost_core::sysinfo::PowerSource) -> Node {
    let name = if p.model.is_empty() { p.name.clone() } else { p.model.clone() };
    let mut node = Node::named(name).field("Type", or_dash(&p.kind)).field("Status", or_dash(&p.status));
    if let Some(c) = p.capacity_percent {
        node = node.field("Charge", format!("{c}%"));
    }
    if let Some(h) = p.health_percent() {
        node = node.field("Health", format!("{h}%"));
    }
    if let Some(c) = p.cycles {
        node = node.field("Cycle Count", c.to_string());
    }
    node
}

fn software(r: &Report, d: Detail) -> Vec<Node> {
    let version = format!("{} {}", r.os_name, r.os_version).trim().to_string();
    let mut out = vec![Node::named("System Software Overview").field("System Version", or_dash(&version)).field("Kernel Version", or_dash(&r.kernel))];
    if d == Detail::Full {
        let apps = r.apps.iter().map(|a| Node::named(a.id.clone()).field("Version", a.version.clone())).collect();
        out.push(Node::named("Installed Apps").with(apps));
    }
    out
}

fn network(r: &Report, d: Detail) -> Vec<Node> {
    r.net
        .iter()
        .map(|n| {
            let kind = if n.wireless { "Wi-Fi" } else { "Ethernet" };
            let mut node = Node::named(n.name.clone()).field("Type", kind).field("Status", capitalize(&n.state)).field("Default Route", yes_no(n.default_route));
            if d != Detail::Mini {
                node = node.field("MAC Address", or_dash(&n.mac));
                if !n.addresses.is_empty() {
                    node = node.field("IPv6 Addresses", n.addresses.join(", "));
                }
            }
            node
        })
        .collect()
}

fn storage(r: &Report, d: Detail) -> Vec<Node> {
    r.disks
        .iter()
        .map(|disk| {
            let name = if disk.model.is_empty() { disk.name.clone() } else { disk.model.clone() };
            let mut node = Node::named(name)
                .field("Device Name", disk.name.clone())
                .field("Capacity", size_label(disk.size_bytes, 1_000_000_000, "GB"))
                .field("Medium", if disk.rotational { "HDD" } else { "SSD" })
                .field("Removable", yes_no(disk.removable))
                .field("Partitions", disk.partitions.len().to_string());
            if d == Detail::Full {
                node = node.with(disk.partitions.iter().map(|p| Node::named(p.clone())).collect());
            }
            node
        })
        .collect()
}

fn displays(r: &Report) -> Vec<Node> {
    r.displays
        .iter()
        .map(|d| Node::named(d.connector.clone()).field("Connected", yes_no(d.connected)).field("Resolution", or_dash(&d.mode)))
        .collect()
}

fn audio_card(c: &AudioCard) -> Node {
    Node::named(c.name.clone()).field("Card Index", c.index.to_string()).field("Driver", or_dash(&c.driver)).field("Device ID", or_dash(&c.id))
}

fn usb(r: &Report, d: Detail) -> Vec<Node> {
    r.usb
        .iter()
        .map(|u| {
            let name = if u.product.is_empty() { format!("Device {:04x}:{:04x}", u.vendor_id, u.product_id) } else { u.product.clone() };
            let mut node = Node::named(name)
                .field("Vendor ID", format!("0x{:04x}", u.vendor_id))
                .field("Product ID", format!("0x{:04x}", u.product_id))
                .field("Speed", usb_speed(u.speed_mbps))
                .field("Location", u.port.clone());
            if d != Detail::Mini {
                node = node.field("Manufacturer", or_dash(&u.manufacturer));
            }
            node
        })
        .collect()
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => "Unknown".to_string(),
    }
}

/// The text report: a heading for each data type, then its items indented under it.
pub fn render_text(sections: &[(DataType, Vec<Node>)]) -> String {
    let mut out = String::new();
    for (t, items) in sections {
        out.push_str(&format!("{}:\n\n", t.title()));
        for item in items {
            write_node(&mut out, item, 1);
        }
        out.push('\n');
    }
    out
}

fn write_node(out: &mut String, node: &Node, depth: usize) {
    let pad = "    ".repeat(depth);
    out.push_str(&format!("{pad}{}:\n\n", node.name));
    for (k, v) in &node.fields {
        out.push_str(&format!("{pad}  {k}: {v}\n"));
    }
    if !node.fields.is_empty() {
        out.push('\n');
    }
    for child in &node.children {
        write_node(out, child, depth + 1);
    }
}

/// The JSON report: an object with one array per data type. Each item has `_name`, its fields, and `_items` when it has
/// children.
pub fn render_json(sections: &[(DataType, Vec<Node>)]) -> String {
    let parts: Vec<String> = sections
        .iter()
        .map(|(t, items)| format!("{}:[{}]", quote(t.id()), items.iter().map(node_json).collect::<Vec<_>>().join(",")))
        .collect();
    format!("{{{}}}\n", parts.join(","))
}

fn node_json(node: &Node) -> String {
    let mut parts = vec![format!("\"_name\":{}", quote(&node.name))];
    for (k, v) in &node.fields {
        parts.push(format!("{}:{}", quote(k), quote(v)));
    }
    if !node.children.is_empty() {
        parts.push(format!("\"_items\":[{}]", node.children.iter().map(node_json).collect::<Vec<_>>().join(",")));
    }
    format!("{{{}}}", parts.join(","))
}

/// A JSON string literal.
pub fn quote(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_core::sysinfo::{Disk, Display, NetInterface, UsbDevice};

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn sample() -> Report {
        let mut r = Report::default();
        r.firmware.product = "Laptop 13".into();
        r.firmware.serial = "SN123".into();
        r.cpu.model = "Intel Core".into();
        r.cpu.logical = 8;
        r.cpu.mhz = 1800.0;
        r.memory_bytes = 16 << 30;
        r.net.push(NetInterface { name: "wlan0".into(), wireless: true, state: "up".into(), mac: "aa:bb".into(), default_route: true, addresses: vec!["2001:db8::1".into()], ..Default::default() });
        r.disks.push(Disk { name: "nvme0n1".into(), model: "SSD 980".into(), size_bytes: 500_000_000_000, partitions: vec!["nvme0n1p1".into()], ..Default::default() });
        r.displays.push(Display { connector: "eDP-1".into(), connected: true, mode: "1920x1080".into() });
        r.usb.push(UsbDevice { port: "1-1".into(), vendor_id: 0x046d, product_id: 0xc52b, product: "Receiver".into(), manufacturer: "Logitech".into(), speed_mbps: 12 });
        r.os_name = "BifrOSt".into();
        r.os_version = "0.6".into();
        r.kernel = "6.1.0".into();
        r
    }

    #[test]
    fn options_read_types_levels_and_flags() {
        let o = parse(&args(&["-json", "-detailLevel", "full", "hardware", "SPUSBDataType"])).unwrap();
        assert!(o.json);
        assert_eq!(o.detail, Detail::Full);
        assert_eq!(o.types, [DataType::Hardware, DataType::Usb]);
        assert!(parse(&args(&["-listDataTypes"])).unwrap().list);
        assert_eq!(parse(&args(&["-timeout", "0"])).unwrap().types, []);
    }

    #[test]
    fn bad_options_are_named() {
        assert!(parse(&args(&["-xml"])).unwrap_err().contains("-json"));
        assert!(parse(&args(&["-detailLevel", "huge"])).unwrap_err().contains("unknown detail level"));
        assert!(parse(&args(&["Nope"])).unwrap_err().contains("unknown data type"));
        assert!(parse(&args(&["-frob"])).unwrap_err().contains("unknown option"));
    }

    #[test]
    fn every_data_type_has_an_id_that_parses_back() {
        for t in ALL {
            assert_eq!(DataType::parse(t.id()), Some(t));
            assert_eq!(DataType::parse(t.title()), Some(t));
        }
    }

    #[test]
    fn mini_hides_the_serial_number_and_the_addresses() {
        let r = sample();
        let mini = items(DataType::Hardware, &r, &[], Detail::Mini);
        assert!(!mini[0].fields.iter().any(|(k, _)| k == "Serial Number"));
        let basic = items(DataType::Hardware, &r, &[], Detail::Basic);
        assert!(basic[0].fields.contains(&("Serial Number".to_string(), "SN123".to_string())));
        let net_mini = items(DataType::Network, &r, &[], Detail::Mini);
        assert_eq!(net_mini[0].fields.iter().filter(|(k, _)| k == "MAC Address").count(), 0);
        let net_basic = items(DataType::Network, &r, &[], Detail::Basic);
        assert!(net_basic[0].fields.contains(&("IPv6 Addresses".to_string(), "2001:db8::1".to_string())));
    }

    #[test]
    fn full_adds_partitions_and_device_lists() {
        let r = sample();
        let storage = items(DataType::Storage, &r, &[], Detail::Full);
        assert_eq!(storage[0].children[0].name, "nvme0n1p1");
        assert!(items(DataType::Storage, &r, &[], Detail::Basic)[0].children.is_empty());
        let hw = items(DataType::Hardware, &r, &[], Detail::Full);
        assert!(hw.iter().any(|n| n.name == "PCI Devices"));
    }

    #[test]
    fn sizes_and_speeds_use_the_shared_labels() {
        let r = sample();
        let hw = items(DataType::Hardware, &r, &[], Detail::Basic);
        assert!(hw[0].fields.contains(&("Memory".to_string(), "16 GB".to_string())));
        assert!(hw[0].fields.contains(&("Processor Speed".to_string(), "1.80 GHz".to_string())));
        let usb = items(DataType::Usb, &r, &[], Detail::Basic);
        assert_eq!(usb[0].name, "Receiver");
        assert!(usb[0].fields.contains(&("Speed".to_string(), "Full speed".to_string())));
    }

    #[test]
    fn audio_cards_parse_from_the_kernel_listing() {
        let text = " 0 [PCH            ]: HDA-Intel - HDA Intel PCH\n                      HDA Intel PCH at 0xf7c10000 irq 33\n 1 [NVidia         ]: HDA-Intel - HDA NVidia\n";
        let cards = parse_audio_cards(text);
        assert_eq!(cards.len(), 2);
        assert_eq!(cards[0], AudioCard { index: 0, id: "PCH".into(), driver: "HDA-Intel".into(), name: "HDA Intel PCH".into() });
        assert_eq!(cards[1].index, 1);
        assert!(parse_audio_cards("").is_empty());
    }

    #[test]
    fn text_report_indents_items_under_their_heading() {
        let node = Node::named("Hardware Overview").field("Model Name", "Laptop 13");
        let text = render_text(&[(DataType::Hardware, vec![node])]);
        assert_eq!(text, "Hardware:\n\n    Hardware Overview:\n\n      Model Name: Laptop 13\n\n\n");
    }

    #[test]
    fn json_report_keys_each_type_and_nests_children() {
        let parent = Node::named("Disk").field("Capacity", "500 GB").with(vec![Node::named("p1")]);
        let json = render_json(&[(DataType::Storage, vec![parent])]);
        assert_eq!(json, "{\"SPStorageDataType\":[{\"_name\":\"Disk\",\"Capacity\":\"500 GB\",\"_items\":[{\"_name\":\"p1\"}]}]}\n");
    }

    #[test]
    fn json_strings_are_escaped() {
        assert_eq!(quote("a\"b\\c\nd\u{1}"), "\"a\\\"b\\\\c\\nd\\u0001\"");
    }
}
