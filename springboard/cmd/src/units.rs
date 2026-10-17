//! `units`: converts a quantity from one unit of measure to another, or runs the interactive two-prompt mode. Quantities
//! carry their value in base units (metre, kilogram, second, kelvin, bit) and a vector of base dimension exponents, so
//! a conversion is refused with a "conformability error" when the dimensions differ.
//!
//! The built-in database is the table `BUILTIN` (a name and a definition written in the same expression language as
//! the user's input, such as `inch = 2.54 cm`). Prefixes (`PREFIXES`) apply to any defined unit, and the temperature
//! scales `tempC`, `tempF` and `tempK` are functions of one number. `-f file` adds `name definition` lines on top of the
//! built-in table. The definitions are public SI and US customary values, written here from those published values.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::io::{self, BufRead, Write};

pub const USAGE: &str = "usage: units [-f file] [-qtvUV] [from-unit to-unit]";
pub const VERSION: &str = concat!("units (BifrOSt) ", env!("CARGO_PKG_VERSION"));
/// What `-U` prints when no `-f` file is given.
pub const BUILTIN_PATH: &str = "builtin";

/// Names of the base dimensions, in the order of the exponent vector.
pub const BASE_NAMES: [&str; 5] = ["length", "mass", "time", "temperature", "information"];

/// Exponents of the base dimensions: length, mass, time, temperature, information.
pub type Dims = [i32; 5];
pub const NO_DIMS: Dims = [0; 5];
const TEMPERATURE: Dims = [0, 0, 0, 1, 0];

/// Base units, each with its dimension index. Their value is 1 in the base unit of their dimension.
const BASE_UNITS: &[(&str, usize)] = &[("m", 0), ("kg", 1), ("s", 2), ("K", 3), ("bit", 4)];

/// SI and binary prefixes. Long and short forms; the longest prefix that leaves a defined unit wins.
const PREFIXES: &[(&str, f64)] = &[
    ("yotta", 1e24),
    ("zetta", 1e21),
    ("exa", 1e18),
    ("peta", 1e15),
    ("tera", 1e12),
    ("giga", 1e9),
    ("mega", 1e6),
    ("kilo", 1e3),
    ("hecto", 1e2),
    ("deka", 1e1),
    ("deca", 1e1),
    ("deci", 1e-1),
    ("centi", 1e-2),
    ("milli", 1e-3),
    ("micro", 1e-6),
    ("nano", 1e-9),
    ("pico", 1e-12),
    ("femto", 1e-15),
    ("atto", 1e-18),
    ("zepto", 1e-21),
    ("yocto", 1e-24),
    ("kibi", 1024.0),
    ("mebi", 1048576.0),
    ("gibi", 1073741824.0),
    ("tebi", 1099511627776.0),
    ("pebi", 1125899906842624.0),
    ("exbi", 1152921504606846976.0),
    ("zebi", 1.180591620717411303424e21),
    ("yobi", 1.208925819614629174706176e24),
    ("Y", 1e24),
    ("Z", 1e21),
    ("E", 1e18),
    ("P", 1e15),
    ("T", 1e12),
    ("G", 1e9),
    ("M", 1e6),
    ("k", 1e3),
    ("h", 1e2),
    ("da", 1e1),
    ("d", 1e-1),
    ("c", 1e-2),
    ("m", 1e-3),
    ("u", 1e-6),
    ("n", 1e-9),
    ("p", 1e-12),
    ("f", 1e-15),
    ("a", 1e-18),
    ("z", 1e-21),
    ("y", 1e-24),
];

