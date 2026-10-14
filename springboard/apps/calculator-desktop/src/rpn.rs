//! Reverse Polish notation: an operand stack where each operator pops its operands and pushes the result. Entry,
//! enter, drop, swap and roll as on the stock Calculator's RPN mode. Independent of the touch UI.
#![allow(dead_code)] // the RPN toggle and keypad wiring are the next step; tests drive it

use super::engine::Operator;

/// Deepest stack kept; the oldest value falls off the bottom beyond this.
pub const DEPTH: usize = 64;

#[derive(Clone, Debug, Default)]
pub struct Rpn {
    stack: Vec<f64>,
    /// Digits being typed; `None` when the top of the stack is a finished value.
    entry: Option<String>,
}

impl Rpn {
    pub fn new() -> Self {
        Self::default()
    }

    /// The stack bottom to top, with a value being typed counted as the top.
    pub fn stack(&self) -> Vec<f64> {
        let mut out = self.stack.clone();
        if let Some(v) = self.entry.as_deref().and_then(parse) {
            out.push(v);
        }
        out
    }

    /// What the display shows: the entry being typed, else the top of the stack, else 0.
    pub fn display(&self) -> String {
        match (&self.entry, self.stack.last()) {
            (Some(text), _) => text.clone(),
            (None, Some(v)) => crate::format::format_number(*v),
            (None, None) => "0".to_string(),
        }
    }

    pub fn digit(&mut self, digit: u8) {
        let text = self.entry.get_or_insert_with(String::new);
        if text == "0" {
            text.clear();
        }
        if text.len() < crate::format::MAX_DIGITS + 1 {
            text.push((b'0' + digit.min(9)) as char);
        }
    }

    pub fn decimal(&mut self) {
        let text = self.entry.get_or_insert_with(String::new);
        if !text.contains('.') {
            if text.is_empty() {
                text.push('0');
            }
            text.push('.');
        }
    }

    pub fn backspace(&mut self) {
        if let Some(text) = self.entry.as_mut() {
            text.pop();
            if text.is_empty() {
                self.entry = None;
            }
        }
    }

    /// Flips the sign of the entry or of the top value.
    pub fn sign(&mut self) {
        if let Some(text) = self.entry.as_mut() {
            if let Some(rest) = text.strip_prefix('-') {
                *text = rest.to_string();
            } else if !text.is_empty() {
                text.insert(0, '-');
            }
        } else if let Some(top) = self.stack.last_mut() {
            *top = -*top;
        }
    }

    fn commit(&mut self) {
        if let Some(v) = self.entry.take().as_deref().and_then(parse) {
            self.push(v);
        }
    }

    fn push(&mut self, value: f64) {
        if self.stack.len() == DEPTH {
            self.stack.remove(0);
        }
        self.stack.push(value);
    }

    /// Enter: commits the entry; with nothing typed it duplicates the top value.
    pub fn enter(&mut self) {
        if self.entry.is_some() {
            self.commit();
        } else if let Some(top) = self.stack.last().copied() {
            self.push(top);
        }
    }

    /// Applies a binary operator to the top two values. False when fewer than two values exist or the result is
    /// not a finite number (division by zero); the stack is left as it was in that case.
    pub fn operate(&mut self, op: Operator) -> bool {
        self.commit();
        if self.stack.len() < 2 {
            return false;
        }
        let right = self.stack[self.stack.len() - 1];
        let left = self.stack[self.stack.len() - 2];
        let value = match op {
            Operator::Add => left + right,
            Operator::Subtract => left - right,
            Operator::Multiply => left * right,
            Operator::Divide => left / right,
        };
        if !value.is_finite() {
            return false;
        }
        self.stack.truncate(self.stack.len() - 2);
        self.stack.push(value);
        true
    }

    /// Applies a one-operand function to the top value.
    pub fn unary(&mut self, f: impl Fn(f64) -> f64) -> bool {
        self.commit();
        let Some(top) = self.stack.last_mut() else { return false };
        let value = f(*top);
        if !value.is_finite() {
            return false;
        }
        *top = value;
        true
    }

    pub fn drop_top(&mut self) {
        if self.entry.take().is_none() {
            self.stack.pop();
        }
    }

