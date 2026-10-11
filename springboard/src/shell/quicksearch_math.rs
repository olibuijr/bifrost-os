//! Search answers: inline calculator results, unit conversion
//! (length, weight, temperature) and Settings keyword aliases.

/// "12*(3+4)" -> "84"; "5 km to mi" -> "3.10686 mi". None when the query is neither.
pub fn answer(query: &str) -> Option<String> {
    convert(query).or_else(|| calc(query))
}

/// Settings pages a keyword stands for ("bluetooth" -> none, "brightness" -> Display).
pub fn settings_aliases() -> &'static [(&'static str, &'static str)] {
    &[
        ("brightness", "Display & Brightness"),
        ("dark mode", "Display & Brightness"),
        ("appearance", "Display & Brightness"),
        ("text size", "Display & Brightness"),
        ("night shift", "Display & Brightness"),
        ("wifi", "Wi-Fi"),
        ("wireless", "Wi-Fi"),
        ("network", "Wi-Fi"),
        ("internet", "Wi-Fi"),
        ("background", "Wallpaper"),
        ("lock screen", "Wallpaper"),
        ("version", "About"),
        ("storage", "About"),
        ("serial number", "About"),
        ("software", "About"),
        ("power", "Battery"),
        ("low power", "Battery"),
        ("charging", "Battery"),
        ("dnd", "Do Not Disturb"),
        ("silence", "Do Not Disturb"),
        ("alerts", "Notifications"),
        ("banners", "Notifications"),
        ("voiceover", "Accessibility"),
        ("zoom", "Accessibility"),
        ("magnifier", "Accessibility"),
        ("touch", "AssistiveTouch"),
        ("home button", "AssistiveTouch"),
    ]
}

// -- calculator --

struct P<'a> {
    s: &'a [u8],
    i: usize,
}

impl P<'_> {
    fn ws(&mut self) {
        while self.s.get(self.i).is_some_and(|c| *c == b' ') {
            self.i += 1;
        }
    }
    fn eat(&mut self, c: u8) -> bool {
        self.ws();
        if self.s.get(self.i) == Some(&c) {
            self.i += 1;
            true
        } else {
            false
        }
    }
    fn expr(&mut self) -> Option<f64> {
        let mut v = self.term()?;
        loop {
            if self.eat(b'+') {
                v += self.term()?;
            } else if self.eat(b'-') {
                v -= self.term()?;
            } else {
                return Some(v);
            }
        }
    }
    fn term(&mut self) -> Option<f64> {
        let mut v = self.power()?;
        loop {
            if self.eat(b'*') {
                v *= self.power()?;
            } else if self.eat(b'/') {
                v /= self.power()?;
            } else if self.eat(b'%') {
                v %= self.power()?;
            } else {
                return Some(v);
            }
        }
    }
    fn power(&mut self) -> Option<f64> {
        let b = self.unary()?;
        if self.eat(b'^') {
            let e = self.power()?;
            return Some(b.powf(e));
        }
        Some(b)
    }
    fn unary(&mut self) -> Option<f64> {
        if self.eat(b'-') {
            return self.unary().map(|v| -v);
        }
        if self.eat(b'+') {
            return self.unary();
        }
        self.atom()
    }
    fn atom(&mut self) -> Option<f64> {
        if self.eat(b'(') {
            let v = self.expr()?;
            return self.eat(b')').then_some(v);
        }
        self.ws();
        let start = self.i;
        while self.s.get(self.i).is_some_and(|c| c.is_ascii_digit() || *c == b'.') {
            self.i += 1;
        }
        std::str::from_utf8(&self.s[start..self.i]).ok()?.parse().ok()
    }
}

