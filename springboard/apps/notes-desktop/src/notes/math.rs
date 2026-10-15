//! Math Results: a line that ends with `=` after a calculation gets the answer. The expression parser is a small
//! recursive descent over `+ - * / % ^` and parentheses; it works on text only, so it is tested on the host.

use springboard::shell::md;

/// A parse in progress: the chars, the position, and whether an operator or a bracket was used (a bare number
/// is not a calculation).
struct Parser<'a> {
    chars: &'a [char],
    at: usize,
    operated: bool,
}

impl Parser<'_> {
    fn skip_space(&mut self) {
        while self.chars.get(self.at).is_some_and(|c| c.is_whitespace()) {
            self.at += 1;
        }
    }

    fn eat(&mut self, c: char) -> bool {
        self.skip_space();
        if self.chars.get(self.at) == Some(&c) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    fn peek(&mut self) -> Option<char> {
        self.skip_space();
        self.chars.get(self.at).copied()
    }

    fn expr(&mut self) -> Option<f64> {
        let mut v = self.term()?;
        loop {
            match self.peek() {
                Some('+') => {
                    self.at += 1;
                    self.operated = true;
                    v += self.term()?;
                }
                Some('-') => {
                    self.at += 1;
                    self.operated = true;
                    v -= self.term()?;
                }
                _ => return Some(v),
            }
        }
    }

    fn term(&mut self) -> Option<f64> {
        let mut v = self.unary()?;
        loop {
            match self.peek() {
                Some('*') | Some('\u{d7}') => {
                    self.at += 1;
                    self.operated = true;
                    v *= self.unary()?;
                }
                Some('/') | Some('\u{f7}') => {
                    self.at += 1;
                    self.operated = true;
                    let d = self.unary()?;
                    if d == 0.0 {
                        return None;
                    }
                    v /= d;
                }
                Some('%') => {
                    self.at += 1;
                    self.operated = true;
                    let d = self.unary()?;
                    if d == 0.0 {
                        return None;
                    }
                    v %= d;
                }
                _ => return Some(v),
            }
        }
    }

    /// A sign applies to the whole power after it: `-2 ^ 2` is -4.
    fn unary(&mut self) -> Option<f64> {
        match self.peek() {
            Some('-') => {
                self.at += 1;
                Some(-self.unary()?)
            }
            Some('+') => {
                self.at += 1;
                self.unary()
            }
            _ => self.power(),
        }
    }

    /// `^` is right-associative: `2 ^ 3 ^ 2` is 2 ^ 9.
    fn power(&mut self) -> Option<f64> {
        let base = self.atom()?;
        if self.eat('^') {
            self.operated = true;
            let exp = self.unary()?;
            return Some(base.powf(exp)).filter(|v| v.is_finite());
        }
        Some(base)
    }

    fn atom(&mut self) -> Option<f64> {
        if self.eat('(') {
            self.operated = true;
            let v = self.expr()?;
            return self.eat(')').then_some(v);
        }
        self.number()
    }

    fn number(&mut self) -> Option<f64> {
        self.skip_space();
        let from = self.at;
        let mut dot = false;
        while let Some(&c) = self.chars.get(self.at) {
            if c.is_ascii_digit() {
                self.at += 1;
            } else if c == '.' && !dot {
                dot = true;
                self.at += 1;
            } else {
                break;
            }
        }
        let text: String = self.chars[from..self.at].iter().collect();
        if !text.chars().any(|c| c.is_ascii_digit()) {
            return None;
        }
        text.parse().ok()
    }
}

/// The value of `expr`, or `None` when it is not a whole calculation (garbage, a bare number, a division by zero,
/// an overflow).
pub fn eval(expr: &str) -> Option<f64> {
    let chars: Vec<char> = expr.chars().collect();
    let mut p = Parser { chars: &chars, at: 0, operated: false };
    let v = p.expr()?;
    p.skip_space();
    (p.at == chars.len() && p.operated && v.is_finite()).then_some(v)
}

