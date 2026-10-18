//! Parsers for math source: a LaTeX subset (`parse_tex`) and plain linear input (`parse_linear`).
//!
//! LaTeX subset: `\frac \dfrac \tfrac \sqrt[n]{x}`, `^` and `_` (with limits on `\sum \prod
//! \int`), Greek letters, common operators and relations, `\left ... \right` delimiters,
//! `\begin{matrix|pmatrix|bmatrix|vmatrix|Vmatrix|Bmatrix} ... \end{...}`, `\text{}`
//! (and `\mathrm{}`), spacing commands `\, \; \: \! \quad \qquad`, and named functions such as
//! `\sin` and `\lim`.
//!
//! Linear input: `a/b` (the numerator is the preceding atom or parenthesised group, the
//! denominator the following one), `x^2`, `x_i`, `sqrt(x)`, parentheses, and Greek letters
//! written out (`pi`, `alpha`). Whitespace is ignored.

use super::{Class, Node};

/// A parse failure with the token or character index where it was detected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    pub msg: String,
    pub at: usize,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} (at {})", self.msg, self.at)
    }
}

type PResult<T> = Result<T, ParseError>;

fn err<T>(msg: impl Into<String>, at: usize) -> PResult<T> {
    Err(ParseError { msg: msg.into(), at })
}

/// Parses LaTeX-subset math source into a tree.
pub fn parse_tex(src: &str) -> PResult<Node> {
    let mut p = Parser { toks: tokenize(src), pos: 0 };
    let items = p.seq(false)?;
    p.skip_space();
    match p.peek() {
        Tok::Eof => Ok(Node::Row(items)),
        other => err(format!("unexpected {:?}", other), p.pos),
    }
}

/// Parses linear math input (`a/b`, `x^2`, `sqrt(x)`) into a tree.
pub fn parse_linear(src: &str) -> PResult<Node> {
    let mut p = Linear { cs: src.chars().filter(|c| !c.is_whitespace()).collect(), pos: 0 };
    let items = p.seq(false)?;
    Ok(Node::Row(items))
}

/// The atom class of a bare character.
pub fn class_of(c: char) -> Class {
    match c {
        '+' | '-' | '*' | '×' | '·' | '±' | '∓' | '÷' | '−' => Class::Bin,
        '=' | '<' | '>' => Class::Rel,
        '(' | '[' => Class::Open,
        ')' | ']' | '!' | '?' => Class::Close,
        ',' | ';' => Class::Punct,
        _ => Class::Ord,
    }
}