/// Built-in unit definitions: `(name, definition)`. A definition is an expression in the same language as the input.
pub const BUILTIN: &[(&str, &str)] = &[
    // length
    ("inch", "2.54 cm"),
    ("in", "inch"),
    ("foot", "12 inch"),
    ("ft", "foot"),
    ("yard", "3 foot"),
    ("yd", "yard"),
    ("mile", "5280 foot"),
    ("mi", "mile"),
    ("nmi", "1852 m"),
    ("mil", "0.001 inch"),
    ("furlong", "220 yard"),
    ("fathom", "6 foot"),
    ("rod", "16.5 foot"),
    ("league", "3 mile"),
    ("point", "1/72 inch"),
    ("angstrom", "1e-10 m"),
    ("au", "149597870700 m"),
    ("ly", "9.4607304725808e15 m"),
    ("parsec", "3.0856775814913673e16 m"),
    // mass
    ("gram", "0.001 kg"),
    ("g", "gram"),
    ("pound", "0.45359237 kg"),
    ("lb", "pound"),
    ("ounce", "1/16 pound"),
    ("oz", "ounce"),
    ("ton", "2000 pound"),
    ("tonne", "1000 kg"),
    ("grain", "1/7000 pound"),
    ("stone", "14 pound"),
    ("carat", "0.2 g"),
    // time
    ("sec", "s"),
    ("minute", "60 s"),
    ("min", "minute"),
    ("hour", "60 min"),
    ("hr", "hour"),
    ("h", "hour"),
    ("day", "24 hour"),
    ("week", "7 day"),
    ("year", "365.25 day"),
    ("yr", "year"),
    ("hertz", "1/s"),
    ("Hz", "hertz"),
    // volume
    ("liter", "0.001 m^3"),
    ("l", "liter"),
    ("L", "liter"),
    ("gallon", "231 inch^3"),
    ("gal", "gallon"),
    ("quart", "1/4 gallon"),
    ("qt", "quart"),
    ("pint", "1/2 quart"),
    ("cup", "1/2 pint"),
    ("fluidounce", "1/16 pint"),
    ("floz", "fluidounce"),
    ("teaspoon", "5 ml"),
    ("tsp", "teaspoon"),
    ("tablespoon", "15 ml"),
    ("tbsp", "tablespoon"),
    // area
    ("are", "100 m^2"),
    ("hectare", "100 are"),
    ("ha", "hectare"),
    ("acre", "43560 ft^2"),
    ("barn", "1e-28 m^2"),
    // speed
    ("knot", "nmi/hour"),
    ("kt", "knot"),
    ("mph", "mile/hour"),
    ("kph", "km/hour"),
    // force and pressure
    ("newton", "kg m / s^2"),
    ("N", "newton"),
    ("lbf", "lb 9.80665 m / s^2"),
    ("psi", "lbf/inch^2"),
    ("pascal", "N/m^2"),
    ("Pa", "pascal"),
    ("bar", "100000 Pa"),
    ("atm", "101325 Pa"),
    ("torr", "atm/760"),
    // energy and power
    ("joule", "N m"),
    ("J", "joule"),
    ("calorie", "4.184 J"),
    ("cal", "calorie"),
    ("btu", "1055.06 J"),
    ("eV", "1.602176634e-19 J"),
    ("kWh", "3.6e6 J"),
    ("watt", "J/s"),
    ("W", "watt"),
    ("horsepower", "745.7 W"),
    ("hp", "horsepower"),
    // data
    ("byte", "8 bit"),
    // numbers
    ("percent", "0.01"),
    ("dozen", "12"),
    ("gross", "144"),
    ("pi", "3.14159265358979323846"),
];

/// A temperature scale, written as a function of one number: `tempF(212)`.
#[derive(Clone, Copy, Debug)]
pub struct Scale {
    pub name: &'static str,
    pub to_kelvin: fn(f64) -> f64,
    pub from_kelvin: fn(f64) -> f64,
}

fn celsius_to_kelvin(x: f64) -> f64 {
    x + 273.15
}

fn kelvin_to_celsius(k: f64) -> f64 {
    k - 273.15
}

fn fahrenheit_to_kelvin(x: f64) -> f64 {
    (x - 32.0) * 5.0 / 9.0 + 273.15
}

fn kelvin_to_fahrenheit(k: f64) -> f64 {
    (k - 273.15) * 9.0 / 5.0 + 32.0
}

fn identity(x: f64) -> f64 {
    x
}

pub const SCALES: &[Scale] = &[
    Scale { name: "tempC", to_kelvin: celsius_to_kelvin, from_kelvin: kelvin_to_celsius },
    Scale { name: "tempF", to_kelvin: fahrenheit_to_kelvin, from_kelvin: kelvin_to_fahrenheit },
    Scale { name: "tempK", to_kelvin: identity, from_kelvin: identity },
];

fn find_scale(name: &str) -> Option<&'static Scale> {
    SCALES.iter().find(|s| s.name == name)
}

