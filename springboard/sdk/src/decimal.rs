//! `Decimal` and `RoundingMode`: exact base-10 numbers for money and other values where `f64`
//! would drift (0.1 + 0.2 is exactly 0.3).
//!
//! A value is `mantissa * 10^exponent` with an `i128` mantissa (about 38 digits). Arithmetic is checked: results that
//! do not fit give [`DecimalError::Overflow`], and division by zero gives [`DecimalError::DivideByZero`] (the reference platform's
//! the reference number type reports the same conditions). `+`, `-` and `*` operators exist and panic on overflow; use
//! `adding`, `subtracting`, `multiplying` and `dividing` to handle it. Division keeps up to 18 fractional digits and
//! truncates. Equality, ordering and hashing compare the numeric value, so 1.0 == 1.
//!
//! Gaps: no object wrapper, no `power`, no NaN.
use crate::locale::Locale;
use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::{Add, Mul, Neg, Sub};

/// Rounding modes for `rounded`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoundingMode {
    /// Half away from zero (`.plain`).
    Plain,
    /// Toward zero (`.down`).
    Down,
    /// Away from zero (`.up`).
    Up,
    /// Half to even (`.bankers`).
    Bankers,
}

/// Why a decimal operation failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecimalError {
    Overflow,
    DivideByZero,
}

impl fmt::Display for DecimalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            DecimalError::Overflow => "decimal overflow",
            DecimalError::DivideByZero => "decimal divide by zero",
        })
    }
}

impl std::error::Error for DecimalError {}

/// Most fractional digits a quotient keeps.
const DIVISION_SCALE: i32 = 18;

fn pow10(n: u32) -> Option<i128> {
    10i128.checked_pow(n)
}

/// An exact decimal number.
#[derive(Clone, Copy, Debug, Default)]
pub struct Decimal {
    mantissa: i128,
    exponent: i32,
}

impl Decimal {
    pub const ZERO: Decimal = Decimal { mantissa: 0, exponent: 0 };

    pub fn new(mantissa: i128, exponent: i32) -> Decimal {
        Decimal { mantissa, exponent }
    }
    pub fn from_i64(v: i64) -> Decimal {
        Decimal::new(i128::from(v), 0)
    }
    pub fn mantissa(&self) -> i128 {
        self.mantissa
    }
    pub fn exponent(&self) -> i32 {
        self.exponent
    }
    pub fn is_zero(&self) -> bool {
        self.mantissa == 0
    }

    /// `Decimal(string:)` with the `.` separator.
    pub fn parse(text: &str) -> Option<Decimal> {
        Decimal::parse_with_separator(text, '.')
    }

    /// `Decimal(string:locale:)`: the locale's decimal separator is the fraction mark.
    pub fn from_string(text: &str, locale: &Locale) -> Option<Decimal> {
        Decimal::parse_with_separator(text, locale.decimal_separator())
    }

    fn parse_with_separator(text: &str, sep: char) -> Option<Decimal> {
        let t = text.trim();
        let (neg, rest) = match t.chars().next()? {
            '-' => (true, &t[1..]),
            '+' => (false, &t[1..]),
            _ => (false, t),
        };
        let (num, exp) = match rest.find(['e', 'E']) {
            Some(i) => (&rest[..i], rest[i + 1..].parse::<i32>().ok()?),
            None => (rest, 0),
        };
        let (int_part, frac_part) = num.split_once(sep).unwrap_or((num, ""));
        if int_part.is_empty() && frac_part.is_empty() {
            return None;
        }
        let mut mantissa: i128 = 0;
        for c in int_part.chars().chain(frac_part.chars()) {
            let digit = c.to_digit(10)?;
            mantissa = mantissa.checked_mul(10)?.checked_add(i128::from(digit))?;
        }
        let exponent = exp.checked_sub(i32::try_from(frac_part.chars().count()).ok()?)?;
        Some(Decimal::new(if neg { -mantissa } else { mantissa }, exponent))
    }

    /// Both mantissas scaled to the smaller exponent.
    fn aligned(&self, other: &Decimal) -> Result<(i128, i128, i32), DecimalError> {
        let exp = self.exponent.min(other.exponent);
        let scale = |d: &Decimal| -> Result<i128, DecimalError> {
            let p = pow10((d.exponent - exp) as u32).ok_or(DecimalError::Overflow)?;
            d.mantissa.checked_mul(p).ok_or(DecimalError::Overflow)
        };
        Ok((scale(self)?, scale(other)?, exp))
    }

    pub fn adding(&self, other: &Decimal) -> Result<Decimal, DecimalError> {
        let (a, b, exp) = self.aligned(other)?;
        Ok(Decimal::new(a.checked_add(b).ok_or(DecimalError::Overflow)?, exp))
    }

