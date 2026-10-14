//! Math results: a line that ends with `=` is an expression whose result is written after the sign
//! (`12 * 4 =` becomes `12 * 4 = 48`). The evaluator is a small recursive-descent parser over `+ - * / % ^`,
//! parentheses, unary minus, the constant `pi` and `sqrt(...)`. Anything it does not understand leaves the line alone.
use springboard::shell::md;

/// Evaluates `expr`; None when it is empty, malformed or has a non-finite result (division by zero).
pub fn eval(expr: &str) -> Option<f64> {
    let mut p = Parser { chars: expr.chars().filter(|c| !c.is_whitespace()).collect(), pos: 0 };
    let value = p.sum()?;
    (p.pos == p.chars.len() && value.is_finite()).then_some(value)
}

struct Parser {
    chars: Vec<char>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn sum(&mut self) -> Option<f64> {
        let mut v = self.product()?;
        loop {
            if self.eat('+') {
                v += self.product()?;
            } else if self.eat('-') {
                v -= self.product()?;
            } else {
                return Some(v);
            }
        }
    }

    fn product(&mut self) -> Option<f64> {
        let mut v = self.power()?;
        loop {
            if self.eat('*') || self.eat('\u{d7}') {
                v *= self.power()?;
            } else if self.eat('/') || self.eat('\u{f7}') {
                v /= self.power()?;
            } else if self.eat('%') {
                v %= self.power()?;
            } else {
                return Some(v);
            }
        }
    }

    /// `^` is right-associative and binds tighter than the product.
    fn power(&mut self) -> Option<f64> {
        let base = self.unary()?;
        if self.eat('^') {
            return Some(base.powf(self.power()?));
        }
        Some(base)
    }

    fn unary(&mut self) -> Option<f64> {
        if self.eat('-') {
            return Some(-self.unary()?);
        }
        if self.eat('+') {
            return self.unary();
        }
        self.atom()
    }

    fn atom(&mut self) -> Option<f64> {
        if self.eat('(') {
            let v = self.sum()?;
            return self.eat(')').then_some(v);
        }
        if self.peek()?.is_ascii_alphabetic() {
            let start = self.pos;
            while self.peek().is_some_and(|c| c.is_ascii_alphabetic()) {
                self.pos += 1;
            }
            let name: String = self.chars[start..self.pos].iter().collect();
            return match name.as_str() {
                "pi" => Some(std::f64::consts::PI),
                "sqrt" => {
                    if !self.eat('(') {
                        return None;
                    }
                    let v = self.sum()?;
                    (self.eat(')') && v >= 0.0).then(|| v.sqrt())
                }
                _ => None,
            };
        }
        let start = self.pos;
        while self.peek().is_some_and(|c| c.is_ascii_digit() || c == '.' || c == ',') {
            self.pos += 1;
        }
        let digits: String = self.chars[start..self.pos].iter().filter(|c| **c != ',').collect();
        digits.parse().ok()
    }
}

/// A result as text: whole numbers without a fraction, otherwise at most 8 decimals with trailing zeros cut.
pub fn format_result(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        return format!("{}", v as i64);
    }
    let s = format!("{v:.8}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// The line with its result after the trailing `=`; None when the line does not end in `=` or does not evaluate.
/// A list marker in front of the expression is kept and not evaluated.
pub fn solve_line(line: &str) -> Option<String> {
    let trimmed = line.trim_end();
    let body = trimmed.strip_suffix('=')?;
    let prefix = md::classify(body).prefix.min(body.chars().count());
    let expr: String = body.chars().skip(prefix).collect();
    if !expr.chars().any(|c| c.is_ascii_digit()) {
        return None;
    }
    let value = eval(&expr)?;
    Some(format!("{trimmed} {}", format_result(value)))
}

/// Solves every line of `text` that ends in `=`; the text is returned unchanged when none does.
pub fn solve_all(text: &str) -> String {
    text.split('\n').map(|l| solve_line(l).unwrap_or_else(|| l.to_string())).collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn precedence_parentheses_and_unary_minus() {
        assert_eq!(eval("2+3*4"), Some(14.0));
        assert_eq!(eval("(2+3)*4"), Some(20.0));
        assert_eq!(eval("-2^2"), Some(4.0), "unary minus binds tighter than the power");
        assert_eq!(eval("2^3^2"), Some(512.0));
        assert_eq!(eval("10 % 4"), Some(2.0));
        assert_eq!(eval("7 / 2"), Some(3.5));
        assert_eq!(eval("1,000 + 1"), Some(1001.0), "thousands separators are ignored");
    }

    #[test]
    fn functions_and_constants() {
        assert_eq!(eval("sqrt(16)"), Some(4.0));
        assert!((eval("pi * 2").unwrap() - 6.283185307).abs() < 1e-6);
        assert_eq!(eval("sqrt(-1)"), None);
        assert_eq!(eval("foo(2)"), None);
    }

    #[test]
    fn malformed_and_non_finite_expressions_are_refused() {
        for e in ["", "2+", "(1", "1/0", "abc", "2 2", "1)"] {
            assert_eq!(eval(e), None, "{e}");
        }
    }

    #[test]
    fn results_drop_empty_fractions() {
        assert_eq!(format_result(48.0), "48");
        assert_eq!(format_result(3.5), "3.5");
        assert_eq!(format_result(1.0 / 3.0), "0.33333333");
        assert_eq!(format_result(-0.25), "-0.25");
    }

    #[test]
    fn a_line_ending_in_equals_gets_its_result() {
        assert_eq!(solve_line("12 * 4 =").as_deref(), Some("12 * 4 = 48"));
        assert_eq!(solve_line("12*4=  ").as_deref(), Some("12*4= 48"));
        assert_eq!(solve_line("- [ ] 2+2 =").as_deref(), Some("- [ ] 2+2 = 4"), "the list marker is kept");
        assert_eq!(solve_line("12 * 4 = 48"), None, "a solved line is left alone");
        assert_eq!(solve_line("x ="), None);
        assert_eq!(solve_line("="), None);
    }

    #[test]
    fn solve_all_touches_only_the_calculation_lines() {
        assert_eq!(solve_all("Budget\n1200 + 300 =\nnotes"), "Budget\n1200 + 300 = 1500\nnotes");
        assert_eq!(solve_all("nothing here"), "nothing here");
    }
}