/// Named symbols: Greek letters, operators, relations and miscellany.
const SYMBOLS: &[(&str, char, Class)] = &[
    ("alpha", 'α', Class::Ord), ("beta", 'β', Class::Ord), ("gamma", 'γ', Class::Ord),
    ("delta", 'δ', Class::Ord), ("epsilon", 'ϵ', Class::Ord), ("varepsilon", 'ε', Class::Ord),
    ("zeta", 'ζ', Class::Ord), ("eta", 'η', Class::Ord), ("theta", 'θ', Class::Ord),
    ("vartheta", 'ϑ', Class::Ord), ("iota", 'ι', Class::Ord), ("kappa", 'κ', Class::Ord),
    ("lambda", 'λ', Class::Ord), ("mu", 'μ', Class::Ord), ("nu", 'ν', Class::Ord),
    ("xi", 'ξ', Class::Ord), ("pi", 'π', Class::Ord), ("varpi", 'ϖ', Class::Ord),
    ("rho", 'ρ', Class::Ord), ("sigma", 'σ', Class::Ord), ("tau", 'τ', Class::Ord),
    ("upsilon", 'υ', Class::Ord), ("phi", 'ϕ', Class::Ord), ("varphi", 'φ', Class::Ord),
    ("chi", 'χ', Class::Ord), ("psi", 'ψ', Class::Ord), ("omega", 'ω', Class::Ord),
    ("Gamma", 'Γ', Class::Ord), ("Delta", 'Δ', Class::Ord), ("Theta", 'Θ', Class::Ord),
    ("Lambda", 'Λ', Class::Ord), ("Xi", 'Ξ', Class::Ord), ("Pi", 'Π', Class::Ord),
    ("Sigma", 'Σ', Class::Ord), ("Upsilon", 'Υ', Class::Ord), ("Phi", 'Φ', Class::Ord),
    ("Psi", 'Ψ', Class::Ord), ("Omega", 'Ω', Class::Ord),
    ("times", '×', Class::Bin), ("div", '÷', Class::Bin), ("cdot", '·', Class::Bin),
    ("pm", '±', Class::Bin), ("mp", '∓', Class::Bin), ("cup", '∪', Class::Bin),
    ("cap", '∩', Class::Bin), ("circ", '∘', Class::Bin), ("oplus", '⊕', Class::Bin),
    ("otimes", '⊗', Class::Bin), ("wedge", '∧', Class::Bin), ("vee", '∨', Class::Bin),
    ("setminus", '∖', Class::Bin), ("ast", '∗', Class::Bin),
    ("leq", '≤', Class::Rel), ("le", '≤', Class::Rel), ("geq", '≥', Class::Rel),
    ("ge", '≥', Class::Rel), ("neq", '≠', Class::Rel), ("ne", '≠', Class::Rel),
    ("approx", '≈', Class::Rel), ("equiv", '≡', Class::Rel), ("sim", '∼', Class::Rel),
    ("simeq", '≃', Class::Rel), ("cong", '≅', Class::Rel), ("propto", '∝', Class::Rel),
    ("in", '∈', Class::Rel), ("notin", '∉', Class::Rel), ("subset", '⊂', Class::Rel),
    ("subseteq", '⊆', Class::Rel), ("supset", '⊃', Class::Rel), ("supseteq", '⊇', Class::Rel),
    ("to", '→', Class::Rel), ("rightarrow", '→', Class::Rel), ("leftarrow", '←', Class::Rel),
    ("gets", '←', Class::Rel), ("Rightarrow", '⇒', Class::Rel), ("Leftarrow", '⇐', Class::Rel),
    ("leftrightarrow", '↔', Class::Rel), ("Leftrightarrow", '⇔', Class::Rel),
    ("iff", '⇔', Class::Rel), ("mapsto", '↦', Class::Rel), ("ll", '≪', Class::Rel),
    ("gg", '≫', Class::Rel), ("perp", '⊥', Class::Rel), ("parallel", '∥', Class::Rel),
    ("infty", '∞', Class::Ord), ("partial", '∂', Class::Ord), ("nabla", '∇', Class::Ord),
    ("forall", '∀', Class::Ord), ("exists", '∃', Class::Ord), ("emptyset", '∅', Class::Ord),
    ("aleph", 'ℵ', Class::Ord), ("hbar", 'ℏ', Class::Ord), ("ell", 'ℓ', Class::Ord),
    ("Re", 'ℜ', Class::Ord), ("Im", 'ℑ', Class::Ord), ("prime", '′', Class::Ord),
    ("neg", '¬', Class::Ord), ("vdots", '⋮', Class::Ord), ("ddots", '⋱', Class::Ord),
    ("cdots", '⋯', Class::Inner), ("ldots", '…', Class::Inner), ("dots", '…', Class::Inner),
    ("vert", '|', Class::Ord), ("Vert", '‖', Class::Ord), ("|", '‖', Class::Ord),
];

/// Function names set upright, as operators.
const FUNCTIONS: &[&str] = &[
    "sin", "cos", "tan", "cot", "sec", "csc", "arcsin", "arccos", "arctan", "sinh", "cosh",
    "tanh", "log", "ln", "exp", "lim", "max", "min", "sup", "inf", "det", "gcd", "arg", "dim",
    "ker", "deg", "Pr",
];

fn symbol(name: &str) -> Option<(char, Class)> {
    SYMBOLS.iter().find(|e| e.0 == name).map(|e| (e.1, e.2))
}

/// A Greek letter by name (`pi`, `alpha`, `Gamma`), used by linear input.
fn greek(name: &str) -> Option<char> {
    symbol(name).and_then(|(ch, _)| if ('\u{370}'..='\u{3ff}').contains(&ch) { Some(ch) } else { None })
}

