//! USB devices for apps: the claim planning of the device and interface classes (the configuration and
//! alternate-setting requests in order, and the pipes the claimed interface opens) from `bifrost_core::usb_claim`,
//! with the descriptor and setup-packet types an app needs to read them.
//!
//! The model is pure. An app can plan a claim and inspect the requests and pipes it would use, but it cannot send
//! them: the app sandbox binds no USB device nodes (springboard/src/bare/app_policy.rs), and no transport is offered.
pub use bifrost_core::usb_claim::*;
pub use bifrost_core::usb_descriptors::{
    ConfigurationDescriptor, DeviceDescriptor, EndpointDescriptor, InterfaceDescriptor, SetupPacket, TransferType,
};
pub use bifrost_core::usb_transfer::Pipe;
