//! The Convert mode of Calculator: unit conversion over five categories.
//!
//! Length, mass and temperature use the SDK's `Measurement` model (`bifrost_app::locale`). That model has
//! no speed or volume dimension yet, so those two use the scale factors below (the SDK's `Unit` cannot hold
//! them). Numbers are shown with the SDK's `NumberFormatter` in the app's locale.

use crate::format::{self, Locale};
use bifrost_app::locale::{Locale as SdkLocale, Measurement, NumberFormatter, NumberStyle, Unit as Std};

/// Digits the entry line holds.
pub const MAX_DIGITS: usize = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Length,
    Mass,
    Temperature,
    Speed,
    Volume,
}

impl Category {
    pub const ALL: [Category; 5] = [Category::Length, Category::Mass, Category::Temperature, Category::Speed, Category::Volume];

    pub fn title(self) -> &'static str {
        match self {
            Category::Length => "Length",
            Category::Mass => "Mass",
            Category::Temperature => "Temperature",
            Category::Speed => "Speed",
            Category::Volume => "Volume",
        }
    }

    pub fn units(self) -> &'static [UnitDef] {
        match self {
            Category::Length => LENGTH,
            Category::Mass => MASS,
            Category::Temperature => TEMPERATURE,
            Category::Speed => SPEED,
            Category::Volume => VOLUME,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    /// A unit the SDK's `Measurement` model converts.
    Std(Std),
    /// Base units (metre per second, litre) per one of this unit.
    Scale(f64),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UnitDef {
    pub name: &'static str,
    pub symbol: &'static str,
    pub kind: Kind,
}

const LENGTH: &[UnitDef] = &[
    UnitDef { name: "Meters", symbol: "m", kind: Kind::Std(Std::Meters) },
    UnitDef { name: "Kilometers", symbol: "km", kind: Kind::Std(Std::Kilometers) },
    UnitDef { name: "Miles", symbol: "mi", kind: Kind::Std(Std::Miles) },
    UnitDef { name: "Feet", symbol: "ft", kind: Kind::Std(Std::Feet) },
];

const MASS: &[UnitDef] = &[
    UnitDef { name: "Grams", symbol: "g", kind: Kind::Std(Std::Grams) },
    UnitDef { name: "Kilograms", symbol: "kg", kind: Kind::Std(Std::Kilograms) },
    UnitDef { name: "Pounds", symbol: "lb", kind: Kind::Std(Std::Pounds) },
];

const TEMPERATURE: &[UnitDef] = &[
    UnitDef { name: "Celsius", symbol: "\u{b0}C", kind: Kind::Std(Std::Celsius) },
    UnitDef { name: "Fahrenheit", symbol: "\u{b0}F", kind: Kind::Std(Std::Fahrenheit) },
    UnitDef { name: "Kelvin", symbol: "K", kind: Kind::Std(Std::Kelvin) },
];

const SPEED: &[UnitDef] = &[
    UnitDef { name: "Meters per Second", symbol: "m/s", kind: Kind::Scale(1.0) },
    UnitDef { name: "Kilometers per Hour", symbol: "km/h", kind: Kind::Scale(0.277_777_777_777_777_8) },
    UnitDef { name: "Miles per Hour", symbol: "mph", kind: Kind::Scale(0.44704) },
    UnitDef { name: "Knots", symbol: "kn", kind: Kind::Scale(0.514_444) },
];

const VOLUME: &[UnitDef] = &[
    UnitDef { name: "Milliliters", symbol: "mL", kind: Kind::Scale(0.001) },
    UnitDef { name: "Liters", symbol: "L", kind: Kind::Scale(1.0) },
    UnitDef { name: "US Fluid Ounces", symbol: "fl oz", kind: Kind::Scale(0.029_573_529_562_5) },
    UnitDef { name: "US Gallons", symbol: "gal", kind: Kind::Scale(3.785_411_784) },
];

/// `value` in unit `from` expressed in unit `to` of the same category; None for an index out of range.
pub fn convert(category: Category, value: f64, from: usize, to: usize) -> Option<f64> {
    let units = category.units();
    let (a, b) = (units.get(from)?, units.get(to)?);
    match (a.kind, b.kind) {
        (Kind::Std(x), Kind::Std(y)) => Measurement::new(value, x).converted(y).map(|m| m.value),
        (Kind::Scale(x), Kind::Scale(y)) => Some(value * x / y),
        _ => None,
    }
}

/// The conversion as the user edits it: an entry in the `from` unit, shown converted in the `to` unit.
#[derive(Clone, Debug, PartialEq)]
pub struct Converter {
    pub category: Category,
    pub from: usize,
    pub to: usize,
    /// The entry with `.` as the decimal point: "", "-", "12.5".
    input: String,
}

impl Default for Converter {
    fn default() -> Self {
        Converter { category: Category::Length, from: 0, to: 1, input: String::new() }
    }
}

impl Converter {
    pub fn from_unit(&self) -> &'static UnitDef {
        &self.category.units()[self.from]
    }

    pub fn to_unit(&self) -> &'static UnitDef {
        &self.category.units()[self.to]
    }

    /// The entry as a number (an empty or partial entry counts as 0).
    pub fn value(&self) -> f64 {
        self.input.parse().unwrap_or(0.0)
    }

    /// The entry as typed, with `0` when empty (ASCII, `.` decimal). Localise it for display with `format::localize`.
    pub fn entry_text(&self) -> String {
        if self.input.is_empty() || self.input == "-" {
            return if self.input.is_empty() { "0".into() } else { "-0".into() };
        }
        self.input.clone()
    }

    /// The converted value, or None if the units do not belong to the category.
    pub fn result(&self) -> Option<f64> {
        convert(self.category, self.value(), self.from, self.to)
    }

    /// The converted value as the display shows it in `locale` (grouping, decimal comma, scientific when extreme).
    pub fn result_text(&self, locale: Locale) -> String {
        match self.result() {
            Some(v) => format_result(v, locale),
            None => "Error".into(),
        }
    }

    /// Entry key: a digit appended, replacing a lone leading zero.
    pub fn digit(&mut self, digit: u8) {
        if self.input.contains('e') || self.input.chars().filter(char::is_ascii_digit).count() >= MAX_DIGITS {
            return;
        }
        if self.input.trim_start_matches('-') == "0" {
            self.input.pop();
        }
        self.input.push((b'0' + digit.min(9)) as char);
    }

    /// Entry key: the decimal point, after a leading 0 when none was typed.
    pub fn decimal(&mut self) {
        if self.input.contains('.') || self.input.contains('e') {
            return;
        }
        if self.input.is_empty() || self.input == "-" {
            self.input.push('0');
        }
        self.input.push('.');
    }

    /// Change Sign (+/−) on the entry.
    pub fn sign(&mut self) {
        if let Some(rest) = self.input.strip_prefix('-') {
            self.input = rest.to_string();
        } else {
            self.input.insert(0, '-');
            if self.input == "-" {
                self.input.push('0');
            }
        }
    }

    pub fn backspace(&mut self) {
        self.input.pop();
    }

    pub fn clear(&mut self) {
        self.input.clear();
    }

    /// Sets the entry to `value` (Paste). Non-finite values are ignored.
    pub fn set_value(&mut self, value: f64) {
        if value.is_finite() {
            self.input = format::format_number(value);
        }
    }

    /// Switches category; the units become the first two of it.
    pub fn select_category(&mut self, category: Category) {
        if self.category != category {
            self.category = category;
            self.from = 0;
            self.to = 1;
        }
    }

    pub fn next_from(&mut self) {
        self.from = (self.from + 1) % self.category.units().len();
    }

    pub fn next_to(&mut self) {
        self.to = (self.to + 1) % self.category.units().len();
    }

    pub fn swap(&mut self) {
        std::mem::swap(&mut self.from, &mut self.to);
    }

    /// The state for `describe`: category, units and entry.
    pub fn describe(&self) -> String {
        format!("{}:{}->{}:{}", self.category.title(), self.from_unit().symbol, self.to_unit().symbol, self.entry_text())
    }
}