/// The character behind a delimiter command (`\langle`, `\{`, ...).
fn delim_char(name: &str) -> Option<char> {
    Some(match name {
        "{" | "lbrace" => '{',
        "}" | "rbrace" => '}',
        "langle" => '⟨',
        "rangle" => '⟩',
        "lvert" | "rvert" | "vert" => '|',
        "|" | "Vert" | "lVert" | "rVert" => '‖',
        "lfloor" => '⌊',
        "rfloor" => '⌋',
        "lceil" => '⌈',
        "rceil" => '⌉',
        _ => return None,
    })
}

/// Attaches scripts to a base. A big operator takes them as limits-or-not; any other base
/// gets a `Scripts` wrapper. A second superscript or subscript is an error.
fn attach(base: Node, sub: Option<Node>, sup: Option<Node>, at: usize) -> PResult<Node> {
    match base {
        Node::BigOp { ch, limits, sub: s0, sup: p0 } => {
            if (sub.is_some() && s0.is_some()) || (sup.is_some() && p0.is_some()) {
                return err("double script", at);
            }
            Ok(Node::BigOp { ch, limits, sub: sub.map(Box::new).or(s0), sup: sup.map(Box::new).or(p0) })
        }
        Node::Scripts { base: inner, sub: s0, sup: p0 } => {
            if (sub.is_some() && s0.is_some()) || (sup.is_some() && p0.is_some()) {
                return err("double script", at);
            }
            Ok(Node::Scripts { base: inner, sub: sub.map(Box::new).or(s0), sup: sup.map(Box::new).or(p0) })
        }
        other => Ok(Node::Scripts { base: Box::new(other), sub: sub.map(Box::new), sup: sup.map(Box::new) }),
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Cmd(String),
    Ch(char),
    LBrace,
    RBrace,
    Sup,
    Sub,
    Amp,
    NewLine,
    Space,
    Eof,
}

fn tokenize(src: &str) -> Vec<Tok> {
    let cs: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        i += 1;
        match c {
            '\\' => {
                if i >= cs.len() {
                    out.push(Tok::Ch('\\'));
                    break;
                }
                let d = cs[i];
                if d.is_ascii_alphabetic() {
                    let start = i;
                    while i < cs.len() && cs[i].is_ascii_alphabetic() {
                        i += 1;
                    }
                    out.push(Tok::Cmd(cs[start..i].iter().collect()));
                } else if d == '\\' {
                    i += 1;
                    out.push(Tok::NewLine);
                } else {
                    i += 1;
                    out.push(Tok::Cmd(d.to_string()));
                }
            }
            '{' => out.push(Tok::LBrace),
            '}' => out.push(Tok::RBrace),
            '^' => out.push(Tok::Sup),
            '_' => out.push(Tok::Sub),
            '&' => out.push(Tok::Amp),
            '%' => {
                while i < cs.len() && cs[i] != '\n' {
                    i += 1;
                }
            }
            c if c.is_whitespace() => {
                if out.last() != Some(&Tok::Space) {
                    out.push(Tok::Space);
                }
            }
            c => out.push(Tok::Ch(c)),
        }
    }
    out.push(Tok::Eof);
    out
}

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> &Tok {
        let i = self.pos.min(self.toks.len() - 1);
        &self.toks[i]
    }

    fn bump(&mut self) -> Tok {
        let t = self.peek().clone();
        if self.pos < self.toks.len() - 1 {
            self.pos += 1;
        }
        t
    }

    fn skip_space(&mut self) {
        while *self.peek() == Tok::Space {
            self.bump();
        }
    }

    fn expect(&mut self, want: Tok, what: &str) -> PResult<()> {
        self.skip_space();
        if *self.peek() == want {
            self.bump();
            Ok(())
        } else {
            err(format!("expected {}", what), self.pos)
        }
    }

    /// Atoms with their scripts, up to a closing token: `}`, `&`, `\\`, `\right`, `\end`,
    /// end of input, or `]` when `stop_bracket` is set (for the optional `\sqrt[n]`).
    fn seq(&mut self, stop_bracket: bool) -> PResult<Vec<Node>> {
        let mut out = Vec::new();
        loop {
            self.skip_space();
            let stop = match self.peek() {
                Tok::RBrace | Tok::Eof | Tok::Amp | Tok::NewLine => true,
                Tok::Cmd(c) => c.as_str() == "right" || c.as_str() == "end",
                Tok::Ch(']') => stop_bracket,
                _ => false,
            };
            if stop {
                return Ok(out);
            }
            let at = self.pos;
            let base = self.atom()?;
            let mut sub = None;
            let mut sup = None;
            loop {
                self.skip_space();
                match self.peek().clone() {
                    Tok::Sup => {
                        self.bump();
                        if sup.is_some() {
                            return err("double superscript", self.pos);
                        }
                        sup = Some(self.arg()?);
                    }
                    Tok::Sub => {
                        self.bump();
                        if sub.is_some() {
                            return err("double subscript", self.pos);
                        }
                        sub = Some(self.arg()?);
                    }
                    _ => break,
                }
            }
            if sub.is_some() || sup.is_some() {
                out.push(attach(base, sub, sup, at)?);
            } else {
                out.push(base);
            }
        }
    }

    /// A braced group or a single atom, as a command or script argument.
    fn arg(&mut self) -> PResult<Node> {
        self.skip_space();
        if *self.peek() == Tok::LBrace {
            self.bump();
            let items = self.seq(false)?;
            self.expect(Tok::RBrace, "}")?;
            return Ok(Node::Row(items));
        }
        match self.peek().clone() {
            Tok::Eof | Tok::Sup | Tok::Sub | Tok::RBrace => err("missing argument", self.pos),
            _ => self.atom(),
        }
    }

    fn atom(&mut self) -> PResult<Node> {
        let at = self.pos;
        match self.bump() {
            Tok::LBrace => {
                let items = self.seq(false)?;
                self.expect(Tok::RBrace, "}")?;
                Ok(Node::Row(items))
            }
            Tok::Ch(c) => Ok(Node::Sym { ch: c, class: class_of(c) }),
            Tok::Cmd(name) => self.command(&name, at),
            other => err(format!("unexpected {:?}", other), at),
        }
    }

    fn command(&mut self, name: &str, at: usize) -> PResult<Node> {
        match name {
            "frac" | "dfrac" | "tfrac" => {
                let num = self.arg()?;
                let den = self.arg()?;
                Ok(Node::Frac { num: Box::new(num), den: Box::new(den) })
            }
            "sqrt" => {
                self.skip_space();
                let index = if *self.peek() == Tok::Ch('[') {
                    self.bump();
                    let items = self.seq(true)?;
                    self.expect(Tok::Ch(']'), "]")?;
                    Some(Box::new(Node::Row(items)))
                } else {
                    None
                };
                let body = self.arg()?;
                Ok(Node::Radical { index, body: Box::new(body) })
            }
            "sum" | "prod" | "int" | "iint" | "oint" | "bigcup" | "bigcap" => {
                let (ch, mut limits) = match name {
                    "sum" => ('∑', true),
                    "prod" => ('∏', true),
                    "bigcup" => ('⋃', true),
                    "bigcap" => ('⋂', true),
                    "oint" => ('∮', false),
                    _ => ('∫', false),
                };
                self.skip_space();
                if let Tok::Cmd(m) = self.peek().clone() {
                    if m.as_str() == "limits" {
                        self.bump();
                        limits = true;
                    } else if m.as_str() == "nolimits" {
                        self.bump();
                        limits = false;
                    }
                }
                Ok(Node::BigOp { ch, limits, sub: None, sup: None })
            }
            "left" => {
                let open = self.delim(at)?;
                let body = self.seq(false)?;
                self.skip_space();
                match self.bump() {
                    Tok::Cmd(r) if r.as_str() == "right" => {}
                    _ => return err("\\left without \\right", at),
                }
                let close = self.delim(at)?;
                Ok(Node::Fenced { open, close, body: Box::new(Node::Row(body)) })
            }
            "begin" => self.matrix(at),
            "text" | "textrm" | "textit" | "textbf" | "mathrm" | "mathbf" | "mathit" | "mathtt"
            | "operatorname" => {
                self.skip_space();
                if self.bump() != Tok::LBrace {
                    return err("expected { after text command", at);
                }
                let mut s = String::new();
                let mut depth = 0usize;
                loop {
                    match self.bump() {
                        Tok::RBrace if depth == 0 => break,
                        Tok::RBrace => depth -= 1,
                        Tok::LBrace => depth += 1,
                        Tok::Ch(c) => s.push(c),
                        Tok::Space => s.push(' '),
                        Tok::Eof => return err("unclosed text group", at),
                        _ => {}
                    }
                }
                Ok(Node::Text(s))
            }
            "," => Ok(Node::Space(3.0 / 18.0)),
            ";" => Ok(Node::Space(5.0 / 18.0)),
            ":" => Ok(Node::Space(4.0 / 18.0)),
            "!" => Ok(Node::Space(-3.0 / 18.0)),
            " " => Ok(Node::Space(0.25)),
            "quad" => Ok(Node::Space(1.0)),
            "qquad" => Ok(Node::Space(2.0)),
            "limits" | "nolimits" => err("\\limits applies only to large operators", at),
            _ => {
                if let Some((ch, class)) = symbol(name) {
                    return Ok(Node::Sym { ch, class });
                }
                if FUNCTIONS.contains(&name) {
                    return Ok(Node::Text(name.to_string()));
                }
                let mut chars = name.chars();
                if let (Some(c), None) = (chars.next(), chars.next()) {
                    return Ok(Node::Sym { ch: c, class: class_of(c) });
                }
                err(format!("unknown command \\{}", name), at)
            }
        }
    }

    /// The delimiter after `\left` or `\right`; `.` is the invisible delimiter (`None`).
    fn delim(&mut self, at: usize) -> PResult<Option<char>> {
        self.skip_space();
        match self.bump() {
            Tok::Ch('.') => Ok(None),
            Tok::Ch(c) if "()[]|/<>".contains(c) => Ok(Some(c)),
            Tok::Cmd(name) => match delim_char(&name) {
                Some(c) => Ok(Some(c)),
                None => err(format!("unknown delimiter \\{}", name), at),
            },
            other => err(format!("bad delimiter {:?}", other), at),
        }
    }

    /// `\begin{env} cells \end{env}`, rows split by `\\`, cells by `&`.
    fn matrix(&mut self, at: usize) -> PResult<Node> {
        let env = self.env_name(at)?;
        let (open, close) = match env.as_str() {
            "matrix" => (None, None),
            "pmatrix" => (Some('('), Some(')')),
            "bmatrix" => (Some('['), Some(']')),
            "vmatrix" => (Some('|'), Some('|')),
            "Vmatrix" => (Some('‖'), Some('‖')),
            "Bmatrix" => (Some('{'), Some('}')),
            _ => return err(format!("unknown environment {}", env), at),
        };
        let mut rows: Vec<Vec<Node>> = Vec::new();
        let mut row: Vec<Node> = Vec::new();
        loop {
            let cell = Node::Row(self.seq(false)?);
            let cell_empty = matches!(&cell, Node::Row(v) if v.is_empty());
            match self.bump() {
                Tok::Amp => row.push(cell),
                Tok::NewLine => {
                    row.push(cell);
                    rows.push(std::mem::take(&mut row));
                }
                Tok::Cmd(c) if c.as_str() == "end" => {
                    if !(row.is_empty() && cell_empty) {
                        row.push(cell);
                        rows.push(row);
                    }
                    break;
                }
                _ => return err("unterminated matrix", at),
            }
        }
        let end = self.env_name(at)?;
        if end != env {
            return err(format!("\\end{{{}}} does not match \\begin{{{}}}", end, env), at);
        }
        Ok(Node::Matrix { open, close, rows })
    }

    /// `{name}` after `\begin` or `\end`.
    fn env_name(&mut self, at: usize) -> PResult<String> {
        self.skip_space();
        if self.bump() != Tok::LBrace {
            return err("expected {environment name}", at);
        }
        let mut name = String::new();
        loop {
            match self.bump() {
                Tok::RBrace => return Ok(name),
                Tok::Ch(c) => name.push(c),
                _ => return err("bad environment name", at),
            }
        }
    }
}

