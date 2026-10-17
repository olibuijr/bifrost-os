//! Recursive-descent parser for the scripting language. A script is handlers and statements; statements end at a
//! newline, or at the word that closes them (`end if`, `else`, `end`). Operator precedence, lowest first: `or`, `and`,
//! `not`, comparisons (`=`, `≠`, `is`, `is not`, `<`, `<=`, `>`, `>=`), `&`, `+ -`, `* / mod`, unary `-`, then a
//! primary (a literal, a variable, `prop of x`, `item n of x`, a specifier, a handler call, `do shell script`).

use super::ast::{BinOp, Expr, Handler, Program, RepeatKind, Selector, Stmt};
use super::lexer::{lex, ParseError, Tok, Token};

type R<T> = Result<T, ParseError>;

/// Words that end a command word run (`save document x with title "t"`: `save document` is the phrase).
const COMMAND_STOP: &[&str] = &["of", "with", "to", "then", "is", "and", "or", "not", "from", "in", "times", "whose", "else", "end", "mod", "as"];

/// Words that can never start a command word.
const RESERVED: &[&str] = &[
    "set", "get", "make", "delete", "display", "if", "repeat", "return", "exit", "try", "tell", "delay", "do", "end", "else", "then", "to", "of", "with", "on", "times", "from", "in", "whose", "is", "and", "or",
    "not", "mod", "as", "it", "new", "properties", "dialog", "notification", "title", "error", "true", "false", "missing", "every", "item", "value", "shell", "script", "app", "application",
];

/// Parses a script's source text.
pub fn parse(src: &str) -> R<Program> {
    let mut p = Parser { toks: lex(src)?, pos: 0 };
    let mut handlers = Vec::new();
    let mut body = Vec::new();
    loop {
        p.skip_newlines();
        if matches!(p.tok(), Tok::Eof) {
            break;
        }
        if p.is_word("on") {
            handlers.push(p.handler()?);
        } else {
            body.push(p.statement()?);
        }
    }
    Ok(Program { handlers, body })
}

struct Parser {
    toks: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn tok(&self) -> &Tok {
        &self.toks[self.pos.min(self.toks.len() - 1)].tok
    }

    fn tok_at(&self, n: usize) -> &Tok {
        &self.toks[(self.pos + n).min(self.toks.len() - 1)].tok
    }

    fn line(&self) -> usize {
        self.toks[self.pos.min(self.toks.len() - 1)].line
    }

    /// Moves past the current token; never past `Eof`.
    fn bump(&mut self) {
        if self.pos < self.toks.len() - 1 {
            self.pos += 1;
        }
    }

    fn error(&self, message: String) -> ParseError {
        ParseError { line: self.line(), message }
    }

    fn describe(&self) -> String {
        match self.tok() {
            Tok::Ident(w) => format!("\"{w}\""),
            Tok::Str(_) => "a string".into(),
            Tok::Num(n) => format!("{n}"),
            Tok::Sym(s) => format!("\"{s}\""),
            Tok::Newline => "the end of the line".into(),
            Tok::Eof => "the end of the script".into(),
        }
    }

    fn is_word(&self, w: &str) -> bool {
        matches!(self.tok(), Tok::Ident(s) if s == w)
    }

    fn word_at(&self, n: usize, w: &str) -> bool {
        matches!(self.tok_at(n), Tok::Ident(s) if s == w)
    }

