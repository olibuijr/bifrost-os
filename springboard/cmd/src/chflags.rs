//! `chflags`: sets or clears a file's flags, through `chattr`. The flags map onto the Linux attributes: `uchg` and `schg`
//! (immutable) are the `i` attribute, `uappnd` and `sappnd` (append only) are `a`, and `nodump` (skip in backups) is `d`.
//! A `no` prefix clears a flag, and `0` clears all three. Gap: `hidden` has no Linux attribute and is refused, and the
//! reference tool's `-h` (act on a link itself) is not supported. Options: `-R` (recurse into folders).

pub const USAGE: &str = "usage: chflags [-R] flags file ...";

/// The `chattr` letter for each Linux-backed flag word.
const FLAG_LETTERS: &[(&str, char)] = &[("uchg", 'i'), ("schg", 'i'), ("uappnd", 'a'), ("sappnd", 'a'), ("nodump", 'd')];

/// The result of reading the command line.
#[derive(Debug, PartialEq, Eq)]
pub enum Parsed {
    Help,
    Run(Request),
}

/// One `chflags` run: the `chattr` modes to apply in order, whether to recurse, and the files.
#[derive(Debug, PartialEq, Eq)]
pub struct Request {
    pub recursive: bool,
    pub modes: Vec<String>,
    pub files: Vec<String>,
}

/// Reads the arguments: an optional `-R`, the flags, then one or more files.
pub fn parse(args: &[String]) -> Result<Parsed, String> {
    let mut recursive = false;
    let mut positional: Vec<&String> = Vec::new();
    for arg in args {
        match arg.as_str() {
            "-h" | "--help" => return Ok(Parsed::Help),
            "-R" => recursive = true,
            other if other.starts_with('-') && other.len() > 1 => {
                return Err(format!("option {other} is not supported yet\n{USAGE}"));
            }
            _ => positional.push(arg),
        }
    }
    if positional.len() < 2 {
        return Err(USAGE.to_string());
    }
    let modes = modes_for(positional[0])?;
    let files = positional[1..].iter().map(|s| s.to_string()).collect();
    Ok(Parsed::Run(Request { recursive, modes, files }))
}

/// Turns a comma-separated flag list into `chattr` modes. Letters with the same sign share one mode, so `uchg,nodump`
/// is `+id` and `uchg,nouappnd` is `+i` then `-a`. `0` is one mode that clears every attribute this maps to.
pub fn modes_for(flags: &str) -> Result<Vec<String>, String> {
    if flags == "0" {
        return Ok(vec!["-iad".to_string()]);
    }
    let mut plus = String::new();
    let mut minus = String::new();
    for word in flags.split(',').filter(|w| !w.is_empty()) {
        let (set, name) = match word.strip_prefix("no") {
            Some(rest) if FLAG_LETTERS.iter().any(|(n, _)| *n == rest) => (false, rest),
            _ => (true, word),
        };
        let Some((_, letter)) = FLAG_LETTERS.iter().find(|(n, _)| *n == name) else {
            return Err(if name == "hidden" {
                "hidden has no Linux attribute; name the file with a leading dot instead".to_string()
            } else {
                format!("unknown flag {word}\n{USAGE}")
            });
        };
        let bucket = if set { &mut plus } else { &mut minus };
        if !bucket.contains(*letter) {
            bucket.push(*letter);
        }
    }
    let mut modes = Vec::new();
    if !plus.is_empty() {
        modes.push(format!("+{plus}"));
    }
    if !minus.is_empty() {
        modes.push(format!("-{minus}"));
    }
    if modes.is_empty() {
        return Err(USAGE.to_string());
    }
    Ok(modes)
}

/// The argument list for one `chattr` run.
pub fn chattr_args(mode: &str, recursive: bool, files: &[String]) -> Vec<String> {
    let mut out = Vec::with_capacity(files.len() + 2);
    if recursive {
        out.push("-R".to_string());
    }
    out.push(mode.to_string());
    out.extend(files.iter().cloned());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_immutable_and_append_flags_map_to_chattr_letters() {
        assert_eq!(modes_for("uchg"), Ok(vec!["+i".to_string()]));
        assert_eq!(modes_for("nouchg"), Ok(vec!["-i".to_string()]));
        assert_eq!(modes_for("schg,uappnd"), Ok(vec!["+ia".to_string()]));
        assert_eq!(modes_for("nosappnd"), Ok(vec!["-a".to_string()]));
    }

    #[test]
    fn set_and_clear_flags_make_one_mode_each() {
        assert_eq!(modes_for("uchg,nodump"), Ok(vec!["+id".to_string()]));
        assert_eq!(modes_for("uchg,nouappnd"), Ok(vec!["+i".to_string(), "-a".to_string()]));
    }

    #[test]
    fn zero_clears_every_mapped_flag() {
        assert_eq!(modes_for("0"), Ok(vec!["-iad".to_string()]));
    }

    #[test]
    fn hidden_and_unknown_flags_are_refused_with_a_reason() {
        assert!(modes_for("hidden").unwrap_err().contains("leading dot"));
        assert!(modes_for("bogus").unwrap_err().contains("unknown flag bogus"));
    }

    #[test]
    fn the_command_line_reads_recursion_flags_and_files() {
        assert_eq!(
            parse(&args(&["-R", "uchg", "/a", "/b"])),
            Ok(Parsed::Run(Request { recursive: true, modes: vec!["+i".into()], files: vec!["/a".into(), "/b".into()] }))
        );
        assert_eq!(parse(&args(&["-h"])), Ok(Parsed::Help));
        assert_eq!(parse(&args(&["uchg"])), Err(USAGE.to_string()));
        assert!(parse(&args(&["-x", "uchg", "/a"])).unwrap_err().contains("-x"));
    }

    #[test]
    fn chattr_gets_recursion_then_mode_then_files() {
        assert_eq!(chattr_args("+i", true, &args(&["/a"])), args(&["-R", "+i", "/a"]));
        assert_eq!(chattr_args("-a", false, &args(&["/a", "/b"])), args(&["-a", "/a", "/b"]));
    }
}
