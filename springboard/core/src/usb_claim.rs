//! Claiming a USB interface: the control requests that select a configuration and an alternate setting, and the pipes
//! the claimed interface then opens.
//!
//! This is the planning half of `IOUSBHostDevice` and `IOUSBHostInterface`. The model is pure: it reads the descriptors
//! the caller already parsed and returns the setup packets to send, in order, and the pipes that become usable. Sending
//! the packets and opening the endpoints need a transport, and none is wired to apps yet (the app sandbox binds no USB
//! device nodes). A claim plan is checked before any request is sent, so a bad descriptor fails here, not on the bus.

use crate::usb_descriptors::{
    set_configuration, set_interface, ConfigurationDescriptor, DeviceDescriptor, SetupPacket, TransferType,
};
use crate::usb_transfer::Pipe;

/// Why an interface cannot be claimed from the descriptors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClaimError {
    /// The device reports no configurations, or the configuration value is zero, which means unconfigured.
    NoConfiguration,
    /// The configuration has no interface with this number and alternate setting.
    NoInterface,
    /// Two endpoints of the interface share an address, which the descriptors must not do.
    DuplicateEndpoint,
}

impl std::fmt::Display for ClaimError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClaimError::NoConfiguration => write!(f, "The device has no usable configuration"),
            ClaimError::NoInterface => write!(f, "The configuration has no such interface"),
            ClaimError::DuplicateEndpoint => write!(f, "The interface lists an endpoint address twice"),
        }
    }
}

/// The requests to send and the pipes to open to claim one interface alternate setting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaimPlan {
    /// The control requests, in the order to send them.
    pub requests: Vec<SetupPacket>,
    /// The pipes the claimed interface opens, in descriptor order.
    pub pipes: Vec<Pipe>,
}

impl ClaimPlan {
    /// The pipe at an endpoint address of the claimed interface.
    pub fn pipe(&self, address: u8) -> Option<&Pipe> {
        self.pipes.iter().find(|p| p.address == address)
    }

    /// The pipes of one transfer type, in descriptor order.
    pub fn pipes_of(&self, transfer: TransferType) -> Vec<&Pipe> {
        self.pipes.iter().filter(|p| p.transfer == transfer).collect()
    }
}

