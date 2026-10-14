//! Location sharing: the addresses a person's location is shared with, each until an end time or indefinitely. A grant
//! counts only while it has not ended, so `active` at a given time is the current list.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Duration {
    Hour,
    Day,
    Indefinite,
}

impl Duration {
    /// The end time for a share that starts at `now_unix`; `None` for a share that does not end.
    fn ends_at(self, now_unix: i64) -> Option<i64> {
        match self {
            Duration::Hour => Some(now_unix + 3_600),
            Duration::Day => Some(now_unix + 86_400),
            Duration::Indefinite => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grant {
    pub to: String,
    pub until_unix: Option<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShareError {
    InvalidAddress,
}

/// An address has a local part, and a domain with a dot in it and no spaces or second `@`.
pub fn valid_address(address: &str) -> bool {
    let a = address.trim();
    let Some((local, domain)) = a.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && !domain.contains('@')
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !a.chars().any(char::is_whitespace)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Shares {
    grants: Vec<Grant>,
}

impl Shares {
    /// Shares the location with `address` from `now_unix` for `duration`. A grant already held for that address is
    /// replaced. Addresses compare without case or surrounding spaces.
    pub fn share(&mut self, address: &str, duration: Duration, now_unix: i64) -> Result<(), ShareError> {
        if !valid_address(address) {
            return Err(ShareError::InvalidAddress);
        }
        let to = address.trim().to_lowercase();
        self.grants.retain(|g| g.to != to);
        let until_unix = duration.ends_at(now_unix);
        self.grants.push(Grant { to, until_unix });
        Ok(())
    }

    /// Stops sharing with `address`.
    pub fn stop(&mut self, address: &str) {
        let to = address.trim().to_lowercase();
        self.grants.retain(|g| g.to != to);
    }

    /// The grants that have not ended at `now_unix`.
    pub fn active(&self, now_unix: i64) -> Vec<&Grant> {
        self.grants.iter().filter(|g| g.until_unix.map_or(true, |t| t > now_unix)).collect()
    }
}
