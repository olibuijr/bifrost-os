//! Network link conditioning (`NetworkLinkConditioner` parity): a developer profile that makes a network interface slower,
//! laggier or lossier, so an app can be tested on a bad connection.
//!
//! A [`Profile`] holds the rate limit, the delay with its jitter and the packet loss. [`Profile::validate`] refuses values
//! the kernel cannot apply. [`Profile::tc_args`] turns a valid profile into the arguments of a `tc qdisc` command that
//! installs a `netem` queue on the interface's egress, and [`clear_args`] removes it again. Only egress is shaped: the
//! kernel cannot shape ingress without an intermediate device, so a download rate is recorded but not applied to the
//! interface (see [`Profile::down_kbit`]).
//!
//! Not here: running the commands. The caller runs `tc` (it needs the privilege), and it must clear the queue when the
//! developer turns the conditioner off.

use std::fmt;

/// A conditioning profile. Rates are in kilobits per second; `0` means no limit.
#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    pub name: &'static str,
    /// Download rate. Recorded for the settings page; not applied (ingress is not shaped).
    pub down_kbit: u32,
    /// Upload rate applied to the egress queue. `0` applies no rate limit.
    pub up_kbit: u32,
    pub delay_ms: u32,
    pub jitter_ms: u32,
    /// Packet loss, in percent (0 to 100).
    pub loss_percent: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ConditionError {
    EmptyName,
    BadInterface(String),
    LossOutOfRange(f32),
    JitterAboveDelay { delay_ms: u32, jitter_ms: u32 },
}

impl fmt::Display for ConditionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConditionError::EmptyName => write!(f, "a profile needs a name"),
            ConditionError::BadInterface(name) => write!(f, "interface name {name:?} is not usable"),
            ConditionError::LossOutOfRange(loss) => write!(f, "packet loss {loss} is outside 0 to 100 percent"),
            ConditionError::JitterAboveDelay { delay_ms, jitter_ms } => {
                write!(f, "jitter {jitter_ms} ms is larger than the delay {delay_ms} ms")
            }
        }
    }
}

/// The built-in profiles, in the order the settings page lists them.
pub const PROFILES: [Profile; 4] = [
    Profile { name: "Wi-Fi, poor", down_kbit: 6_000, up_kbit: 2_000, delay_ms: 60, jitter_ms: 20, loss_percent: 0.5 },
    Profile { name: "DSL", down_kbit: 8_000, up_kbit: 1_000, delay_ms: 40, jitter_ms: 5, loss_percent: 0.0 },
    Profile { name: "Mobile, slow", down_kbit: 400, up_kbit: 100, delay_ms: 300, jitter_ms: 50, loss_percent: 1.0 },
    Profile { name: "Offline, lossy", down_kbit: 0, up_kbit: 0, delay_ms: 0, jitter_ms: 0, loss_percent: 100.0 },
];

/// The built-in profile called `name`, if there is one.
pub fn profile(name: &str) -> Option<&'static Profile> {
    PROFILES.iter().find(|p| p.name == name)
}

/// Interface names are the kernel's: non-empty, short, and free of separators and blanks.
fn check_interface(iface: &str) -> Result<(), ConditionError> {
    let usable = !iface.is_empty()
        && iface.len() <= 15
        && iface.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if usable {
        Ok(())
    } else {
        Err(ConditionError::BadInterface(iface.to_string()))
    }
}

impl Profile {
    /// Refuses a profile the kernel cannot apply.
    pub fn validate(&self) -> Result<(), ConditionError> {
        if self.name.trim().is_empty() {
            return Err(ConditionError::EmptyName);
        }
        if !self.loss_percent.is_finite() || !(0.0..=100.0).contains(&self.loss_percent) {
            return Err(ConditionError::LossOutOfRange(self.loss_percent));
        }
        if self.jitter_ms > self.delay_ms {
            return Err(ConditionError::JitterAboveDelay { delay_ms: self.delay_ms, jitter_ms: self.jitter_ms });
        }
        Ok(())
    }

    /// The `netem` options for this profile: `delay`, `loss` and, when set, `rate`. Loss is printed with two decimals,
    /// so 0.5 percent becomes `0.50%`.
    pub fn netem_options(&self) -> Vec<String> {
        let mut opts = Vec::new();
        if self.delay_ms > 0 || self.jitter_ms > 0 {
            opts.push("delay".to_string());
            opts.push(format!("{}ms", self.delay_ms));
            if self.jitter_ms > 0 {
                opts.push(format!("{}ms", self.jitter_ms));
            }
        }
        if self.loss_percent > 0.0 {
            opts.push("loss".to_string());
            opts.push(format!("{:.2}%", self.loss_percent));
        }
        if self.up_kbit > 0 {
            opts.push("rate".to_string());
            opts.push(format!("{}kbit", self.up_kbit));
        }
        opts
    }

