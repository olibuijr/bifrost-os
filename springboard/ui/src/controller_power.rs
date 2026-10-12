//! Battery and light of a game controller: the charge the device reports, and the colour of its light.
//!
//! Pure: no device, no HID report decoding. The caller turns the raw report into a percentage and a charge state and
//! passes them in. The battery state values are the four the framework names (unknown, discharging, charging, full).
//! Light colour components are normalised to 0 to 1, as the framework's colour type is; `Color::to_8bit` gives the
//! 0 to 255 form a report needs.

/// Whether the battery has power and whether it is charging, as the device reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BatteryState {
    Unknown,
    Discharging,
    /// Has power and is charging, but is not full.
    Charging,
    /// Has power and is fully charged.
    Full,
}

/// The charge level and state of a controller battery. `level` is `None` while the device has not reported one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Battery {
    /// Charge from 0 to 1.
    pub level: Option<f32>,
    pub state: BatteryState,
}

impl Battery {
    /// A battery with no report yet.
    pub fn unknown() -> Self {
        Battery { level: None, state: BatteryState::Unknown }
    }

    /// A battery from a whole-number percentage. Values above 100 are read as 100.
    pub fn from_percent(percent: u8, state: BatteryState) -> Self {
        let level = (f32::from(percent.min(100))) / 100.0;
        Battery { level: Some(level), state }
    }

    /// The level as a whole percentage, rounded to the nearest, or `None` without a report.
    pub fn percent(&self) -> Option<u8> {
        self.level.map(|l| (l * 100.0).round() as u8)
    }
}

/// A colour with normalised components, 0 to 1 each.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub red: f32,
    pub green: f32,
    pub blue: f32,
}

impl Color {
    /// A colour from normalised components. Each component is clamped to 0 to 1; NaN becomes 0.
    pub fn new(red: f32, green: f32, blue: f32) -> Self {
        Color { red: clamp_unit(red), green: clamp_unit(green), blue: clamp_unit(blue) }
    }

    /// A colour from 0 to 255 components, as a report carries them.
    pub fn from_8bit(red: u8, green: u8, blue: u8) -> Self {
        Color::new(f32::from(red) / 255.0, f32::from(green) / 255.0, f32::from(blue) / 255.0)
    }

    /// The components as 0 to 255 values, rounded to the nearest.
    pub fn to_8bit(&self) -> [u8; 3] {
        [unit_to_byte(self.red), unit_to_byte(self.green), unit_to_byte(self.blue)]
    }
}

fn clamp_unit(v: f32) -> f32 {
    if v.is_nan() {
        0.0
    } else {
        v.clamp(0.0, 1.0)
    }
}

fn unit_to_byte(v: f32) -> u8 {
    (clamp_unit(v) * 255.0).round() as u8
}

/// The colour of a controller's light. A device without a light reports none, which the caller models with `None`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Light {
    pub color: Color,
}

impl Light {
    /// A light that starts black.
    pub fn new() -> Self {
        Light { color: Color::new(0.0, 0.0, 0.0) }
    }

    /// Sets the colour. The stored colour is the clamped one.
    pub fn set_color(&mut self, color: Color) {
        self.color = color;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_percentage_becomes_a_level_and_clamps_above_one_hundred() {
        let half = Battery::from_percent(50, BatteryState::Discharging);
        assert_eq!(half.level, Some(0.5));
        assert_eq!(half.percent(), Some(50));
        let over = Battery::from_percent(250, BatteryState::Full);
        assert_eq!(over.level, Some(1.0));
        assert_eq!(over.state, BatteryState::Full);
    }

    #[test]
    fn an_unreported_battery_has_no_level_and_no_percent() {
        let unknown = Battery::unknown();
        assert_eq!(unknown.level, None);
        assert_eq!(unknown.state, BatteryState::Unknown);
        assert_eq!(unknown.percent(), None);
    }

    #[test]
    fn percent_rounds_to_the_nearest_whole_value() {
        // 0.625 * 100 = 62.5, which rounds away from zero to 63
        let b = Battery { level: Some(0.625), state: BatteryState::Charging };
        assert_eq!(b.percent(), Some(63));
        let low = Battery { level: Some(0.004), state: BatteryState::Discharging };
        assert_eq!(low.percent(), Some(0));
    }

    #[test]
    fn colour_components_clamp_to_the_unit_range_and_nan_is_zero() {
        let c = Color::new(1.5, -0.2, 0.5);
        assert_eq!(c, Color { red: 1.0, green: 0.0, blue: 0.5 });
        assert_eq!(Color::new(f32::NAN, 0.25, 0.0).red, 0.0);
    }

    #[test]
    fn eight_bit_components_convert_both_ways_with_rounding() {
        let c = Color::from_8bit(255, 0, 128);
        assert_eq!(c.red, 1.0);
        assert_eq!(c.green, 0.0);
        // 128 / 255 is 0.50196..., and back again gives 128
        assert_eq!(c.to_8bit(), [255, 0, 128]);
        // 0.5 * 255 = 127.5 rounds away from zero to 128
        assert_eq!(Color::new(0.5, 0.5, 0.5).to_8bit(), [128, 128, 128]);
        assert_eq!(Color::new(0.0, 0.0, 0.0).to_8bit(), [0, 0, 0]);
    }

    #[test]
    fn setting_the_light_replaces_the_colour_and_starts_black() {
        let mut light = Light::new();
        assert_eq!(light.color.to_8bit(), [0, 0, 0]);
        light.set_color(Color::from_8bit(0, 0, 255));
        assert_eq!(light.color.to_8bit(), [0, 0, 255]);
        light.set_color(Color::new(2.0, 0.0, 0.0));
        assert_eq!(light.color.red, 1.0, "the clamped colour is what is stored");
    }
}
