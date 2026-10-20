//! Scientific keypad state: an editable expression evaluated on `=`,
//! with memory keys, the 2nd-function toggle and the Rad/Deg switch.

use super::eval::{evaluate, is_function, Angle};
use super::format::format_number;
use super::history::Entry;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SciKey {
    Digit(u8),
    Decimal,
    /// One of `+ - * /`.
    Op(char),
    Open,
    Close,
    /// Inserts `name(`.
    Func(&'static str),
    /// Inserts a constant or literal text such as `π` or `^2`.
    Text(&'static str),
    /// A postfix such as `!` or `%` that continues from a shown result.
    Postfix(&'static str),
    Sign,
    Equals,
    Clear,
    Backspace,
    MemClear,
    MemAdd,
    MemSub,
    MemRecall,
    Answer,
    /// `Rand`: a random number in [0, 1) typed as a literal.
    Rand,
    /// `EE`: scientific-notation exponent after a number.
    Exp,
}

#[derive(Debug, Default)]
pub struct Sci {
    pub expr: String,
    pub result: Option<f64>,
    pub error: bool,
    pub memory: f64,
    pub second: bool,
    pub angle: Angle,
    /// The expression that produced `result`, shown above it.
    pub last_expr: String,
    pub last: f64,
}

/// xorshift64 seeded from the clock; good enough for the Rand key.
fn next_random() -> f64 {
    use std::cell::Cell;
    thread_local!(static STATE: Cell<u64> = const { Cell::new(0) });
    STATE.with(|st| {
        let mut x = st.get();
        if x == 0 {
            x = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0x9E37_79B9_7F4A_7C15, |d| d.as_nanos() as u64)
                | 1;
        }
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        st.set(x);
        (x >> 11) as f64 / (1u64 << 53) as f64
    })
}

/// The infix keys of the second layer as they are typed into the expression.
const INFIX_WORDS: [&str; 2] = [" logy ", " rootx "];

fn is_op(c: char) -> bool {
    matches!(c, '+' | '-' | '*' | '/' | '×' | '÷' | '−')
}

/// Whether `expr` is `-( ... )` with the closing parenthesis matching the first.
fn negated_group(expr: &str) -> bool {
    let Some(inner) = expr.strip_prefix("-(") else { return false };
    let mut depth = 1;
    for (i, c) in inner.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return i + 1 == inner.len();
                }
            }
            _ => {}
        }
    }
    false
}

impl Sci {
    pub fn display(&self) -> String {
        if self.error {
            "Error".into()
        } else if let Some(v) = self.result {
            format_number(v)
        } else if self.expr.is_empty() {
            "0".into()
        } else {
            self.expr.clone()
        }
    }

    /// The small line above the display: the finished expression.
    pub fn expression_line(&self) -> &str {
        if self.result.is_some() {
            &self.last_expr
        } else {
            ""
        }
    }

