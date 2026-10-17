//! The bus source for the accessory manager: reads the connected USB devices from sysfs
//! (`/sys/bus/usb/devices`) and turns them into [`Accessory`] records, so a poll of the bus reports real attach and
//! detach to the pure [`AccessoryManager`].
//!
//! Each device directory carries the device descriptor fields as files (`idVendor`, `bDeviceClass`, `bcdDevice`,
//! ...), the manufacturer, product and serial strings, and one child directory per interface
//! (`<port>:<configuration>.<interface>`) with the interface class triple. Those are enough to rebuild the device
//! descriptor and a configuration with its interfaces, which is what [`MatchingCriteria`] needs to match a real
//! accessory. Endpoints are not read (the model only matches on device and interface fields).
//!
//! It only reads files below the directory the caller names, so tests point it at a temporary tree. Root hubs
//! (`usbN`) are skipped, as are interface directories at the top level.
//!
//! The registry id is `(busnum << 16) | devnum`, the identity the kernel gives a device for as long as it stays
//! plugged in; a re-plug gets a new devnum and so a new registry id.

use crate::accessory_manager::{Accessory, AccessoryManager, SyncReport};
use crate::usb_descriptors::{ConfigurationDescriptor, DeviceDescriptor, InterfaceDescriptor};
use std::fs;
use std::path::Path;

/// The kernel's USB device directory.
pub const SYSFS_USB_DEVICES: &str = "/sys/bus/usb/devices";

fn read_trim(path: &Path) -> String {
    fs::read_to_string(path).map(|s| s.trim().to_string()).unwrap_or_default()
}

fn hex(path: &Path) -> Option<u32> {
    u32::from_str_radix(read_trim(path).trim_start_matches("0x"), 16).ok()
}

fn dec(path: &Path) -> Option<u32> {
    read_trim(path).parse().ok()
}

fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(dir)
        .map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect())
        .unwrap_or_default();
    v.sort();
    v
}

/// `bcdUSB` from the `version` attribute, which sysfs writes as " 2.00" or " 3.10" (major, then the BCD minor).
pub fn parse_bcd_version(text: &str) -> Option<u16> {
    let (major, minor) = text.trim().split_once('.')?;
    let major: u16 = major.trim().parse().ok()?;
    let minor = u16::from_str_radix(minor.trim(), 16).ok()?;
    (major <= 0xFF && minor <= 0xFF).then_some((major << 8) | minor)
}

/// `bMaxPower` as milliamps from the `bMaxPower` attribute ("100mA").
fn max_power_ma(text: &str) -> u16 {
    text.trim().trim_end_matches("mA").trim().parse().unwrap_or(0)
}

fn interfaces(device_dir: &Path, port: &str) -> Vec<InterfaceDescriptor> {
    let prefix = format!("{port}:");
    names(device_dir)
        .into_iter()
        .filter(|n| n.starts_with(&prefix))
        .filter_map(|n| {
            let dir = device_dir.join(&n);
            Some(InterfaceDescriptor {
                number: hex(&dir.join("bInterfaceNumber"))? as u8,
                alternate: hex(&dir.join("bAlternateSetting")).unwrap_or(0) as u8,
                class: hex(&dir.join("bInterfaceClass")).unwrap_or(0) as u8,
                subclass: hex(&dir.join("bInterfaceSubClass")).unwrap_or(0) as u8,
                protocol: hex(&dir.join("bInterfaceProtocol")).unwrap_or(0) as u8,
                endpoints: Vec::new(),
            })
        })
        .collect()
}

/// One device directory as an accessory; `None` for a directory that is not a device (no ids, no bus number).
pub fn read_accessory(dir: &Path, port: &str) -> Option<Accessory> {
    let vendor_id = hex(&dir.join("idVendor"))? as u16;
    let product_id = hex(&dir.join("idProduct"))? as u16;
    let registry_id = ((dec(&dir.join("busnum"))? as u64) << 16) | dec(&dir.join("devnum"))? as u64;
    let device = DeviceDescriptor {
        usb_version: parse_bcd_version(&read_trim(&dir.join("version"))).unwrap_or(0x0200),
        class: hex(&dir.join("bDeviceClass")).unwrap_or(0) as u8,
        subclass: hex(&dir.join("bDeviceSubClass")).unwrap_or(0) as u8,
        protocol: hex(&dir.join("bDeviceProtocol")).unwrap_or(0) as u8,
        max_packet_size0: dec(&dir.join("bMaxPacketSize0")).unwrap_or(0) as u8,
        vendor_id,
        product_id,
        device_version: hex(&dir.join("bcdDevice")).unwrap_or(0) as u16,
        configurations: dec(&dir.join("bNumConfigurations")).unwrap_or(1) as u8,
    };
    let mut accessory = Accessory::new(registry_id, device);
    let interfaces = interfaces(dir, port);
    if !interfaces.is_empty() {
        accessory.configuration = Some(ConfigurationDescriptor {
            value: dec(&dir.join("bConfigurationValue")).unwrap_or(1) as u8,
            attributes: hex(&dir.join("bmAttributes")).unwrap_or(0) as u8,
            max_power_ma: max_power_ma(&read_trim(&dir.join("bMaxPower"))),
            interfaces,
        });
    }
    accessory.manufacturer = read_trim(&dir.join("manufacturer"));
    accessory.name = read_trim(&dir.join("product"));
    accessory.model_number = accessory.name.clone();
    accessory.serial_number = read_trim(&dir.join("serial"));
    Some(accessory)
}

