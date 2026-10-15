//! Force-feedback cooperation and coordinates: which app may drive a device's effects, and how a force direction is written.
//!
//! Rules of this model:
//! - A cooperative level is an access (`Shared` or `Exclusive`) and a focus (`Foreground` or `Background`). Exclusive access
//!   needs foreground focus, so a background app never holds a device's effects exclusively.
//! - A device has at most one exclusive holder. An exclusive request is refused while any other app holds the device. A
//!   shared request is refused while another app holds the device exclusively. An app may change its own level.
//! - A polar direction is an angle in degrees, counter-clockwise from the positive x axis, in `[0, 360)`. A Cartesian
//!   direction is a vector; only its angle matters, and its Cartesian form is the unit vector.
//! - Only the two-axis (plane) forms are modelled. Three-axis spherical directions are not.
//!
//! This is the pure model. Nothing grabs an input node or uploads an effect with it yet.

use std::collections::BTreeMap;

/// Whether other apps may hold the device at the same time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    Shared,
    Exclusive,
}

/// Whether the app holding the device is in front.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    Foreground,
    Background,
}

/// The access and focus an app asks for when it cooperates on a device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CooperativeLevel {
    access: Access,
    focus: Focus,
}

/// Why a cooperative level or a request on a device is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CooperationError {
    /// Exclusive access was asked for in the background.
    ExclusiveInBackground,
    /// Another app holds the device, so exclusive access is refused.
    DeviceInUse,
    /// Another app holds the device exclusively, so shared access is refused.
    DeviceHeldExclusively,
    /// The app does not hold the device, so there is nothing to release.
    NotHolder,
}

impl CooperativeLevel {
    pub fn new(access: Access, focus: Focus) -> Result<CooperativeLevel, CooperationError> {
        if access == Access::Exclusive && focus == Focus::Background {
            return Err(CooperationError::ExclusiveInBackground);
        }
        Ok(CooperativeLevel { access, focus })
    }

    pub fn access(self) -> Access {
        self.access
    }

    pub fn focus(self) -> Focus {
        self.focus
    }
}

/// Records which app holds which device at which level.
#[derive(Debug, Default)]
pub struct Arbiter {
    holders: BTreeMap<String, BTreeMap<String, CooperativeLevel>>,
}

impl Arbiter {
    pub fn new() -> Arbiter {
        Arbiter::default()
    }

    /// Gives `app` the level on `device`, or refuses it when another app's level conflicts. An app that already holds
    /// the device replaces its own level.
    pub fn request(&mut self, device: &str, app: &str, level: CooperativeLevel) -> Result<(), CooperationError> {
        let others: Vec<CooperativeLevel> = self
            .holders
            .get(device)
            .map(|apps| apps.iter().filter(|(name, _)| name.as_str() != app).map(|(_, held)| *held).collect())
            .unwrap_or_default();
        match level.access {
            Access::Exclusive if !others.is_empty() => return Err(CooperationError::DeviceInUse),
            Access::Shared if others.iter().any(|held| held.access == Access::Exclusive) => {
                return Err(CooperationError::DeviceHeldExclusively);
            }
            _ => {}
        }
        self.holders.entry(device.to_string()).or_default().insert(app.to_string(), level);
        Ok(())
    }

    /// Ends `app`'s hold on `device`. The device is free again when no app holds it.
    pub fn release(&mut self, device: &str, app: &str) -> Result<(), CooperationError> {
        let apps = self.holders.get_mut(device).ok_or(CooperationError::NotHolder)?;
        apps.remove(app).ok_or(CooperationError::NotHolder)?;
        if apps.is_empty() {
            self.holders.remove(device);
        }
        Ok(())
    }

    /// The level `app` holds on `device`, if any.
    pub fn level(&self, device: &str, app: &str) -> Option<CooperativeLevel> {
        self.holders.get(device)?.get(app).copied()
    }

    /// The app holding `device` exclusively, if one does.
    pub fn exclusive_holder(&self, device: &str) -> Option<&str> {
        self.holders
            .get(device)?
            .iter()
            .find(|(_, held)| held.access == Access::Exclusive)
            .map(|(app, _)| app.as_str())
    }

    /// How many apps hold `device`.
    pub fn holder_count(&self, device: &str) -> usize {
        self.holders.get(device).map_or(0, BTreeMap::len)
    }
}

/// The coordinate form a direction is written in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoordinateSystem {
    Cartesian,
    Polar,
}

/// A force direction on a plane.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Direction {
    /// An angle in degrees, counter-clockwise from the positive x axis, in `[0, 360)`.
    Polar(f32),
    /// A vector. Its length is not significant; only its angle is.
    Cartesian { x: f32, y: f32 },
}

/// Why a direction is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoordinateError {
    /// A component is NaN or infinite.
    NotFinite,
    /// A Cartesian vector of length zero has no direction.
    ZeroVector,
}

impl Direction {
    pub fn polar(degrees: f32) -> Result<Direction, CoordinateError> {
        if !degrees.is_finite() {
            return Err(CoordinateError::NotFinite);
        }
        Ok(Direction::Polar(wrap_degrees(degrees)))
    }

    pub fn cartesian(x: f32, y: f32) -> Result<Direction, CoordinateError> {
        if !x.is_finite() || !y.is_finite() {
            return Err(CoordinateError::NotFinite);
        }
        if x == 0.0 && y == 0.0 {
            return Err(CoordinateError::ZeroVector);
        }
        Ok(Direction::Cartesian { x, y })
    }

    /// The angle in degrees, in `[0, 360)`.
    pub fn angle_degrees(self) -> f32 {
        match self {
            Direction::Polar(degrees) => degrees,
            Direction::Cartesian { x, y } => wrap_degrees(y.atan2(x).to_degrees()),
        }
    }

    /// The unit vector that points in this direction.
    pub fn unit_vector(self) -> (f32, f32) {
        let radians = self.angle_degrees().to_radians();
        (radians.cos(), radians.sin())
    }

    /// The same direction written in `system`.
    pub fn in_system(self, system: CoordinateSystem) -> Direction {
        match system {
            CoordinateSystem::Polar => Direction::Polar(self.angle_degrees()),
            CoordinateSystem::Cartesian => {
                let (x, y) = self.unit_vector();
                Direction::Cartesian { x, y }
            }
        }
    }
}

/// Maps any finite angle into `[0, 360)`. Rounding can land exactly on 360 for a tiny negative input, which becomes 0.
fn wrap_degrees(degrees: f32) -> f32 {
    let wrapped = degrees.rem_euclid(360.0);
    if wrapped >= 360.0 {
        0.0
    } else {
        wrapped
    }
}
