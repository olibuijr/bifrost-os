//! Display options of the Calculator: thousands separator on or off, a fixed number of decimal places (or
//! automatic), scientific notation, and the stored form of the three settings. `render` turns a value into the text
//! the display shows under those options. Pure std.
#![allow(dead_code)] // View > Thousands Separator / Decimal Places / Scientific Notation wire these in next

/// Most decimal places the menu offers.
pub const MAX_PLACES: u8 = 15;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    pub thousands: bool,
    /// None is automatic: as many places as the value needs, up to ten.
    pub places: Option<u8>,
    pub scientific: bool,
    pub decimal: char,
    pub group: char,
}

impl Default for Options {
    fn default() -> Self {
        Options { thousands: true, places: None, scientific: false, decimal: '.', group: ',' }
    }
}

impl Options {
    /// The decimal-places menu: Auto, then 0 to 15.
    pub fn place_choices() -> Vec<Option<u8>> {
        std::iter::once(None).chain((0..=MAX_PLACES).map(Some)).collect()
    }

    pub fn to_text(&self) -> String {
        let p = self.places.map_or("auto".to_string(), |p| p.to_string());
        format!("thousands={}\nplaces={p}\nscientific={}\n", self.thousands as u8, self.scientific as u8)
    }

    pub fn from_text(text: &str) -> Options {
        let mut o = Options::default();
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            match k {
                "thousands" => o.thousands = v == "1",
                "places" => o.places = v.parse::<u8>().ok().map(|p| p.min(MAX_PLACES)),
                "scientific" => o.scientific = v == "1",
                _ => {}
            }
        }
        o
    }

    /// The text for `value`. Non-finite values show as `Error`.
    pub fn render(&self, value: f64) -> String {
        if !value.is_finite() {
            return "Error".to_string();
        }
        if self.scientific {
            return self.scientific_text(value);
        }
        let mut text = match self.places {
            Some(p) => format!("{:.*}", usize::from(p), value),
            None => {
                // keep about twelve significant digits so binary noise past them is rounded away
                let digits = if value.abs() >= 1.0 { value.abs().log10().floor() as i32 + 1 } else { 1 };
                let t = format!("{:.*}", (12 - digits).clamp(0, 10) as usize, value);
                if t.contains('.') { t.trim_end_matches('0').trim_end_matches('.').to_string() } else { t }
            }
        };
        if text == "-0" {
            text = "0".to_string();
        }
        self.separate(&text)
    }

    fn scientific_text(&self, value: f64) -> String {
        if value == 0.0 {
            return "0e0".to_string();
        }
        let exp = value.abs().log10().floor() as i32;
        let mut mantissa = value / 10f64.powi(exp);
        let places = usize::from(self.places.unwrap_or(6).min(MAX_PLACES));
        let mut text = format!("{mantissa:.places$}");
        // rounding can carry the mantissa to 10
        let mut exp = exp;
        if text.trim_start_matches('-').starts_with("10") {
            exp += 1;
            mantissa /= 10.0;
            text = format!("{mantissa:.places$}");
        }
        if self.places.is_none() && text.contains('.') {
            text = text.trim_end_matches('0').trim_end_matches('.').to_string();
        }
        format!("{}e{exp}", text.replace('.', &self.decimal.to_string()))
    }

    /// Groups the integer part by thousands and swaps in the locale's marks.
    fn separate(&self, text: &str) -> String {
        let (sign, rest) = match text.strip_prefix('-') {
            Some(r) => ("-", r),
            None => ("", text),
        };
        let (int, frac) = match rest.split_once('.') {
            Some((i, f)) => (i, Some(f)),
            None => (rest, None),
        };
        let mut grouped = String::new();
        let len = int.chars().count();
        for (i, c) in int.chars().enumerate() {
            if self.thousands && i > 0 && (len - i) % 3 == 0 {
                grouped.push(self.group);
            }
            grouped.push(c);
        }
        match frac {
            Some(f) => format!("{sign}{grouped}{}{f}", self.decimal),
            None => format!("{sign}{grouped}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thousands_on_and_off() {
        let mut o = Options::default();
        assert_eq!(o.render(1234567.891), "1,234,567.891");
        assert_eq!(o.render(-1234.5), "-1,234.5");
        assert_eq!(o.render(999.0), "999");
        o.thousands = false;
        assert_eq!(o.render(1234567.0), "1234567");
    }

    #[test]
    fn fixed_places_and_locale_marks() {
        let mut o = Options::default();
        o.places = Some(2);
        assert_eq!(o.render(3.14159), "3.14");
        assert_eq!(o.render(2.0), "2.00");
        o.places = Some(0);
        assert_eq!(o.render(2.4), "2");
        o.places = Some(2);
        o.decimal = ',';
        o.group = '.';
        assert_eq!(o.render(1234.5), "1.234,50");
    }

    #[test]
    fn automatic_drops_float_noise() {
        let o = Options::default();
        assert_eq!(o.render(0.1 + 0.2), "0.3");
        assert_eq!(o.render(-0.0), "0");
        assert_eq!(o.render(f64::NAN), "Error");
        assert_eq!(o.render(f64::INFINITY), "Error");
    }

    #[test]
    fn scientific() {
        let mut o = Options::default();
        o.scientific = true;
        assert_eq!(o.render(12345.0), "1.2345e4");
        assert_eq!(o.render(0.00025), "2.5e-4");
        assert_eq!(o.render(0.0), "0e0");
        o.places = Some(2);
        assert_eq!(o.render(12345.0), "1.23e4");
        assert_eq!(o.render(9.999), "1.00e1");
    }

    #[test]
    fn stored_form_round_trips() {
        let o = Options { thousands: false, places: Some(4), scientific: true, ..Options::default() };
        assert_eq!(Options::from_text(&o.to_text()), o);
        assert_eq!(Options::from_text("places=99").places, Some(MAX_PLACES));
        assert_eq!(Options::from_text("places=auto").places, None);
        assert_eq!(Options::place_choices().len(), 17);
    }
}
