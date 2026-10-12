//! USB printers for apps: the printer-class rules (interface protocol, bulk endpoints, `GET_DEVICE_ID`, `GET_PORT_STATUS`,
//! `SOFT_RESET`, port status and the IEEE 1284 device ID) from `bifrost_core::usb_printer`. The model is pure. An app
//! gets the setup packets and parses the replies, but opening the device and sending the requests are not offered here
//! yet: the app sandbox binds no USB device nodes (springboard/src/bare/app_policy.rs), so print jobs still go through
//! `print` and CUPS.
pub use bifrost_core::usb_printer::*;