/// A value in base units with its dimensions.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quantity {
    pub value: f64,
    pub dims: Dims,
}

impl Quantity {
    fn number(value: f64) -> Quantity {
        Quantity { value, dims: NO_DIMS }
    }

    fn times(self, o: Quantity) -> Quantity {
        Quantity { value: self.value * o.value, dims: add_dims(self.dims, o.dims, 1) }
    }

    fn per(self, o: Quantity) -> Quantity {
        Quantity { value: self.value / o.value, dims: add_dims(self.dims, o.dims, -1) }
    }

    fn pow(self, n: i32) -> Quantity {
        Quantity { value: self.value.powi(n), dims: self.dims.map(|d| d * n) }
    }
}

fn add_dims(a: Dims, b: Dims, sign: i32) -> Dims {
    let mut r = a;
    for (x, y) in r.iter_mut().zip(b.iter()) {
        *x += sign * *y;
    }
    r
}

/// The result of evaluating an expression: a quantity, or a bare temperature scale (only at the top level).
#[derive(Clone, Copy, Debug)]
pub enum Value {
    Quantity(Quantity),
    Scale(&'static Scale),
}

/// The unit database: definitions by name. `builtin()` holds the table; `add_definitions` adds a file on top.
#[derive(Clone, Debug, Default)]
pub struct Database {
    defs: BTreeMap<String, String>,
}

/// How deep definitions may nest before the lookup gives up (a loop in a user file).
const MAX_DEPTH: usize = 32;

impl Database {
    pub fn builtin() -> Database {
        let mut db = Database::default();
        for &(name, def) in BUILTIN {
            db.defs.insert(name.to_string(), def.to_string());
        }
        db
    }

    /// Adds `name definition` lines over the current definitions. Blank lines, `#` comments and `!` directives are
    /// skipped; a name with no definition is an error.
    pub fn add_definitions(&mut self, text: &str) -> Result<(), String> {
        for (n, raw) in text.lines().enumerate() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() || line.starts_with('!') {
                continue;
            }
            let mut parts = line.splitn(2, char::is_whitespace);
            let name = parts.next().unwrap_or("");
            let def = parts.next().unwrap_or("").trim();
            if def.is_empty() {
                return Err(format!("line {}: no definition for '{name}'", n + 1));
            }
            self.defs.insert(name.to_string(), def.to_string());
        }
        Ok(())
    }

    /// Evaluates an expression such as `2.5 km`, `kg m / s^2`, `mile per hour` or `tempF(212)`.
    pub fn evaluate(&self, text: &str) -> Result<Value, String> {
        self.evaluate_at(text, 0)
    }

    /// Evaluates an expression that must be a quantity (a bare temperature scale is refused).
    pub fn quantity(&self, text: &str) -> Result<Quantity, String> {
        match self.evaluate(text)? {
            Value::Quantity(q) => Ok(q),
            Value::Scale(s) => Err(format!("{}() needs a number, such as {}(20)", s.name, s.name)),
        }
    }

    fn evaluate_at(&self, text: &str, depth: usize) -> Result<Value, String> {
        let toks = tokenize(text)?;
        if toks.is_empty() {
            return Err("empty expression".to_string());
        }
        let mut p = Parser { db: self, toks, pos: 0, depth };
        let v = p.expr()?;
        if p.pos < p.toks.len() {
            return Err(format!("cannot read '{text}' past the unit it names"));
        }
        Ok(v)
    }

    /// The quantity a unit name stands for, in base units. Plurals (`miles`) and prefixes (`km`, `kibibyte`) resolve.
    fn resolve(&self, name: &str, depth: usize) -> Result<Quantity, String> {
        if depth > MAX_DEPTH {
            return Err(format!("unit '{name}' is defined in a loop"));
        }
        let mut forms = vec![name];
        if let Some(stem) = name.strip_suffix('s') {
            if !stem.is_empty() {
                forms.push(stem);
            }
        }
        for form in forms {
            if let Some(q) = self.exact(form, depth)? {
                return Ok(q);
            }
            if let Some(q) = self.prefixed(form, depth)? {
                return Ok(q);
            }
        }
        Err(format!("unknown unit '{name}'"))
    }