    fn eat_word(&mut self, w: &str) -> bool {
        if self.is_word(w) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn expect_word(&mut self, w: &str) -> R<()> {
        if self.eat_word(w) {
            Ok(())
        } else {
            Err(self.error(format!("expected \"{w}\" but found {}", self.describe())))
        }
    }

    fn is_sym(&self, s: &str) -> bool {
        matches!(self.tok(), Tok::Sym(x) if *x == s)
    }

    fn eat_sym(&mut self, s: &str) -> bool {
        if self.is_sym(s) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn expect_sym(&mut self, s: &str) -> R<()> {
        if self.eat_sym(s) {
            Ok(())
        } else {
            Err(self.error(format!("expected \"{s}\" but found {}", self.describe())))
        }
    }

    fn at_line_end(&self) -> bool {
        matches!(self.tok(), Tok::Newline | Tok::Eof)
    }

    fn skip_newlines(&mut self) {
        while matches!(self.tok(), Tok::Newline) {
            self.bump();
        }
    }

    fn ident(&mut self) -> R<String> {
        match self.tok().clone() {
            Tok::Ident(w) => {
                self.bump();
                Ok(w)
            }
            _ => Err(self.error(format!("expected a name but found {}", self.describe()))),
        }
    }

    /// `end kw`: the closing word of a block.
    fn end_block(&mut self, kw: &str) -> R<()> {
        self.expect_word("end")?;
        self.expect_word(kw)
    }

    fn handler(&mut self) -> R<Handler> {
        let line = self.line();
        self.expect_word("on")?;
        let name = self.ident()?;
        let mut params = Vec::new();
        if self.eat_sym("(") {
            if !self.eat_sym(")") {
                loop {
                    params.push(self.ident()?);
                    if self.eat_sym(",") {
                        continue;
                    }
                    self.expect_sym(")")?;
                    break;
                }
            }
        } else {
            while let Tok::Ident(w) = self.tok().clone() {
                params.push(w);
                self.bump();
            }
        }
        let body = self.block()?;
        self.expect_word("end")?;
        if self.is_word(&name) {
            self.bump();
        }
        Ok(Handler { name, params, body, line })
    }

    /// Statements up to the word that closes the block (`end`, `else`, or `on error`), not consumed.
    fn block(&mut self) -> R<Vec<Stmt>> {
        let mut out = Vec::new();
        loop {
            self.skip_newlines();
            let closes = match self.tok() {
                Tok::Eof => true,
                Tok::Ident(w) => w == "end" || w == "else" || (w == "on" && self.word_at(1, "error")),
                _ => false,
            };
            if closes {
                break;
            }
            out.push(self.statement()?);
        }
        Ok(out)
    }

    fn statement(&mut self) -> R<Stmt> {
        let first = self.tok().clone();
        match first {
            Tok::Ident(w) => match w.as_str() {
                "set" => self.set_stmt(),
                "get" => {
                    self.bump();
                    Ok(Stmt::Get(self.expr()?))
                }
                "make" => {
                    self.bump();
                    self.expect_word("new")?;
                    let class = self.ident()?;
                    let props = if self.eat_word("with") {
                        self.expect_word("properties")?;
                        Some(self.expr()?)
                    } else {
                        None
                    };
                    Ok(Stmt::Make { class, props })
                }
                "delete" => {
                    self.bump();
                    Ok(Stmt::Delete(self.expr()?))
                }
                "display" => {
                    self.bump();
                    let dialog = if self.eat_word("dialog") {
                        true
                    } else if self.eat_word("notification") {
                        false
                    } else {
                        return Err(self.error(format!("expected \"dialog\" or \"notification\" but found {}", self.describe())));
                    };
                    let text = self.expr()?;
                    let title = if self.eat_word("with") {
                        self.expect_word("title")?;
                        Some(self.expr()?)
                    } else {
                        None
                    };
                    Ok(Stmt::Display { dialog, text, title })
                }
                "if" => self.if_stmt(),
                "repeat" => self.repeat_stmt(),
                "return" => {
                    self.bump();
                    if self.at_line_end() || self.is_word("end") || self.is_word("else") {
                        Ok(Stmt::Return(None))
                    } else {
                        Ok(Stmt::Return(Some(self.expr()?)))
                    }
                }
                "exit" => {
                    self.bump();
                    self.expect_word("repeat")?;
                    Ok(Stmt::Exit)
                }
                "try" => self.try_stmt(),
                "tell" => self.tell_stmt(),
                "delay" => {
                    self.bump();
                    Ok(Stmt::Delay(self.expr()?))
                }
                "do" => Ok(Stmt::Expr(self.expr()?)),
                _ if matches!(self.tok_at(1), Tok::Sym("(")) => Ok(Stmt::Expr(self.expr()?)),
                _ if RESERVED.contains(&w.as_str()) => Err(self.error(format!("expected a statement but found \"{w}\""))),
                _ => self.command_stmt(),
            },
            Tok::Str(_) | Tok::Num(_) | Tok::Sym("(") | Tok::Sym("{") => Ok(Stmt::Expr(self.expr()?)),
            _ => Err(self.error(format!("expected a statement but found {}", self.describe()))),
        }
    }

    fn set_stmt(&mut self) -> R<Stmt> {
        self.bump();
        let target = self.expr()?;
        self.expect_word("to")?;
        let value = self.expr()?;
        match target {
            Expr::Var(name) => Ok(Stmt::Set { name, value }),
            Expr::Prop { prop, of } => Ok(Stmt::SetProp { prop, target: *of, value }),
            _ => Err(self.error("set needs a variable or a property of an object".into())),
        }
    }

    fn command_stmt(&mut self) -> R<Stmt> {
        let mut words: Vec<String> = Vec::new();
        while let Tok::Ident(w) = self.tok().clone() {
            if COMMAND_STOP.contains(&w.as_str()) || matches!(self.tok_at(1), Tok::Sym("(")) {
                break;
            }
            words.push(w);
            self.bump();
        }
        let direct_starts = match self.tok() {
            Tok::Str(_) | Tok::Num(_) => true,
            Tok::Sym(s) => matches!(*s, "{" | "(" | "-"),
            Tok::Ident(w) => matches!(w.as_str(), "true" | "false" | "missing"),
            _ => false,
        };
        let direct = if direct_starts { Some(self.expr()?) } else { None };
        let mut with = Vec::new();
        while self.eat_word("with") {
            let label = self.ident()?;
            with.push((label, self.expr()?));
        }
        Ok(Stmt::Command { phrase: words.join(" "), direct, with })
    }

    fn if_stmt(&mut self) -> R<Stmt> {
        self.bump();
        let cond = self.expr()?;
        self.expect_word("then")?;
        if !matches!(self.tok(), Tok::Newline) {
            // one line: `if c then statement [else statement]`
            let then_body = vec![self.statement()?];
            let else_body = if self.eat_word("else") { vec![self.statement()?] } else { Vec::new() };
            return Ok(Stmt::If { cond, then_body, else_body });
        }
        self.skip_newlines();
        let then_body = self.block()?;
        if self.eat_word("else") {
            if self.is_word("if") {
                // `else if` shares the closing `end if` of its own statement
                let nested = self.if_stmt()?;
                return Ok(Stmt::If { cond, then_body, else_body: vec![nested] });
            }
            let else_body = self.block()?;
            self.end_block("if")?;
            return Ok(Stmt::If { cond, then_body, else_body });
        }
        self.end_block("if")?;
        Ok(Stmt::If { cond, then_body, else_body: Vec::new() })
    }

    fn repeat_stmt(&mut self) -> R<Stmt> {
        self.bump();
        let kind = if self.eat_word("with") {
            let var = self.ident()?;
            if self.eat_word("from") {
                let from = self.expr()?;
                self.expect_word("to")?;
                let to = self.expr()?;
                RepeatKind::Range { var, from, to }
            } else if self.eat_word("in") {
                RepeatKind::Each { var, list: self.expr()? }
            } else {
                return Err(self.error(format!("expected \"from\" or \"in\" but found {}", self.describe())));
            }
        } else if self.at_line_end() {
            RepeatKind::Forever
        } else {
            let count = self.expr()?;
            self.expect_word("times")?;
            RepeatKind::Times(count)
        };
        self.skip_newlines();
        let body = self.block()?;
        self.end_block("repeat")?;
        Ok(Stmt::Repeat { kind, body })
    }

    fn try_stmt(&mut self) -> R<Stmt> {
        self.bump();
        let body = self.block()?;
        let mut error_var = None;
        let mut handler = None;
        if self.is_word("on") && self.word_at(1, "error") {
            self.bump();
            self.bump();
            if let Tok::Ident(w) = self.tok().clone() {
                if w != "number" {
                    error_var = Some(w);
                    self.bump();
                }
            }
            handler = Some(self.block()?);
        }
        self.end_block("try")?;
        Ok(Stmt::Try { body, error_var, handler })
    }

    fn tell_stmt(&mut self) -> R<Stmt> {
        self.bump();
        if !(self.eat_word("app") || self.eat_word("application")) {
            return Err(self.error(format!("expected \"app\" but found {}", self.describe())));
        }
        let app = self.expr()?;
        if self.eat_word("to") {
            let statement = self.statement()?;
            return Ok(Stmt::Tell { app, body: vec![statement] });
        }
        self.skip_newlines();
        let body = self.block()?;
        self.end_block("tell")?;
        Ok(Stmt::Tell { app, body })
    }

    // ----- expressions -----

    fn expr(&mut self) -> R<Expr> {
        let mut left = self.and_expr()?;
        while self.eat_word("or") {
            left = Expr::Bin(BinOp::Or, Box::new(left), Box::new(self.and_expr()?));
        }
        Ok(left)
    }

    fn and_expr(&mut self) -> R<Expr> {
        let mut left = self.not_expr()?;
        while self.eat_word("and") {
            left = Expr::Bin(BinOp::And, Box::new(left), Box::new(self.not_expr()?));
        }
        Ok(left)
    }

    fn not_expr(&mut self) -> R<Expr> {
        if self.eat_word("not") {
            return Ok(Expr::Not(Box::new(self.not_expr()?)));
        }
        self.comparison()
    }

    fn comparison(&mut self) -> R<Expr> {
        let mut left = self.concat()?;
        loop {
            let op = match self.tok().clone() {
                Tok::Sym("=") => {
                    self.bump();
                    BinOp::Eq
                }
                Tok::Sym("≠") => {
                    self.bump();
                    BinOp::Ne
                }
                Tok::Sym("<") => {
                    self.bump();
                    BinOp::Lt
                }
                Tok::Sym("<=") => {
                    self.bump();
                    BinOp::Le
                }
                Tok::Sym(">") => {
                    self.bump();
                    BinOp::Gt
                }
                Tok::Sym(">=") => {
                    self.bump();
                    BinOp::Ge
                }
                Tok::Ident(w) if w == "is" => {
                    self.bump();
                    if self.eat_word("not") {
                        BinOp::Ne
                    } else {
                        BinOp::Eq
                    }
                }
                _ => break,
            };
            left = Expr::Bin(op, Box::new(left), Box::new(self.concat()?));
        }
        Ok(left)
    }

    fn concat(&mut self) -> R<Expr> {
        let mut left = self.additive()?;
        while self.eat_sym("&") {
            left = Expr::Bin(BinOp::Concat, Box::new(left), Box::new(self.additive()?));
        }
        Ok(left)
    }

    fn additive(&mut self) -> R<Expr> {
        let mut left = self.multiplicative()?;
        loop {
            let op = if self.eat_sym("+") {
                BinOp::Add
            } else if self.eat_sym("-") {
                BinOp::Sub
            } else {
                break;
            };
            left = Expr::Bin(op, Box::new(left), Box::new(self.multiplicative()?));
        }
        Ok(left)
    }

    fn multiplicative(&mut self) -> R<Expr> {
        let mut left = self.unary()?;
        loop {
            let op = if self.eat_sym("*") {
                BinOp::Mul
            } else if self.eat_sym("/") {
                BinOp::Div
            } else if self.eat_word("mod") {
                BinOp::Mod
            } else {
                break;
            };
            left = Expr::Bin(op, Box::new(left), Box::new(self.unary()?));
        }
        Ok(left)
    }

    fn unary(&mut self) -> R<Expr> {
        if self.eat_sym("-") {
            return Ok(Expr::Neg(Box::new(self.unary()?)));
        }
        self.primary()
    }

    fn primary(&mut self) -> R<Expr> {
        let tok = self.tok().clone();
        match tok {
            Tok::Num(n) => {
                self.bump();
                Ok(Expr::Num(n))
            }
            Tok::Str(t) => {
                self.bump();
                Ok(Expr::Str(t))
            }
            Tok::Sym("(") => {
                self.bump();
                let e = self.expr()?;
                self.expect_sym(")")?;
                Ok(e)
            }
            Tok::Sym("{") => self.list_or_record(),
            Tok::Ident(w) => self.word_primary(&w),
            _ => Err(self.error(format!("expected a value but found {}", self.describe()))),
        }
    }

    fn word_primary(&mut self, w: &str) -> R<Expr> {
        match w {
            "true" => {
                self.bump();
                return Ok(Expr::Bool(true));
            }
            "false" => {
                self.bump();
                return Ok(Expr::Bool(false));
            }
            "missing" if self.word_at(1, "value") => {
                self.bump();
                self.bump();
                return Ok(Expr::Missing);
            }
            "it" => {
                self.bump();
                return Ok(Expr::Var("it".into()));
            }
            "do" => {
                self.bump();
                self.expect_word("shell")?;
                self.expect_word("script")?;
                return Ok(Expr::Shell(Box::new(self.concat()?)));
            }
            "every" => {
                self.bump();
                let class = self.ident()?;
                let sel = if self.eat_word("whose") {
                    let prop = self.ident()?;
                    if !self.eat_word("is") {
                        self.expect_sym("=")?;
                    }
                    Selector::Whose { prop, value: Box::new(self.additive()?) }
                } else {
                    Selector::All
                };
                return Ok(Expr::Spec { class, sel });
            }
            "item" => {
                self.bump();
                let index = self.primary()?;
                self.expect_word("of")?;
                let of = self.primary()?;
                return Ok(Expr::Item { index: Box::new(index), of: Box::new(of) });
            }
            _ => {}
        }
        if self.word_at(1, "of") {
            let prop = w.to_string();
            self.bump();
            self.bump();
            return Ok(Expr::Prop { prop, of: Box::new(self.primary()?) });
        }
        if matches!(self.tok_at(1), Tok::Sym("(")) {
            let name = w.to_string();
            self.bump();
            self.bump();
            let mut args = Vec::new();
            if !self.eat_sym(")") {
                loop {
                    args.push(self.expr()?);
                    if self.eat_sym(",") {
                        continue;
                    }
                    self.expect_sym(")")?;
                    break;
                }
            }
            return Ok(Expr::Call { name, args });
        }
        if matches!(self.tok_at(1), Tok::Num(_) | Tok::Str(_)) {
            let class = w.to_string();
            self.bump();
            let key = self.primary()?;
            let sel = match key {
                Expr::Str(_) => Selector::Name(Box::new(key)),
                other => Selector::Index(Box::new(other)),
            };
            return Ok(Expr::Spec { class, sel });
        }
        if RESERVED.contains(&w) {
            return Err(self.error(format!("expected a value but found \"{w}\"")));
        }
        let name = w.to_string();
        self.bump();
        Ok(Expr::Var(name))
    }

    fn list_or_record(&mut self) -> R<Expr> {
        self.bump();
        if self.eat_sym("}") {
            return Ok(Expr::List(Vec::new()));
        }
        if matches!(self.tok(), Tok::Ident(_)) && matches!(self.tok_at(1), Tok::Sym(":")) {
            let mut fields = Vec::new();
            loop {
                let key = self.ident()?;
                self.expect_sym(":")?;
                fields.push((key, self.expr()?));
                if self.eat_sym(",") {
                    continue;
                }
                self.expect_sym("}")?;
                return Ok(Expr::Record(fields));
            }
        }
        let mut items = Vec::new();
        loop {
            items.push(self.expr()?);
            if self.eat_sym(",") {
                continue;
            }
            self.expect_sym("}")?;
            return Ok(Expr::List(items));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(src: &str) -> Vec<Stmt> {
        parse(src).unwrap().body
    }

    #[test]
    fn set_and_arithmetic_precedence() {
        let b = body("set x to 1 + 2 * 3");
        match &b[0] {
            Stmt::Set { name, value: Expr::Bin(BinOp::Add, _, right) } => {
                assert_eq!(name, "x");
                assert!(matches!(**right, Expr::Bin(BinOp::Mul, _, _)));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn concatenation_binds_looser_than_addition() {
        let b = body("set s to \"n\" & 1 + 2");
        assert!(matches!(&b[0], Stmt::Set { value: Expr::Bin(BinOp::Concat, _, _), .. }));
    }

    #[test]
    fn handlers_with_parameters_and_return() {
        let p = parse("on double(n)\n return n * 2\nend double\nset r to double(4)").unwrap();
        assert_eq!(p.handlers.len(), 1);
        assert_eq!(p.handlers[0].name, "double");
        assert_eq!(p.handlers[0].params, vec!["n"]);
        assert!(matches!(p.handlers[0].body[0], Stmt::Return(Some(_))));
        assert!(matches!(&p.body[0], Stmt::Set { value: Expr::Call { .. }, .. }));
    }

    #[test]
    fn if_with_else_if_and_one_line_form() {
        let b = body("if 1 < 2 then\n set a to 1\nelse if 2 < 1 then\n set a to 2\nelse\n set a to 3\nend if");
        match &b[0] {
            Stmt::If { else_body, .. } => assert!(matches!(else_body[0], Stmt::If { .. })),
            other => panic!("unexpected {other:?}"),
        }
        let one = body("if true then set a to 1 else set a to 2");
        assert!(matches!(&one[0], Stmt::If { then_body, else_body, .. } if then_body.len() == 1 && else_body.len() == 1));
    }

    #[test]
    fn repeat_forms() {
        assert!(matches!(&body("repeat 3 times\nset a to 1\nend repeat")[0], Stmt::Repeat { kind: RepeatKind::Times(_), .. }));
        assert!(matches!(&body("repeat with i from 1 to 3\nend repeat")[0], Stmt::Repeat { kind: RepeatKind::Range { .. }, .. }));
        assert!(matches!(&body("repeat with x in {1, 2}\nend repeat")[0], Stmt::Repeat { kind: RepeatKind::Each { .. }, .. }));
        assert!(matches!(&body("repeat\nexit repeat\nend repeat")[0], Stmt::Repeat { kind: RepeatKind::Forever, .. }));
    }

    #[test]
    fn tell_block_and_one_line_tell_with_a_command() {
        let b = body("tell app \"Notes\"\n make new note with properties {name:\"a\", body:\"b\"}\nend tell");
        match &b[0] {
            Stmt::Tell { body, .. } => assert!(matches!(&body[0], Stmt::Make { class, props: Some(Expr::Record(f)) } if class == "note" && f.len() == 2)),
            other => panic!("unexpected {other:?}"),
        }
        let one = body("tell application \"Notes\" to activate");
        assert!(matches!(&one[0], Stmt::Tell { body, .. } if matches!(&body[0], Stmt::Command { phrase, .. } if phrase == "activate")));
    }

    #[test]
    fn command_words_direct_argument_and_labels() {
        let b = body("tell app \"Notes\" to save note \"x\" with force {}");
        match &b[0] {
            Stmt::Tell { body, .. } => match &body[0] {
                Stmt::Command { phrase, direct: Some(Expr::Str(s)), with } => {
                    assert_eq!(phrase, "save note");
                    assert_eq!(s, "x");
                    assert_eq!(with[0].0, "force");
                }
                other => panic!("unexpected {other:?}"),
            },
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn specifiers_properties_and_get_set() {
        let b = body("tell app \"Notes\"\nget name of note 1\nset name of window \"a\" to \"b\"\nget every note whose name is \"x\"\nend tell");
        let Stmt::Tell { body, .. } = &b[0] else { panic!() };
        assert!(matches!(&body[0], Stmt::Get(Expr::Prop { prop, of }) if prop == "name" && matches!(**of, Expr::Spec { .. })));
        assert!(matches!(&body[1], Stmt::SetProp { prop, .. } if prop == "name"));
        assert!(matches!(&body[2], Stmt::Get(Expr::Spec { sel: Selector::Whose { .. }, .. })));
    }

    #[test]
    fn try_with_error_handler_and_without() {
        let b = body("try\n set a to 1\non error e\n set a to 2\nend try\ntry\n set a to 3\nend try");
        assert!(matches!(&b[0], Stmt::Try { error_var: Some(v), handler: Some(_), .. } if v == "e"));
        assert!(matches!(&b[1], Stmt::Try { handler: None, .. }));
    }

    #[test]
    fn display_and_shell_and_delay() {
        let b = body("display dialog \"hi\" with title \"T\"\nset o to do shell script \"echo x\"\ndelay 0.5");
        assert!(matches!(&b[0], Stmt::Display { dialog: true, title: Some(_), .. }));
        assert!(matches!(&b[1], Stmt::Set { value: Expr::Shell(_), .. }));
        assert!(matches!(&b[2], Stmt::Delay(Expr::Num(n)) if *n == 0.5));
    }

    #[test]
    fn errors_point_at_the_line() {
        let e = parse("set a to 1\nset b to )").unwrap_err();
        assert_eq!(e.line, 2);
        assert!(parse("if 1 then\nset a to 1\n").is_err(), "a block if needs end if");
        assert!(parse("on f(\n").is_err());
        assert!(parse("repeat with i from 1\nend repeat").is_err(), "a range needs its to");
    }
}
