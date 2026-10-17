//! `newsyslog`: rotates log files by the rules in a `newsyslog.conf` file, by translating each rule into a logrotate
//! stanza and running logrotate on it. Rule format (one per line, `#` starts a comment):
//!
//! `logfile  [owner:group]  mode  count  size  when  flags  [pid_file [signal]]`
//!
//! Translated: `count` (kept rotations), `size` in kilobytes (`*` means none), `when` (`$D0` daily, `$W0` weekly,
//! `$M1` monthly, `@T..` daily at a time, which is treated as daily), `Z` (gzip) and `J` (bzip2), `N` (no signal, no
//! change), `C` (create the new file), `mode` and the `owner:group`. Gap: `B` (binary), `X`, `G` and `M` flags are not
//! translated and are reported, and the signal and pid file are not used because logrotate runs the postrotate hook of
//! the daemon instead.

use std::fmt::Write as _;

pub const USAGE: &str = "usage: newsyslog [-n] [-f newsyslog.conf]\n  -n  print the logrotate configuration instead of running it";

/// One translated rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub path: String,
    pub owner: Option<String>,
    pub mode: Option<String>,
    pub count: u32,
    /// Rotate above this many kilobytes; `None` for `*`.
    pub size_kb: Option<u64>,
    pub when: When,
    pub compress: Compress,
    pub create: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum When {
    Daily,
    Weekly,
    Monthly,
    /// Size only: no calendar schedule.
    Never,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compress {
    None,
    Gzip,
    Bzip2,
}

/// Parses a `newsyslog.conf` text. Returns the rules and the warnings for lines or flags that were not translated.
pub fn parse(text: &str) -> (Vec<Rule>, Vec<String>) {
    let mut rules = Vec::new();
    let mut warnings = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        match parse_line(line, &mut warnings) {
            Ok(rule) => rules.push(rule),
            Err(e) => warnings.push(format!("line {}: {e}", i + 1)),
        }
    }
    (rules, warnings)
}

fn parse_line(line: &str, warnings: &mut Vec<String>) -> Result<Rule, String> {
    let f: Vec<&str> = line.split_whitespace().collect();
    if f.len() < 6 {
        return Err("expected at least: logfile mode count size when flags".into());
    }
    let (owner, rest) = if f[1].contains(':') { (Some(f[1].to_string()), &f[2..]) } else { (None, &f[1..]) };
    if rest.len() < 5 {
        return Err("expected: mode count size when flags".into());
    }
    let mode = match rest[0] {
        "-" => None,
        m => Some(m.to_string()),
    };
    let count: u32 = rest[1].parse().map_err(|_| format!("bad count {}", rest[1]))?;
    let size_kb = match rest[2] {
        "*" => None,
        s => Some(s.parse::<u64>().map_err(|_| format!("bad size {s}"))?),
    };
    let when = match rest[3] {
        "*" => When::Never,
        w if w.starts_with("$W") => When::Weekly,
        w if w.starts_with("$M") => When::Monthly,
        w if w.starts_with("$D") || w.starts_with("@") => When::Daily,
        w => return Err(format!("bad when {w}")),
    };
    let mut compress = Compress::None;
    let mut create = false;
    if rest[4] != "-" {
        for flag in rest[4].chars() {
            match flag {
                'Z' => compress = Compress::Gzip,
                'J' => compress = Compress::Bzip2,
                'C' => create = true,
                'N' => {}
                other => warnings.push(format!("{}: flag {other} is not translated", f[0])),
            }
        }
    }
    Ok(Rule { path: f[0].to_string(), owner, mode, count, size_kb, when, compress, create })
}

/// Renders the rules as a logrotate configuration.
pub fn render(rules: &[Rule]) -> String {
    let mut out = String::new();
    for r in rules {
        let _ = writeln!(out, "{} {{", r.path);
        let _ = writeln!(out, "    rotate {}", r.count);
        match r.when {
            When::Daily => out.push_str("    daily\n"),
            When::Weekly => out.push_str("    weekly\n"),
            When::Monthly => out.push_str("    monthly\n"),
            When::Never => {}
        }
        if let Some(kb) = r.size_kb {
            let _ = writeln!(out, "    size {kb}k");
        }
        match r.compress {
            Compress::None => {}
            Compress::Gzip => out.push_str("    compress\n"),
            Compress::Bzip2 => out.push_str("    compress\n    compresscmd /usr/bin/bzip2\n    uncompresscmd /usr/bin/bunzip2\n    compressext .bz2\n"),
        }
        // logrotate's `create [mode owner group]` takes the mode and owner in one line.
        match (&r.mode, &r.owner, r.create) {
            (Some(mode), Some(owner), _) => {
                let _ = writeln!(out, "    create {mode} {}", owner.replace(':', " "));
            }
            (Some(mode), None, _) => {
                let _ = writeln!(out, "    create {mode}");
            }
            (None, _, true) => out.push_str("    create\n"),
            (None, _, false) => {}
        }
        if let Some(owner) = &r.owner {
            let _ = writeln!(out, "    su {}", owner.replace(':', " "));
        }
        out.push_str("}\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_daily_compressed_rule_with_size_translates() {
        let (rules, warnings) = parse("/var/log/system.log  root:admin  640  7  *  $D0  Z  /var/run/syslog.pid\n");
        assert!(warnings.is_empty());
        assert_eq!(rules.len(), 1);
        let r = &rules[0];
        assert_eq!(r.path, "/var/log/system.log");
        assert_eq!(r.owner.as_deref(), Some("root:admin"));
        assert_eq!(r.mode.as_deref(), Some("640"));
        assert_eq!(r.count, 7);
        assert_eq!(r.size_kb, None);
        assert_eq!(r.when, When::Daily);
        assert_eq!(r.compress, Compress::Gzip);
    }

    #[test]
    fn a_size_rule_without_owner_and_with_dash_mode() {
        let (rules, _) = parse("/var/log/app.log  -  2  1000  *  BC\n");
        let r = &rules[0];
        assert_eq!(r.owner, None);
        assert_eq!(r.mode, None);
        assert_eq!(r.size_kb, Some(1000));
        assert_eq!(r.when, When::Never);
        assert!(r.create);
    }

    #[test]
    fn comments_blank_lines_and_untranslated_flags_are_reported() {
        let (rules, warnings) = parse("# header\n\n/var/log/x.log 644 3 * $W0 XB\n");
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].when, When::Weekly);
        assert_eq!(warnings.len(), 2);
        assert!(warnings[0].contains("flag X"));
    }

    #[test]
    fn short_or_bad_lines_are_skipped_with_a_line_number() {
        let (rules, warnings) = parse("/a 644 3\n/b 644 x * $D0 -\n");
        assert!(rules.is_empty());
        assert!(warnings[0].starts_with("line 1"));
        assert!(warnings[1].contains("bad count"));
    }

    #[test]
    fn render_writes_a_logrotate_stanza() {
        let (rules, _) = parse("/var/log/a.log root:wheel 600 4 500 $M1 ZC\n");
        let text = render(&rules);
        assert!(text.starts_with("/var/log/a.log {\n"));
        assert!(text.contains("    rotate 4\n"));
        assert!(text.contains("    monthly\n"));
        assert!(text.contains("    size 500k\n"));
        assert!(text.contains("    compress\n"));
        assert!(text.contains("    create 600 root wheel\n"));
        assert!(text.contains("    su root wheel\n"));
        assert!(text.ends_with("}\n"));
    }
}