    fn exact(&self, name: &str, depth: usize) -> Result<Option<Quantity>, String> {
        if let Some((_, dim)) = BASE_UNITS.iter().find(|(n, _)| *n == name) {
            let mut dims = NO_DIMS;
            dims[*dim] = 1;
            return Ok(Some(Quantity { value: 1.0, dims }));
        }
        match self.defs.get(name) {
            Some(def) => match self.evaluate_at(def, depth + 1)? {
                Value::Quantity(q) => Ok(Some(q)),
                Value::Scale(s) => Err(format!("'{name}' is defined as the function {}()", s.name)),
            },
            None => Ok(None),
        }
    }

    /// The longest prefix that leaves a defined unit, applied to that unit.
    fn prefixed(&self, name: &str, depth: usize) -> Result<Option<Quantity>, String> {
        let mut best: Option<Quantity> = None;
        let mut best_len = 0;
        for &(prefix, factor) in PREFIXES {
            if prefix.len() <= best_len {
                continue;
            }
            if let Some(rest) = name.strip_prefix(prefix) {
                if rest.is_empty() {
                    continue;
                }
                if let Some(q) = self.exact(rest, depth)? {
                    best = Some(Quantity { value: q.value * factor, dims: q.dims });
                    best_len = prefix.len();
                }
            }
        }
        Ok(best)
    }
}

/// Converts `from` to the unit `to`. Returns the number of `to` units in `from`, or a conformability error.
pub fn convert(db: &Database, from: &str, to: &str) -> Result<f64, String> {
    let from_q = db.quantity(from)?;
    match db.evaluate(to)? {
        Value::Scale(scale) => {
            if from_q.dims != TEMPERATURE {
                return Err(conformability(from_q.dims, TEMPERATURE));
            }
            Ok((scale.from_kelvin)(from_q.value))
        }
        Value::Quantity(to_q) => {
            if from_q.dims != to_q.dims {
                return Err(conformability(from_q.dims, to_q.dims));
            }
            Ok(from_q.value / to_q.value)
        }
    }
}

/// The conformability error text: the two dimension lists that do not match.
pub fn conformability(a: Dims, b: Dims) -> String {
    format!("conformability error\n\t{} != {}", dims_label(a), dims_label(b))
}

/// Names a dimension vector, such as `length mass / time^2`. Dimensionless quantities read `dimensionless`.
pub fn dims_label(d: Dims) -> String {
    if d == NO_DIMS {
        return "dimensionless".to_string();
    }
    let mut num: Vec<String> = Vec::new();
    let mut den: Vec<String> = Vec::new();
    for (i, &e) in d.iter().enumerate() {
        match e.cmp(&0) {
            Ordering::Greater => num.push(power_label(i, e)),
            Ordering::Less => den.push(power_label(i, -e)),
            Ordering::Equal => {}
        }
    }
    let top = if num.is_empty() { "1".to_string() } else { num.join(" ") };
    if den.is_empty() {
        top
    } else {
        format!("{top} / {}", den.join(" "))
    }
}

fn power_label(i: usize, e: i32) -> String {
    if e == 1 {
        BASE_NAMES[i].to_string()
    } else {
        format!("{}^{}", BASE_NAMES[i], e)
    }
}

/// A number with about seven significant digits, trailing zeros removed. Large and tiny values use exponent form.
pub fn format_number(v: f64) -> String {
    if v == 0.0 {
        return "0".to_string();
    }
    if !v.is_finite() {
        return v.to_string();
    }
    let mag = v.abs().log10().floor() as i32;
    if (-5..9).contains(&mag) {
        let decimals = (6 - mag).max(0) as usize;
        trim_zeros(&format!("{:.*}", decimals, v))
    } else {
        let s = format!("{:.6e}", v);
        match s.split_once('e') {
            Some((mantissa, exp)) => format!("{}e{}", trim_zeros(mantissa), exp),
            None => s,
        }
    }
}

fn trim_zeros(s: &str) -> String {
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s.to_string()
    }
}

/// One line of the non-interactive answer: the number alone (`-t`), `from = result to` (`-v`), or a tab and the number.
pub fn one_shot(db: &Database, from: &str, to: &str, opts: &Opts) -> Result<String, String> {
    let x = convert(db, from, to)?;
    let shown = format_number(x);
    Ok(if opts.terse {
        shown
    } else if opts.verbose {
        format!("{from} = {shown} {to}")
    } else {
        format!("\t{shown}")
    })
}

