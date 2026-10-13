//! RPN mode: the basic keypad drives a stack. A number is typed, Enter pushes it, and an operator takes the top two
//! values and leaves its result on the stack. Swap, Drop and the two rolls reorder the stack. Pure logic, no UI.

use super::format::format_number;

/// The deepest stack; pushing past it drops the oldest value.
pub const MAX_DEPTH: usize = 64;

/// Digits an entry holds.
pub const MAX_DIGITS: usize = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RKey {
    Digit(u8),
    Decimal,
    Sign,
    Backspace,
    /// C clears the entry being typed; with none, AC clears the stack.
    Clear,
    Enter,
    Add,
    Sub,
    Mul,
    Div,
    Percent,
    Swap,
    Drop,
    /// Moves the top value to the bottom (roll down), or the bottom value to the top (roll up).
    RollDown,
    RollUp,
}

impl RKey {
    fn symbol(self) -> &'static str {
        match self {
            RKey::Add => "+",
            RKey::Sub => "-",
            RKey::Mul => "×",
            _ => "÷",
        }
    }
}

/// A finished calculation for the history: the expression, the result text and its value.
pub type Finished = (String, String, f64);

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Rpn {
    /// Committed values, the top (X) last.
    stack: Vec<f64>,
    /// The number being typed; it is X until Enter or an operator pushes it.
    entry: Option<String>,
    error: bool,
}

fn parse_entry(entry: &str) -> f64 {
    entry.parse().unwrap_or(0.0)
}

impl Rpn {
    /// The committed stack, X last (an entry still being typed is not in it).
    pub fn stack(&self) -> &[f64] {
        &self.stack
    }

    /// The values below X, X's neighbour last: everything committed while an entry is typed, else all but the top.
    pub fn below(&self) -> &[f64] {
        if self.entry.is_some() || self.stack.is_empty() {
            &self.stack
        } else {
            &self.stack[..self.stack.len() - 1]
        }
    }

    /// X as a number: the entry being typed, else the top of the stack.
    pub fn x(&self) -> Option<f64> {
        if self.error {
            return None;
        }
        match &self.entry {
            Some(entry) => Some(parse_entry(entry)),
            None => self.stack.last().copied(),
        }
    }

    /// What the display shows (ASCII, `.` decimal): the entry as typed, else X, else 0.
    pub fn display(&self) -> String {
        if self.error {
            return "Error".into();
        }
        match &self.entry {
            Some(entry) if entry.is_empty() => "0".into(),
            Some(entry) if entry == "-" => "-0".into(),
            Some(entry) => entry.clone(),
            None => self.stack.last().map_or_else(|| "0".to_string(), |v| format_number(*v)),
        }
    }

    /// X when it is a result on the stack (not an entry being typed, not an error), for display styling.
    pub fn result(&self) -> Option<f64> {
        if self.error || self.entry.is_some() {
            None
        } else {
            self.stack.last().copied()
        }
    }