/// A converted value in `locale`: grouping and decimal separators from the SDK's `NumberFormatter`, at most six
/// fraction digits. Very large or very small values fall back to the calculator's scientific notation.
pub fn format_result(value: f64, locale: Locale) -> String {
    let a = value.abs();
    if !value.is_finite() || (a != 0.0 && !(1e-6..1e9).contains(&a)) {
        return format::display(value, locale);
    }
    let mut nf = NumberFormatter::new(SdkLocale::new(locale.code()), NumberStyle::Decimal);
    nf.max_fraction = 6;
    nf.string(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() <= 1e-9 * b.abs().max(1.0)
    }

    fn index_of(category: Category, symbol: &str) -> usize {
        category.units().iter().position(|u| u.symbol == symbol).unwrap()
    }

    fn conv(category: Category, value: f64, from: &str, to: &str) -> f64 {
        convert(category, value, index_of(category, from), index_of(category, to)).unwrap()
    }

    #[test]
    fn length_mass_and_temperature_use_the_sdk_measurement_model() {
        assert!(close(conv(Category::Length, 1.0, "m", "ft"), 3.280_839_895));
        assert!(close(conv(Category::Length, 1.0, "mi", "km"), 1.609_344));
        assert!(close(conv(Category::Mass, 1.0, "kg", "lb"), 2.204_622_622));
        assert!(close(conv(Category::Temperature, 100.0, "\u{b0}C", "\u{b0}F"), 212.0));
        assert!(close(conv(Category::Temperature, 0.0, "\u{b0}C", "K"), 273.15));
        assert!(close(conv(Category::Temperature, -40.0, "\u{b0}F", "\u{b0}C"), -40.0));
    }

    #[test]
    fn speed_and_volume_use_their_scale_factors() {
        assert!(close(conv(Category::Speed, 60.0, "mph", "km/h"), 96.56064));
        assert!(close(conv(Category::Speed, 36.0, "km/h", "m/s"), 10.0));
        assert!(close(conv(Category::Volume, 1.0, "L", "mL"), 1000.0));
        assert!(close(conv(Category::Volume, 1.0, "gal", "L"), 3.785_411_784));
        assert!(close(conv(Category::Volume, 1.0, "L", "fl oz"), 33.814_022_7));
    }

    #[test]
    fn conversions_round_trip_and_out_of_range_units_give_none() {
        for category in Category::ALL {
            let n = category.units().len();
            for a in 0..n {
                for b in 0..n {
                    let there = convert(category, 12.5, a, b).unwrap();
                    assert!(close(convert(category, there, b, a).unwrap(), 12.5), "{category:?} {a} {b}");
                }
            }
        }
        assert_eq!(convert(Category::Length, 1.0, 0, 9), None);
    }

    #[test]
    fn entry_takes_digits_one_decimal_point_and_a_sign() {
        let mut c = Converter::default();
        assert_eq!(c.entry_text(), "0");
        c.digit(0);
        c.digit(7);
        assert_eq!(c.entry_text(), "7");
        c.decimal();
        c.digit(5);
        c.decimal();
        assert_eq!(c.entry_text(), "7.5");
        c.sign();
        assert_eq!(c.entry_text(), "-7.5");
        c.sign();
        assert_eq!(c.entry_text(), "7.5");
        c.backspace();
        c.backspace();
        c.backspace();
        c.backspace();
        assert_eq!(c.entry_text(), "0");
        c.sign();
        assert_eq!(c.entry_text(), "-0");
        c.decimal();
        c.digit(2);
        assert_eq!(c.entry_text(), "-0.2");
        assert!(close(c.value(), -0.2));
        c.clear();
        assert_eq!(c.entry_text(), "0");
    }

    #[test]
    fn entry_is_capped_at_twelve_digits_and_refuses_a_second_point() {
        let mut c = Converter::default();
        for _ in 0..20 {
            c.digit(9);
        }
        assert_eq!(c.entry_text().len(), MAX_DIGITS);
        c.decimal();
        c.decimal();
        assert_eq!(c.entry_text().matches('.').count(), 1);
    }

    #[test]
    fn set_value_puts_a_result_back_into_the_entry() {
        let mut c = Converter::default();
        c.set_value(2.5);
        assert_eq!(c.entry_text(), "2.5");
        c.set_value(f64::NAN);
        assert_eq!(c.entry_text(), "2.5");
    }

    #[test]
    fn choosing_a_category_resets_the_units_and_swap_and_cycle_work() {
        let mut c = Converter::default();
        c.next_from();
        c.next_to();
        c.next_to();
        c.select_category(Category::Speed);
        assert_eq!((c.from, c.to), (0, 1));
        assert_eq!(c.from_unit().symbol, "m/s");
        c.swap();
        assert_eq!((c.from, c.to), (1, 0));
        c.next_from();
        c.next_from();
        c.next_from();
        assert_eq!(c.from, 0, "cycles back to the first unit");
        c.next_to();
        c.next_to();
        c.next_to();
        assert_eq!(c.to, 3);
        c.next_to();
        assert_eq!(c.to, 0, "cycles over the four speed units");
    }

    #[test]
    fn results_are_formatted_with_the_app_locale() {
        assert_eq!(format_result(3.280_839_895, Locale::En), "3.28084");
        assert_eq!(format_result(3.280_839_895, Locale::Is), "3,28084");
        assert_eq!(format_result(1_234_567.0, Locale::En), "1,234,567");
        assert_eq!(format_result(1_234_567.0, Locale::Is), "1.234.567");
        assert_eq!(format_result(0.0, Locale::En), "0");
        assert_eq!(format_result(-2.5, Locale::Is), "-2,5");
        assert_eq!(format_result(1e12, Locale::En), "1e12");
    }

    #[test]
    fn the_converter_shows_its_result_for_the_entry() {
        let mut c = Converter::default();
        c.digit(1);
        c.digit(0);
        c.next_to();
        c.next_to(); // from metres (index 0) to feet (index 3)
        assert_eq!(c.to_unit().symbol, "ft");
        assert_eq!(c.result_text(Locale::En), "32.808399");
        c.select_category(Category::Temperature);
        c.clear();
        c.digit(5);
        assert_eq!(c.result_text(Locale::En), "41");
        c.sign();
        assert_eq!(c.result_text(Locale::En), "23");
        assert_eq!(c.describe(), "Temperature:\u{b0}C->\u{b0}F:-5");
    }
}