/// The answer to one interactive pair of lines. An empty `want` shows the definition of `have` in base units.
pub fn answer(db: &Database, have: &str, want: &str) -> String {
    if want.trim().is_empty() {
        return match db.quantity(have) {
            Ok(q) => format!("\tdefinition: {} {}", format_number(q.value), dims_label(q.dims)),
            Err(e) => e,
        };
    }
    match convert(db, have, want) {
        Ok(x) => format!("\t* {}\n\t/ {}", format_number(x), format_number(1.0 / x)),
        Err(e) => e,
    }
}

fn read_line(input: &mut impl BufRead) -> io::Result<Option<String>> {
    let mut line = String::new();
    if input.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    Ok(Some(line.trim().to_string()))
}

/// The interactive mode: `You have:` and `You want:` prompts until an empty `have` line or the end of input.
/// `quiet` drops the prompts.
pub fn interactive(db: &Database, input: &mut impl BufRead, out: &mut impl Write, quiet: bool) -> io::Result<()> {
    loop {
        if !quiet {
            write!(out, "You have: ")?;
            out.flush()?;
        }
        let have = match read_line(input)? {
            Some(line) if !line.is_empty() => line,
            _ => break,
        };
        if !quiet {
            write!(out, "You want: ")?;
            out.flush()?;
        }
        let want = read_line(input)?.unwrap_or_default();
        writeln!(out, "{}", answer(db, &have, &want))?;
    }
    Ok(())
}

// --- command line ---

/// The options that change how the answer is printed or which database is read.
#[derive(Clone, Debug, Default)]
pub struct Opts {
    /// `-q`: no prompts in interactive mode.
    pub quiet: bool,
    /// `-t`: only the number.
    pub terse: bool,
    /// `-v`: `from = result to`.
    pub verbose: bool,
    /// `-U`: print the database path and stop.
    pub print_path: bool,
    /// `-f file`: definitions added to the built-in table.
    pub file: Option<String>,
}

/// What the command line asks for.
pub enum Request {
    Help,
    Version,
    Run(Opts, Vec<String>),
}

/// Reads the command line. `-V` and `-h` stop at once; `--` ends the options.
pub fn parse(args: &[String]) -> Result<Request, String> {
    let mut opts = Opts::default();
    let mut operands: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        i += 1;
        if arg == "--" {
            operands.extend(args[i..].iter().cloned());
            break;
        }
        let Some(flags) = arg.strip_prefix('-') else {
            operands.push(arg.to_string());
            continue;
        };
        if flags.is_empty() {
            operands.push(arg.to_string());
            continue;
        }
        for (k, c) in flags.char_indices() {
            match c {
                'q' => opts.quiet = true,
                't' => opts.terse = true,
                'v' => opts.verbose = true,
                'U' => opts.print_path = true,
                'V' => return Ok(Request::Version),
                'h' => return Ok(Request::Help),
                'f' => {
                    let rest = &flags[k + 1..];
                    let path = if rest.is_empty() {
                        let next = args.get(i).ok_or_else(|| "-f needs a file name".to_string())?;
                        i += 1;
                        next.clone()
                    } else {
                        rest.to_string()
                    };
                    opts.file = Some(path);
                    break;
                }
                other => return Err(format!("unknown option -{other}")),
            }
        }
    }
    Ok(Request::Run(opts, operands))
}

// --- expression language ---

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Num(f64),
    /// A unit or function name, with the integer exponent written directly after it (`m2` is `m` with 2).
    Name(String, i32),
    Op(char),
}

