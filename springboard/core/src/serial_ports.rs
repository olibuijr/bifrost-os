//! Serial port model: which tty nodes are real serial ports, with their kind and index.
//!
//! Pure: `ports_from` takes the node names with a flag saying whether the kernel links the node to a device.
//! `scan_in` reads a `/dev` directory and the tty class directory to get those flags. Nothing is opened. The
//! official serial port enumeration returns a dial-in and a call-out path per port; Linux gives one node per port,
//! so each port has one `path`. Opening a modem (the official serial port sample) is not modelled here.
use std::fs;
use std::path::Path;

/// The driver family of a port, from the node name prefix.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SerialKind {
    /// Onboard 16550-style UART (`ttyS`).
    Uart,
    /// USB to serial adapter (`ttyUSB`).
    UsbSerial,
    /// USB modem or CDC-ACM device (`ttyACM`).
    UsbModem,
    /// UART on an ARM system-on-chip (`ttyAMA`).
    SocUart,
}

const KINDS: [SerialKind; 4] = [SerialKind::Uart, SerialKind::UsbSerial, SerialKind::UsbModem, SerialKind::SocUart];

impl SerialKind {
    pub fn prefix(self) -> &'static str {
        match self {
            SerialKind::Uart => "ttyS",
            SerialKind::UsbSerial => "ttyUSB",
            SerialKind::UsbModem => "ttyACM",
            SerialKind::SocUart => "ttyAMA",
        }
    }
}

/// The kind and index of a node name such as `ttyS0` or `ttyUSB12`. Names with no digits after the prefix, and
/// other nodes such as `tty0` or `console`, are not serial ports.
pub fn classify(name: &str) -> Option<(SerialKind, u32)> {
    for kind in KINDS {
        if let Some(rest) = name.strip_prefix(kind.prefix()) {
            if !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()) {
                return rest.parse::<u32>().ok().map(|n| (kind, n));
            }
        }
    }
    None
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SerialPort {
    pub kind: SerialKind,
    pub index: u32,
    /// The node name, for example `ttyUSB0`.
    pub name: String,
    /// The device node, for example `/dev/ttyUSB0`.
    pub path: String,
}

/// Ports among `nodes` (name, linked to a device). Unlinked nodes are dropped, so a `ttyS` node with no hardware
/// behind it is not reported. The result is sorted by kind, then by index.
pub fn ports_from(nodes: &[(&str, bool)]) -> Vec<SerialPort> {
    let mut out: Vec<SerialPort> = nodes
        .iter()
        .filter(|(_, linked)| *linked)
        .filter_map(|(name, _)| {
            classify(name).map(|(kind, index)| SerialPort {
                kind,
                index,
                name: (*name).to_owned(),
                path: format!("/dev/{name}"),
            })
        })
        .collect();
    out.sort_by_key(|a| (a.kind, a.index));
    out
}

/// Serial ports from a `/dev` directory. A node counts when `tty_class/<name>/device` exists, which the kernel
/// makes only for hardware. A missing `/dev` gives an empty list.
pub fn scan_in(dev: &Path, tty_class: &Path) -> Vec<SerialPort> {
    let Ok(dir) = fs::read_dir(dev) else { return Vec::new() };
    let names: Vec<String> = dir
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| classify(n).is_some())
        .collect();
    let nodes: Vec<(&str, bool)> = names.iter().map(|n| (n.as_str(), tty_class.join(n).join("device").exists())).collect();
    ports_from(&nodes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_reads_kind_and_index() {
        assert_eq!(classify("ttyS0"), Some((SerialKind::Uart, 0)));
        assert_eq!(classify("ttyUSB12"), Some((SerialKind::UsbSerial, 12)));
        assert_eq!(classify("ttyACM1"), Some((SerialKind::UsbModem, 1)));
        assert_eq!(classify("ttyAMA0"), Some((SerialKind::SocUart, 0)));
        assert_eq!(classify("ttyS007"), Some((SerialKind::Uart, 7)));
    }

    #[test]
    fn classify_rejects_other_nodes() {
        assert_eq!(classify("ttyS"), None);
        assert_eq!(classify("ttyUSBx"), None);
        assert_eq!(classify("tty0"), None);
        assert_eq!(classify("console"), None);
        assert_eq!(classify("ttyS1a"), None);
    }

    #[test]
    fn unlinked_nodes_are_not_ports() {
        let ports = ports_from(&[("ttyS0", false), ("ttyUSB0", true)]);
        assert_eq!(ports.len(), 1);
        assert_eq!(ports[0].name, "ttyUSB0");
        assert_eq!(ports[0].path, "/dev/ttyUSB0");
    }

    #[test]
    fn ports_sort_by_kind_then_numeric_index() {
        let ports = ports_from(&[("ttyUSB1", true), ("ttyS10", true), ("ttyS2", true), ("ttyUSB0", true)]);
        let names: Vec<&str> = ports.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["ttyS2", "ttyS10", "ttyUSB0", "ttyUSB1"]);
        assert_eq!(ports[1].index, 10);
        assert_eq!(ports[1].kind, SerialKind::Uart);
    }

    #[test]
    fn missing_dev_directory_gives_no_ports() {
        assert!(scan_in(Path::new("/nonexistent/bifrost-dev"), Path::new("/nonexistent/bifrost-tty")).is_empty());
    }
}