/// A result as it is written in the note: whole numbers without a fraction, others to at most six decimals.
pub fn format_number(v: f64) -> String {
    if v == v.trunc() && v.abs() < 1e15 {
        return format!("{}", v as i64);
    }
    let s = format!("{v:.6}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// The answer for a line that ends with `=`: the longest tail of the line (after any list marker) that is a
/// calculation, so "Total: 3 + 4 =" is 7. `None` when the line does not end with `=` or has no calculation.
pub fn result_for_line(line: &str) -> Option<String> {
    let body = line.trim_end().strip_suffix('=')?;
    let skip = md::classify(body).prefix;
    let chars: Vec<char> = body.chars().skip(skip).collect();
    (0..chars.len()).find_map(|start| {
        let first = chars[start..].iter().find(|c| !c.is_whitespace())?;
        if !(first.is_ascii_digit() || matches!(first, '(' | '-' | '+' | '.')) {
            return None;
        }
        let tail: String = chars[start..].iter().collect();
        eval(&tail).map(format_number)
    })
}

/// The line with its answer after the `=`; the line itself when it asks nothing.
pub fn complete_line(line: &str) -> String {
    match result_for_line(line) {
        Some(r) => format!("{} {r}", line.trim_end()),
        None => line.to_string(),
    }
}

/// Every line of `text` that ends with `=` gets its answer.
pub fn complete_text(text: &str) -> String {
    text.split('\n').map(complete_line).collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arithmetic_follows_precedence_brackets_and_signs() {
        assert_eq!(eval("2 + 3 * 4"), Some(14.0));
        assert_eq!(eval("(2 + 3) * 4"), Some(20.0));
        assert_eq!(eval("10 / 4"), Some(2.5));
        assert_eq!(eval("10 % 4"), Some(2.0));
        assert_eq!(eval("2 ^ 3 ^ 2"), Some(512.0), "power is right-associative");
        assert_eq!(eval("-2 ^ 2"), Some(-4.0));
        assert_eq!(eval("2 ^ -1"), Some(0.5));
        assert_eq!(eval("-(1 + 2)"), Some(-3.0));
        assert_eq!(eval("1.5 + 1.5"), Some(3.0));
        assert_eq!(eval("2 \u{d7} 3 \u{f7} 6"), Some(1.0));
    }

    #[test]
    fn things_that_are_not_a_calculation_have_no_value() {
        for bad in ["", "5", "(5)x", "2 +", "2 + * 3", "1 / 0", "1 % 0", "abc", "1.2.3 + 1", "(1 + 2", "2 3", "9 ^ 9 ^ 9 ^ 9"] {
            assert_eq!(eval(bad), None, "{bad}");
        }
    }

    #[test]
    fn numbers_print_without_noise() {
        assert_eq!(format_number(4.0), "4");
        assert_eq!(format_number(-3.0), "-3");
        assert_eq!(format_number(2.5), "2.5");
        assert_eq!(format_number(1.0 / 3.0), "0.333333");
        assert_eq!(format_number(0.1 + 0.2), "0.3");
        assert_eq!(format_number(-0.0), "0");
    }

    #[test]
    fn a_line_ending_with_equals_gets_its_answer() {
        assert_eq!(result_for_line("2+2="), Some("4".into()));
        assert_eq!(result_for_line("2 + 2 = "), Some("4".into()), "trailing space after the sign");
        assert_eq!(result_for_line("Total: 3 + 4 ="), Some("7".into()));
        assert_eq!(result_for_line("- 10 / 4 ="), Some("2.5".into()), "a list marker is not part of the sum");
        assert_eq!(result_for_line("- [ ] (1+2)*3="), Some("9".into()));
        assert_eq!(result_for_line("2+2"), None, "no equals sign");
        assert_eq!(result_for_line("2+2= 4"), None, "already answered");
        assert_eq!(result_for_line("hello="), None);
        assert_eq!(result_for_line("5="), None, "a bare number asks nothing");
        assert_eq!(result_for_line("-5="), None, "so does a signed one");
    }

    #[test]
    fn completing_text_answers_every_question_and_leaves_the_rest() {
        let text = "Budget\n12 * 3 =\nrent 500 + 120=\nnothing here\n7-2= 5";
        assert_eq!(complete_text(text), "Budget\n12 * 3 = 36\nrent 500 + 120= 620\nnothing here\n7-2= 5");
        assert_eq!(complete_line("plain"), "plain");
        assert_eq!(complete_text(&complete_text(text)), complete_text(text), "completing twice changes nothing");
    }
}
