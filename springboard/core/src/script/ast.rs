//! The syntax tree of a script, and its JSON form (what `osacompile` writes next to the canonical source; see
//! `script::compile` and the "Scripting" section of `docs/bifrost-ui.md`).

use crate::json::Json;

/// A whole script: its handlers (`on name(params) ... end`) and the statements that run when it loads.
#[derive(Clone, Debug, PartialEq)]
pub struct Program {
    pub handlers: Vec<Handler>,
    pub body: Vec<Stmt>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Handler {
    pub name: String,
    pub params: Vec<String>,
    pub body: Vec<Stmt>,
    pub line: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Stmt {
    /// `set name to value`
    Set { name: String, value: Expr },
    /// `set property of target to value`
    SetProp { prop: String, target: Expr, value: Expr },
    /// `get expr`: a specifier is resolved through the current `tell`; the result becomes `it`.
    Get(Expr),
    /// `make new class [with properties record]`
    Make { class: String, props: Option<Expr> },
    /// `delete specifier`
    Delete(Expr),
    /// `display dialog text [with title t]` (`dialog`) or `display notification text [with title t]`.
    Display { dialog: bool, text: Expr, title: Option<Expr> },
    /// A command word run inside a `tell`: `phrase [direct] [with label value]...`, resolved through the app's dictionary.
    Command { phrase: String, direct: Option<Expr>, with: Vec<(String, Expr)> },
    /// `tell app "name" ... end tell`, or `tell app "name" to statement`.
    Tell { app: Expr, body: Vec<Stmt> },
    /// `if cond then ... else ... end if` (or the one-line form).
    If { cond: Expr, then_body: Vec<Stmt>, else_body: Vec<Stmt> },
    Repeat { kind: RepeatKind, body: Vec<Stmt> },
    /// `exit repeat`
    Exit,
    Return(Option<Expr>),
    /// `try ... on error [name] ... end try`. Without a handler the error goes on to the caller.
    Try { body: Vec<Stmt>, error_var: Option<String>, handler: Option<Vec<Stmt>> },
    /// `delay seconds`
    Delay(Expr),
    /// An expression run for its effect (a handler call or `do shell script`); its value becomes `it`.
    Expr(Expr),
}

#[derive(Clone, Debug, PartialEq)]
pub enum RepeatKind {
    /// `repeat` with no count: runs until `exit repeat`.
    Forever,
    /// `repeat n times`
    Times(Expr),
    /// `repeat with var from a to b`
    Range { var: String, from: Expr, to: Expr },
    /// `repeat with var in list`
    Each { var: String, list: Expr },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Num(f64),
    Str(String),
    Bool(bool),
    Missing,
    Var(String),
    List(Vec<Expr>),
    Record(Vec<(String, Expr)>),
    Not(Box<Expr>),
    Neg(Box<Expr>),
    Bin(BinOp, Box<Expr>, Box<Expr>),
    /// `name(args)`: a handler call.
    Call { name: String, args: Vec<Expr> },
    /// `do shell script command`
    Shell(Box<Expr>),
    /// `prop of target`
    Prop { prop: String, of: Box<Expr> },
    /// `item n of list` (1-based)
    Item { index: Box<Expr>, of: Box<Expr> },
    /// An object specifier: `class n`, `class "name"`, `every class`, `every class whose prop = value`.
    Spec { class: String, sel: Selector },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Selector {
    Index(Box<Expr>),
    Name(Box<Expr>),
    All,
    Whose { prop: String, value: Box<Expr> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    Concat,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
}

impl BinOp {
    /// The operator's name in the JSON form.
    pub fn name(self) -> &'static str {
        match self {
            BinOp::Eq => "=",
            BinOp::Ne => "≠",
            BinOp::Lt => "<",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            BinOp::Ge => ">=",
            BinOp::And => "and",
            BinOp::Or => "or",
            BinOp::Concat => "&",
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Mod => "mod",
        }
    }
}

fn node(kind: &str, fields: Vec<(&str, Json)>) -> Json {
    let mut all = vec![("node".to_string(), Json::Str(kind.to_string()))];
    all.extend(fields.into_iter().map(|(k, v)| (k.to_string(), v)));
    Json::Obj(all)
}

fn s(text: &str) -> Json {
    Json::Str(text.to_string())
}

fn block(stmts: &[Stmt]) -> Json {
    Json::Arr(stmts.iter().map(stmt_json).collect())
}

fn opt(e: &Option<Expr>) -> Json {
    match e {
        Some(e) => expr_json(e),
        None => Json::Null,
    }
}

/// The JSON form of a whole program: `{"handlers": [...], "body": [...]}`.
pub fn program_json(p: &Program) -> Json {
    let handlers = p
        .handlers
        .iter()
        .map(|h| {
            Json::Obj(vec![
                ("name".into(), s(&h.name)),
                ("params".into(), Json::Arr(h.params.iter().map(|x| s(x)).collect())),
                ("line".into(), Json::Num(h.line as f64)),
                ("body".into(), block(&h.body)),
            ])
        })
        .collect();
    Json::Obj(vec![("handlers".into(), Json::Arr(handlers)), ("body".into(), block(&p.body))])
}

fn stmt_json(st: &Stmt) -> Json {
    match st {
        Stmt::Set { name, value } => node("set", vec![("name", s(name)), ("value", expr_json(value))]),
        Stmt::SetProp { prop, target, value } => node("set-property", vec![("prop", s(prop)), ("target", expr_json(target)), ("value", expr_json(value))]),
        Stmt::Get(e) => node("get", vec![("expr", expr_json(e))]),
        Stmt::Make { class, props } => node("make", vec![("class", s(class)), ("props", opt(props))]),
        Stmt::Delete(e) => node("delete", vec![("expr", expr_json(e))]),
        Stmt::Display { dialog, text, title } => node("display", vec![("dialog", Json::Bool(*dialog)), ("text", expr_json(text)), ("title", opt(title))]),
        Stmt::Command { phrase, direct, with } => {
            let with = with.iter().map(|(k, v)| Json::Obj(vec![("label".into(), s(k)), ("value".into(), expr_json(v))])).collect();
            node("command", vec![("phrase", s(phrase)), ("direct", opt(direct)), ("with", Json::Arr(with))])
        }
        Stmt::Tell { app, body } => node("tell", vec![("app", expr_json(app)), ("body", block(body))]),
        Stmt::If { cond, then_body, else_body } => node("if", vec![("cond", expr_json(cond)), ("then", block(then_body)), ("else", block(else_body))]),
        Stmt::Repeat { kind, body } => {
            let kind_json = match kind {
                RepeatKind::Forever => node("forever", vec![]),
                RepeatKind::Times(e) => node("times", vec![("count", expr_json(e))]),
                RepeatKind::Range { var, from, to } => node("range", vec![("var", s(var)), ("from", expr_json(from)), ("to", expr_json(to))]),
                RepeatKind::Each { var, list } => node("each", vec![("var", s(var)), ("list", expr_json(list))]),
            };
            node("repeat", vec![("kind", kind_json), ("body", block(body))])
        }
        Stmt::Exit => node("exit-repeat", vec![]),
        Stmt::Return(e) => node("return", vec![("value", opt(e))]),
        Stmt::Try { body, error_var, handler } => node(
            "try",
            vec![
                ("body", block(body)),
                ("error_var", error_var.as_deref().map(s).unwrap_or(Json::Null)),
                ("handler", handler.as_ref().map(|h| block(h)).unwrap_or(Json::Null)),
            ],
        ),
        Stmt::Delay(e) => node("delay", vec![("seconds", expr_json(e))]),
        Stmt::Expr(e) => node("expr", vec![("expr", expr_json(e))]),
    }
}

fn expr_json(e: &Expr) -> Json {
    match e {
        Expr::Num(n) => node("number", vec![("value", Json::Num(*n))]),
        Expr::Str(t) => node("string", vec![("value", s(t))]),
        Expr::Bool(b) => node("boolean", vec![("value", Json::Bool(*b))]),
        Expr::Missing => node("missing", vec![]),
        Expr::Var(name) => node("variable", vec![("name", s(name))]),
        Expr::List(items) => node("list", vec![("items", Json::Arr(items.iter().map(expr_json).collect()))]),
        Expr::Record(fields) => {
            let fields = fields.iter().map(|(k, v)| Json::Obj(vec![("key".into(), s(k)), ("value".into(), expr_json(v))])).collect();
            node("record", vec![("fields", Json::Arr(fields))])
        }
        Expr::Not(a) => node("not", vec![("operand", expr_json(a))]),
        Expr::Neg(a) => node("negate", vec![("operand", expr_json(a))]),
        Expr::Bin(op, a, b) => node("binary", vec![("op", s(op.name())), ("left", expr_json(a)), ("right", expr_json(b))]),
        Expr::Call { name, args } => node("call", vec![("name", s(name)), ("args", Json::Arr(args.iter().map(expr_json).collect()))]),
        Expr::Shell(cmd) => node("shell", vec![("command", expr_json(cmd))]),
        Expr::Prop { prop, of } => node("property", vec![("prop", s(prop)), ("of", expr_json(of))]),
        Expr::Item { index, of } => node("item", vec![("index", expr_json(index)), ("of", expr_json(of))]),
        Expr::Spec { class, sel } => {
            let sel_json = match sel {
                Selector::Index(e) => node("index", vec![("value", expr_json(e))]),
                Selector::Name(e) => node("name", vec![("value", expr_json(e))]),
                Selector::All => node("all", vec![]),
                Selector::Whose { prop, value } => node("whose", vec![("prop", s(prop)), ("value", expr_json(value))]),
            };
            node("specifier", vec![("class", s(class)), ("selector", sel_json)])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_json_names_its_nodes() {
        let p = Program { handlers: vec![], body: vec![Stmt::Set { name: "x".into(), value: Expr::Num(1.0) }] };
        let text = program_json(&p).encode();
        assert!(text.contains(r#""node":"set""#) && text.contains(r#""name":"x""#), "{text}");
        assert!(text.contains(r#""handlers":[]"#));
    }

    #[test]
    fn operators_have_their_script_spelling() {
        assert_eq!(BinOp::Concat.name(), "&");
        assert_eq!(BinOp::Ne.name(), "≠");
    }
}
