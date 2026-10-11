//! Expression evaluator for the scientific keypad: f64 arithmetic with
//! proper precedence, parentheses, bifrost percent semantics and the
//! scientific functions. Pure logic, no UI.

use std::f64::consts::{E, PI};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Angle {
    #[default]
    Rad,
    Deg,
}

impl Angle {
    pub fn toggled(self) -> Self {
        match self {
            Self::Rad => Self::Deg,
            Self::Deg => Self::Rad,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EvalError {
    Syntax,
    /// Outside a function's domain, such as `1/0` or `ln(-1)`.
    Domain,
    /// The result does not fit an f64.
    Overflow,
}

type Res<T> = Result<T, EvalError>;

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Num(f64),
    Id(String),
    Sym(char),
}

const FUNCTIONS: [&str; 24] = [
    "sin", "cos", "tan", "asin", "acos", "atan", "sinh", "cosh", "tanh", "asinh", "acosh",
    "atanh", "ln", "log", "log2", "sqrt", "cbrt", "exp", "exp10", "abs", "yroot", "√", "∛",
    "sq",
];

/// Whether `name` is a function the evaluator knows (used by backspace to
/// delete `sin(` as one unit).
pub fn is_function(name: &str) -> bool {
    FUNCTIONS.contains(&name)
}

fn lex(src: &str) -> Res<Vec<Tok>> {
    let cs: Vec<char> = src.chars().collect();
    let mut i = 0;
    let mut out = vec![];
    while i < cs.len() {
        let c = cs[i];
        match c {
            ' ' | '\t' | '\u{a0}' => i += 1,
            '0'..='9' | '.' => {
                let start = i;
                let mut dots = 0;
                while i < cs.len() && (cs[i].is_ascii_digit() || cs[i] == '.') {
                    if cs[i] == '.' {
                        dots += 1;
                    }
                    i += 1;
                }
                let text: String = cs[start..i].iter().collect();
                if dots > 1 || text == "." {
                    return Err(EvalError::Syntax);
                }
                let mut value: f64 = text.parse().map_err(|_| EvalError::Syntax)?;
                if i < cs.len() && cs[i] == 'E' {
                    let mut j = i + 1;
                    if j < cs.len() && (cs[j] == '-' || cs[j] == '+') {
                        j += 1;
                    }
                    let digits = j;
                    while j < cs.len() && cs[j].is_ascii_digit() {
                        j += 1;
                    }
                    if j > digits {
                        let exp: String = cs[i + 1..j].iter().collect();
                        let exp: i32 = exp.parse().map_err(|_| EvalError::Syntax)?;
                        value *= 10f64.powi(exp);
                        i = j;
                    }
                }
                out.push(Tok::Num(value));
            }
            '+' | '-' | '*' | '/' | '^' | '!' | '%' | '(' | ')' | ',' => {
                out.push(Tok::Sym(c));
                i += 1;
            }
            '×' => {
                out.push(Tok::Sym('*'));
                i += 1;
            }
            '÷' => {
                out.push(Tok::Sym('/'));
                i += 1;
            }
            '−' => {
                out.push(Tok::Sym('-'));
                i += 1;
            }
            'π' => {
                out.push(Tok::Id("pi".into()));
                i += 1;
            }
            '√' | '∛' => {
                out.push(Tok::Id(c.to_string()));
                i += 1;
            }
            c if c.is_ascii_alphabetic() => {
                let start = i;
                while i < cs.len() && cs[i].is_ascii_alphanumeric() {
                    i += 1;
                }
                out.push(Tok::Id(cs[start..i].iter().collect()));
            }
            _ => return Err(EvalError::Syntax),
        }
    }
    Ok(out)
}

struct Parser {
    toks: Vec<Tok>,
    i: usize,
    angle: Angle,
    depth: usize,
}

/// A value plus whether it was written as a bare `x%` (for bifrost percent).
type Val = (f64, bool);

impl Parser {
    fn sym(&self, c: char) -> bool {
        matches!(self.toks.get(self.i), Some(Tok::Sym(s)) if *s == c)
    }

    fn enter(&mut self) -> Res<()> {
        self.depth += 1;
        if self.depth > 200 {
            Err(EvalError::Syntax)
        } else {
            Ok(())
        }
    }

    fn expr(&mut self) -> Res<Val> {
        self.enter()?;
        let (mut v, mut pct) = self.term()?;
        loop {
            let plus = if self.sym('+') {
                true
            } else if self.sym('-') {
                false
            } else {
                break;
            };
            self.i += 1;
            let (r, right_pct) = self.term()?;
            // bifrost: `200 + 10%` adds ten percent *of 200*.
            let r = if right_pct { v * r } else { r };
            v = if plus { v + r } else { v - r };
            pct = false;
        }
        self.depth -= 1;
        Ok((v, pct))
    }

    fn term(&mut self) -> Res<Val> {
        let (mut v, mut pct) = self.unary()?;
        loop {
            if self.sym('*') {
                self.i += 1;
                v *= self.unary()?.0;
            } else if self.sym('/') {
                self.i += 1;
                let r = self.unary()?.0;
                if r == 0.0 {
                    return Err(EvalError::Domain);
                }
                v /= r;
            } else if matches!(
                self.toks.get(self.i),
                Some(Tok::Num(_) | Tok::Id(_) | Tok::Sym('('))
            ) {
                v *= self.unary()?.0;
            } else {
                break;
            }
            pct = false;
        }
        Ok((v, pct))
    }

    fn unary(&mut self) -> Res<Val> {
        if self.sym('-') {
            self.i += 1;
            self.enter()?;
            let (v, _) = self.unary()?;
            self.depth -= 1;
            return Ok((-v, false));
        }
        if self.sym('+') {
            self.i += 1;
            return self.unary();
        }
        self.power()
    }

    fn power(&mut self) -> Res<Val> {
        let (base, pct) = self.postfix()?;
        if self.sym('^') {
            self.i += 1;
            self.enter()?;
            let exponent = self.exponent()?;
            self.depth -= 1;
            return pow(base, exponent).map(|v| (v, false));
        }
        Ok((base, pct))
    }

    /// Right associative and signed: `2^-3`, `2^3^2`.
    fn exponent(&mut self) -> Res<f64> {
        if self.sym('-') {
            self.i += 1;
            self.enter()?;
            let v = self.exponent()?;
            self.depth -= 1;
            return Ok(-v);
        }
        if self.sym('+') {
            self.i += 1;
        }
        Ok(self.power()?.0)
    }

    fn postfix(&mut self) -> Res<Val> {
        let mut v = self.atom()?;
        let mut pct = false;
        loop {
            if self.sym('!') {
                self.i += 1;
                v = factorial(v)?;
                pct = false;
            } else if self.sym('%') {
                self.i += 1;
                v /= 100.0;
                pct = true;
            } else {
                break;
            }
        }
        Ok((v, pct))
    }

    fn close(&mut self) -> Res<()> {
        if self.sym(')') {
            self.i += 1;
            Ok(())
        } else if self.i >= self.toks.len() {
            // Unclosed parentheses at the end close themselves.
            Ok(())
        } else {
            Err(EvalError::Syntax)
        }
    }

    fn atom(&mut self) -> Res<f64> {
        let tok = self.toks.get(self.i).cloned();
        match tok {
            Some(Tok::Num(n)) => {
                self.i += 1;
                Ok(n)
            }
            Some(Tok::Sym('(')) => {
                self.i += 1;
                let (v, _) = self.expr()?;
                self.close()?;
                Ok(v)
            }
            Some(Tok::Id(name)) => {
                self.i += 1;
                match name.as_str() {
                    "pi" => return Ok(PI),
                    "e" => return Ok(E),
                    _ => {}
                }
                let args = if self.sym('(') {
                    self.i += 1;
                    let mut args = vec![self.expr()?.0];
                    while self.sym(',') {
                        self.i += 1;
                        args.push(self.expr()?.0);
                    }
                    self.close()?;
                    args
                } else {
                    self.enter()?;
                    let v = self.unary()?.0;
                    self.depth -= 1;
                    vec![v]
                };
                call(&name, &args, self.angle)
            }
            _ => Err(EvalError::Syntax),
        }
    }
}

fn finite(v: f64) -> Res<f64> {
    if v.is_finite() {
        Ok(v)
    } else {
        Err(EvalError::Overflow)
    }
}

fn pow(base: f64, exponent: f64) -> Res<f64> {
    if base == 0.0 && exponent < 0.0 {
        return Err(EvalError::Domain);
    }
    if base < 0.0 && exponent.fract() != 0.0 {
        return Err(EvalError::Domain);
    }
    finite(base.powf(exponent))
}

fn factorial(v: f64) -> Res<f64> {
    if v < 0.0 || v.fract() != 0.0 {
        return Err(EvalError::Domain);
    }
    if v > 170.0 {
        return Err(EvalError::Overflow);
    }
    let mut r = 1.0;
    for k in 2..=(v as u32) {
        r *= k as f64;
    }
    Ok(r)
}

/// The `y`-th root of `x`; odd integer roots of negatives are allowed.
fn root(x: f64, y: f64) -> Res<f64> {
    if y == 0.0 {
        return Err(EvalError::Domain);
    }
    if x < 0.0 {
        let odd = y.fract() == 0.0 && (y as i64) % 2 != 0;
        if !odd {
            return Err(EvalError::Domain);
        }
        return finite(-(-x).powf(1.0 / y));
    }
    finite(x.powf(1.0 / y))
}

fn tidy(v: f64) -> f64 {
    if v.abs() < 1e-15 {
        0.0
    } else {
        v
    }
}

fn to_rad(x: f64, angle: Angle) -> f64 {
    match angle {
        Angle::Rad => x,
        Angle::Deg => (x % 360.0).to_radians(),
    }
}

fn from_rad(x: f64, angle: Angle) -> f64 {
    match angle {
        Angle::Rad => x,
        Angle::Deg => x.to_degrees(),
    }
}

fn call(name: &str, a: &[f64], angle: Angle) -> Res<f64> {
    if name == "yroot" {
        return match a {
            [x, y] => root(*x, *y),
            _ => Err(EvalError::Syntax),
        };
    }
    let [x] = *a else {
        return Err(EvalError::Syntax);
    };
    let domain = |ok: bool| if ok { Ok(()) } else { Err(EvalError::Domain) };
    let r = match name {
        "sin" => tidy(to_rad(x, angle).sin()),
        "cos" => tidy(to_rad(x, angle).cos()),
        "tan" => {
            if angle == Angle::Deg && ((x - 90.0) % 180.0) == 0.0 {
                return Err(EvalError::Domain);
            }
            tidy(to_rad(x, angle).tan())
        }
        "asin" => {
            domain(x.abs() <= 1.0)?;
            from_rad(x.asin(), angle)
        }
        "acos" => {
            domain(x.abs() <= 1.0)?;
            from_rad(x.acos(), angle)
        }
        "atan" => from_rad(x.atan(), angle),
        "sinh" => x.sinh(),
        "cosh" => x.cosh(),
        "tanh" => x.tanh(),
        "asinh" => x.asinh(),
        "acosh" => {
            domain(x >= 1.0)?;
            x.acosh()
        }
        "atanh" => {
            domain(x.abs() < 1.0)?;
            x.atanh()
        }
        "ln" => {
            domain(x > 0.0)?;
            x.ln()
        }
        "log" => {
            domain(x > 0.0)?;
            x.log10()
        }
        "log2" => {
            domain(x > 0.0)?;
            x.log2()
        }
        "sqrt" | "√" => {
            domain(x >= 0.0)?;
            x.sqrt()
        }
        "cbrt" | "∛" => x.cbrt(),
        "exp" => x.exp(),
        "exp10" => 10f64.powf(x),
        "abs" => x.abs(),
        "sq" => x * x,
        _ => return Err(EvalError::Syntax),
    };
    finite(r)
}

/// Evaluates `src`; the symbols `×`, `÷`, `−`, `π`, `√` are accepted.
pub fn evaluate(src: &str, angle: Angle) -> Result<f64, EvalError> {
    let toks = lex(src)?;
    if toks.is_empty() {
        return Err(EvalError::Syntax);
    }
    let mut p = Parser { toks, i: 0, angle, depth: 0 };
    let (v, _) = p.expr()?;
    if p.i != p.toks.len() {
        return Err(EvalError::Syntax);
    }
    finite(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(s: &str) -> Result<f64, EvalError> {
        evaluate(s, Angle::Rad)
    }

    fn deg(s: &str) -> Result<f64, EvalError> {
        evaluate(s, Angle::Deg)
    }

    fn near(s: &str, expected: f64) {
        let v = ev(s).unwrap_or_else(|e| panic!("{s}: {e:?}"));
        assert!(
            (v - expected).abs() <= 1e-9 * expected.abs().max(1.0),
            "{s} = {v}, expected {expected}"
        );
    }

    fn near_deg(s: &str, expected: f64) {
        let v = deg(s).unwrap_or_else(|e| panic!("{s}: {e:?}"));
        assert!((v - expected).abs() <= 1e-9, "{s} = {v}, expected {expected}");
    }

    #[test]
    fn precedence_and_associativity() {
        near("2+3*4", 14.0);
        near("2*3+4", 10.0);
        near("10-4-3", 3.0);
        near("100/10/5", 2.0);
        near("2^3^2", 512.0);
        near("-2^2", -4.0);
        near("2^-1", 0.5);
        near("2*-3", -6.0);
        near("0.1+0.2", 0.30000000000000004);
        near("9×3÷9−2", 1.0);
    }

    #[test]
    fn parentheses_implicit_multiplication_and_auto_close() {
        near("(2+3)*4", 20.0);
        near("2(3+4)", 14.0);
        near("(1+2)(3+4)", 21.0);
        near("2π", 2.0 * PI);
        near("3e", 3.0 * E);
        near("sin(0", 0.0);
        near("((2)+(3))", 5.0);
    }

    #[test]
    fn percent_follows_mobile_rules() {
        near("50%", 0.5);
        near("200+10%", 220.0);
        near("200-10%", 180.0);
        near("200*10%", 20.0);
        near("200/10%", 2000.0);
        near("50%%", 0.005);
        near("200+10%+5%", 231.0);
    }

    #[test]
    fn constants() {
        near("π", PI);
        near("e", E);
        near("pi*2", 2.0 * PI);
    }

    #[test]
    fn trig_in_radians() {
        near("sin(π/2)", 1.0);
        near("cos(0)", 1.0);
        near("cos(π)", -1.0);
        near("tan(π/4)", 1.0);
        near("asin(1)", PI / 2.0);
        near("acos(0)", PI / 2.0);
        near("atan(1)", PI / 4.0);
        assert_eq!(ev("sin(π)"), Ok(0.0));
    }

    #[test]
    fn trig_in_degrees() {
        near_deg("sin(30)", 0.5);
        near_deg("cos(60)", 0.5);
        near_deg("tan(45)", 1.0);
        near_deg("sin(390)", 0.5);
        near_deg("asin(1)", 90.0);
        near_deg("acos(0.5)", 60.0);
        near_deg("atan(1)", 45.0);
        assert_eq!(deg("sin(180)"), Ok(0.0));
        assert_eq!(deg("cos(90)"), Ok(0.0));
        assert_eq!(deg("tan(90)"), Err(EvalError::Domain));
        assert_eq!(deg("tan(270)"), Err(EvalError::Domain));
    }

    #[test]
    fn hyperbolic_functions_and_inverses() {
        near("sinh(1)", 1.1752011936438014);
        near("cosh(1)", 1.5430806348152437);
        near("tanh(1)", 0.7615941559557649);
        near("asinh(1)", 0.881373587019543);
        near("acosh(2)", 1.3169578969248166);
        near("atanh(0.5)", 0.5493061443340548);
        assert_eq!(ev("acosh(0.5)"), Err(EvalError::Domain));
        assert_eq!(ev("atanh(1)"), Err(EvalError::Domain));
    }

    #[test]
    fn logarithms_and_exponentials() {
        near("ln(e)", 1.0);
        near("ln(1)", 0.0);
        near("log(1000)", 3.0);
        near("log2(8)", 3.0);
        near("exp(1)", E);
        near("exp10(3)", 1000.0);
        near("2^10", 1024.0);
        assert_eq!(ev("ln(0)"), Err(EvalError::Domain));
        assert_eq!(ev("log(-1)"), Err(EvalError::Domain));
        assert_eq!(ev("log2(0)"), Err(EvalError::Domain));
    }

    #[test]
    fn powers_and_roots() {
        near("3^2", 9.0);
        near("sq(5)", 25.0);
        near("2^0.5", 2f64.sqrt());
        near("sqrt(16)", 4.0);
        near("√16", 4.0);
        near("√(9+16)", 5.0);
        near("cbrt(27)", 3.0);
        near("∛(-8)", -2.0);
        near("yroot(81,4)", 3.0);
        near("yroot(-8,3)", -2.0);
        assert_eq!(ev("sqrt(-1)"), Err(EvalError::Domain));
        assert_eq!(ev("yroot(-4,2)"), Err(EvalError::Domain));
        assert_eq!(ev("yroot(4,0)"), Err(EvalError::Domain));
        assert_eq!(ev("(-8)^0.5"), Err(EvalError::Domain));
        assert_eq!(ev("0^-1"), Err(EvalError::Domain));
        near("abs(-3)", 3.0);
    }

    #[test]
    fn factorial_postfix() {
        near("5!", 120.0);
        near("0!", 1.0);
        near("3!+1", 7.0);
        near("(2+1)!", 6.0);
        near("2^3!", 64.0);
        assert_eq!(ev("170!").map(|v| v > 7e306), Ok(true));
        assert_eq!(ev("171!"), Err(EvalError::Overflow));
        assert_eq!(ev("2.5!"), Err(EvalError::Domain));
        assert_eq!(ev("(-1)!"), Err(EvalError::Domain));
    }

    #[test]
    fn errors() {
        assert_eq!(ev("1/0"), Err(EvalError::Domain));
        assert_eq!(ev(""), Err(EvalError::Syntax));
        assert_eq!(ev("2+"), Err(EvalError::Syntax));
        assert_eq!(ev("*2"), Err(EvalError::Syntax));
        assert_eq!(ev("(2))"), Err(EvalError::Syntax));
        assert_eq!(ev("1.2.3"), Err(EvalError::Syntax));
        assert_eq!(ev("foo(2)"), Err(EvalError::Syntax));
        assert_eq!(ev("2 $ 3"), Err(EvalError::Syntax));
        assert_eq!(ev("sin()"), Err(EvalError::Syntax));
        assert_eq!(ev("10^400"), Err(EvalError::Overflow));
        assert_eq!(ev(&"(".repeat(500)), Err(EvalError::Syntax));
    }

    #[test]
    fn function_names_are_recognised() {
        assert!(is_function("sin"));
        assert!(is_function("√"));
        assert!(!is_function("e"));
        assert!(!is_function("pi"));
    }

    #[test]
    fn functions_without_parentheses_take_the_next_operand() {
        near("sin 0", 0.0);
        near("√9+1", 4.0);
        near("2√9", 6.0);
    }
}
