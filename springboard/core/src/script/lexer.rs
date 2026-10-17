//! Tokens of the scripting language. Identifiers are lowercased (the language is case-insensitive for names) and
//! string literals keep their case. `--` starts a comment that runs to the end of the line. A line ends with a newline
//! token; a `¬` at the end of a line joins it to the next one.

use std::fmt;

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    /// A word: a keyword, a variable, a handler or class name, or a command word. Lowercased.
    Ident(String),
    /// A string literal with its escapes resolved.
    Str(String),
    Num(f64),
    /// Operators and punctuation: `= ≠ < <= > >= & + - * / ( ) { } , :`.
    Sym(&'static str),
    Newline,
    Eof,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub tok: Tok,
    pub line: usize,
}

/// A syntax error with the line it was found on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    pub line: usize,
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "syntax error on line {}: {}", self.line, self.message)
    }
}

fn sym_of(c: char) -> Option<&'static str> {
    Some(match c {
        '=' => "=",
        '≠' => "≠",
        '&' => "&",
        '+' => "+",
        '-' => "-",
        '*' => "*",
        '/' => "/",
        '(' => "(",
        ')' => ")",
        '{' => "{",
        '}' => "}",
        ',' => ",",
        ':' => ":",
        _ => return None,
    })
}

/// Splits a source text into tokens. The last token is always `Eof`.
pub fn lex(src: &str) -> Result<Vec<Token>, ParseError> {
    let chars: Vec<char> = src.chars().collect();
    let n = chars.len();
    let mut out: Vec<Token> = Vec::new();
    let mut line = 1usize;
    let mut i = 0usize;
    let err = |line: usize, message: String| ParseError { line, message };
    while i < n {
        let c = chars[i];
        match c {
            '\n' => {
                out.push(Token { tok: Tok::Newline, line });
                line += 1;
                i += 1;
            }
            ' ' | '\t' | '\r' => i += 1,
            '¬' => {
                // line continuation: skip the rest of this line's blanks and its newline
                i += 1;
                while i < n && matches!(chars[i], ' ' | '\t' | '\r') {
                    i += 1;
                }
                if i < n && chars[i] == '\n' {
                    line += 1;
                    i += 1;
                }
            }
            '-' if chars.get(i + 1) == Some(&'-') => {
                while i < n && chars[i] != '\n' {
                    i += 1;
                }
            }
            '"' => {
                let start_line = line;
                i += 1;
                let mut text = String::new();
                loop {
                    if i >= n {
                        return Err(err(start_line, "unterminated string".into()));
                    }
                    let ch = chars[i];
                    i += 1;
                    match ch {
                        '"' => break,
                        '\\' => {
                            let Some(&esc) = chars.get(i) else {
                                return Err(err(line, "unterminated string".into()));
                            };
                            i += 1;
                            text.push(match esc {
                                'n' => '\n',
                                't' => '\t',
                                'r' => '\r',
                                other => other,
                            });
                        }
                        '\n' => {
                            line += 1;
                            text.push('\n');
                        }
                        other => text.push(other),
                    }
                }
                out.push(Token { tok: Tok::Str(text), line: start_line });
            }
            c if c.is_ascii_digit() => {
                let start = i;
                while i < n && chars[i].is_ascii_digit() {
                    i += 1;
                }
                if i + 1 < n && chars[i] == '.' && chars[i + 1].is_ascii_digit() {
                    i += 1;
                    while i < n && chars[i].is_ascii_digit() {
                        i += 1;
                    }
                }
                let text: String = chars[start..i].iter().collect();
                let value: f64 = text.parse().map_err(|_| err(line, format!("bad number {text}")))?;
                out.push(Token { tok: Tok::Num(value), line });
            }
            c if c.is_alphabetic() || c == '_' => {
                let start = i;
                while i < n && (chars[i].is_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                let word: String = chars[start..i].iter().collect::<String>().to_lowercase();
                out.push(Token { tok: Tok::Ident(word), line });
            }
            '<' | '>' => {
                let two = chars.get(i + 1) == Some(&'=');
                let sym = match (c, two) {
                    ('<', true) => "<=",
                    ('<', false) => "<",
                    ('>', true) => ">=",
                    _ => ">",
                };
                i += if two { 2 } else { 1 };
                out.push(Token { tok: Tok::Sym(sym), line });
            }
            other => match sym_of(other) {
                Some(sym) => {
                    out.push(Token { tok: Tok::Sym(sym), line });
                    i += 1;
                }
                None => return Err(err(line, format!("unexpected character '{other}'"))),
            },
        }
    }
    out.push(Token { tok: Tok::Eof, line });
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<Tok> {
        lex(src).unwrap().into_iter().map(|t| t.tok).collect()
    }

    #[test]
    fn words_are_lowercased_and_strings_keep_their_case() {
        assert_eq!(kinds("Set X to \"Hi\""), vec![Tok::Ident("set".into()), Tok::Ident("x".into()), Tok::Ident("to".into()), Tok::Str("Hi".into()), Tok::Eof]);
    }

    #[test]
    fn numbers_strings_and_escapes() {
        assert_eq!(kinds("3.5 7"), vec![Tok::Num(3.5), Tok::Num(7.0), Tok::Eof]);
        assert_eq!(kinds(r#""a\"b\nc""#), vec![Tok::Str("a\"b\nc".into()), Tok::Eof]);
    }

    #[test]
    fn comparison_and_concatenation_operators() {
        assert_eq!(kinds("<= >= < > = ≠ &"), vec![Tok::Sym("<="), Tok::Sym(">="), Tok::Sym("<"), Tok::Sym(">"), Tok::Sym("="), Tok::Sym("≠"), Tok::Sym("&"), Tok::Eof]);
    }

    #[test]
    fn comments_run_to_the_end_of_the_line_and_newlines_count() {
        let toks = lex("set a to 1 -- note\nset b to 2").unwrap();
        assert!(toks.iter().any(|t| t.tok == Tok::Newline && t.line == 1));
        assert_eq!(toks.last().unwrap().line, 2);
        assert!(!toks.iter().any(|t| matches!(&t.tok, Tok::Ident(w) if w == "note")));
    }

    #[test]
    fn a_continuation_mark_joins_lines() {
        assert_eq!(kinds("set a to ¬\n1"), vec![Tok::Ident("set".into()), Tok::Ident("a".into()), Tok::Ident("to".into()), Tok::Num(1.0), Tok::Eof]);
    }

    #[test]
    fn bad_input_reports_its_line() {
        let e = lex("set a to 1\nset b to \"open").unwrap_err();
        assert_eq!(e.line, 2);
        assert!(lex("set a to 1 ?").is_err());
    }
}