struct Linear {
    cs: Vec<char>,
    pos: usize,
}

impl Linear {
    fn peek(&self) -> Option<char> {
        self.cs.get(self.pos).copied()
    }

    /// Atoms up to end of input, or up to `)` when inside a parenthesis group.
    fn seq(&mut self, in_paren: bool) -> PResult<Vec<Node>> {
        let mut out: Vec<Node> = Vec::new();
        loop {
            let c = match self.peek() {
                None if in_paren => return err("missing )", self.pos),
                None => return Ok(out),
                Some(c) => c,
            };
            match c {
                ')' => {
                    if in_paren {
                        self.pos += 1;
                        return Ok(out);
                    }
                    return err("unmatched )", self.pos);
                }
                '^' | '_' => {
                    let at = self.pos;
                    self.pos += 1;
                    let arg = self.script_arg()?;
                    let base = match out.pop() {
                        Some(b) => b,
                        None => return err("missing base before ^ or _", at),
                    };
                    let node = if c == '_' {
                        attach(base, Some(arg), None, at)?
                    } else {
                        attach(base, None, Some(arg), at)?
                    };
                    out.push(node);
                }
                '/' => {
                    let at = self.pos;
                    self.pos += 1;
                    let num = self.numerator(&mut out, at)?;
                    let den = self.denominator()?;
                    out.push(Node::Frac { num: Box::new(num), den: Box::new(den) });
                }
                _ => out.push(self.atom()?),
            }
        }
    }