fn tokenize(text: &str) -> Result<Vec<Tok>, String> {
    let chars: Vec<char> = text.chars().collect();
    let mut toks = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c.is_ascii_digit() || (c == '.' && chars.get(i + 1).is_some_and(|d| d.is_ascii_digit())) {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            if i < chars.len() && (chars[i] == 'e' || chars[i] == 'E') {
                let mut j = i + 1;
                if j < chars.len() && (chars[j] == '+' || chars[j] == '-') {
                    j += 1;
                }
                if j < chars.len() && chars[j].is_ascii_digit() {
                    while j < chars.len() && chars[j].is_ascii_digit() {
                        j += 1;
                    }
                    i = j;
                }
            }
            let digits: String = chars[start..i].iter().collect();
            let value = digits.parse::<f64>().map_err(|_| format!("bad number '{digits}'"))?;
            toks.push(Tok::Num(value));
        } else if c.is_ascii_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_alphabetic() || chars[i] == '_') {
                i += 1;
            }
            let name: String = chars[start..i].iter().collect();
            let digits_start = i;
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            let exp = if i > digits_start {
                let digits: String = chars[digits_start..i].iter().collect();
                digits.parse::<i32>().map_err(|_| format!("bad exponent '{digits}'"))?
            } else {
                1
            };
            if name == "per" {
                toks.push(Tok::Op('/'));
            } else {
                toks.push(Tok::Name(name, exp));
            }
        } else if "*/^()-".contains(c) {
            toks.push(Tok::Op(c));
            i += 1;
        } else {
            return Err(format!("unexpected character '{c}'"));
        }
    }
    Ok(toks)
}

#[derive(Clone, Copy)]
enum Mul {
    Times,
    Per,
}

fn combine(a: Value, b: Value, op: Mul) -> Result<Value, String> {
    match (a, b) {
        (Value::Quantity(x), Value::Quantity(y)) => Ok(Value::Quantity(match op {
            Mul::Times => x.times(y),
            Mul::Per => x.per(y),
        })),
        (Value::Scale(s), _) | (_, Value::Scale(s)) => Err(scale_error(s)),
    }
}

fn scale_error(s: &Scale) -> String {
    format!("{}() needs a number in brackets, such as {}(20)", s.name, s.name)
}

fn negate(v: Value) -> Result<Value, String> {
    match v {
        Value::Quantity(q) => Ok(Value::Quantity(Quantity { value: -q.value, dims: q.dims })),
        Value::Scale(s) => Err(scale_error(s)),
    }
}

fn power(v: Value, n: i32) -> Result<Value, String> {
    match v {
        Value::Quantity(q) => Ok(Value::Quantity(q.pow(n))),
        Value::Scale(s) => Err(scale_error(s)),
    }
}