    /// The arguments of the `tc` command that installs this profile on `iface`'s egress queue. The `tc` binary is not
    /// included: the first element is `qdisc`.
    pub fn tc_args(&self, iface: &str) -> Result<Vec<String>, ConditionError> {
        self.validate()?;
        check_interface(iface)?;
        let mut args: Vec<String> =
            ["qdisc", "replace", "dev", iface, "root", "netem"].iter().map(|s| s.to_string()).collect();
        args.extend(self.netem_options());
        Ok(args)
    }
}

/// The arguments that remove the conditioning queue from `iface`.
pub fn clear_args(iface: &str) -> Result<Vec<String>, ConditionError> {
    check_interface(iface)?;
    Ok(["qdisc", "del", "dev", iface, "root"].iter().map(|s| s.to_string()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn custom(delay_ms: u32, jitter_ms: u32, loss_percent: f32) -> Profile {
        Profile { name: "custom", down_kbit: 0, up_kbit: 0, delay_ms, jitter_ms, loss_percent }
    }

    #[test]
    fn every_built_in_profile_is_valid() {
        for p in &PROFILES {
            assert_eq!(p.validate(), Ok(()), "{}", p.name);
        }
    }

    #[test]
    fn a_profile_is_found_by_its_name() {
        assert_eq!(profile("DSL").map(|p| p.up_kbit), Some(1_000));
        assert!(profile("no such profile").is_none());
    }

    #[test]
    fn loss_must_be_a_percentage() {
        assert_eq!(custom(0, 0, 100.5).validate(), Err(ConditionError::LossOutOfRange(100.5)));
        assert_eq!(custom(0, 0, -1.0).validate(), Err(ConditionError::LossOutOfRange(-1.0)));
        assert!(custom(0, 0, f32::NAN).validate().is_err());
        assert!(custom(0, 0, 100.0).validate().is_ok());
    }

    #[test]
    fn jitter_cannot_exceed_the_delay_it_varies() {
        assert_eq!(
            custom(10, 20, 0.0).validate(),
            Err(ConditionError::JitterAboveDelay { delay_ms: 10, jitter_ms: 20 })
        );
    }

    #[test]
    fn a_profile_needs_a_name() {
        let mut p = custom(0, 0, 0.0);
        p.name = "  ";
        assert_eq!(p.validate(), Err(ConditionError::EmptyName));
    }

    #[test]
    fn netem_options_list_only_what_the_profile_sets() {
        assert_eq!(
            profile("Mobile, slow").unwrap().netem_options(),
            ["delay", "300ms", "50ms", "loss", "1.00%", "rate", "100kbit"]
        );
        assert_eq!(profile("DSL").unwrap().netem_options(), ["delay", "40ms", "5ms", "rate", "1000kbit"]);
        assert!(custom(0, 0, 0.0).netem_options().is_empty());
    }

    #[test]
    fn a_total_loss_profile_sets_loss_without_delay() {
        assert_eq!(profile("Offline, lossy").unwrap().netem_options(), ["loss", "100.00%"]);
    }

    #[test]
    fn tc_args_build_a_replace_on_the_named_interface() {
        let args = profile("DSL").unwrap().tc_args("wlan0").unwrap();
        assert_eq!(
            args,
            ["qdisc", "replace", "dev", "wlan0", "root", "netem", "delay", "40ms", "5ms", "rate", "1000kbit"]
        );
    }

    #[test]
    fn tc_args_refuse_a_bad_interface_or_profile() {
        let dsl = profile("DSL").unwrap();
        assert!(matches!(dsl.tc_args("eth 0"), Err(ConditionError::BadInterface(_))));
        assert!(matches!(dsl.tc_args("../etc"), Err(ConditionError::BadInterface(_))));
        assert!(matches!(dsl.tc_args(&"x".repeat(16)), Err(ConditionError::BadInterface(_))));
        assert!(custom(0, 0, 200.0).tc_args("eth0").is_err());
    }

    #[test]
    fn clear_args_remove_the_root_queue() {
        assert_eq!(clear_args("eth0").unwrap(), ["qdisc", "del", "dev", "eth0", "root"]);
        assert!(clear_args("").is_err());
    }
}