    pub fn swap(&mut self) {
        self.commit();
        let n = self.stack.len();
        if n >= 2 {
            self.stack.swap(n - 1, n - 2);
        }
    }

    /// Rolls the stack: the top goes to the bottom (`down`), or the bottom comes to the top.
    pub fn roll(&mut self, down: bool) {
        self.commit();
        if self.stack.len() < 2 {
            return;
        }
        if down {
            let top = self.stack.pop().unwrap_or(0.0);
            self.stack.insert(0, top);
        } else {
            let bottom = self.stack.remove(0);
            self.stack.push(bottom);
        }
    }

    pub fn clear(&mut self) {
        self.stack.clear();
        self.entry = None;
    }
}

fn parse(text: &str) -> Option<f64> {
    text.trim_end_matches('.').parse::<f64>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn type_number(r: &mut Rpn, text: &str) {
        for ch in text.chars() {
            match ch {
                '.' => r.decimal(),
                d => r.digit(d as u8 - b'0'),
            }
        }
    }

    #[test]
    fn two_plus_three_with_enter() {
        let mut r = Rpn::new();
        type_number(&mut r, "2");
        r.enter();
        type_number(&mut r, "3");
        assert!(r.operate(Operator::Add));
        assert_eq!(r.stack(), vec![5.0]);
        assert_eq!(r.display(), "5");
    }

    #[test]
    fn order_matters_for_subtract_and_divide() {
        let mut r = Rpn::new();
        type_number(&mut r, "10");
        r.enter();
        type_number(&mut r, "4");
        r.operate(Operator::Subtract);
        assert_eq!(r.stack(), vec![6.0]);
        type_number(&mut r, "2");
        r.operate(Operator::Divide);
        assert_eq!(r.stack(), vec![3.0]);
    }

    #[test]
    fn divide_by_zero_and_underflow_change_nothing() {
        let mut r = Rpn::new();
        assert!(!r.operate(Operator::Add));
        type_number(&mut r, "5");
        r.enter();
        type_number(&mut r, "0");
        assert!(!r.operate(Operator::Divide));
        assert_eq!(r.stack(), vec![5.0, 0.0]);
    }

    #[test]
    fn enter_on_empty_entry_duplicates_the_top() {
        let mut r = Rpn::new();
        type_number(&mut r, "7");
        r.enter();
        r.enter();
        assert_eq!(r.stack(), vec![7.0, 7.0]);
    }

    #[test]
    fn swap_drop_and_roll() {
        let mut r = Rpn::new();
        for n in ["1", "2", "3"] {
            type_number(&mut r, n);
            r.enter();
        }
        r.swap();
        assert_eq!(r.stack(), vec![1.0, 3.0, 2.0]);
        r.roll(true);
        assert_eq!(r.stack(), vec![2.0, 1.0, 3.0]);
        r.roll(false);
        assert_eq!(r.stack(), vec![1.0, 3.0, 2.0]);
        r.drop_top();
        assert_eq!(r.stack(), vec![1.0, 3.0]);
    }

    #[test]
    fn sign_decimal_and_backspace_edit_the_entry() {
        let mut r = Rpn::new();
        type_number(&mut r, "1.5");
        r.sign();
        assert_eq!(r.display(), "-1.5");
        r.backspace();
        assert_eq!(r.display(), "-1.");
        r.enter();
        assert_eq!(r.stack(), vec![-1.0]);
        r.sign();
        assert_eq!(r.stack(), vec![1.0]);
    }

    #[test]
    fn unary_functions_apply_to_the_top() {
        let mut r = Rpn::new();
        type_number(&mut r, "9");
        assert!(r.unary(f64::sqrt));
        assert_eq!(r.stack(), vec![3.0]);
        assert!(!r.unary(|v| v / 0.0));
        assert_eq!(r.stack(), vec![3.0]);
    }

    #[test]
    fn depth_is_capped() {
        let mut r = Rpn::new();
        for i in 0..(DEPTH + 5) {
            type_number(&mut r, &(i % 9).to_string());
            r.enter();
        }
        assert_eq!(r.stack().len(), DEPTH);
    }
}