struct Parser<'a> {
    db: &'a Database,
    toks: Vec<Tok>,
    pos: usize,
    depth: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<Tok> {
        self.toks.get(self.pos).cloned()
    }

    fn advance(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.pos).cloned();
        self.pos += 1;
        t
    }

    fn expect(&mut self, c: char) -> Result<(), String> {
        match self.advance() {
            Some(Tok::Op(x)) if x == c => Ok(()),
            _ => Err(format!("expected '{c}'")),
        }
    }

    /// products and quotients, where a unit written next to another multiplies it (`1 mile`, `kg m`)
    fn expr(&mut self) -> Result<Value, String> {
        let mut acc = self.term()?;
        loop {
            match self.peek() {
                Some(Tok::Op('*')) => {
                    self.pos += 1;
                    let rhs = self.term()?;
                    acc = combine(acc, rhs, Mul::Times)?;
                }
                Some(Tok::Op('/')) => {
                    self.pos += 1;
                    let rhs = self.term()?;
                    acc = combine(acc, rhs, Mul::Per)?;
                }
                Some(Tok::Num(_)) | Some(Tok::Name(_, _)) | Some(Tok::Op('(')) => {
                    let rhs = self.term()?;
                    acc = combine(acc, rhs, Mul::Times)?;
                }
                _ => return Ok(acc),
            }
        }
    }

    /// a unary minus, then a primary with an optional `^n` (n a whole number, may be negative)
    fn term(&mut self) -> Result<Value, String> {
        if matches!(self.peek(), Some(Tok::Op('-'))) {
            self.pos += 1;
            return negate(self.term()?);
        }
        let base = self.primary()?;
        if matches!(self.peek(), Some(Tok::Op('^'))) {
            self.pos += 1;
            let neg = matches!(self.peek(), Some(Tok::Op('-')));
            if neg {
                self.pos += 1;
            }
            let n = match self.advance() {
                Some(Tok::Num(x)) if x.fract() == 0.0 && x.abs() <= 64.0 => x as i32,
                _ => return Err("a power needs a whole number, such as ^2".to_string()),
            };
            return power(base, if neg { -n } else { n });
        }
        Ok(base)
    }

    fn primary(&mut self) -> Result<Value, String> {
        match self.advance() {
            Some(Tok::Num(x)) => Ok(Value::Quantity(Quantity::number(x))),
            Some(Tok::Op('(')) => {
                let v = self.expr()?;
                self.expect(')')?;
                Ok(v)
            }
            Some(Tok::Name(name, exp)) => {
                if let Some(scale) = find_scale(&name) {
                    if exp != 1 {
                        return Err(format!("{name}() cannot be raised to a power"));
                    }
                    if matches!(self.peek(), Some(Tok::Op('('))) {
                        self.pos += 1;
                        return self.call(scale);
                    }
                    return Ok(Value::Scale(scale));
                }
                if matches!(self.peek(), Some(Tok::Op('('))) {
                    return Err(format!("unknown function '{name}'"));
                }
                let q = self.db.resolve(&name, self.depth)?;
                Ok(Value::Quantity(q.pow(exp)))
            }
            Some(Tok::Op(c)) => Err(format!("unexpected '{c}'")),
            None => Err("missing a unit or a number".to_string()),
        }
    }

    /// The argument of a temperature function; the opening bracket is already read.
    fn call(&mut self, scale: &'static Scale) -> Result<Value, String> {
        let arg = self.expr()?;
        self.expect(')')?;
        match arg {
            Value::Quantity(q) if q.dims == NO_DIMS => Ok(Value::Quantity(Quantity {
                value: (scale.to_kelvin)(q.value),
                dims: TEMPERATURE,
            })),
            _ => Err(format!("{}() needs a plain number", scale.name)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conv(from: &str, to: &str) -> f64 {
        convert(&Database::builtin(), from, to).unwrap()
    }

    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() <= 1e-9 * b.abs().max(1.0)
    }

    #[test]
    fn miles_are_kilometres() {
        assert!(near(conv("1 mile", "km"), 1.609344));
        assert!(near(conv("1 mph", "km/hour"), 1.609344));
    }

    #[test]
    fn inches_are_centimetres() {
        assert!(near(conv("1 inch", "cm"), 2.54));
        assert!(near(conv("1 foot", "inch"), 12.0));
    }

    #[test]
    fn fahrenheit_boiling_point_is_one_hundred_celsius() {
        assert!(near(conv("tempF(212)", "tempC"), 100.0));
        assert!(near(conv("tempC(100)", "tempF"), 212.0));
    }

    #[test]
    fn celsius_zero_is_fahrenheit_thirty_two_and_kelvin_zero_is_minus_273() {
        assert!(near(conv("tempC(0)", "tempF"), 32.0));
        assert!(near(conv("tempK(0)", "tempC"), -273.15));
    }

    #[test]
    fn prefixes_and_plurals_resolve() {
        assert!(near(conv("miles", "kilometers"), 1.609344));
        assert!(near(conv("2 kilometer", "m"), 2000.0));
        assert!(near(conv("kibibyte", "byte"), 1024.0));
        assert!(near(conv("kilobyte", "byte"), 1000.0));
        assert!(near(conv("kg", "g"), 1000.0));
        assert!(near(conv("ms", "s"), 0.001));
    }

    #[test]
    fn numbers_exponents_and_powers_parse() {
        assert!(near(conv("2.5e3 m", "m"), 2500.0));
        assert!(near(conv("m2", "m^2"), 1.0));
        assert!(near(conv("m^3", "liter"), 1000.0));
        assert!(near(conv("km2", "hectare"), 100.0));
        assert!(near(conv("1/s", "hertz"), 1.0));
        assert!(near(conv("60 mile per hour", "mph"), 60.0));
    }

    #[test]
    fn mismatched_dimensions_are_a_conformability_error() {
        let db = Database::builtin();
        let err = convert(&db, "m", "s").unwrap_err();
        assert!(err.starts_with("conformability error"), "{err}");
        let err = convert(&db, "m", "tempC").unwrap_err();
        assert!(err.starts_with("conformability error"), "{err}");
        let err = convert(&db, "tempC(20)", "kg").unwrap_err();
        assert!(err.starts_with("conformability error"), "{err}");
    }

    #[test]
    fn unknown_units_and_bad_input_are_errors() {
        let db = Database::builtin();
        assert!(convert(&db, "1 frobnicator", "m").unwrap_err().contains("unknown unit"));
        assert!(db.evaluate("").is_err());
        assert!(db.evaluate("1 +").is_err());
        assert!(db.evaluate("m^1.5").is_err());
        assert!(db.evaluate("tempC(m)").is_err());
        assert!(db.quantity("tempC").is_err());
        assert!(matches!(db.evaluate("tempC"), Ok(Value::Scale(_))));
    }

    #[test]
    fn definitions_from_a_file_extend_the_table() {
        let mut db = Database::builtin();
        db.add_definitions("# a joke unit\nsmoot 67 inch\n").unwrap();
        assert!(near(convert(&db, "1 smoot", "m").unwrap(), 1.7018));
        assert!(db.add_definitions("lonely\n").is_err());
    }

    #[test]
    fn a_definition_loop_is_caught() {
        let mut db = Database::builtin();
        db.add_definitions("aa bb\nbb aa\n").unwrap();
        assert!(convert(&db, "1 aa", "m").unwrap_err().contains("loop"));
    }

    #[test]
    fn dimensions_are_named() {
        let db = Database::builtin();
        let q = db.quantity("newton").unwrap();
        assert_eq!(dims_label(q.dims), "length mass / time^2");
        assert_eq!(dims_label(NO_DIMS), "dimensionless");
    }

    #[test]
    fn numbers_print_with_seven_significant_digits() {
        assert_eq!(format_number(1.6093440000000001), "1.609344");
        assert_eq!(format_number(100.0), "100");
        assert_eq!(format_number(0.0254), "0.0254");
        assert_eq!(format_number(2.54), "2.54");
        assert_eq!(format_number(1e9), "1e9");
        assert_eq!(format_number(0.0), "0");
    }

    #[test]
    fn one_shot_answers_follow_the_options() {
        let db = Database::builtin();
        let terse = Opts { terse: true, ..Opts::default() };
        assert_eq!(one_shot(&db, "1 inch", "cm", &terse).unwrap(), "2.54");
        let plain = Opts::default();
        assert_eq!(one_shot(&db, "1 inch", "cm", &plain).unwrap(), "\t2.54");
        let verbose = Opts { verbose: true, ..Opts::default() };
        assert_eq!(one_shot(&db, "1 inch", "cm", &verbose).unwrap(), "1 inch = 2.54 cm");
    }

    #[test]
    fn parse_reads_flags_and_operands() {
        let args = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<String>>();
        match parse(&args(&["-tv", "mile", "km"])).unwrap() {
            Request::Run(o, ops) => {
                assert!(o.terse && o.verbose && !o.quiet);
                assert_eq!(ops, args(&["mile", "km"]));
            }
            _ => panic!("expected a run"),
        }
        match parse(&args(&["-f", "defs.txt", "1", "m"])).unwrap() {
            Request::Run(o, _) => assert_eq!(o.file.as_deref(), Some("defs.txt")),
            _ => panic!("expected a run"),
        }
        match parse(&args(&["-ffile.txt"])).unwrap() {
            Request::Run(o, _) => assert_eq!(o.file.as_deref(), Some("file.txt")),
            _ => panic!("expected a run"),
        }
        assert!(matches!(parse(&args(&["-V"])).unwrap(), Request::Version));
        assert!(parse(&args(&["-x"])).is_err());
        assert!(parse(&args(&["-f"])).is_err());
    }

    #[test]
    fn interactive_mode_prints_the_ratio_and_its_inverse() {
        let db = Database::builtin();
        let mut input: &[u8] = b"1 mile\nkm\n\n";
        let mut out: Vec<u8> = Vec::new();
        interactive(&db, &mut input, &mut out, false).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("You have: "));
        assert!(text.contains("You want: "));
        assert!(text.contains("\t* 1.609344\n\t/ 0.6213712"), "{text}");
    }

    #[test]
    fn quiet_interactive_mode_drops_the_prompts() {
        let db = Database::builtin();
        let mut input: &[u8] = b"1 inch\ncm\n";
        let mut out: Vec<u8> = Vec::new();
        interactive(&db, &mut input, &mut out, true).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(!text.contains("You have"));
        assert!(text.contains("* 2.54"));
    }
}