/// Plans the claim of interface `interface`, alternate setting `alternate`, of `config`.
///
/// `configured_value` is the configuration the device reports as active, if the caller has read it. When it already
/// matches `config.value`, `SET_CONFIGURATION` is not sent again. The `SET_INTERFACE` request is always sent, because
/// the alternate setting of an interface is not known to be the default without asking.
pub fn plan_claim(
    device: &DeviceDescriptor,
    config: &ConfigurationDescriptor,
    interface: u8,
    alternate: u8,
    configured_value: Option<u8>,
) -> Result<ClaimPlan, ClaimError> {
    if device.configurations == 0 || config.value == 0 {
        return Err(ClaimError::NoConfiguration);
    }
    let iface = config.interface(interface, alternate).ok_or(ClaimError::NoInterface)?;
    for (at, endpoint) in iface.endpoints.iter().enumerate() {
        if iface.endpoints[..at].iter().any(|earlier| earlier.address == endpoint.address) {
            return Err(ClaimError::DuplicateEndpoint);
        }
    }

    let mut requests = Vec::with_capacity(2);
    if configured_value != Some(config.value) {
        requests.push(set_configuration(config.value));
    }
    requests.push(set_interface(interface, alternate));
    let pipes = iface.endpoints.iter().map(Pipe::from_endpoint).collect();
    Ok(ClaimPlan { requests, pipes })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usb_descriptors::{EndpointDescriptor, InterfaceDescriptor};

    fn device(configurations: u8) -> DeviceDescriptor {
        DeviceDescriptor {
            usb_version: 0x0200,
            class: 0,
            subclass: 0,
            protocol: 0,
            max_packet_size0: 64,
            vendor_id: 0x1234,
            product_id: 0x0007,
            device_version: 0x0100,
            configurations,
        }
    }

    fn endpoint(address: u8, attributes: u8, max_packet_size: u16) -> EndpointDescriptor {
        EndpointDescriptor { address, attributes, max_packet_size, interval: 0 }
    }

    /// Configuration 1 with interface 0 (alternate 0 with bulk in and out, alternate 1 with an isochronous pair)
    /// and interface 1 (alternate 0, one interrupt in).
    fn config() -> ConfigurationDescriptor {
        ConfigurationDescriptor {
            value: 1,
            attributes: 0x80,
            max_power_ma: 100,
            interfaces: vec![
                InterfaceDescriptor {
                    number: 0,
                    alternate: 0,
                    class: 0xff,
                    subclass: 0,
                    protocol: 0,
                    endpoints: vec![endpoint(0x01, 0x02, 512), endpoint(0x81, 0x02, 512)],
                },
                InterfaceDescriptor {
                    number: 0,
                    alternate: 1,
                    class: 0xff,
                    subclass: 0,
                    protocol: 0,
                    endpoints: vec![endpoint(0x82, 0x01, 0x0400)],
                },
                InterfaceDescriptor {
                    number: 1,
                    alternate: 0,
                    class: 0x03,
                    subclass: 0,
                    protocol: 0,
                    endpoints: vec![endpoint(0x83, 0x03, 8)],
                },
            ],
        }
    }

    #[test]
    fn unconfigured_device_gets_set_configuration_then_set_interface() {
        let plan = plan_claim(&device(1), &config(), 0, 0, None).unwrap();
        assert_eq!(plan.requests, vec![set_configuration(1), set_interface(0, 0)]);
    }

    #[test]
    fn already_configured_device_skips_set_configuration() {
        let plan = plan_claim(&device(1), &config(), 0, 0, Some(1)).unwrap();
        assert_eq!(plan.requests, vec![set_interface(0, 0)]);
    }

    #[test]
    fn a_different_active_configuration_is_selected_again() {
        let plan = plan_claim(&device(2), &config(), 0, 0, Some(2)).unwrap();
        assert_eq!(plan.requests[0], set_configuration(1));
    }

    #[test]
    fn pipes_follow_the_claimed_alternate_setting() {
        let plan = plan_claim(&device(1), &config(), 0, 1, Some(1)).unwrap();
        assert_eq!(plan.pipes.len(), 1);
        assert!(plan.pipe(0x82).is_some());
        assert!(plan.pipe(0x01).is_none());
        assert_eq!(plan.pipe(0x82).unwrap().packet_size, 1024);
        assert_eq!(plan.pipe(0x82).unwrap().transactions, 1);
    }

    #[test]
    fn pipes_of_filters_by_transfer_type() {
        let plan = plan_claim(&device(1), &config(), 0, 0, Some(1)).unwrap();
        let bulk: Vec<u8> = plan.pipes_of(TransferType::Bulk).iter().map(|p| p.address).collect();
        assert_eq!(bulk, vec![0x01, 0x81]);
        assert!(plan.pipes_of(TransferType::Isochronous).is_empty());
    }

    #[test]
    fn missing_interface_or_alternate_is_refused() {
        assert_eq!(plan_claim(&device(1), &config(), 2, 0, None), Err(ClaimError::NoInterface));
        assert_eq!(plan_claim(&device(1), &config(), 1, 3, None), Err(ClaimError::NoInterface));
    }

    #[test]
    fn zero_configuration_value_or_no_configurations_is_refused() {
        let mut unconfigured = config();
        unconfigured.value = 0;
        assert_eq!(plan_claim(&device(1), &unconfigured, 0, 0, None), Err(ClaimError::NoConfiguration));
        assert_eq!(plan_claim(&device(0), &config(), 0, 0, None), Err(ClaimError::NoConfiguration));
    }

    #[test]
    fn duplicate_endpoint_addresses_are_refused() {
        let mut bad = config();
        bad.interfaces[0].endpoints.push(endpoint(0x01, 0x02, 64));
        assert_eq!(plan_claim(&device(1), &bad, 0, 0, None), Err(ClaimError::DuplicateEndpoint));
    }

    #[test]
    fn claim_errors_have_messages() {
        assert_eq!(ClaimError::NoInterface.to_string(), "The configuration has no such interface");
    }
}