fn calc(query: &str) -> Option<String> {
    let q: String = query.trim().trim_end_matches('=').replace(['x', 'X', '×'], "*").replace('÷', "/").replace(',', ".");
    if !q.bytes().any(|c| c.is_ascii_digit()) || !q.bytes().all(|c| c.is_ascii_digit() || b" .+-*/%^()".contains(&c)) {
        return None;
    }
    // a bare number or "-5" is not a calculation
    if !q.bytes().skip(1).any(|c| b"+-*/%^".contains(&c)) {
        return None;
    }
    let mut p = P { s: q.as_bytes(), i: 0 };
    let v = p.expr()?;
    p.ws();
    (p.i == q.len() && v.is_finite()).then(|| fmt(v))
}

fn fmt(v: f64) -> String {
    if v.abs() >= 1e15 || (v != 0.0 && v.abs() < 1e-6) {
        return format!("{v:e}");
    }
    let s = format!("{v:.6}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.into() }
}

// -- unit conversion --

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Length,
    Weight,
    Temp,
}

/// (names, kind, factor to the base unit: metre, kilogram; kelvin-less for temp)
const UNITS: &[(&[&str], Kind, f64, &str)] = &[
    (&["mm", "millimeter", "millimeters", "millimetre", "millimetres"], Kind::Length, 0.001, "mm"),
    (&["cm", "centimeter", "centimeters", "centimetre", "centimetres"], Kind::Length, 0.01, "cm"),
    (&["m", "meter", "meters", "metre", "metres"], Kind::Length, 1.0, "m"),
    (&["km", "kilometer", "kilometers", "kilometre", "kilometres"], Kind::Length, 1000.0, "km"),
    (&["in", "inch", "inches"], Kind::Length, 0.0254, "in"),
    (&["ft", "foot", "feet"], Kind::Length, 0.3048, "ft"),
    (&["yd", "yard", "yards"], Kind::Length, 0.9144, "yd"),
    (&["mi", "mile", "miles"], Kind::Length, 1609.344, "mi"),
    (&["mg", "milligram", "milligrams"], Kind::Weight, 1e-6, "mg"),
    (&["g", "gram", "grams"], Kind::Weight, 0.001, "g"),
    (&["kg", "kilo", "kilos", "kilogram", "kilograms"], Kind::Weight, 1.0, "kg"),
    (&["oz", "ounce", "ounces"], Kind::Weight, 0.028349523125, "oz"),
    (&["lb", "lbs", "pound", "pounds"], Kind::Weight, 0.45359237, "lb"),
    (&["st", "stone"], Kind::Weight, 6.35029318, "st"),
    (&["c", "celsius", "°c"], Kind::Temp, 1.0, "°C"),
    (&["f", "fahrenheit", "°f"], Kind::Temp, 1.0, "°F"),
    (&["k", "kelvin"], Kind::Temp, 1.0, "K"),
];

fn unit(name: &str) -> Option<&'static (&'static [&'static str], Kind, f64, &'static str)> {
    let n = name.trim().to_lowercase();
    UNITS.iter().find(|u| u.0.contains(&n.as_str()))
}

fn to_celsius(v: f64, sym: &str) -> f64 {
    match sym {
        "°F" => (v - 32.0) * 5.0 / 9.0,
        "K" => v - 273.15,
        _ => v,
    }
}

fn from_celsius(c: f64, sym: &str) -> f64 {
    match sym {
        "°F" => c * 9.0 / 5.0 + 32.0,
        "K" => c + 273.15,
        _ => c,
    }
}

fn convert(query: &str) -> Option<String> {
    let q = query.trim().to_lowercase().replace(',', ".");
    let split = q.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))?;
    let value: f64 = q[..split].parse().ok()?;
    let rest = q[split..].trim();
    let (from, to) = [" to ", " in ", " = ", " as "].iter().find_map(|sep| rest.split_once(sep))?;
    let (from, to) = (unit(from)?, unit(to)?);
    if from.1 != to.1 {
        return None;
    }
    let out = if from.1 == Kind::Temp {
        from_celsius(to_celsius(value, from.3), to.3)
    } else {
        value * from.2 / to.2
    };
    let sep = if to.1 == Kind::Temp { "" } else { " " };
    Some(format!("{}{sep}{}", fmt((out * 1e5).round() / 1e5), to.3))
}