    pub fn subtracting(&self, other: &Decimal) -> Result<Decimal, DecimalError> {
        let (a, b, exp) = self.aligned(other)?;
        Ok(Decimal::new(a.checked_sub(b).ok_or(DecimalError::Overflow)?, exp))
    }

    pub fn multiplying(&self, other: &Decimal) -> Result<Decimal, DecimalError> {
        let m = self.mantissa.checked_mul(other.mantissa).ok_or(DecimalError::Overflow)?;
        let e = self.exponent.checked_add(other.exponent).ok_or(DecimalError::Overflow)?;
        Ok(Decimal::new(m, e))
    }

    /// Quotient truncated to at most 18 fractional digits.
    pub fn dividing(&self, other: &Decimal) -> Result<Decimal, DecimalError> {
        if other.mantissa == 0 {
            return Err(DecimalError::DivideByZero);
        }
        // Finest scale first; back off while the scaled numerator overflows.
        for digits in (0..=DIVISION_SCALE).rev() {
            if let Some(num) = pow10(digits as u32).and_then(|p| self.mantissa.checked_mul(p)) {
                let exponent = self.exponent - other.exponent - digits;
                return Ok(Decimal::new(num / other.mantissa, exponent));
            }
        }
        Err(DecimalError::Overflow)
    }

    /// Rounds to `scale` fractional digits (negative scales round to tens, hundreds, ...).
    pub fn rounded(&self, scale: i32, mode: RoundingMode) -> Decimal {
        let target = -scale;
        if self.exponent >= target {
            return *self;
        }
        let drop = (target - self.exponent) as u32;
        let magnitude = self.mantissa.unsigned_abs();
        // More than 38 dropped digits leaves a quotient of 0 and a remainder below one half.
        let (quotient, bump) = match pow10(drop) {
            Some(p) => {
                let p = p as u128;
                let (q, r) = (magnitude / p, magnitude % p);
                let half = p / 2;
                let bump = match mode {
                    RoundingMode::Down => false,
                    RoundingMode::Up => r != 0,
                    RoundingMode::Plain => r >= half,
                    RoundingMode::Bankers => r > half || (r == half && q % 2 == 1),
                };
                (q, bump)
            }
            None => (0, mode == RoundingMode::Up && magnitude != 0),
        };
        let m = (quotient + u128::from(bump)) as i128;
        Decimal::new(if self.mantissa < 0 { -m } else { m }, target)
    }

    /// Value as `f64` (`doubleValue`).
    pub fn to_f64(&self) -> f64 {
        self.mantissa as f64 * 10f64.powi(self.exponent)
    }

    /// Mantissa with trailing zeros removed (zero normalises to 0e0).
    fn normalized(&self) -> (i128, i32) {
        if self.mantissa == 0 {
            return (0, 0);
        }
        let (mut m, mut e) = (self.mantissa, self.exponent);
        while m % 10 == 0 {
            m /= 10;
            e += 1;
        }
        (m, e)
    }
}

impl PartialEq for Decimal {
    fn eq(&self, other: &Decimal) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl Eq for Decimal {}

impl PartialOrd for Decimal {
    fn partial_cmp(&self, other: &Decimal) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Decimal {
    fn cmp(&self, other: &Decimal) -> Ordering {
        if let Ok((a, b, _)) = self.aligned(other) {
            return a.cmp(&b);
        }
        // Alignment overflowed: compare sign, then the position of the leading digit.
        let (sa, sb) = (self.mantissa.signum(), other.mantissa.signum());
        if sa != sb {
            return sa.cmp(&sb);
        }
        let (ma, ea) = self.normalized();
        let (mb, eb) = other.normalized();
        let lead = |m: i128, e: i32| m.unsigned_abs().to_string().len() as i64 + i64::from(e);
        let by_size = lead(ma, ea).cmp(&lead(mb, eb));
        if sa >= 0 { by_size } else { by_size.reverse() }
    }
}

impl Hash for Decimal {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.normalized().hash(state);
    }
}

/// `description`: plain notation, no trailing zeros.
impl fmt::Display for Decimal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (m, e) = self.normalized();
        let digits = m.unsigned_abs().to_string();
        let sign = if m < 0 { "-" } else { "" };
        if e >= 0 {
            let zeros = if m == 0 { 0 } else { e as usize };
            return write!(f, "{sign}{digits}{}", "0".repeat(zeros));
        }
        let frac = (-e) as usize;
        if digits.len() > frac {
            let (int_digits, frac_digits) = digits.split_at(digits.len() - frac);
            write!(f, "{sign}{int_digits}.{frac_digits}")
        } else {
            write!(f, "{sign}0.{}{digits}", "0".repeat(frac - digits.len()))
        }
    }
}