    /// The numerator of `/`: a parenthesised group, else the run of letters and digits before it.
    fn numerator(&mut self, out: &mut Vec<Node>, at: usize) -> PResult<Node> {
        if matches!(out.last(), Some(Node::Row(_))) {
            if let Some(n) = out.pop() {
                return Ok(Node::Row(n.into_row()));
            }
        }
        let mut run = Vec::new();
        while matches!(out.last(), Some(Node::Sym { ch, .. }) if ch.is_alphanumeric()) {
            if let Some(n) = out.pop() {
                run.push(n);
            }
        }
        if run.is_empty() {
            return err("missing numerator before /", at);
        }
        run.reverse();
        Ok(Node::Row(run))
    }

    /// The denominator of `/`: a number, a parenthesised group, or one atom.
    fn denominator(&mut self) -> PResult<Node> {
        match self.peek() {
            None => err("missing denominator", self.pos),
            Some(c) if c.is_ascii_digit() => {
                let mut run = Vec::new();
                while let Some(d) = self.peek().filter(|d| d.is_ascii_digit()) {
                    self.pos += 1;
                    run.push(Node::Sym { ch: d, class: Class::Ord });
                }
                Ok(Node::Row(run))
            }
            Some(_) => Ok(Node::Row(self.atom()?.into_row())),
        }
    }

