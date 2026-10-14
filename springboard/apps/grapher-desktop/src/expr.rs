//! Expressions in the variables `x` and `t` (`theta` is another name for `t`): a tokenizer, a recursive-descent parser
//! and an evaluator on `f64`. Operators bind as usual: `^` above `*` and `/`, which bind above `+` and `-`; `^` is
//! right-associative, and a unary minus binds looser than `^` (so `-2^2` is -4). A product may be written without the
//! `*` (`2x`, `3(x + 1)`, `x(x - 1)`).

use std::f64::consts::{E, PI, TAU};
use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Num(f64),
    /// The variable `x`.
    X,
    /// The variable `t` (also written `theta`).
    T,
    Neg(Box<Expr>),
    Bin(Op, Box<Expr>, Box<Expr>),
    Call(Func, Box<Expr>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Func {
    Sin,
    Cos,
    Tan,
    Asin,
    Acos,
    Atan,
    Sinh,
    Cosh,
    Tanh,
    Sqrt,
    Abs,
    Ln,
    Log,
    Exp,
    Floor,
    Ceil,
}

/// A syntax error: the character position (0 based, in bytes) and what was wrong.
#[derive(Debug, Clone, PartialEq)]
pub struct ParseError {
    pub pos: usize,
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at position {}", self.message, self.pos + 1)
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Num(f64),
    /// A name, lower-cased: a variable, a constant or a function.
    Ident(String),
    /// One of `+ - * / ^`.
    Op(char),
    LParen,
    RParen,
}

/// Parses an expression. Empty or blank input is an error.
pub fn parse(src: &str) -> Result<Expr, ParseError> {
    let tokens = tokenize(src)?;
    if tokens.is_empty() {
        return Err(ParseError { pos: 0, message: "empty expression".to_string() });
    }
    let mut parser = Parser { tokens, at: 0, end: src.len() };
    let expr = parser.expr()?;
    if parser.at < parser.tokens.len() {
        return Err(ParseError { pos: parser.pos(), message: "unexpected input after the expression".to_string() });
    }
    Ok(expr)
}

fn tokenize(src: &str) -> Result<Vec<(usize, Token)>, ParseError> {
    let chars: Vec<(usize, char)> = src.char_indices().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let (pos, c) = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c.is_ascii_digit() || c == '.' {
            let start = i;
            while i < chars.len() && (chars[i].1.is_ascii_digit() || chars[i].1 == '.') {
                i += 1;
            }
            // an exponent counts only when a digit follows it: "2e3" is 2000, "2e" is 2 times the constant e
            if i < chars.len() && matches!(chars[i].1, 'e' | 'E') {
                let mut j = i + 1;
                if j < chars.len() && matches!(chars[j].1, '+' | '-') {
                    j += 1;
                }
                if j < chars.len() && chars[j].1.is_ascii_digit() {
                    while j < chars.len() && chars[j].1.is_ascii_digit() {
                        j += 1;
                    }
                    i = j;
                }
            }
            let text: String = chars[start..i].iter().map(|&(_, c)| c).collect();
            let value = text
                .parse::<f64>()
                .map_err(|_| ParseError { pos, message: format!("'{text}' is not a number") })?;
            out.push((pos, Token::Num(value)));
        } else if c.is_alphabetic() {
            let start = i;
            while i < chars.len() && chars[i].1.is_alphanumeric() {
                i += 1;
            }
            let name: String = chars[start..i].iter().map(|&(_, c)| c).collect();
            out.push((pos, Token::Ident(name.to_lowercase())));
        } else {
            let token = match c {
                '+' | '-' | '*' | '/' | '^' => Token::Op(c),
                '(' => Token::LParen,
                ')' => Token::RParen,
                other => return Err(ParseError { pos, message: format!("unexpected character '{other}'") }),
            };
            out.push((pos, token));
            i += 1;
        }
    }
    Ok(out)
}

