//! `bzegrep`: searches bzip2-compressed files with extended regular expressions. The bzip2 package ships only `bzgrep`,
//! so this runs it with `-E` in front of the arguments the caller gave, and passes its exit status back. Gap: the
//! reference tool's own option parsing is not reproduced; every argument is handed to `bzgrep` unchanged.

pub const USAGE: &str = "usage: bzegrep [grep options] pattern [file ...]";

/// The program that does the search. It comes from the bzip2 package.
pub const PROGRAM: &str = "bzgrep";

/// What the command line asks for: the help text, or the arguments to pass to `bzgrep`.
#[derive(Debug, PartialEq, Eq)]
pub enum Parsed {
    Help,
    Run(Vec<String>),
}

/// Reads the arguments. `-h` or `--help` asks for the help text; otherwise `-E` is put first, so the pattern is an
/// extended regular expression.
pub fn parse(args: &[String]) -> Parsed {
    if args.iter().any(|a| a == "-h" || a == "--help") {
        return Parsed::Help;
    }
    let mut out = Vec::with_capacity(args.len() + 1);
    out.push("-E".to_string());
    out.extend(args.iter().cloned());
    Parsed::Run(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn extended_regular_expressions_are_forced_first() {
        assert_eq!(parse(&args(&["a|b", "log.bz2"])), Parsed::Run(args(&["-E", "a|b", "log.bz2"])));
    }

    #[test]
    fn help_is_asked_for_by_either_flag() {
        assert_eq!(parse(&args(&["-h"])), Parsed::Help);
        assert_eq!(parse(&args(&["x", "--help"])), Parsed::Help);
    }
}
