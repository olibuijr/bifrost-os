//! Calculator arithmetic and entry state, independent of the touch UI.

pub use super::format::format_number;
use super::format::MAX_DIGITS;
const MAX_HISTORY: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operator {
    Add,
    Subtract,
    Multiply,
    Divide,
}

impl Operator {
    pub fn label(self) -> &'static str {
        match self {
            Self::Add => "+",
            Self::Subtract => "−",
            Self::Multiply => "×",
            Self::Divide => "÷",
        }
    }

    fn apply(self, left: f64, right: f64) -> Option<f64> {
        if self == Self::Divide && right == 0.0 {
            return None;
        }
        let value = match self {
            Self::Add => left + right,
            Self::Subtract => left - right,
            Self::Multiply => left * right,
            Self::Divide => left / right,
        };
        value.is_finite().then_some(value)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Digit(u8),
    Decimal,
    Operator(Operator),
    Equals,
    Clear,
    Sign,
    Percent,
    Backspace,
}

#[derive(Clone, Debug)]
pub struct Calculation {
    pub expression: String,
    pub result: String,
    pub value: f64,
}

#[derive(Debug)]
pub struct Engine {
    entry: String,
    value: f64,
    replace: bool,
    /// A new right operand has been supplied after the pending operator.
    operand_ready: bool,
    pending: Option<(Operator, f64)>,
    repeat: Option<(Operator, f64)>,
    error: bool,
    pub expression: String,
    pub history: Vec<Calculation>,
    /// Counts finished calculations, so the UI can mirror new ones.
    pub serial: u64,
}

impl Default for Engine {
    fn default() -> Self {
        Self {
            entry: "0".into(),
            value: 0.0,
            replace: true,
            operand_ready: false,
            pending: None,
            repeat: None,
            error: false,
            expression: String::new(),
            history: vec![],
            serial: 0,
        }
    }
}

impl Engine {
    pub fn display(&self) -> &str {
        if self.error {
            "Error"
        } else {
            &self.entry
        }
    }

