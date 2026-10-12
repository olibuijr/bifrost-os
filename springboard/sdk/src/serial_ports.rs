//! Serial ports for apps: the real serial nodes the system exposes (onboard UARTs, USB serial adapters, USB modems),
//! with a lookup by name and by kind. Discovery lives in `bifrost_core::serial_ports`. Opening a port and driving a
//! modem are not offered here yet.
pub use bifrost_core::serial_ports::{classify, SerialKind, SerialPort};
use bifrost_core::serial_ports;
use std::path::Path;

/// The device directory and the kernel's tty class directory.
pub const DEV_DIR: &str = "/dev";
pub const TTY_CLASS: &str = "/sys/class/tty";

/// Serial ports under `dev`, using `tty_class` to see which nodes have hardware behind them.
pub fn ports_in(dev: &Path, tty_class: &Path) -> Vec<SerialPort> {
    serial_ports::scan_in(dev, tty_class)
}

/// Every serial port the system exposes, sorted by kind and index.
pub fn ports() -> Vec<SerialPort> {
    ports_in(Path::new(DEV_DIR), Path::new(TTY_CLASS))
}

/// The port whose node name is `name`, such as `ttyUSB0`.
pub fn port_named<'a>(ports: &'a [SerialPort], name: &str) -> Option<&'a SerialPort> {
    ports.iter().find(|p| p.name == name)
}

/// The ports of one kind, in the order given.
pub fn of_kind(ports: &[SerialPort], kind: SerialKind) -> Vec<&SerialPort> {
    ports.iter().filter(|p| p.kind == kind).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<SerialPort> {
        serial_ports::ports_from(&[("ttyS0", true), ("ttyUSB0", true), ("ttyUSB1", true), ("ttyACM0", true)])
    }

    #[test]
    fn port_named_finds_the_node() {
        let all = sample();
        let found = port_named(&all, "ttyUSB1").expect("ttyUSB1 is in the list");
        assert_eq!(found.path, "/dev/ttyUSB1");
        assert_eq!(found.kind, SerialKind::UsbSerial);
    }

    #[test]
    fn port_named_is_none_for_an_absent_node() {
        let all = sample();
        assert!(port_named(&all, "ttyS9").is_none());
        assert!(port_named(&[], "ttyS0").is_none());
    }

    #[test]
    fn of_kind_keeps_only_that_kind_in_order() {
        let all = sample();
        let usb: Vec<&str> = of_kind(&all, SerialKind::UsbSerial).iter().map(|p| p.name.as_str()).collect();
        assert_eq!(usb, vec!["ttyUSB0", "ttyUSB1"]);
    }

    #[test]
    fn of_kind_is_empty_when_no_port_matches() {
        let all = sample();
        assert!(of_kind(&all, SerialKind::SocUart).is_empty());
    }

    #[test]
    fn ports_in_a_missing_dev_directory_is_empty() {
        let found = ports_in(Path::new("/nonexistent/bifrost-dev"), Path::new("/nonexistent/bifrost-tty"));
        assert!(found.is_empty());
    }
}
