//! The Convert mode of Calculator: unit conversion over thirteen categories.
//!
//! Length, mass and temperature use the SDK's `Measurement` model (`bifrost_app::locale`). That model has
//! no other dimension yet, so the rest (speed, volume, area, time, data size, energy, pressure, power, angle and
//! fuel economy) use the scale factors below (the SDK's `Unit` cannot hold them). Numbers are shown with the
//! SDK's `NumberFormatter` in the app's locale.

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
    Area,
    Time,
    DataSize,
    Energy,
    Pressure,
    Power,
    Angle,
    FuelEconomy,
}

impl Category {
    pub const ALL: [Category; 13] = [
        Category::Length,
        Category::Mass,
        Category::Temperature,
        Category::Speed,
        Category::Volume,
        Category::Area,
        Category::Time,
        Category::DataSize,
        Category::Energy,
        Category::Pressure,
        Category::Power,
        Category::Angle,
        Category::FuelEconomy,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Category::Length => "Length",
            Category::Mass => "Mass",
            Category::Temperature => "Temperature",
            Category::Speed => "Speed",
            Category::Volume => "Volume",
            Category::Area => "Area",
            Category::Time => "Time",
            Category::DataSize => "Data Size",
            Category::Energy => "Energy",
            Category::Pressure => "Pressure",
            Category::Power => "Power",
            Category::Angle => "Angle",
            Category::FuelEconomy => "Fuel",
        }
    }

    pub fn units(self) -> &'static [UnitDef] {
        match self {
            Category::Length => LENGTH,
            Category::Mass => MASS,
            Category::Temperature => TEMPERATURE,
            Category::Speed => SPEED,
            Category::Volume => VOLUME,
            Category::Area => AREA,
            Category::Time => TIME,
            Category::DataSize => DATA_SIZE,
            Category::Energy => ENERGY,
            Category::Pressure => PRESSURE,
            Category::Power => POWER,
            Category::Angle => ANGLE,
            Category::FuelEconomy => FUEL_ECONOMY,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    /// A unit the SDK's `Measurement` model converts.
    Std(Std),
    /// Base units (metre per second, litre, square metre, second, byte, joule, pascal, watt, degree, kilometre per
    /// litre) per one of this unit.
    Scale(f64),
    /// A unit that is the reciprocal of the base unit: `factor / base` (litres per 100 km against km per litre).
    Reciprocal(f64),
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

const AREA: &[UnitDef] = &[
    UnitDef { name: "Square Meters", symbol: "m\u{b2}", kind: Kind::Scale(1.0) },
    UnitDef { name: "Square Kilometers", symbol: "km\u{b2}", kind: Kind::Scale(1e6) },
    UnitDef { name: "Hectares", symbol: "ha", kind: Kind::Scale(1e4) },
    UnitDef { name: "Square Feet", symbol: "ft\u{b2}", kind: Kind::Scale(0.092_903_04) },
    UnitDef { name: "Acres", symbol: "ac", kind: Kind::Scale(4_046.856_422_4) },
    UnitDef { name: "Square Miles", symbol: "mi\u{b2}", kind: Kind::Scale(2_589_988.110_336) },
];

const TIME: &[UnitDef] = &[
    UnitDef { name: "Seconds", symbol: "s", kind: Kind::Scale(1.0) },
    UnitDef { name: "Minutes", symbol: "min", kind: Kind::Scale(60.0) },
    UnitDef { name: "Hours", symbol: "h", kind: Kind::Scale(3_600.0) },
    UnitDef { name: "Days", symbol: "d", kind: Kind::Scale(86_400.0) },
    UnitDef { name: "Weeks", symbol: "wk", kind: Kind::Scale(604_800.0) },
];

const DATA_SIZE: &[UnitDef] = &[
    UnitDef { name: "Bits", symbol: "bit", kind: Kind::Scale(0.125) },
    UnitDef { name: "Bytes", symbol: "B", kind: Kind::Scale(1.0) },
    UnitDef { name: "Kilobytes", symbol: "kB", kind: Kind::Scale(1e3) },
    UnitDef { name: "Megabytes", symbol: "MB", kind: Kind::Scale(1e6) },
    UnitDef { name: "Gigabytes", symbol: "GB", kind: Kind::Scale(1e9) },
    UnitDef { name: "Terabytes", symbol: "TB", kind: Kind::Scale(1e12) },
    UnitDef { name: "Kibibytes", symbol: "KiB", kind: Kind::Scale(1_024.0) },
    UnitDef { name: "Mebibytes", symbol: "MiB", kind: Kind::Scale(1_048_576.0) },
    UnitDef { name: "Gibibytes", symbol: "GiB", kind: Kind::Scale(1_073_741_824.0) },
];

const ENERGY: &[UnitDef] = &[
    UnitDef { name: "Joules", symbol: "J", kind: Kind::Scale(1.0) },
    UnitDef { name: "Kilojoules", symbol: "kJ", kind: Kind::Scale(1e3) },
    UnitDef { name: "Calories", symbol: "cal", kind: Kind::Scale(4.184) },
    UnitDef { name: "Kilocalories", symbol: "kcal", kind: Kind::Scale(4_184.0) },
    UnitDef { name: "Watt Hours", symbol: "Wh", kind: Kind::Scale(3_600.0) },
    UnitDef { name: "Kilowatt Hours", symbol: "kWh", kind: Kind::Scale(3.6e6) },
];

const PRESSURE: &[UnitDef] = &[
    UnitDef { name: "Pascals", symbol: "Pa", kind: Kind::Scale(1.0) },
    UnitDef { name: "Kilopascals", symbol: "kPa", kind: Kind::Scale(1e3) },
    UnitDef { name: "Bars", symbol: "bar", kind: Kind::Scale(1e5) },
    UnitDef { name: "Pounds per Square Inch", symbol: "psi", kind: Kind::Scale(6_894.757_293_168) },
    UnitDef { name: "Atmospheres", symbol: "atm", kind: Kind::Scale(101_325.0) },
    UnitDef { name: "Millimeters of Mercury", symbol: "mmHg", kind: Kind::Scale(133.322_387_415) },
];

const POWER: &[UnitDef] = &[
    UnitDef { name: "Watts", symbol: "W", kind: Kind::Scale(1.0) },
    UnitDef { name: "Kilowatts", symbol: "kW", kind: Kind::Scale(1e3) },
    UnitDef { name: "Megawatts", symbol: "MW", kind: Kind::Scale(1e6) },
    UnitDef { name: "Horsepower", symbol: "hp", kind: Kind::Scale(745.699_871_582_27) },
];

const ANGLE: &[UnitDef] = &[
    UnitDef { name: "Degrees", symbol: "\u{b0}", kind: Kind::Scale(1.0) },
    UnitDef { name: "Radians", symbol: "rad", kind: Kind::Scale(57.295_779_513_082_32) },
    UnitDef { name: "Gradians", symbol: "grad", kind: Kind::Scale(0.9) },
    UnitDef { name: "Revolutions", symbol: "rev", kind: Kind::Scale(360.0) },
];

const FUEL_ECONOMY: &[UnitDef] = &[
    UnitDef { name: "Miles per Gallon", symbol: "mpg", kind: Kind::Scale(0.425_143_707) },
    UnitDef { name: "Kilometers per Liter", symbol: "km/L", kind: Kind::Scale(1.0) },
    UnitDef { name: "Liters per 100 Kilometers", symbol: "L/100km", kind: Kind::Reciprocal(100.0) },
];

/// `value` of a scale or reciprocal unit in the category's base unit; None for a reciprocal of zero or an SDK unit.
fn to_base(kind: Kind, value: f64) -> Option<f64> {
    match kind {
        Kind::Scale(x) => Some(value * x),
        Kind::Reciprocal(k) => (value != 0.0).then(|| k / value),
        Kind::Std(_) => None,
    }
}

/// `base` expressed in a scale or reciprocal unit.
fn from_base(kind: Kind, base: f64) -> Option<f64> {
    match kind {
        Kind::Scale(x) => Some(base / x),
        Kind::Reciprocal(k) => (base != 0.0).then(|| k / base),
        Kind::Std(_) => None,
    }
}

/// `value` in unit `from` expressed in unit `to` of the same category; None for an index out of range.
pub fn convert(category: Category, value: f64, from: usize, to: usize) -> Option<f64> {
    let units = category.units();
    let (a, b) = (units.get(from)?, units.get(to)?);
    match (a.kind, b.kind) {
        (Kind::Std(x), Kind::Std(y)) => Measurement::new(value, x).converted(y).map(|m| m.value),
        (Kind::Std(_), _) | (_, Kind::Std(_)) => None,
        (from_kind, to_kind) => from_base(to_kind, to_base(from_kind, value)?),
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
    #[cfg(test)]
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
    fn the_other_categories_convert_through_their_base_unit() {
        assert!(close(conv(Category::Area, 1.0, "ha", "m\u{b2}"), 10_000.0));
        assert!(close(conv(Category::Area, 1.0, "mi\u{b2}", "ac"), 640.0));
        assert!(close(conv(Category::Time, 2.0, "h", "min"), 120.0));
        assert!(close(conv(Category::Time, 1.0, "wk", "d"), 7.0));
        assert!(close(conv(Category::DataSize, 1.0, "GiB", "MiB"), 1024.0));
        assert!(close(conv(Category::DataSize, 1.0, "kB", "bit"), 8000.0));
        assert!(close(conv(Category::Energy, 1.0, "kcal", "kJ"), 4.184));
        assert!(close(conv(Category::Energy, 1.0, "kWh", "Wh"), 1000.0));
        assert!(close(conv(Category::Pressure, 1.0, "atm", "kPa"), 101.325));
        assert!(close(conv(Category::Pressure, 14.5, "psi", "bar"), 0.999_739_807_509_36));
        assert!(close(conv(Category::Power, 1.0, "hp", "W"), 745.699_871_582_27));
        assert!(close(conv(Category::Angle, 180.0, "\u{b0}", "rad"), std::f64::consts::PI));
        assert!(close(conv(Category::Angle, 1.0, "rev", "grad"), 400.0));
    }

    #[test]
    fn fuel_economy_includes_the_reciprocal_unit() {
        assert!(close(conv(Category::FuelEconomy, 5.0, "L/100km", "km/L"), 20.0));
        assert!(close(conv(Category::FuelEconomy, 20.0, "km/L", "L/100km"), 5.0));
        assert!(close(conv(Category::FuelEconomy, 30.0, "mpg", "L/100km"), 7.840_486_119_046_173));
        assert!(close(conv(Category::FuelEconomy, 10.0, "L/100km", "mpg"), 23.521_458_357_138_52));
        let (mpg, per_100) = (index_of(Category::FuelEconomy, "mpg"), index_of(Category::FuelEconomy, "L/100km"));
        assert_eq!(convert(Category::FuelEconomy, 0.0, mpg, per_100), None, "zero mpg has no consumption");
        assert_eq!(convert(Category::FuelEconomy, 0.0, per_100, mpg), None);
    }

    #[test]
    fn every_category_has_units_a_title_and_distinct_symbols() {
        assert_eq!(Category::ALL.len(), 13);
        for category in Category::ALL {
            let units = category.units();
            assert!(units.len() >= 3, "{category:?}");
            assert!(!category.title().is_empty());
            for (i, a) in units.iter().enumerate() {
                assert!(units.iter().skip(i + 1).all(|b| b.symbol != a.symbol && b.name != a.name), "{category:?} {}", a.name);
            }
        }
        let mut titles: Vec<&str> = Category::ALL.iter().map(|c| c.title()).collect();
        titles.sort_unstable();
        titles.dedup();
        assert_eq!(titles.len(), Category::ALL.len());
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