    pub fn clear_label(&self) -> &'static str {
        if self.error || self.entry == "0" || self.entry == "-0" {
            "AC"
        } else {
            "C"
        }
    }

    pub fn selected_operator(&self) -> Option<Operator> {
        self.pending
            .filter(|_| !self.operand_ready)
            .map(|(op, _)| op)
    }

    fn reset(&mut self) {
        let history = std::mem::take(&mut self.history);
        let serial = self.serial;
        *self = Self::default();
        self.history = history;
        self.serial = serial;
    }

    fn set_result(&mut self, value: f64) {
        self.value = value;
        self.entry = format_number(value);
        self.replace = true;
        self.operand_ready = true;
    }

    fn fail(&mut self) {
        self.error = true;
        self.pending = None;
        self.repeat = None;
        self.replace = true;
        self.expression = "Cannot divide by zero or exceed the number range".into();
    }

    fn prepare_entry(&mut self) {
        if self.error {
            self.reset();
        }
        if self.replace {
            self.entry = "0".into();
            self.replace = false;
        }
        if self.pending.is_none() {
            self.expression.clear();
        }
        self.repeat = None;
        self.operand_ready = true;
    }

    fn read_entry(&mut self) {
        self.value = self.entry.parse().unwrap_or(0.0);
    }

    pub fn press(&mut self, key: Key) {
        if self.error
            && !matches!(
                key,
                Key::Digit(_) | Key::Decimal | Key::Clear | Key::Backspace
            )
        {
            return;
        }
        match key {
            Key::Digit(digit) if digit < 10 => {
                self.prepare_entry();
                let n = self.entry.bytes().filter(u8::is_ascii_digit).count();
                if self.entry == "0" || self.entry == "-0" {
                    let sign = if self.entry.starts_with('-') { "-" } else { "" };
                    self.entry = format!("{sign}{digit}");
                } else if n < MAX_DIGITS {
                    self.entry.push((b'0' + digit) as char);
                }
                self.read_entry();
            }
            Key::Digit(_) => {}
            Key::Decimal => {
                self.prepare_entry();
                if !self.entry.contains('.') {
                    self.entry.push('.');
                }
            }
            Key::Clear => {
                if self.clear_label() == "AC" {
                    self.reset();
                } else {
                    self.entry = "0".into();
                    self.value = 0.0;
                    self.replace = true;
                    self.operand_ready = false;
                    self.repeat = None;
                    if self.pending.is_none() {
                        self.expression.clear();
                    }
                }
            }
            Key::Backspace => {
                if self.error {
                    self.reset();
                } else if !self.replace && !self.entry.contains('e') {
                    self.entry.pop();
                    if self.entry.is_empty() || self.entry == "-" {
                        self.entry = "0".into();
                    }
                    self.read_entry();
                }
            }
            Key::Sign => {
                if self.pending.is_some() && !self.operand_ready {
                    self.entry = "-0".into();
                    self.value = -0.0;
                    self.replace = false;
                    self.operand_ready = true;
                } else {
                    if self.entry.starts_with('-') {
                        self.entry.remove(0);
                    } else {
                        self.entry.insert(0, '-');
                    }
                    self.value = -self.value;
                    if self.value == 0.0 {
                        self.replace = false;
                    }
                }
            }
            Key::Percent => {
                let mut value = self.value / 100.0;
                if let Some((Operator::Add | Operator::Subtract, left)) = self.pending {
                    value *= left;
                }
                if value.is_finite() {
                    self.set_result(value);
                    self.repeat = None;
                } else {
                    self.fail();
                }
            }
            Key::Operator(operator) => {
                if let Some((op, left)) = self.pending {
                    if self.operand_ready {
                        let Some(value) = op.apply(left, self.value) else {
                            self.fail();
                            return;
                        };
                        self.set_result(value);
                    }
                }
                self.pending = Some((operator, self.value));
                self.repeat = None;
                self.replace = true;
                self.operand_ready = false;
                self.expression = format!("{} {}", format_number(self.value), operator.label());
            }
            Key::Equals => {
                let operation = self
                    .pending
                    .map(|(op, left)| {
                        (op, left, if self.operand_ready { self.value } else { left })
                    })
                    .or_else(|| self.repeat.map(|(op, right)| (op, self.value, right)));
                if let Some((op, left, right)) = operation {
                    let Some(value) = op.apply(left, right) else {
                        self.fail();
                        return;
                    };
                    self.expression = format!(
                        "{} {} {} =",
                        format_number(left),
                        op.label(),
                        format_number(right)
                    );
                    self.set_result(value);
                    self.pending = None;
                    self.repeat = Some((op, right));
                    self.history.insert(
                        0,
                        Calculation {
                            expression: self.expression.clone(),
                            result: self.entry.clone(),
                            value,
                        },
                    );
                    self.history.truncate(MAX_HISTORY);
                    self.serial += 1;
                } else {
                    self.replace = true;
                }
            }
        }
    }

    pub fn recall(&mut self, index: usize) {
        if let Some(value) = self.history.get(index).map(|c| c.value) {
            self.recall_value(value);
        }
    }

    /// Starts afresh from `value` (a recalled history result).
    pub fn recall_value(&mut self, value: f64) {
        self.reset();
        self.set_result(value);
    }

    /// Types `value` as the current operand, keeping a pending operator.
    pub fn paste_value(&mut self, value: f64) {
        if !value.is_finite() {
            return;
        }
        self.prepare_entry();
        self.entry = format_number(value);
        self.value = value;
        // A scientific-notation entry cannot be extended digit by digit.
        self.replace = self.entry.contains('e');
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enter(e: &mut Engine, input: &str) {
        for ch in input.chars() {
            e.press(match ch {
                '0'..='9' => Key::Digit(ch as u8 - b'0'),
                '.' => Key::Decimal,
                '+' => Key::Operator(Operator::Add),
                '-' => Key::Operator(Operator::Subtract),
                '*' => Key::Operator(Operator::Multiply),
                '/' => Key::Operator(Operator::Divide),
                '=' => Key::Equals,
                '%' => Key::Percent,
                _ => panic!("unknown input"),
            });
        }
    }

    #[test]
    fn arithmetic_chains_and_operator_replacement() {
        for (keys, result) in [
            ("12+3=", "15"),
            ("9-15=", "-6"),
            ("8*7=", "56"),
            ("9/4=", "2.25"),
            ("2+3*4=", "20"),
            ("9+-3=", "6"),
            ("5+=", "10"),
        ] {
            let mut e = Engine::default();
            enter(&mut e, keys);
            assert_eq!(e.display(), result, "{keys}");
        }
    }

    #[test]
    fn repeated_equals_and_fresh_digit_after_result() {
        let mut e = Engine::default();
        enter(&mut e, "5+3===");
        assert_eq!(e.display(), "14");
        assert_eq!(e.history.len(), 3);
        enter(&mut e, "7=");
        assert_eq!(e.display(), "7");
        enter(&mut e, "+2==");
        assert_eq!(e.display(), "11");
    }

    #[test]
    fn contextual_percent_adds_subtracts_or_multiplies() {
        for (keys, result) in [
            ("200+10%=", "220"),
            ("200-10%=", "180"),
            ("200*10%=", "20"),
            ("200/10%=", "2000"),
            ("50%", "0.5"),
            ("200+10%==", "240"),
        ] {
            let mut e = Engine::default();
            enter(&mut e, keys);
            assert_eq!(e.display(), result, "{keys}");
        }
    }

    #[test]
    fn preserves_decimal_entry_and_ignores_duplicate_decimal_and_excess_digits() {
        let mut e = Engine::default();
        enter(&mut e, "0001.20.0");
        assert_eq!(e.display(), "1.200");
        e.press(Key::Backspace);
        assert_eq!(e.display(), "1.20");
        enter(&mut e, "+0.1=");
        assert_eq!(e.display(), "1.3");
        enter(&mut e, "1234567890123456789");
        assert_eq!(e.display(), "123456789");
    }

    #[test]
    fn sign_can_start_negative_rhs_and_negate_results() {
        let mut e = Engine::default();
        e.press(Key::Sign);
        enter(&mut e, ".5");
        assert_eq!(e.display(), "-0.5");
        e.press(Key::Clear);
        e.press(Key::Clear);
        e.press(Key::Sign);
        enter(&mut e, "7");
        assert_eq!(e.display(), "-7");
        e.press(Key::Clear);
        e.press(Key::Clear);
        enter(&mut e, "5+");
        e.press(Key::Sign);
        enter(&mut e, "3=");
        assert_eq!(e.display(), "2");
        e.press(Key::Sign);
        assert_eq!(e.display(), "-2");
        e.press(Key::Sign);
        assert_eq!(e.display(), "2");
    }

    #[test]
    fn percent_can_be_repeated_and_replaced_with_fresh_entry() {
        let mut e = Engine::default();
        enter(&mut e, "50%%");
        assert_eq!(e.display(), "0.005");
        enter(&mut e, "8");
        assert_eq!(e.display(), "8");
    }

    #[test]
    fn clear_entry_keeps_pending_operation_and_double_clear_resets() {
        let mut e = Engine::default();
        enter(&mut e, "20+85");
        e.press(Key::Clear);
        assert_eq!(e.clear_label(), "AC");
        enter(&mut e, "3=");
        assert_eq!(e.display(), "23");
        enter(&mut e, "+8");
        e.press(Key::Clear);
        e.press(Key::Clear);
        enter(&mut e, "4=");
        assert_eq!(e.display(), "4");
    }

    #[test]
    fn error_recovers_by_digit_decimal_clear_or_backspace() {
        for recovery in [Key::Digit(7), Key::Decimal, Key::Clear, Key::Backspace] {
            let mut e = Engine::default();
            enter(&mut e, "1/0=");
            assert_eq!(e.display(), "Error");
            e.press(Key::Equals);
            assert_eq!(e.display(), "Error");
            e.press(recovery);
            assert_ne!(e.display(), "Error");
            e.press(Key::Operator(Operator::Add));
            enter(&mut e, "2=");
            assert_eq!(
                e.display(),
                if recovery == Key::Digit(7) { "9" } else { "2" }
            );
        }
    }

    #[test]
    fn rounds_binary_float_noise_and_keeps_small_and_large_results() {
        let mut e = Engine::default();
        enter(&mut e, "0.1+0.2=");
        assert_eq!(e.display(), "0.3");
        assert_eq!(format_number(1e-15), "1e-15");
        assert_eq!(format_number(1e15), "1e15");
        assert_eq!(format_number(-0.0), "0");
        assert_eq!(format_number(1.0 / 3.0), "0.333333333");
        e.value = f64::MAX;
        e.press(Key::Operator(Operator::Multiply));
        enter(&mut e, "2=");
        assert_eq!(e.display(), "Error");
    }

    #[test]
    fn paste_value_replaces_the_operand_and_keeps_the_pending_operator() {
        let mut e = Engine::default();
        enter(&mut e, "5+");
        e.paste_value(1234.5);
        assert_eq!(e.display(), "1234.5");
        enter(&mut e, "=");
        assert_eq!(e.display(), "1239.5");
        e.paste_value(f64::NAN);
        assert_eq!(e.display(), "1239.5");
        e.paste_value(-2.0);
        assert_eq!(e.display(), "-2");
        enter(&mut e, "*3=");
        assert_eq!(e.display(), "-6");
        assert_eq!(e.serial, 2);
    }

    #[test]
    fn history_is_bounded_and_recalled_results_start_fresh_operations() {
        let mut e = Engine::default();
        enter(&mut e, "1+1==========");
        assert_eq!(e.history.len(), MAX_HISTORY);
        e.recall(2);
        assert_eq!(e.display(), "9");
        enter(&mut e, "*2=");
        assert_eq!(e.display(), "18");
        e.recall(999);
        assert_eq!(e.display(), "18");
    }
}
