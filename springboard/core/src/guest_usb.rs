//! USB passthrough policy for a guest: which host USB devices may be attached to it, and how many at once.
//!
//! Pure model. A launcher asks [`Passthrough::attach`] before it hands a host device to a guest, and calls
//! [`Passthrough::detach`] when the device goes away. Rules are checked in order and the first match decides; when no
//! rule matches the default is to refuse. Hubs are never passed through, because a guest that owns a hub could reach
//! devices the policy did not name. The official Virtualization documentation lists "USB Devices" as a device topic
//! for guests. The rule model and the limit are BifrOSt's own; they are not values the documentation gives.

/// BifrOSt limit on devices attached to one guest at a time. This is a project choice, not a documented framework limit.
pub const MAX_ATTACHED: usize = 8;

/// USB base class code for hubs.
pub const CLASS_HUB: u8 = 0x09;

/// A host USB device as the policy sees it. `id` is the host's own key for the device (for example its bus port path),
/// because two identical devices must be told apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UsbDevice {
    pub id: u32,
    pub vendor: u16,
    pub product: u16,
    pub class: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Allow,
    Deny,
}

/// One rule. A `None` field matches any value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rule {
    pub vendor: Option<u16>,
    pub product: Option<u16>,
    pub class: Option<u8>,
    pub action: Action,
}

impl Rule {
    pub fn matches(&self, dev: &UsbDevice) -> bool {
        self.vendor.map_or(true, |v| v == dev.vendor)
            && self.product.map_or(true, |p| p == dev.product)
            && self.class.map_or(true, |c| c == dev.class)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// No rule allows the device, or a hub.
    Denied,
    /// This host device is already attached.
    AlreadyAttached,
    /// The guest already has the maximum number of devices; the limit is reported.
    Limit(usize),
    /// The device is not attached to this guest.
    NotAttached,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Passthrough {
    rules: Vec<Rule>,
    attached: Vec<UsbDevice>,
}

impl Passthrough {
    pub fn new(rules: Vec<Rule>) -> Self {
        Passthrough { rules, attached: Vec::new() }
    }

    pub fn attached(&self) -> &[UsbDevice] {
        &self.attached
    }

    /// The decision for `dev` alone: hubs are denied; otherwise the first matching rule decides, default deny.
    pub fn decide(&self, dev: &UsbDevice) -> Action {
        if dev.class == CLASS_HUB {
            return Action::Deny;
        }
        self.rules.iter().find(|r| r.matches(dev)).map_or(Action::Deny, |r| r.action)
    }

    /// Attaches `dev` if the policy allows it, it is not attached yet, and the guest has room.
    pub fn attach(&mut self, dev: UsbDevice) -> Result<(), Error> {
        if self.decide(&dev) == Action::Deny {
            return Err(Error::Denied);
        }
        if self.attached.iter().any(|d| d.id == dev.id) {
            return Err(Error::AlreadyAttached);
        }
        if self.attached.len() >= MAX_ATTACHED {
            return Err(Error::Limit(MAX_ATTACHED));
        }
        self.attached.push(dev);
        Ok(())
    }

    /// Detaches the device with host key `id`.
    pub fn detach(&mut self, id: u32) -> Result<UsbDevice, Error> {
        let pos = self.attached.iter().position(|d| d.id == id).ok_or(Error::NotAttached)?;
        Ok(self.attached.remove(pos))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(id: u32, vendor: u16, product: u16, class: u8) -> UsbDevice {
        UsbDevice { id, vendor, product, class }
    }

    #[test]
    fn default_is_deny_with_no_rules() {
        let mut p = Passthrough::new(Vec::new());
        assert_eq!(p.attach(dev(1, 0x1234, 0x0001, 0x03)), Err(Error::Denied));
        assert!(p.attached().is_empty());
    }

    #[test]
    fn first_matching_rule_decides() {
        let rules = vec![
            Rule { vendor: Some(0x1234), product: None, class: None, action: Action::Deny },
            Rule { vendor: None, product: None, class: Some(0x03), action: Action::Allow },
        ];
        let p = Passthrough::new(rules);
        assert_eq!(p.decide(&dev(1, 0x1234, 0x0001, 0x03)), Action::Deny);
        assert_eq!(p.decide(&dev(2, 0x5678, 0x0001, 0x03)), Action::Allow);
        assert_eq!(p.decide(&dev(3, 0x5678, 0x0001, 0x08)), Action::Deny);
    }

    #[test]
    fn hubs_are_never_passed_through() {
        let rules = vec![Rule { vendor: Some(0x0424), product: None, class: None, action: Action::Allow }];
        let mut p = Passthrough::new(rules);
        assert_eq!(p.attach(dev(9, 0x0424, 0x2514, CLASS_HUB)), Err(Error::Denied));
        assert!(p.attach(dev(10, 0x0424, 0x0001, 0x03)).is_ok());
    }

    #[test]
    fn attach_limit_and_duplicates() {
        let rules = vec![Rule { vendor: None, product: None, class: None, action: Action::Allow }];
        let mut p = Passthrough::new(rules);
        for id in 0..MAX_ATTACHED as u32 {
            p.attach(dev(id, 0x1111, 0x0001, 0x08)).unwrap();
        }
        assert_eq!(p.attach(dev(100, 0x1111, 0x0001, 0x08)), Err(Error::Limit(MAX_ATTACHED)));
        assert_eq!(p.attach(dev(0, 0x1111, 0x0001, 0x08)), Err(Error::AlreadyAttached));
        assert_eq!(p.attached().len(), MAX_ATTACHED);
    }

    #[test]
    fn duplicate_host_key_is_refused_until_detached() {
        let rules = vec![Rule { vendor: None, product: Some(0x0002), class: None, action: Action::Allow }];
        let mut p = Passthrough::new(rules);
        p.attach(dev(5, 0x2222, 0x0002, 0x03)).unwrap();
        assert_eq!(p.attach(dev(5, 0x2222, 0x0002, 0x03)), Err(Error::AlreadyAttached));
        assert_eq!(p.detach(7), Err(Error::NotAttached));
        let gone = p.detach(5).unwrap();
        assert_eq!(gone.vendor, 0x2222);
        assert!(p.attach(dev(5, 0x2222, 0x0002, 0x03)).is_ok());
    }
}
