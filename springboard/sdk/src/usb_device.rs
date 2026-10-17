//! USB devices for apps: the claim planning of the device and interface classes (the configuration and
//! alternate-setting requests in order, and the pipes the claimed interface opens) from `bifrost_core::usb_claim`,
//! with the descriptor and setup-packet types an app needs to read them.
//!
//! The plan is pure. `UsbHostDevice` (`bifrost_core::usb_host`, the usbfs transport) opens a device node, claims an
//! interface, runs the plan's requests and moves control, bulk and interrupt transfers. A sandboxed app reaches only
//! the device nodes its sandbox binds (springboard/src/bare/app_policy.rs), and the kernel's `authorized` attribute
//! (`usb_approval`) decides whether a new device is usable at all.
pub use bifrost_core::usb_claim::*;
pub use bifrost_core::usb_descriptors::{
    ConfigurationDescriptor, DeviceDescriptor, EndpointDescriptor, InterfaceDescriptor, SetupPacket, TransferType,
};
pub use bifrost_core::usb_transfer::Pipe;
pub use bifrost_core::usb_host::{
    classify, list_nodes, node_path, split_descriptors, Failure, UsbHostDevice, UsbNode, MAX_TRANSFER,
};
