//! Script syntax check parity: tells whether a script is well formed before it is run.
//!
//! This is a lexical check, not a parser. It reads string, template and comment syntax, regular expression literals
//! (a `/` after an operator, or at the start, opens one) and the nesting of `()`, `[]`, `{}` and `${...}`. It reports
//! an unterminated string, comment, regular expression or template, a group that is never closed, and a closing
//! bracket that does not match. It does not check keywords, declarations or operator placement, so a script can pass
//! here and still fail when it runs.

/// The first problem found in a script. `line` and `column` are 1-based; the column counts characters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyntaxError {
    pub line: u32,
    pub column: u32,
    pub message: String,
}

impl std::fmt::Display for SyntaxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}: {}", self.line, self.column, self.message)
    }
}

impl std::error::Error for SyntaxError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Frame {
    /// An opening bracket, with its position.
    Group(char, u32, u32),
    /// A template literal, with the position of its opening backtick.
    Template(u32, u32),
    /// A `${` substitution inside a template.
    Interp(u32, u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Lex {
    Code,
    LineComment,
    BlockComment,
    Quoted(char),
    Regex { in_class: bool },
}

/// Checks `source` and returns the first problem found, or `Ok(())` when its brackets, strings, templates, comments and
/// regular expression literals are all closed.
pub fn check(source: &str) -> Result<(), SyntaxError> {
    let chars: Vec<char> = source.chars().collect();
    let mut stack: Vec<Frame> = Vec::new();
    let mut lex = Lex::Code;
    // Where the current string, comment or regular expression began.
    let mut start = (1u32, 1u32);
    // The last significant code character, to tell a regular expression from a division.
    let mut prev: Option<char> = None;
    let (mut line, mut col) = (1u32, 1u32);
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        let here = (line, col);
        let mut width = 1usize;
        let in_template = matches!(stack.last(), Some(Frame::Template(..)));
        if in_template {
            match c {
                '\\' if next.is_some() => width = 2,
                '`' => {
                    stack.pop();
                    prev = Some('a');
                }
                '$' if next == Some('{') => {
                    stack.push(Frame::Interp(line, col));
                    prev = Some('{');
                    width = 2;
                }
                _ => {}
            }
        } else {
            match lex {
                Lex::LineComment => {
                    if c == '\n' {
                        lex = Lex::Code;
                    }
                }
                Lex::BlockComment => {
                    if c == '*' && next == Some('/') {
                        lex = Lex::Code;
                        width = 2;
                    }
                }
                Lex::Quoted(quote) => match c {
                    '\\' if next.is_some() => width = 2,
                    '\n' => return Err(err_at(start, "unterminated string")),
                    _ if c == quote => {
                        lex = Lex::Code;
                        prev = Some('a');
                    }
                    _ => {}
                },
                Lex::Regex { in_class } => match c {
                    '\\' if next.is_some() => width = 2,
                    '\n' => return Err(err_at(start, "unterminated regular expression")),
                    '[' => lex = Lex::Regex { in_class: true },
                    ']' if in_class => lex = Lex::Regex { in_class: false },
                    '/' if !in_class => {
                        lex = Lex::Code;
                        prev = Some('a');
                    }
                    _ => {}
                },
                Lex::Code => match c {
                    '/' if next == Some('/') => {
                        lex = Lex::LineComment;
                        width = 2;
                    }
                    '/' if next == Some('*') => {
                        lex = Lex::BlockComment;
                        start = here;
                        width = 2;
                    }
                    '/' if regex_allowed(prev) => {
                        lex = Lex::Regex { in_class: false };
                        start = here;
                    }
                    '/' => prev = Some('/'),
                    '"' | '\'' => {
                        lex = Lex::Quoted(c);
                        start = here;
                    }
                    '`' => stack.push(Frame::Template(line, col)),
                    '(' | '[' | '{' => {
                        stack.push(Frame::Group(c, line, col));
                        prev = Some(c);
                    }
                    ')' | ']' | '}' => {
                        close_group(&mut stack, c, here)?;
                        prev = Some(c);
                    }
                    c if c.is_whitespace() => {}
                    _ => prev = Some(c),
                },
            }
        }
        for k in 0..width {
            if chars[i + k] == '\n' {
                line += 1;
                col = 1;
            } else {
                col += 1;
            }
        }
        i += width;
    }
    match lex {
        Lex::BlockComment => return Err(err_at(start, "unterminated comment")),
        Lex::Quoted(_) => return Err(err_at(start, "unterminated string")),
        Lex::Regex { .. } => return Err(err_at(start, "unterminated regular expression")),
        Lex::Code | Lex::LineComment => {}
    }
    match stack.last().copied() {
        None => Ok(()),
        Some(Frame::Group(open, l, c)) => Err(err_at((l, c), format!("'{}' is never closed", open))),
        Some(Frame::Interp(l, c)) => Err(err_at((l, c), "'${' is never closed")),
        Some(Frame::Template(l, c)) => Err(err_at((l, c), "unterminated template literal")),
    }
}

fn err_at(at: (u32, u32), message: impl Into<String>) -> SyntaxError {
    SyntaxError { line: at.0, column: at.1, message: message.into() }
}

fn closer(open: char) -> char {
    match open {
        '(' => ')',
        '[' => ']',
        _ => '}',
    }
}

fn close_group(stack: &mut Vec<Frame>, c: char, at: (u32, u32)) -> Result<(), SyntaxError> {
    match stack.pop() {
        Some(Frame::Group(open, _, _)) if closer(open) == c => Ok(()),
        Some(Frame::Interp(_, _)) if c == '}' => Ok(()),
        Some(Frame::Group(open, l, col)) => Err(err_at(
            at,
            format!("expected '{}' to close '{}' at {}:{}", closer(open), open, l, col),
        )),
        Some(Frame::Interp(l, col)) => Err(err_at(at, format!("expected '}}' to close '${{' at {}:{}", l, col))),
        Some(Frame::Template(_, _)) | None => Err(err_at(at, format!("unexpected '{}'", c))),
    }
}

/// A `/` starts a regular expression after an operator, an opening bracket, or at the start of the script.
fn regex_allowed(prev: Option<char>) -> bool {
    match prev {
        None => true,
        Some(ch) => "(,=:[!&|?{};+-*%<>~^".contains(ch),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn well_formed_script_passes() {
        assert_eq!(check("const a = { b: [1, (2)] }; // note\nlet c = 'x' + \"y\";\n"), Ok(()));
        assert_eq!(check("/* block\n comment */ f(1);"), Ok(()));
        assert_eq!(check(""), Ok(()));
    }

    #[test]
    fn unclosed_group_is_reported_at_its_opening_bracket() {
        let err = check("let x = {\n  a: 1;\n").unwrap_err();
        assert_eq!((err.line, err.column), (1, 9));
        assert_eq!(err.message, "'{' is never closed");
    }

    #[test]
    fn mismatched_closer_names_the_expected_one() {
        let err = check("f(a]").unwrap_err();
        assert_eq!((err.line, err.column), (1, 4));
        assert!(err.message.contains("expected ')'"), "{}", err.message);
    }

    #[test]
    fn stray_closer_is_unexpected() {
        let err = check("a);").unwrap_err();
        assert_eq!((err.line, err.column), (1, 2));
        assert_eq!(err.message, "unexpected ')'");
    }

    #[test]
    fn template_substitution_nests_inside_the_template() {
        assert_eq!(check("const s = `a ${ b({c: 1}) } d`;"), Ok(()));
        let err = check("`a ${ b(`").unwrap_err();
        assert!(err.message.contains("unterminated template literal") || err.message.contains("never closed"));
    }

    #[test]
    fn unterminated_string_comment_and_template_are_reported_at_their_start() {
        let s = check("let s = \"abc\n").unwrap_err();
        assert_eq!((s.line, s.column, s.message.as_str()), (1, 9, "unterminated string"));
        let c = check("/* open").unwrap_err();
        assert_eq!((c.line, c.column, c.message.as_str()), (1, 1, "unterminated comment"));
        let t = check("`abc").unwrap_err();
        assert_eq!((t.line, t.column, t.message.as_str()), (1, 1, "unterminated template literal"));
    }

    #[test]
    fn brackets_inside_regular_expressions_and_strings_do_not_count() {
        assert_eq!(check("const r = /[(]/;"), Ok(()));
        assert_eq!(check("x = a / (b);"), Ok(()));
        assert_eq!(check("s = \"(\" + '[';"), Ok(()));
    }
}
