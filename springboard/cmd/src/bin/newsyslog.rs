//! `newsyslog`: rotates logs by `newsyslog.conf` rules, through logrotate. See `bifrost_cmd::newsyslog`.

use bifrost_cmd::die;
use bifrost_cmd::newsyslog::{self, USAGE};
use std::process::Command;

const DEFAULT_CONF: &str = "/etc/newsyslog.conf";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut print_only = false;
    let mut conf = DEFAULT_CONF.to_string();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-n" => print_only = true,
            "-f" => match it.next() {
                Some(path) => conf = path.clone(),
                None => die("newsyslog", "-f needs a file", 1),
            },
            "-h" | "--help" => {
                println!("{USAGE}");
                return;
            }
            other => die("newsyslog", &format!("unknown argument {other}\n{USAGE}"), 1),
        }
    }
    let text = std::fs::read_to_string(&conf).unwrap_or_else(|e| die("newsyslog", &format!("{conf}: {e}"), 1));
    let (rules, warnings) = newsyslog::parse(&text);
    for w in &warnings {
        eprintln!("newsyslog: {w}");
    }
    let config = newsyslog::render(&rules);
    if print_only {
        print!("{config}");
        return;
    }
    let path = std::env::temp_dir().join(format!("bifrost-newsyslog-{}.conf", std::process::id()));
    std::fs::write(&path, &config).unwrap_or_else(|e| die("newsyslog", &format!("{}: {e}", path.display()), 1));
    let status = Command::new("logrotate").arg(&path).status();
    let _ = std::fs::remove_file(&path);
    match status {
        Ok(s) if s.success() => {}
        Ok(s) => die("newsyslog", "logrotate failed", s.code().unwrap_or(1)),
        Err(e) => die("newsyslog", &format!("cannot run logrotate: {e}"), 1),
    }
}