struct Parser {
    tokens: Vec<(usize, Token)>,
    at: usize,
    /// The length of the source, where errors at the end point.
    end: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at).map(|(_, t)| t)
    }

    fn pos(&self) -> usize {
        self.tokens.get(self.at).map_or(self.end, |(p, _)| *p)
    }

    fn bump(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.at).map(|(_, t)| t.clone());
        self.at += 1;
        token
    }

    /// expr := term { ('+' | '-') term }
    fn expr(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.term()?;
        loop {
            let op = match self.peek().cloned() {
                Some(Token::Op('+')) => Op::Add,
                Some(Token::Op('-')) => Op::Sub,
                _ => break,
            };
            self.at += 1;
            let right = self.term()?;
            left = Expr::Bin(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    /// term := unary { ('*' | '/') unary | unary }, where a bare operand after another is an implicit product
    fn term(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.unary()?;
        loop {
            match self.peek().cloned() {
                Some(Token::Op('*')) => {
                    self.at += 1;
                    let right = self.unary()?;
                    left = Expr::Bin(Op::Mul, Box::new(left), Box::new(right));
                }
                Some(Token::Op('/')) => {
                    self.at += 1;
                    let right = self.unary()?;
                    left = Expr::Bin(Op::Div, Box::new(left), Box::new(right));
                }
                Some(Token::Num(_) | Token::Ident(_) | Token::LParen) => {
                    let right = self.unary()?;
                    left = Expr::Bin(Op::Mul, Box::new(left), Box::new(right));
                }
                _ => break,
            }
        }
        Ok(left)
    }

    /// unary := ('-' | '+') unary | power
    fn unary(&mut self) -> Result<Expr, ParseError> {
        match self.peek().cloned() {
            Some(Token::Op('-')) => {
                self.at += 1;
                Ok(Expr::Neg(Box::new(self.unary()?)))
            }
            Some(Token::Op('+')) => {
                self.at += 1;
                self.unary()
            }
            _ => self.power(),
        }
    }

    /// power := primary [ '^' unary ]
    fn power(&mut self) -> Result<Expr, ParseError> {
        let base = self.primary()?;
        if matches!(self.peek(), Some(Token::Op('^'))) {
            self.at += 1;
            let exponent = self.unary()?;
            return Ok(Expr::Bin(Op::Pow, Box::new(base), Box::new(exponent)));
        }
        Ok(base)
    }

    /// primary := number | name | name '(' expr ')' | '(' expr ')'
    fn primary(&mut self) -> Result<Expr, ParseError> {
        let pos = self.pos();
        match self.bump() {
            Some(Token::Num(value)) => Ok(Expr::Num(value)),
            Some(Token::LParen) => {
                let inner = self.expr()?;
                self.expect_rparen()?;
                Ok(inner)
            }
            Some(Token::Ident(name)) => {
                let call_follows = matches!(self.peek(), Some(Token::LParen));
                if let (true, Some(func)) = (call_follows, Func::from_name(&name)) {
                    self.at += 1;
                    let arg = self.expr()?;
                    self.expect_rparen()?;
                    return Ok(Expr::Call(func, Box::new(arg)));
                }
                match name.as_str() {
                    "x" => Ok(Expr::X),
                    "t" | "theta" => Ok(Expr::T),
                    "pi" => Ok(Expr::Num(PI)),
                    "tau" => Ok(Expr::Num(TAU)),
                    "e" => Ok(Expr::Num(E)),
                    _ => Err(ParseError { pos, message: format!("unknown name '{name}'") }),
                }
            }
            Some(_) => Err(ParseError { pos, message: "unexpected symbol".to_string() }),
            None => Err(ParseError { pos: self.end, message: "the expression ends too early".to_string() }),
        }
    }

    fn expect_rparen(&mut self) -> Result<(), ParseError> {
        let pos = self.pos();
        match self.bump() {
            Some(Token::RParen) => Ok(()),
            _ => Err(ParseError { pos, message: "a closing ')' is missing".to_string() }),
        }
    }
}

impl Func {
    fn from_name(name: &str) -> Option<Func> {
        Some(match name {
            "sin" => Func::Sin,
            "cos" => Func::Cos,
            "tan" => Func::Tan,
            "asin" => Func::Asin,
            "acos" => Func::Acos,
            "atan" => Func::Atan,
            "sinh" => Func::Sinh,
            "cosh" => Func::Cosh,
            "tanh" => Func::Tanh,
            "sqrt" => Func::Sqrt,
            "abs" => Func::Abs,
            "ln" => Func::Ln,
            "log" | "log10" => Func::Log,
            "exp" => Func::Exp,
            "floor" => Func::Floor,
            "ceil" => Func::Ceil,
            _ => return None,
        })
    }

    pub fn apply(self, v: f64) -> f64 {
        match self {
            Func::Sin => v.sin(),
            Func::Cos => v.cos(),
            Func::Tan => v.tan(),
            Func::Asin => v.asin(),
            Func::Acos => v.acos(),
            Func::Atan => v.atan(),
            Func::Sinh => v.sinh(),
            Func::Cosh => v.cosh(),
            Func::Tanh => v.tanh(),
            Func::Sqrt => v.sqrt(),
            Func::Abs => v.abs(),
            Func::Ln => v.ln(),
            Func::Log => v.log10(),
            Func::Exp => v.exp(),
            Func::Floor => v.floor(),
            Func::Ceil => v.ceil(),
        }
    }
}

impl Expr {
    /// The value at `x` and `t`. Outside the domain the result is NaN or infinite; callers treat those as gaps.
    pub fn eval(&self, x: f64, t: f64) -> f64 {
        match self {
            Expr::Num(v) => *v,
            Expr::X => x,
            Expr::T => t,
            Expr::Neg(inner) => -inner.eval(x, t),
            Expr::Bin(op, a, b) => {
                let (a, b) = (a.eval(x, t), b.eval(x, t));
                match op {
                    Op::Add => a + b,
                    Op::Sub => a - b,
                    Op::Mul => a * b,
                    Op::Div => a / b,
                    Op::Pow => a.powf(b),
                }
            }
            Expr::Call(func, arg) => func.apply(arg.eval(x, t)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(src: &str, x: f64, t: f64) -> f64 {
        parse(src).unwrap_or_else(|e| panic!("{src}: {e}")).eval(x, t)
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn precedence_of_sums_products_and_powers() {
        assert!(close(value("2+3*4", 0.0, 0.0), 14.0));
        assert!(close(value("(2+3)*4", 0.0, 0.0), 20.0));
        assert!(close(value("8/4/2", 0.0, 0.0), 1.0));
        assert!(close(value("2^3^2", 0.0, 0.0), 512.0));
        assert!(close(value("-2^2", 0.0, 0.0), -4.0));
        assert!(close(value("2^-1", 0.0, 0.0), 0.5));
    }

    #[test]
    fn variables_and_constants() {
        assert!(close(value("x^2 - 4", 3.0, 0.0), 5.0));
        assert!(close(value("t * 2", 0.0, 1.5), 3.0));
        assert!(close(value("theta", 0.0, 0.25), 0.25));
        assert!(close(value("sin(pi / 2)", 0.0, 0.0), 1.0));
        assert!(close(value("tau", 0.0, 0.0), TAU));
        assert!(close(value("e", 0.0, 0.0), E));
    }

    #[test]
    fn implicit_products() {
        assert!(close(value("2x", 3.0, 0.0), 6.0));
        assert!(close(value("3(x + 1)", 1.0, 0.0), 6.0));
        assert!(close(value("x(x - 1)", 4.0, 0.0), 12.0));
        assert!(close(value("2pi", 0.0, 0.0), 2.0 * PI));
        assert!(close(value("2e3", 0.0, 0.0), 2000.0));
        assert!(close(value("1.5e-1", 0.0, 0.0), 0.15));
    }

    #[test]
    fn functions() {
        assert!(close(value("sqrt(16)", 0.0, 0.0), 4.0));
        assert!(close(value("log(1000)", 0.0, 0.0), 3.0));
        assert!(close(value("ln(e)", 0.0, 0.0), 1.0));
        assert!(close(value("abs(-3)", 0.0, 0.0), 3.0));
        assert!(close(value("floor(2.7) + ceil(2.1)", 0.0, 0.0), 5.0));
        assert!(close(value("SIN(0)", 0.0, 0.0), 0.0));
    }

    #[test]
    fn outside_the_domain_is_not_finite() {
        assert!(value("sqrt(x)", -1.0, 0.0).is_nan());
        assert!(!value("1/x", 0.0, 0.0).is_finite());
        assert!(value("ln(x)", 0.0, 0.0).is_infinite());
    }

    #[test]
    fn errors_have_positions() {
        assert!(parse("").is_err());
        assert!(parse("   ").is_err());
        assert!(parse("2 +").is_err());
        assert!(parse("(1 + 2").is_err());
        assert!(parse("1 + 2)").is_err());
        assert!(parse("foo").is_err());
        assert!(parse("sin").is_err());
        assert!(parse("2 $ 3").is_err());
        assert!(parse("1.2.3").is_err());
        let err = parse("2 $ 3").unwrap_err();
        assert_eq!(err.pos, 2);
        assert_eq!(err.to_string(), "unexpected character '$' at position 3");
    }
}
