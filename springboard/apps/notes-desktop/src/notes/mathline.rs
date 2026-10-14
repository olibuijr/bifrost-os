//! Math results in notes: a line that ends with `=` and holds an arithmetic expression gets its result shown after
//! it. Supports + - * / ^ %, parentheses, unary minus and decimal numbers (`,` or `.`). Pure functions over text.
#![allow(dead_code)] // the Edit > Show Math Results toggle and the editor overlay wire this in next

/// The result of `expr` as text, or None when it is not a complete arithmetic expression (or divides by zero).
pub fn evaluate(expr: &str) -> Option<String> {
    let mut p = Parser { s: expr.chars().filter(|c| !c.is_whitespace()).collect(), i: 0 };
    let v = p.sum()?;
    if p.i != p.s.len() || !v.is_finite() {
        return None;
    }
    Some(format(v))
}

/// A result without float noise: up to 10 decimals, trailing zeros trimmed.
pub fn format(v: f64) -> String {
    if v.abs() >= 1e15 || (v != 0.0 && v.abs() < 1e-10) {
        return format!("{v:e}");
    }
    let text = format!("{v:.10}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" { "0".into() } else { text.to_string() }
}

/// For each line of `text` that ends with `=`, `(line index, result)`.
pub fn results(text: &str) -> Vec<(usize, String)> {
    text.lines()
        .enumerate()
        .filter_map(|(i, line)| {
            let body = line.trim_end().strip_suffix('=')?;
            // the expression may follow a label such as "Total: 2 + 2 ="
            let expr = body.rsplit(|c| c == ':').next().unwrap_or(body);
            if !expr.chars().any(|c| c.is_ascii_digit()) {
                return None;
            }
            evaluate(expr).map(|r| (i, r))
        })
        .collect()
}

struct Parser {
    s: Vec<char>,
    i: usize,
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.s.get(self.i).copied()
    }

    fn sum(&mut self) -> Option<f64> {
        let mut v = self.product()?;
        while let Some(c @ ('+' | '-')) = self.peek() {
            self.i += 1;
            let r = self.product()?;
            v = if c == '+' { v + r } else { v - r };
        }
        Some(v)
    }

    fn product(&mut self) -> Option<f64> {
        let mut v = self.power()?;
        while let Some(c @ ('*' | '/' | '×' | '÷' | '%' | 'x')) = self.peek() {
            self.i += 1;
            let r = self.power()?;
            v = match c {
                '*' | '×' | 'x' => v * r,
                '/' | '÷' => {
                    if r == 0.0 {
                        return None;
                    }
                    v / r
                }
                _ => {
                    if r == 0.0 {
                        return None;
                    }
                    v % r
                }
            };
        }
        Some(v)
    }

    fn power(&mut self) -> Option<f64> {
        let base = self.unary()?;
        if self.peek() == Some('^') {
            self.i += 1;
            let exp = self.power()?; // right associative
            return Some(base.powf(exp));
        }
        Some(base)
    }

    fn unary(&mut self) -> Option<f64> {
        match self.peek()? {
            '-' => {
                self.i += 1;
                Some(-self.unary()?)
            }
            '+' => {
                self.i += 1;
                self.unary()
            }
            _ => self.atom(),
        }
    }

    fn atom(&mut self) -> Option<f64> {
        if self.peek()? == '(' {
            self.i += 1;
            let v = self.sum()?;
            if self.peek()? != ')' {
                return None;
            }
            self.i += 1;
            return Some(v);
        }
        let start = self.i;
        while self.peek().is_some_and(|c| c.is_ascii_digit() || c == '.' || c == ',') {
            self.i += 1;
        }
        let text: String = self.s[start..self.i].iter().map(|&c| if c == ',' { '.' } else { c }).collect();
        text.parse().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn precedence_and_parentheses() {
        assert_eq!(evaluate("2 + 3 * 4").as_deref(), Some("14"));
        assert_eq!(evaluate("(2 + 3) * 4").as_deref(), Some("20"));
        assert_eq!(evaluate("2 ^ 3 ^ 2").as_deref(), Some("512"));
        assert_eq!(evaluate("-2 ^ 2").as_deref(), Some("4"));
        assert_eq!(evaluate("10 % 4").as_deref(), Some("2"));
    }

    #[test]
    fn decimals_without_float_noise() {
        assert_eq!(evaluate("0.1 + 0.2").as_deref(), Some("0.3"));
        assert_eq!(evaluate("1,5 * 2").as_deref(), Some("3"));
        assert_eq!(evaluate("10 / 4").as_deref(), Some("2.5"));
        assert_eq!(evaluate("1 / 3").as_deref(), Some("0.3333333333"));
    }

    #[test]
    fn incomplete_or_invalid_gives_nothing() {
        assert_eq!(evaluate("2 +"), None);
        assert_eq!(evaluate("(2"), None);
        assert_eq!(evaluate("1 / 0"), None);
        assert_eq!(evaluate("hello"), None);
        assert_eq!(evaluate(""), None);
    }

    #[test]
    fn only_lines_ending_with_equals_count() {
        let text = "Budget\n12 * 3 =\nTotal: 5 + 5 =\nnot math =\n2 + 2\n7 - 9 =";
        assert_eq!(
            results(text),
            vec![(1, "36".to_string()), (2, "10".to_string()), (5, "-2".to_string())]
        );
    }
}