    pub fn clear_label(&self) -> &'static str {
        if self.entry.is_some() {
            "C"
        } else {
            "AC"
        }
    }

    fn push(&mut self, value: f64) {
        self.stack.push(value);
        if self.stack.len() > MAX_DEPTH {
            self.stack.remove(0);
        }
    }

    /// Pushes the entry being typed.
    fn commit(&mut self) {
        if let Some(entry) = self.entry.take() {
            self.push(parse_entry(&entry));
        }
    }

    /// Replaces the entry with `value` (Paste, a recalled result).
    pub fn set_value(&mut self, value: f64) {
        if value.is_finite() {
            self.error = false;
            self.entry = Some(format_number(value));
        }
    }

    fn operate(&mut self, key: RKey) -> Option<Finished> {
        self.commit();
        if self.stack.len() < 2 {
            return None;
        }
        let b = self.stack.pop()?;
        let a = self.stack.pop()?;
        let result = match key {
            RKey::Add => a + b,
            RKey::Sub => a - b,
            RKey::Mul => a * b,
            _ if b == 0.0 => f64::NAN,
            _ => a / b,
        };
        if !result.is_finite() {
            self.stack.push(a);
            self.stack.push(b);
            self.error = true;
            return None;
        }
        self.push(result);
        Some((format!("{} {} {}", format_number(a), key.symbol(), format_number(b)), format_number(result), result))
    }

    /// Applies `key`. A finished operation is returned for the history.
    pub fn press(&mut self, key: RKey) -> Option<Finished> {
        self.error = false;
        match key {
            RKey::Digit(d) if d < 10 => {
                let entry = self.entry.get_or_insert_with(String::new);
                if entry.contains('e') || entry.bytes().filter(u8::is_ascii_digit).count() >= MAX_DIGITS {
                    return None;
                }
                if entry.trim_start_matches('-') == "0" {
                    entry.pop();
                }
                entry.push((b'0' + d) as char);
            }
            RKey::Digit(_) => {}
            RKey::Decimal => {
                let entry = self.entry.get_or_insert_with(String::new);
                if entry.contains('.') || entry.contains('e') {
                    return None;
                }
                if entry.is_empty() || entry == "-" {
                    entry.push('0');
                }
                entry.push('.');
            }
            RKey::Sign => match &mut self.entry {
                Some(entry) => {
                    if entry.starts_with('-') {
                        entry.remove(0);
                    } else {
                        entry.insert(0, '-');
                    }
                }
                None => {
                    if let Some(top) = self.stack.last_mut() {
                        *top = -*top;
                    }
                }
            },
            RKey::Backspace => {
                if let Some(entry) = &mut self.entry {
                    entry.pop();
                    if entry.is_empty() {
                        self.entry = None;
                    }
                }
            }
            RKey::Clear => {
                if self.entry.is_some() {
                    self.entry = None;
                } else {
                    self.stack.clear();
                }
            }
            RKey::Enter => {
                if self.entry.is_some() {
                    self.commit();
                } else if let Some(top) = self.stack.last().copied() {
                    self.push(top);
                }
            }
            RKey::Add | RKey::Sub | RKey::Mul | RKey::Div => return self.operate(key),
            RKey::Percent => {
                self.commit();
                if let Some(top) = self.stack.last_mut() {
                    *top /= 100.0;
                }
            }
            RKey::Swap => {
                self.commit();
                let n = self.stack.len();
                if n >= 2 {
                    self.stack.swap(n - 1, n - 2);
                }
            }
            RKey::Drop => {
                if self.entry.is_some() {
                    self.entry = None;
                } else {
                    self.stack.pop();
                }
            }
            RKey::RollDown => {
                self.commit();
                self.stack.rotate_right(1);
            }
            RKey::RollUp => {
                self.commit();
                self.stack.rotate_left(1);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Types `src`: digits, `.`, `_` (Enter), `+ - * /`.
    fn run(r: &mut Rpn, src: &str) -> Option<Finished> {
        let mut last = None;
        for c in src.chars() {
            last = r.press(match c {
                '0'..='9' => RKey::Digit(c as u8 - b'0'),
                '.' => RKey::Decimal,
                '_' => RKey::Enter,
                '+' => RKey::Add,
                '-' => RKey::Sub,
                '*' => RKey::Mul,
                '/' => RKey::Div,
                _ => panic!("unknown key {c}"),
            });
        }
        last
    }

    #[test]
    fn enter_pushes_and_an_operator_takes_the_top_two() {
        let mut r = Rpn::default();
        let done = run(&mut r, "3_4+");
        assert_eq!(done, Some(("3 + 4".to_string(), "7".to_string(), 7.0)));
        assert_eq!(r.display(), "7");
        assert_eq!(r.stack(), &[7.0]);
        run(&mut r, "5*");
        assert_eq!(r.display(), "35", "the result stays as X for the next operation");
        let mut r = Rpn::default();
        run(&mut r, "10_4-");
        assert_eq!(r.display(), "6");
        run(&mut r, "2/");
        assert_eq!(r.display(), "3");
    }

    #[test]
    fn the_stack_shows_below_x() {
        let mut r = Rpn::default();
        run(&mut r, "1_2_3");
        assert_eq!(r.display(), "3");
        assert_eq!(r.below(), &[1.0, 2.0], "3 is still the entry being typed");
        run(&mut r, "_");
        assert_eq!(r.below(), &[1.0, 2.0]);
        assert_eq!(r.x(), Some(3.0));
        assert_eq!(r.stack(), &[1.0, 2.0, 3.0]);
    }

    #[test]
    fn enter_with_no_entry_duplicates_x() {
        let mut r = Rpn::default();
        run(&mut r, "6_");
        r.press(RKey::Enter);
        assert_eq!(r.stack(), &[6.0, 6.0]);
        run(&mut r, "*");
        assert_eq!(r.display(), "36");
        let mut empty = Rpn::default();
        empty.press(RKey::Enter);
        assert!(empty.stack().is_empty());
        assert_eq!(empty.display(), "0");
    }

    #[test]
    fn swap_drop_and_rolls_reorder_the_stack() {
        let mut r = Rpn::default();
        run(&mut r, "1_2_3_");
        r.press(RKey::Swap);
        assert_eq!(r.stack(), &[1.0, 3.0, 2.0]);
        r.press(RKey::RollDown);
        assert_eq!(r.stack(), &[2.0, 1.0, 3.0], "the top goes to the bottom");
        r.press(RKey::RollUp);
        assert_eq!(r.stack(), &[1.0, 3.0, 2.0], "the bottom comes back to the top");
        r.press(RKey::Drop);
        assert_eq!(r.stack(), &[1.0, 3.0]);
        run(&mut r, "9");
        r.press(RKey::Swap);
        assert_eq!(r.stack(), &[1.0, 9.0, 3.0], "an entry is pushed before the swap");
        run(&mut r, "7");
        r.press(RKey::Drop);
        assert_eq!(r.stack(), &[1.0, 9.0, 3.0], "Drop discards an entry being typed first");
    }

    #[test]
    fn editing_the_entry() {
        let mut r = Rpn::default();
        run(&mut r, "0012.50");
        assert_eq!(r.display(), "12.50");
        r.press(RKey::Backspace);
        r.press(RKey::Backspace);
        assert_eq!(r.display(), "12.");
        r.press(RKey::Sign);
        assert_eq!(r.display(), "-12.");
        r.press(RKey::Sign);
        assert_eq!(r.display(), "12.");
        r.press(RKey::Decimal);
        assert_eq!(r.display(), "12.", "one decimal point only");
        r.press(RKey::Clear);
        assert_eq!(r.display(), "0");
        assert_eq!(r.clear_label(), "AC");
        run(&mut r, "5_");
        r.press(RKey::Sign);
        assert_eq!(r.display(), "-5", "Change Sign works on X once it is pushed");
        run(&mut r, ".");
        assert_eq!(r.display(), "0.");
        assert_eq!(r.clear_label(), "C");
        r.press(RKey::Clear);
        r.press(RKey::Clear);
        assert!(r.stack().is_empty());
    }

    #[test]
    fn division_by_zero_is_an_error_that_keeps_the_operands() {
        let mut r = Rpn::default();
        assert_eq!(run(&mut r, "5_0/"), None);
        assert_eq!(r.display(), "Error");
        assert_eq!(r.x(), None);
        assert_eq!(r.stack(), &[5.0, 0.0]);
        r.press(RKey::Drop);
        assert_eq!(r.display(), "5", "any key clears the error");
    }

    #[test]
    fn an_operator_with_one_value_does_nothing() {
        let mut r = Rpn::default();
        assert_eq!(run(&mut r, "5+"), None);
        assert_eq!(r.stack(), &[5.0]);
        assert_eq!(r.display(), "5");
    }

    #[test]
    fn percent_and_pasted_values() {
        let mut r = Rpn::default();
        run(&mut r, "50");
        r.press(RKey::Percent);
        assert_eq!(r.display(), "0.5");
        r.set_value(1234.5);
        assert_eq!(r.display(), "1234.5");
        run(&mut r, "_2*");
        assert_eq!(r.display(), "2469");
        r.set_value(f64::NAN);
        assert_eq!(r.display(), "2469", "a value that is not a number is ignored");
    }

    #[test]
    fn the_stack_is_bounded() {
        let mut r = Rpn::default();
        for n in 0..(MAX_DEPTH as u32 + 5) {
            r.set_value(f64::from(n));
            r.press(RKey::Enter);
        }
        assert_eq!(r.stack().len(), MAX_DEPTH);
        assert_eq!(r.stack()[0], 5.0, "the oldest values fall off the bottom");
    }
}