    fn script_arg(&mut self) -> PResult<Node> {
        match self.peek() {
            None => err("missing script", self.pos),
            Some('(') => {
                self.pos += 1;
                Ok(Node::Row(self.seq(true)?))
            }
            Some(_) => self.atom(),
        }
    }

    fn radicand(&mut self) -> PResult<Node> {
        match self.peek() {
            None => err("missing radicand", self.pos),
            Some('(') => {
                self.pos += 1;
                Ok(Node::Row(self.seq(true)?))
            }
            Some(_) => self.atom(),
        }
    }

    fn atom(&mut self) -> PResult<Node> {
        let at = self.pos;
        let c = match self.peek() {
            Some(c) => c,
            None => return err("unexpected end of input", at),
        };
        if c == '(' {
            self.pos += 1;
            let inner = self.seq(true)?;
            let mut row = vec![Node::Sym { ch: '(', class: Class::Open }];
            row.extend(inner);
            row.push(Node::Sym { ch: ')', class: Class::Close });
            return Ok(Node::Row(row));
        }
        if c.is_ascii_alphabetic() {
            if self.cs[self.pos..].starts_with(&['s', 'q', 'r', 't']) {
                self.pos += 4;
                let body = self.radicand()?;
                return Ok(Node::Radical { index: None, body: Box::new(body) });
            }
            let word: String = self.cs[self.pos..].iter().take_while(|x| x.is_ascii_alphabetic()).collect();
            if let Some(ch) = greek(&word) {
                self.pos += word.chars().count();
                return Ok(Node::Sym { ch, class: Class::Ord });
            }
            self.pos += 1;
            return Ok(Node::Sym { ch: c, class: Class::Ord });
        }
        self.pos += 1;
        let (ch, class) = match c {
            '*' => ('·', Class::Bin),
            '-' => ('−', Class::Bin),
            _ => (c, class_of(c)),
        };
        Ok(Node::Sym { ch, class })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sym(ch: char) -> Node {
        Node::Sym { ch, class: class_of(ch) }
    }

    fn only(n: Node) -> Node {
        match n {
            Node::Row(mut v) if v.len() == 1 => v.remove(0),
            other => panic!("expected a single-item row, got {:?}", other),
        }
    }

    #[test]
    fn tex_fraction_has_row_arguments() {
        let n = only(parse_tex(r"\frac{a}{b}").unwrap());
        assert_eq!(
            n,
            Node::Frac { num: Box::new(Node::Row(vec![sym('a')])), den: Box::new(Node::Row(vec![sym('b')])) }
        );
    }

    #[test]
    fn tex_superscript_on_a_symbol() {
        let n = only(parse_tex("x^2").unwrap());
        assert_eq!(n, Node::Scripts { base: Box::new(sym('x')), sub: None, sup: Some(Box::new(sym('2'))) });
    }

    #[test]
    fn tex_sum_takes_limits_then_body() {
        let items = match parse_tex(r"\sum_{i=1}^{n} i").unwrap() {
            Node::Row(v) => v,
            other => panic!("{:?}", other),
        };
        assert_eq!(items.len(), 2);
        match &items[0] {
            Node::BigOp { ch, limits, sub, sup } => {
                assert_eq!(*ch, '∑');
                assert!(*limits);
                assert_eq!(sub.as_deref(), Some(&Node::Row(vec![sym('i'), sym('='), sym('1')])));
                assert_eq!(sup.as_deref(), Some(&Node::Row(vec![sym('n')])));
            }
            other => panic!("{:?}", other),
        }
        assert_eq!(items[1], sym('i'));
    }

    #[test]
    fn tex_integral_defaults_to_nolimits_and_honours_override() {
        let plain = only(parse_tex(r"\int^1_0").unwrap());
        assert!(matches!(plain, Node::BigOp { limits: false, .. }));
        let forced = only(parse_tex(r"\int\limits^1_0").unwrap());
        assert!(matches!(forced, Node::BigOp { limits: true, .. }));
    }

    #[test]
    fn tex_left_right_fence() {
        let n = only(parse_tex(r"\left( \frac12 \right)").unwrap());
        match n {
            Node::Fenced { open, close, body } => {
                assert_eq!(open, Some('('));
                assert_eq!(close, Some(')'));
                assert!(matches!(*body, Node::Row(ref v) if matches!(v[0], Node::Frac { .. })));
            }
            other => panic!("{:?}", other),
        }
    }

    #[test]
    fn tex_invisible_right_delimiter() {
        let n = only(parse_tex(r"\left[ x \right.").unwrap());
        assert!(matches!(n, Node::Fenced { open: Some('['), close: None, .. }));
    }

    #[test]
    fn tex_root_with_index() {
        let n = only(parse_tex(r"\sqrt[3]{x}").unwrap());
        assert_eq!(
            n,
            Node::Radical { index: Some(Box::new(Node::Row(vec![sym('3')]))), body: Box::new(Node::Row(vec![sym('x')])) }
        );
    }

    #[test]
    fn tex_matrix_rows_and_cells() {
        let src = r"\begin{pmatrix} a & b \\ c & d \end{pmatrix}";
        match only(parse_tex(src).unwrap()) {
            Node::Matrix { open, close, rows } => {
                assert_eq!(open, Some('('));
                assert_eq!(close, Some(')'));
                assert_eq!(rows.len(), 2);
                assert_eq!(rows[0], vec![Node::Row(vec![sym('a')]), Node::Row(vec![sym('b')])]);
                assert_eq!(rows[1].len(), 2);
            }
            other => panic!("{:?}", other),
        }
    }

    #[test]
    fn tex_matrix_trailing_row_separator_adds_no_row() {
        match only(parse_tex(r"\begin{matrix} 1 & 2 \\ 3 & 4 \\ \end{matrix}").unwrap()) {
            Node::Matrix { rows, .. } => assert_eq!(rows.len(), 2),
            other => panic!("{:?}", other),
        }
    }

    #[test]
    fn tex_text_keeps_spaces() {
        assert_eq!(only(parse_tex(r"\text{if x}").unwrap()), Node::Text("if x".into()));
    }

    #[test]
    fn tex_greek_and_operators_map_to_unicode() {
        assert_eq!(only(parse_tex(r"\pi").unwrap()), Node::Sym { ch: 'π', class: Class::Ord });
        assert_eq!(only(parse_tex(r"\leq").unwrap()), Node::Sym { ch: '≤', class: Class::Rel });
        assert_eq!(only(parse_tex(r"\times").unwrap()), Node::Sym { ch: '×', class: Class::Bin });
    }

    #[test]
    fn tex_spacing_commands_are_em_amounts() {
        assert_eq!(only(parse_tex(r"\quad").unwrap()), Node::Space(1.0));
        assert_eq!(only(parse_tex(r"\,").unwrap()), Node::Space(3.0 / 18.0));
    }

    #[test]
    fn tex_errors_are_reported() {
        assert!(parse_tex(r"\frac{a}").is_err());
        assert!(parse_tex(r"\nosuchthing").is_err());
        assert!(parse_tex("{a").is_err());
        assert!(parse_tex(r"\left( x").is_err());
        assert!(parse_tex("x^").is_err());
        assert!(parse_tex(r"\begin{pmatrix} a \end{bmatrix}").is_err());
        assert!(parse_tex("x^1^2").is_err());
    }

    #[test]
    fn linear_fraction_takes_neighbours() {
        let n = only(parse_linear("a/b").unwrap());
        assert_eq!(
            n,
            Node::Frac { num: Box::new(Node::Row(vec![sym('a')])), den: Box::new(Node::Row(vec![sym('b')])) }
        );
    }

    #[test]
    fn linear_fraction_numerator_run_and_group() {
        match only(parse_linear("12/5").unwrap()) {
            Node::Frac { num, den } => {
                assert_eq!(*num, Node::Row(vec![sym('1'), sym('2')]));
                assert_eq!(*den, Node::Row(vec![sym('5')]));
            }
            other => panic!("{:?}", other),
        }
        match only(parse_linear("(a+b)/2").unwrap()) {
            Node::Frac { num, .. } => assert!(matches!(*num, Node::Row(ref v) if v.len() == 5)),
            other => panic!("{:?}", other),
        }
    }

    #[test]
    fn linear_power_and_sqrt() {
        assert_eq!(
            only(parse_linear("x^2").unwrap()),
            Node::Scripts { base: Box::new(sym('x')), sub: None, sup: Some(Box::new(sym('2'))) }
        );
        assert_eq!(
            only(parse_linear("sqrt(x)").unwrap()),
            Node::Radical { index: None, body: Box::new(Node::Row(vec![sym('x')])) }
        );
    }

    #[test]
    fn linear_greek_word_is_one_symbol() {
        assert_eq!(only(parse_linear("pi").unwrap()), Node::Sym { ch: 'π', class: Class::Ord });
        // A word that is not a Greek name is read letter by letter.
        match parse_linear("ab").unwrap() {
            Node::Row(v) => assert_eq!(v.len(), 2),
            other => panic!("{:?}", other),
        }
    }

    #[test]
    fn linear_errors_are_reported() {
        assert!(parse_linear("/b").is_err());
        assert!(parse_linear("a/").is_err());
        assert!(parse_linear("(a").is_err());
        assert!(parse_linear("a)").is_err());
        assert!(parse_linear("^2").is_err());
    }
}
