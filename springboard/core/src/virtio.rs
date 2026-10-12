//! Virtio device driver handshake: the status register and feature negotiation.
//!
//! Pure model: nothing touches a device. A guest driver for a virtio device takes the steps that the VIRTIO 1.x
//! specification (section 3.1.1, driver initialization) defines: reset, acknowledge, announce a driver, negotiate
//! features, confirm FEATURES_OK, set up the queues, then set DRIVER_OK. [`Device`] enforces that order and the feature
//! rules, and a launcher that talks to a real device writes the same bits to the device's status register.
//! The official Virtualization documentation says the framework follows the VIRTIO specification for its network,
//! socket, serial, storage, entropy and memory-balloon devices, and lists "Custom Virtio drivers" as a topic. The
//! status bits and device ids below come from the VIRTIO specification itself.

/// Driver has noticed the device.
pub const ACKNOWLEDGE: u8 = 1;
/// Driver knows how to drive the device.
pub const DRIVER: u8 = 2;
/// Driver is ready and the device may run.
pub const DRIVER_OK: u8 = 4;
/// Feature negotiation is complete and the device accepted the features.
pub const FEATURES_OK: u8 = 8;
/// The device hit an error it cannot recover from without a reset.
pub const DEVICE_NEEDS_RESET: u8 = 64;
/// The driver gave up on the device.
pub const FAILED: u8 = 128;

/// VIRTIO_F_VERSION_1 (feature bit 32): the device follows the modern, non-legacy interface.
pub const F_VERSION_1: u64 = 1 << 32;

/// Device types by their VIRTIO device id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceType {
    Network,
    Block,
    Console,
    Entropy,
    Balloon,
    Gpu,
    Input,
    Socket,
    Sound,
    FileSystem,
    /// Any other device id, kept as is.
    Other(u32),
}

impl DeviceType {
    pub fn id(self) -> u32 {
        match self {
            DeviceType::Network => 1,
            DeviceType::Block => 2,
            DeviceType::Console => 3,
            DeviceType::Entropy => 4,
            DeviceType::Balloon => 5,
            DeviceType::Gpu => 16,
            DeviceType::Input => 18,
            DeviceType::Socket => 19,
            DeviceType::Sound => 25,
            DeviceType::FileSystem => 26,
            DeviceType::Other(id) => id,
        }
    }

    pub fn from_id(id: u32) -> Self {
        match id {
            1 => DeviceType::Network,
            2 => DeviceType::Block,
            3 => DeviceType::Console,
            4 => DeviceType::Entropy,
            5 => DeviceType::Balloon,
            16 => DeviceType::Gpu,
            18 => DeviceType::Input,
            19 => DeviceType::Socket,
            25 => DeviceType::Sound,
            26 => DeviceType::FileSystem,
            other => DeviceType::Other(other),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The step is not allowed in the current status register value.
    OutOfOrder { status: u8 },
    /// The driver asked for feature bits the device does not offer.
    UnknownFeatures(u64),
    /// The driver did not ask for feature bits the device requires.
    MissingFeatures(u64),
    /// FEATURES_OK was cleared by the device, so the driver must not go live.
    FeaturesRejected,
}

/// One virtio device as seen by its driver. A new device starts with the status register at zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Device {
    kind: DeviceType,
    offered: u64,
    required: u64,
    status: u8,
    accepted: u64,
    rejected: bool,
}

impl Device {
    /// `required` must be a subset of `offered`; otherwise the device could never negotiate, so `None` is returned.
    pub fn new(kind: DeviceType, offered: u64, required: u64) -> Option<Self> {
        if required & !offered != 0 {
            return None;
        }
        Some(Device { kind, offered, required, status: 0, accepted: 0, rejected: false })
    }

    pub fn kind(&self) -> DeviceType {
        self.kind
    }

    pub fn status(&self) -> u8 {
        self.status
    }

    /// The feature bits the driver accepted during negotiation, empty before it.
    pub fn accepted_features(&self) -> u64 {
        self.accepted
    }

    /// True once the driver set DRIVER_OK and nothing has failed since.
    pub fn is_live(&self) -> bool {
        self.status & DRIVER_OK != 0 && self.status & FAILED == 0
    }

    /// Step 1: the driver saw the device. Only allowed from a reset device.
    pub fn acknowledge(&mut self) -> Result<u8, Error> {
        self.require(self.status == 0)?;
        self.status = ACKNOWLEDGE;
        Ok(self.status)
    }

    /// Step 2: the driver knows how to drive this device type. Only allowed after ACKNOWLEDGE.
    pub fn set_driver(&mut self) -> Result<u8, Error> {
        self.require(self.status == ACKNOWLEDGE)?;
        self.status |= DRIVER;
        Ok(self.status)
    }