    pub fn clear_label(&self) -> &'static str {
        if self.error || (self.expr.is_empty() && self.result.is_none()) {
            "AC"
        } else {
            "C"
        }
    }

    fn closed(&self) -> String {
        let open = self.expr.matches('(').count();
        let close = self.expr.matches(')').count();
        format!("{}{}", self.expr, ")".repeat(open.saturating_sub(close)))
    }

    /// The number the display stands for, if the expression is complete.
    pub fn current_value(&self) -> Option<f64> {
        if self.error {
            return None;
        }
        self.result.or_else(|| evaluate(&self.closed(), self.angle).ok())
    }

    /// Starts a fresh expression after a result, or continues from it when
    /// `keep` (an operator or postfix was pressed).
    fn begin(&mut self, keep: bool) {
        if self.error {
            *self = Self { memory: self.memory, second: self.second, angle: self.angle, last: self.last, ..Self::default() };
        }
        if let Some(v) = self.result.take() {
            self.expr = if keep { format_number(v) } else { String::new() };
        }
    }

    fn number_run(&self) -> &str {
        let start = self
            .expr
            .rfind(|c: char| !(c.is_ascii_digit() || c == '.'))
            .map(|i| i + self.expr[i..].chars().next().map_or(1, char::len_utf8))
            .unwrap_or(0);
        &self.expr[start..]
    }

    fn literal(value: f64) -> String {
        let text = format!("{value}");
        if value < 0.0 {
            format!("({text})")
        } else {
            text
        }
    }

    /// Applies `key`; a finished calculation is returned for the history.
    pub fn press(&mut self, key: SciKey) -> Option<Entry> {
        if self.error && !matches!(key, SciKey::Digit(_) | SciKey::Decimal | SciKey::Clear | SciKey::Backspace | SciKey::MemClear | SciKey::MemRecall) {
            return None;
        }
        match key {
            SciKey::Digit(d) if d < 10 => {
                self.begin(false);
                self.expr.push((b'0' + d) as char);
            }
            SciKey::Digit(_) => {}
            SciKey::Decimal => {
                self.begin(false);
                let run = self.number_run();
                if !run.contains('.') {
                    if run.is_empty() {
                        self.expr.push('0');
                    }
                    self.expr.push('.');
                }
            }
            SciKey::Op(op) if is_op(op) => {
                self.begin(true);
                if self.expr.is_empty() {
                    if op == '-' {
                        self.expr.push('-');
                    } else {
                        self.expr.push('0');
                        self.expr.push(op);
                    }
                } else if self.expr.ends_with(is_op) {
                    // `(-` and a lone leading `-` are signs, not operators to replace.
                    if self.expr != "-" {
                        self.expr.pop();
                        self.expr.push(op);
                    }
                } else if !self.expr.ends_with('(') || op == '-' {
                    self.expr.push(op);
                }
            }
            SciKey::Op(_) => {}
            SciKey::Open => {
                self.begin(false);
                self.expr.push('(');
            }
            SciKey::Close => {
                let open = self.expr.matches('(').count();
                let close = self.expr.matches(')').count();
                if self.result.is_none()
                    && open > close
                    && !self.expr.ends_with(|c: char| is_op(c) || c == '(')
                {
                    self.expr.push(')');
                }
            }
            SciKey::Func(name) => {
                if let Some(value) = self.result {
                    // A function key after a result applies to the shown value, like the basic keypad's
                    // unary keys; the new calculation is finished at once and goes to the history.
                    self.result = None;
                    self.expr = format!("{name}({})", Self::literal(value));
                    return self.press(SciKey::Equals);
                }
                self.begin(false);
                self.expr.push_str(name);
                self.expr.push('(');
            }
            SciKey::Text(text) => {
                self.begin(false);
                self.expr.push_str(text);
            }
            SciKey::Rand => {
                self.begin(false);
                if self.expr.is_empty() || self.expr.ends_with(|c: char| is_op(c) || c == '(') {
                    self.expr.push_str(&format!("{:.6}", next_random()));
                }
            }
            SciKey::Exp => {
                self.begin(true);
                let run_ok = self.expr.ends_with(|c: char| c.is_ascii_digit()) && !self.number_run().is_empty();
                let has_e = self.expr.rsplit(is_op).next().is_some_and(|t| t.contains('E'));
                if run_ok && !has_e {
                    self.expr.push('E');
                }
            }
            SciKey::Postfix(text) => {
                self.begin(true);
                if !self.expr.is_empty() && !self.expr.ends_with(|c: char| is_op(c) || c == '(' || c == ' ') {
                    self.expr.push_str(text);
                }
            }
            SciKey::Sign => {
                if let Some(v) = self.result {
                    self.result = Some(-v);
                    self.last_expr = format!("-({})", self.last_expr);
                } else if self.expr.is_empty() {
                    self.expr.push('-');
                } else if negated_group(&self.expr) {
                    self.expr = self.expr[2..self.expr.len() - 1].to_string();
                } else if self.expr != "-" {
                    self.expr = format!("-({})", self.expr);
                } else {
                    self.expr.clear();
                }
            }
            SciKey::Equals => {
                if self.result.is_some() || self.expr.is_empty() {
                    return None;
                }
                let expr = self.closed();
                match evaluate(&expr, self.angle) {
                    Ok(value) => {
                        self.result = Some(value);
                        self.last = value;
                        self.last_expr = expr.clone();
                        self.expr = expr.clone();
                        return Some(Entry { expression: expr, result: format_number(value), value });
                    }
                    Err(_) => {
                        self.error = true;
                        self.result = None;
                    }
                }
            }
            SciKey::Clear => {
                let (memory, second, angle, last) = (self.memory, self.second, self.angle, self.last);
                *self = Self { memory, second, angle, last, ..Self::default() };
            }
            SciKey::Backspace => {
                if self.error {
                    self.press(SciKey::Clear);
                    return None;
                }
                if self.result.take().is_some() {
                    // Keep editing the expression that produced the result.
                    self.expr = self.last_expr.clone();
                    return None;
                }
                // the infix words of the second layer go as one unit, like a function name
                for word in INFIX_WORDS {
                    if self.expr.ends_with(word) {
                        self.expr.truncate(self.expr.len() - word.len());
                        return None;
                    }
                }
                if let Some(popped) = self.expr.pop() {
                    if popped == '(' {
                        let name_start = self
                            .expr
                            .rfind(|c: char| !(c.is_ascii_alphanumeric() || c == '√'))
                            .map(|i| i + self.expr[i..].chars().next().map_or(1, char::len_utf8))
                            .unwrap_or(0);
                        if is_function(&self.expr[name_start..]) {
                            self.expr.truncate(name_start);
                        }
                    }
                }
            }
            SciKey::MemClear => self.memory = 0.0,
            SciKey::MemAdd => {
                if let Some(v) = self.current_value() {
                    self.memory += v;
                }
            }
            SciKey::MemSub => {
                if let Some(v) = self.current_value() {
                    self.memory -= v;
                }
            }
            SciKey::MemRecall => {
                self.begin(false);
                self.expr.push_str(&Self::literal(self.memory));
            }
            SciKey::Answer => {
                self.begin(false);
                self.expr.push_str(&Self::literal(self.last));
            }
        }
        None
    }

    /// Loads a pasted or recalled number as a fresh expression.
    pub fn set_value(&mut self, value: f64) {
        self.begin(false);
        self.expr = Self::literal(value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Presses `src`: digits, `.`, operators, parentheses and `=`.
    fn keys(s: &mut Sci, src: &str) {
        for c in src.chars() {
            s.press(match c {
                '0'..='9' => SciKey::Digit(c as u8 - b'0'),
                '.' => SciKey::Decimal,
                '+' | '-' | '*' | '/' => SciKey::Op(c),
                '(' => SciKey::Open,
                ')' => SciKey::Close,
                '=' => SciKey::Equals,
                _ => panic!("unknown key {c}"),
            });
        }
    }

    fn run(src: &str) -> String {
        let mut s = Sci::default();
        keys(&mut s, src);
        s.display()
    }

    #[test]
    fn evaluates_with_precedence_and_parentheses() {
        assert_eq!(run("2+3*4="), "14");
        assert_eq!(run("(2+3)*4="), "20");
        assert_eq!(run("(2+3"), "(2+3");
        assert_eq!(run("(2+3="), "5");
        assert_eq!(run("1/3="), "0.333333333");
        assert_eq!(run("0.1+0.2="), "0.3");
    }

    #[test]
    fn entry_edge_cases() {
        assert_eq!(run("."), "0.");
        assert_eq!(run("1.5.2"), "1.52");
        assert_eq!(run("1.5+.5="), "2");
        assert_eq!(run("-5+2="), "-3");
        assert_eq!(run("+"), "0+");
        assert_eq!(run("5+*3="), "15");
        assert_eq!(run("5)"), "5");
        assert_eq!(run("(+"), "(");
        assert_eq!(run("(-3)*2="), "-6");
        assert_eq!(run("="), "0");
    }

    #[test]
    fn a_result_continues_with_operators_and_restarts_with_digits() {
        assert_eq!(run("2+3=*4="), "20");
        assert_eq!(run("2+3=7"), "7");
        let mut s = Sci::default();
        keys(&mut s, "2+3=");
        s.press(SciKey::Digit(9));
        s.press(SciKey::Func("sqrt"));
        assert_eq!(s.display(), "sqrt(");
        s.press(SciKey::Digit(9));
        s.press(SciKey::Close);
        assert_eq!(s.press(SciKey::Equals).unwrap().result, "3");
    }

    #[test]
    fn a_function_key_after_a_result_applies_to_the_shown_value() {
        let mut s = Sci::default();
        keys(&mut s, "6*6=");
        assert_eq!(s.display(), "36");
        let entry = s.press(SciKey::Func("√")).unwrap();
        assert_eq!(s.display(), "6");
        assert_eq!(entry.result, "6");
        assert_eq!(entry.expression, "√(36)");
        // and it chains on the new result
        s.press(SciKey::Func("sqrt"));
        assert_eq!(s.display(), "2.44948974");
        let mut c = Sci::default();
        keys(&mut c, "27=");
        c.press(SciKey::Func("cbrt"));
        assert_eq!(c.display(), "3");
        c.press(SciKey::Op('+'));
        keys(&mut c, "1=");
        assert_eq!(c.display(), "4");
        // a domain error reads Error
        let mut n = Sci::default();
        keys(&mut n, "0-4=");
        n.press(SciKey::Func("√"));
        assert_eq!(n.display(), "Error");
        // with no result the key still opens a call
        let mut f = Sci::default();
        f.press(SciKey::Func("sin"));
        assert_eq!(f.display(), "sin(");
    }

    #[test]
    fn the_infix_keys_evaluate_and_backspace_removes_them_whole() {
        let mut s = Sci::default();
        keys(&mut s, "8");
        s.press(SciKey::Postfix(" logy "));
        s.press(SciKey::Postfix(" logy "));
        assert_eq!(s.expr, "8 logy ");
        keys(&mut s, "2=");
        assert_eq!(s.display(), "3");
        s.press(SciKey::Clear);
        keys(&mut s, "27");
        s.press(SciKey::Postfix(" rootx "));
        keys(&mut s, "3");
        s.press(SciKey::Backspace);
        s.press(SciKey::Backspace);
        assert_eq!(s.expr, "27");
        let mut s = Sci::default();
        s.press(SciKey::Postfix(" logy "));
        assert_eq!(s.expr, "", "an infix key needs a left operand");
    }

    #[test]
    fn functions_constants_and_postfix_keys() {
        let mut s = Sci::default();
        s.press(SciKey::Func("sin"));
        keys(&mut s, "0");
        s.press(SciKey::Equals);
        assert_eq!(s.display(), "0");
        s.press(SciKey::Clear);
        s.press(SciKey::Text("π"));
        s.press(SciKey::Op('*'));
        keys(&mut s, "2=");
        assert_eq!(s.display(), "6.28318531");
        s.press(SciKey::Clear);
        keys(&mut s, "5");
        s.press(SciKey::Postfix("!"));
        s.press(SciKey::Equals);
        assert_eq!(s.display(), "120");
        s.press(SciKey::Postfix("%"));
        assert_eq!(s.display(), "120%");
        s.press(SciKey::Equals);
        assert_eq!(s.display(), "1.2");
        s.press(SciKey::Clear);
        keys(&mut s, "9");
        s.press(SciKey::Text("^2"));
        s.press(SciKey::Equals);
        assert_eq!(s.display(), "81");
    }

    #[test]
    fn degrees_change_the_result() {
        let mut s = Sci { angle: Angle::Deg, ..Sci::default() };
        s.press(SciKey::Func("sin"));
        keys(&mut s, "30=");
        assert_eq!(s.display(), "0.5");
        let mut r = Sci::default();
        r.press(SciKey::Func("cos"));
        keys(&mut r, "0=");
        assert_eq!(r.display(), "1");
    }

    #[test]
    fn errors_are_shown_and_recover() {
        assert_eq!(run("1/0="), "Error");
        assert_eq!(run("2+="), "Error");
        let mut s = Sci::default();
        keys(&mut s, "1/0=");
        assert!(s.press(SciKey::Op('+')).is_none());
        assert_eq!(s.display(), "Error");
        assert_eq!(s.clear_label(), "AC");
        s.press(SciKey::Digit(4));
        assert_eq!(s.display(), "4");
        keys(&mut s, "/0=");
        s.press(SciKey::Backspace);
        assert_eq!(s.display(), "0");
    }

    #[test]
    fn equals_returns_a_history_entry_once() {
        let mut s = Sci::default();
        keys(&mut s, "(2+3");
        let e = s.press(SciKey::Equals).unwrap();
        assert_eq!(e.expression, "(2+3)");
        assert_eq!(e.result, "5");
        assert_eq!(e.value, 5.0);
        assert_eq!(s.expression_line(), "(2+3)");
        assert!(s.press(SciKey::Equals).is_none());
    }

    #[test]
    fn backspace_removes_whole_function_names_and_reopens_results() {
        let mut s = Sci::default();
        s.press(SciKey::Func("asin"));
        keys(&mut s, "1");
        s.press(SciKey::Backspace);
        s.press(SciKey::Backspace);
        assert_eq!(s.expr, "");
        keys(&mut s, "2*(3");
        s.press(SciKey::Backspace);
        s.press(SciKey::Backspace);
        assert_eq!(s.expr, "2*");
        s.press(SciKey::Text("e"));
        s.press(SciKey::Open);
        s.press(SciKey::Backspace);
        assert_eq!(s.expr, "2*e");
        s.press(SciKey::Clear);
        keys(&mut s, "2+2=");
        s.press(SciKey::Backspace);
        assert_eq!(s.expr, "2+2");
        assert_eq!(s.result, None);
    }

    #[test]
    fn rand_and_ee_keys() {
        let mut s = Sci::default();
        s.press(SciKey::Rand);
        let v = s.current_value();
        assert!(s.expr.starts_with("0.") && v.is_some_and(|v| (0.0..1.0).contains(&v)));
        let mut t = Sci::default();
        t.press(SciKey::Digit(2));
        t.press(SciKey::Exp);
        t.press(SciKey::Digit(3));
        keys(&mut t, "+1=");
        assert_eq!(t.display(), "2001");
    }

    #[test]
    fn memory_keys() {
        let mut s = Sci::default();
        keys(&mut s, "5");
        s.press(SciKey::MemAdd);
        keys(&mut s, "+3");
        s.press(SciKey::MemAdd);
        assert_eq!(s.memory, 13.0);
        s.press(SciKey::Clear);
        keys(&mut s, "2");
        s.press(SciKey::MemSub);
        assert_eq!(s.memory, 11.0);
        s.press(SciKey::Clear);
        s.press(SciKey::MemRecall);
        s.press(SciKey::Op('*'));
        keys(&mut s, "2=");
        assert_eq!(s.display(), "22");
        s.press(SciKey::Clear);
        assert_eq!(s.memory, 11.0, "clear keeps memory");
        s.press(SciKey::MemClear);
        assert_eq!(s.memory, 0.0);
        s.press(SciKey::MemAdd);
        assert_eq!(s.memory, 0.0, "empty entry adds nothing");
    }

    #[test]
    fn negative_memory_is_parenthesised_and_answer_recalls_last() {
        let mut s = Sci { memory: -4.0, ..Sci::default() };
        s.press(SciKey::MemRecall);
        assert_eq!(s.expr, "(-4)");
        s.press(SciKey::Clear);
        keys(&mut s, "6*7=");
        s.press(SciKey::Clear);
        s.press(SciKey::Answer);
        s.press(SciKey::Op('+'));
        keys(&mut s, "1=");
        assert_eq!(s.display(), "43");
    }

    #[test]
    fn sign_toggles_results_and_expressions() {
        let mut s = Sci::default();
        keys(&mut s, "2+3=");
        s.press(SciKey::Sign);
        assert_eq!(s.display(), "-5");
        s.press(SciKey::Sign);
        assert_eq!(s.display(), "5");
        s.press(SciKey::Clear);
        keys(&mut s, "2+3");
        s.press(SciKey::Sign);
        assert_eq!(s.expr, "-(2+3)");
        s.press(SciKey::Sign);
        assert_eq!(s.expr, "2+3");
        s.press(SciKey::Clear);
        s.press(SciKey::Sign);
        assert_eq!(s.expr, "-");
        s.press(SciKey::Sign);
        assert_eq!(s.expr, "");
        assert!(negated_group("-(2)"));
        assert!(!negated_group("-(2)+(3)"));
        assert!(!negated_group("-(2"));
    }

    #[test]
    fn set_value_loads_a_number_and_clear_label_follows_state() {
        let mut s = Sci::default();
        assert_eq!(s.clear_label(), "AC");
        s.set_value(1234.5);
        assert_eq!(s.display(), "1234.5");
        assert_eq!(s.clear_label(), "C");
        s.set_value(-2.0);
        assert_eq!(s.display(), "(-2)");
        assert_eq!(s.current_value(), Some(-2.0));
        keys(&mut s, "+1=");
        assert_eq!(s.display(), "-1");
    }

    #[test]
    fn memory_keys_and_power_and_history_entry() {
        let mut s = Sci::default();
        keys(&mut s, "2");
        s.press(SciKey::Postfix("^"));
        keys(&mut s, "10=");
        assert_eq!(s.display(), "1024");
        s.press(SciKey::MemAdd);
        s.press(SciKey::MemClear);
        assert_eq!(s.memory, 0.0);
        s.press(SciKey::MemAdd);
        s.press(SciKey::MemRecall);
        assert_eq!(s.display(), "1024");
        let mut t = Sci::default();
        keys(&mut t, "6/2");
        let entry = t.press(SciKey::Equals);
        assert!(entry.is_some());
    }
}