/// Every USB device below `root` as an accessory, in port-name order. Root hubs and interface directories are
/// skipped; a missing `root` gives an empty snapshot (the bus is unreadable, so nothing is connected).
pub fn read_snapshot(root: &Path) -> Vec<Accessory> {
    names(root)
        .into_iter()
        .filter(|p| !p.contains(':') && !p.starts_with("usb"))
        .filter_map(|p| read_accessory(&root.join(&p), &p))
        .collect()
}

/// One poll: reads the bus and brings `manager` in line with it. The report lists the connection ids that
/// changed, and listeners find the matching events queued on the manager.
pub fn poll(manager: &mut AccessoryManager, root: &Path) -> SyncReport {
    manager.sync(read_snapshot(root))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accessory_manager::Event;
    use crate::usb_descriptors::MatchingCriteria;

    fn put(dir: &Path, name: &str, text: &str) {
        fs::write(dir.join(name), format!("{text}\n")).unwrap();
    }

    /// A device with one HID-class interface under `root/<port>`.
    fn plug(root: &Path, port: &str, vid: &str, pid: &str, dev: u32) {
        let d = root.join(port);
        fs::create_dir_all(d.join(format!("{port}:1.0"))).unwrap();
        put(&d, "idVendor", vid);
        put(&d, "idProduct", pid);
        put(&d, "busnum", "1");
        put(&d, "devnum", &dev.to_string());
        put(&d, "version", " 2.00");
        put(&d, "bDeviceClass", "00");
        put(&d, "bMaxPacketSize0", "64");
        put(&d, "bNumConfigurations", "1");
        put(&d, "bcdDevice", "0102");
        put(&d, "bmAttributes", "a0");
        put(&d, "bMaxPower", "100mA");
        put(&d, "manufacturer", "Acme");
        put(&d, "product", "Widget");
        put(&d, "serial", "S1");
        let i = d.join(format!("{port}:1.0"));
        put(&i, "bInterfaceNumber", "00");
        put(&i, "bInterfaceClass", "03");
        put(&i, "bInterfaceSubClass", "01");
        put(&i, "bInterfaceProtocol", "02");
    }

    #[test]
    fn bcd_version_reads_the_sysfs_form() {
        assert_eq!(parse_bcd_version(" 2.00"), Some(0x0200));
        assert_eq!(parse_bcd_version(" 3.10"), Some(0x0310));
        assert_eq!(parse_bcd_version("junk"), None);
    }

    #[test]
    fn a_device_directory_becomes_an_accessory_with_its_interfaces() {
        let tmp = tempfile::tempdir().unwrap();
        plug(tmp.path(), "1-2", "046d", "c077", 7);
        let a = read_accessory(&tmp.path().join("1-2"), "1-2").unwrap();
        assert_eq!(a.device.vendor_id, 0x046d);
        assert_eq!(a.device.product_id, 0xc077);
        assert_eq!(a.device.usb_version, 0x0200);
        assert_eq!(a.device.device_version, 0x0102);
        assert_eq!(a.registry_id, (1 << 16) | 7);
        assert_eq!(a.manufacturer, "Acme");
        assert_eq!(a.name, "Widget");
        assert_eq!(a.serial_number, "S1");
        let cfg = a.configuration.unwrap();
        assert_eq!(cfg.max_power_ma, 100);
        assert_eq!(cfg.interfaces.len(), 1);
        assert_eq!(cfg.interfaces[0].class, 3);
        assert_eq!(cfg.interfaces[0].protocol, 2);
    }

    #[test]
    fn the_snapshot_skips_root_hubs_and_interface_directories() {
        let tmp = tempfile::tempdir().unwrap();
        plug(tmp.path(), "1-2", "046d", "c077", 7);
        plug(tmp.path(), "usb1", "1d6b", "0002", 1);
        fs::create_dir_all(tmp.path().join("1-2:1.0")).unwrap();
        let snap = read_snapshot(tmp.path());
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].device.vendor_id, 0x046d);
        assert!(read_snapshot(&tmp.path().join("missing")).is_empty());
    }

    #[test]
    fn polling_the_bus_reports_attach_and_detach_to_listeners() {
        let tmp = tempfile::tempdir().unwrap();
        let mut manager = AccessoryManager::new();
        let present = manager.register(1, MatchingCriteria::default()).unwrap();
        assert!(present.is_empty());
        plug(tmp.path(), "1-2", "046d", "c077", 7);
        let report = poll(&mut manager, tmp.path());
        assert_eq!(report.attached.len(), 1);
        let id = report.attached[0];
        assert_eq!(manager.take_events(1), vec![Event::Connected(id)]);
        // A second poll with no change reports nothing.
        let again = poll(&mut manager, tmp.path());
        assert!(again.attached.is_empty() && again.detached.is_empty());
        fs::remove_dir_all(tmp.path().join("1-2")).unwrap();
        let report = poll(&mut manager, tmp.path());
        assert_eq!(report.detached, vec![id]);
        assert_eq!(manager.take_events(1), vec![Event::Disconnected(id)]);
    }
}