impl Add for Decimal {
    type Output = Decimal;
    fn add(self, rhs: Decimal) -> Decimal {
        self.adding(&rhs).expect("decimal overflow")
    }
}
impl Sub for Decimal {
    type Output = Decimal;
    fn sub(self, rhs: Decimal) -> Decimal {
        self.subtracting(&rhs).expect("decimal overflow")
    }
}
impl Mul for Decimal {
    type Output = Decimal;
    fn mul(self, rhs: Decimal) -> Decimal {
        self.multiplying(&rhs).expect("decimal overflow")
    }
}
impl Neg for Decimal {
    type Output = Decimal;
    fn neg(self) -> Decimal {
        Decimal::new(-self.mantissa, self.exponent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> Decimal {
        Decimal::parse(s).expect("valid decimal")
    }

    #[test]
    fn parses_and_prints() {
        assert_eq!(d("12.50").to_string(), "12.5");
        assert_eq!(d("-0.05").to_string(), "-0.05");
        assert_eq!(d("1e3").to_string(), "1000");
        assert_eq!(d("0.0").to_string(), "0");
        assert_eq!(d("  7 ").to_string(), "7");
        for bad in ["", "-", ".", "1.2.3", "abc", "1,5", "1e"] {
            assert!(Decimal::parse(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn locale_separator_is_the_fraction_mark() {
        let is = Locale::new("is_IS");
        assert_eq!(Decimal::from_string("1,25", &is), Some(d("1.25")));
        assert_eq!(Decimal::from_string("1.25", &is), None);
        assert_eq!(Decimal::from_string("1.25", &Locale::new("en_US")), Some(d("1.25")));
    }

    #[test]
    fn arithmetic_is_exact() {
        assert_eq!(d("0.1") + d("0.2"), d("0.3"));
        assert_eq!(d("1") - d("0.9"), d("0.1"));
        assert_eq!((d("1.5") * d("2.5")).to_string(), "3.75");
        assert_eq!(d("1").dividing(&d("4")).unwrap().to_string(), "0.25");
        assert_eq!(d("1").dividing(&d("3")).unwrap().to_string(), "0.333333333333333333");
        assert_eq!(d("100").dividing(&d("0.5")).unwrap(), d("200"));
        assert_eq!((-d("2")).to_string(), "-2");
    }

    #[test]
    fn divide_by_zero_and_overflow_are_errors() {
        assert_eq!(d("1").dividing(&Decimal::ZERO), Err(DecimalError::DivideByZero));
        let big = Decimal::new(i128::MAX, 0);
        assert_eq!(big.adding(&big), Err(DecimalError::Overflow));
        assert_eq!(big.multiplying(&d("2")), Err(DecimalError::Overflow));
    }

    #[test]
    fn equality_and_ordering_ignore_trailing_zeros() {
        assert_eq!(d("1.0"), d("1"));
        assert!(d("-2") < d("-1.5"));
        assert!(d("0.30") > d("0.299"));
        let set: std::collections::HashSet<Decimal> = [d("1.0"), d("1.00"), d("1")].into_iter().collect();
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn rounding_modes() {
        let r = |s: &str, scale, mode| d(s).rounded(scale, mode).to_string();
        assert_eq!(r("2.5", 0, RoundingMode::Plain), "3");
        assert_eq!(r("-2.5", 0, RoundingMode::Plain), "-3");
        assert_eq!(r("2.4", 0, RoundingMode::Plain), "2");
        assert_eq!(r("2.9", 0, RoundingMode::Down), "2");
        assert_eq!(r("-2.9", 0, RoundingMode::Down), "-2");
        assert_eq!(r("2.1", 0, RoundingMode::Up), "3");
        assert_eq!(r("-2.1", 0, RoundingMode::Up), "-3");
        assert_eq!(r("2.5", 0, RoundingMode::Bankers), "2");
        assert_eq!(r("3.5", 0, RoundingMode::Bankers), "4");
        assert_eq!(r("2.51", 0, RoundingMode::Bankers), "3");
        assert_eq!(r("1.005", 2, RoundingMode::Plain), "1.01");
        assert_eq!(r("1.239", 2, RoundingMode::Down), "1.23");
        assert_eq!(r("1.5", 3, RoundingMode::Plain), "1.5");
        assert_eq!(r("1250", -2, RoundingMode::Plain), "1300");
        assert_eq!(r("0.0001", 0, RoundingMode::Up), "1");
    }
}