    /// Step 3: the driver asks for `features`. Unknown bits and missing required bits are refused, and the device
    /// keeps the status unchanged. On success FEATURES_OK is set.
    pub fn negotiate(&mut self, features: u64) -> Result<u8, Error> {
        self.require(self.status == ACKNOWLEDGE | DRIVER)?;
        let unknown = features & !self.offered;
        if unknown != 0 {
            return Err(Error::UnknownFeatures(unknown));
        }
        let missing = self.required & !features;
        if missing != 0 {
            return Err(Error::MissingFeatures(missing));
        }
        self.accepted = features;
        self.status |= FEATURES_OK;
        Ok(self.status)
    }

    /// The device clears FEATURES_OK after negotiation, which it does when it does not accept the feature set.
    pub fn device_rejects(&mut self) {
        self.status &= !FEATURES_OK;
        self.rejected = true;
    }

    /// Step 4: the driver re-reads the status, finds FEATURES_OK still set, and sets DRIVER_OK.
    pub fn driver_ok(&mut self) -> Result<u8, Error> {
        if self.status & FAILED != 0 {
            return Err(Error::OutOfOrder { status: self.status });
        }
        if self.rejected {
            return Err(Error::FeaturesRejected);
        }
        self.require(self.status == ACKNOWLEDGE | DRIVER | FEATURES_OK)?;
        self.status |= DRIVER_OK;
        Ok(self.status)
    }

    /// The driver gives up. Allowed in any status.
    pub fn fail(&mut self) -> u8 {
        self.status |= FAILED;
        self.status
    }

    /// Resets the device: status and accepted features go back to the start. Allowed in any status.
    pub fn reset(&mut self) {
        self.status = 0;
        self.accepted = 0;
        self.rejected = false;
    }

    fn require(&self, ok: bool) -> Result<(), Error> {
        if ok && self.status & FAILED == 0 {
            Ok(())
        } else {
            Err(Error::OutOfOrder { status: self.status })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXTRA: u64 = 1 << 5;
    const NEEDED: u64 = 1 << 3;

    fn net() -> Device {
        Device::new(DeviceType::Network, F_VERSION_1 | EXTRA | NEEDED, F_VERSION_1 | NEEDED).unwrap()
    }

    #[test]
    fn full_handshake_reaches_driver_ok() {
        let mut d = net();
        assert_eq!(d.acknowledge(), Ok(1));
        assert_eq!(d.set_driver(), Ok(3));
        assert_eq!(d.negotiate(F_VERSION_1 | NEEDED), Ok(11));
        assert_eq!(d.driver_ok(), Ok(15));
        assert!(d.is_live());
        assert_eq!(d.accepted_features(), F_VERSION_1 | NEEDED);
    }

    #[test]
    fn driver_ok_before_negotiation_is_out_of_order() {
        let mut d = net();
        d.acknowledge().unwrap();
        d.set_driver().unwrap();
        assert_eq!(d.driver_ok(), Err(Error::OutOfOrder { status: 3 }));
        assert!(!d.is_live());
    }

    #[test]
    fn unoffered_feature_bit_is_refused_and_status_is_unchanged() {
        let mut d = net();
        d.acknowledge().unwrap();
        d.set_driver().unwrap();
        assert_eq!(d.negotiate(F_VERSION_1 | NEEDED | (1 << 7)), Err(Error::UnknownFeatures(1 << 7)));
        assert_eq!(d.status(), 3);
    }

    #[test]
    fn missing_required_feature_is_refused() {
        let mut d = net();
        d.acknowledge().unwrap();
        d.set_driver().unwrap();
        assert_eq!(d.negotiate(F_VERSION_1), Err(Error::MissingFeatures(NEEDED)));
    }

    #[test]
    fn device_that_clears_features_ok_blocks_driver_ok() {
        let mut d = net();
        d.acknowledge().unwrap();
        d.set_driver().unwrap();
        d.negotiate(F_VERSION_1 | NEEDED).unwrap();
        d.device_rejects();
        assert_eq!(d.driver_ok(), Err(Error::FeaturesRejected));
    }

    #[test]
    fn acknowledge_twice_and_required_outside_offer_are_refused() {
        let mut d = net();
        d.acknowledge().unwrap();
        assert_eq!(d.acknowledge(), Err(Error::OutOfOrder { status: 1 }));
        // A required bit that is not offered can never be negotiated, so no device is built.
        assert!(Device::new(DeviceType::Sound, F_VERSION_1, F_VERSION_1 | EXTRA).is_none());
    }

    #[test]
    fn failed_device_refuses_steps_until_reset() {
        let mut d = net();
        assert_eq!(d.fail(), 128);
        assert_eq!(d.acknowledge(), Err(Error::OutOfOrder { status: 128 }));
        d.reset();
        assert_eq!(d.status(), 0);
        assert_eq!(d.acknowledge(), Ok(1));
    }

    #[test]
    fn device_type_ids_round_trip() {
        assert_eq!(DeviceType::Sound.id(), 25);
        assert_eq!(DeviceType::from_id(26), DeviceType::FileSystem);
        assert_eq!(DeviceType::from_id(999), DeviceType::Other(999));
        assert_eq!(DeviceType::Other(999).id(), 999);
    }
}
