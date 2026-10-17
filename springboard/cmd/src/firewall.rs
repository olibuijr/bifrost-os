//! `bifrost-firewall`: the application firewall's policy file and its boot step.
//!
//! The policy file (`/etc/bifrost/app-firewall.conf`, format in `bifrost_core::app_firewall`) holds one inbound policy
//! per app. `apply` turns it into an nftables ruleset and loads it with `nft -f -`; the `bifrost-firewall.service`
//! unit runs it at boot. `set`, `remove`, `list` and `show` edit and read the file, so Settings and a terminal write
//! the same policies. A file that does not parse, or a policy that would allow and block one port, is refused and
//! the rules in place stay as they were.

use bifrost_core::app_firewall::{self as fw, AppPolicy, Policy};
use std::io::Write;
use std::path::Path;
use std::process::{Command as Process, Stdio};

pub const POLICY_FILE: &str = "/etc/bifrost/app-firewall.conf";

pub const USAGE: &str = "usage: bifrost-firewall [-f FILE] apply | show | list | clear\n       bifrost-firewall [-f FILE] set APP allow|block [tcp/N,udp/N,...]\n       bifrost-firewall [-f FILE] remove APP";

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// Load the ruleset for the policy file.
    Apply,
    /// Print the ruleset the file would load.
    Show,
    /// Print the policies in the file.
    List,
    /// Remove the firewall's table.
    Clear,
    Set(AppPolicy),
    Remove(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Options {
    pub file: String,
    pub action: Action,
}

#[derive(Debug, PartialEq)]
pub enum Parsed {
    Help,
    Run(Options),
}

pub fn parse(args: &[String]) -> Result<Parsed, String> {
    let mut file = POLICY_FILE.to_string();
    let mut rest: Vec<&str> = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-h" | "--help" => return Ok(Parsed::Help),
            "-f" => file = it.next().ok_or("-f needs a file")?.clone(),
            other => rest.push(other),
        }
    }
    let action = match rest.as_slice() {
        ["apply"] => Action::Apply,
        ["show"] => Action::Show,
        ["list"] => Action::List,
        ["clear"] => Action::Clear,
        ["remove", app] => Action::Remove((*app).to_string()),
        ["set", app, policy] | ["set", app, policy, _] => {
            let policy = match *policy {
                "allow" => Policy::Allow,
                "block" => Policy::Block,
                other => return Err(format!("policy must be allow or block, not {other}")),
            };
            let ports = fw::parse_port_list(rest.get(3).copied().unwrap_or("-"))?;
            Action::Set(AppPolicy { app_id: (*app).to_string(), policy, ports })
        }
        [] => return Err("no command".into()),
        other => return Err(format!("cannot understand: {}", other.join(" "))),
    };
    Ok(Parsed::Run(Options { file, action }))
}

/// Feeds a ruleset script to `nft -f -`.
pub fn nft_load(script: &str) -> Result<(), String> {
    let mut child = Process::new("nft")
        .args(["-f", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot run nft: {e}"))?;
    child.stdin.take().ok_or("nft has no stdin")?.write_all(script.as_bytes()).map_err(|e| e.to_string())?;
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!("nft refused the ruleset: {}", String::from_utf8_lossy(&out.stderr).trim()))
    }
}

fn read_policies(file: &Path) -> Result<Vec<AppPolicy>, String> {
    match std::fs::read_to_string(file) {
        Ok(text) => fw::parse_policies(&text),
        // no file is no policies: the firewall then only sets the accept default
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(format!("{}: {e}", file.display())),
    }
}

/// Runs one action. `load` receives the nftables script for `apply` and `clear`, so tests do not need nft. Returns the
/// text to print.
pub fn run(options: &Options, load: &dyn Fn(&str) -> Result<(), String>) -> Result<String, String> {
    let file = Path::new(&options.file);
    match &options.action {
        Action::Apply => {
            let text = match std::fs::read_to_string(file) {
                Ok(t) => t,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
                Err(e) => return Err(format!("{}: {e}", file.display())),
            };
            let script = fw::plan_from_file(&text)?;
            load(&script)?;
            Ok(format!("applied {} policies\n", fw::parse_policies(&text)?.len()))
        }
        Action::Show => fw::plan_from_file(&std::fs::read_to_string(file).unwrap_or_default()),
        Action::List => Ok(fw::format_policies(&read_policies(file)?)),
        Action::Clear => {
            load(&fw::clear_script())?;
            Ok("cleared\n".into())
        }
        Action::Set(policy) => {
            let mut policies = read_policies(file)?;
            fw::set_policy(&mut policies, policy.clone()).map_err(|e| format!("{e:?}"))?;
            write_policies(file, &policies)?;
            Ok(format!("{} set\n", policy.app_id))
        }
        Action::Remove(app) => {
            let mut policies = read_policies(file)?;
            if !fw::remove_policy(&mut policies, app) {
                return Err(format!("{app} has no policy"));
            }
            write_policies(file, &policies)?;
            Ok(format!("{app} removed\n"))
        }
    }
}

/// Writes the file through a temporary name in the same folder, so a crash never leaves half a file.
fn write_policies(file: &Path, policies: &[AppPolicy]) -> Result<(), String> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let tmp = file.with_extension("tmp");
    std::fs::write(&tmp, fw::format_policies(policies)).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, file).map_err(|e| format!("{}: {e}", file.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    fn tmp(name: &str) -> String {
        let dir = std::env::temp_dir().join(format!("bifrost-firewall-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("app-firewall.conf").to_string_lossy().into_owned()
    }

    #[test]
    fn parse_reads_actions_and_the_file_option() {
        assert_eq!(parse(&args("-h")), Ok(Parsed::Help));
        match parse(&args("-f /x/y apply")).unwrap() {
            Parsed::Run(o) => assert_eq!((o.file.as_str(), o.action), ("/x/y", Action::Apply)),
            _ => panic!(),
        }
        match parse(&args("set is.olibuijr.web block tcp/80,tcp/443")).unwrap() {
            Parsed::Run(Options { file, action: Action::Set(p) }) => {
                assert_eq!(file, POLICY_FILE);
                assert_eq!((p.app_id.as_str(), p.policy, p.ports.len()), ("is.olibuijr.web", Policy::Block, 2));
            }
            _ => panic!(),
        }
        assert!(parse(&args("set a maybe")).is_err());
        assert!(parse(&args("set a block 80")).is_err());
        assert!(parse(&args("")).is_err());
        assert!(parse(&args("frobnicate")).is_err());
    }

    #[test]
    fn set_writes_the_file_and_apply_loads_its_ruleset() {
        let file = tmp("apply");
        let none = |_: &str| -> Result<(), String> { Ok(()) };
        let run_args = |a: &str, load: &dyn Fn(&str) -> Result<(), String>| match parse(&args(&format!("-f {file} {a}"))).unwrap() {
            Parsed::Run(o) => run(&o, load),
            Parsed::Help => unreachable!(),
        };
        run_args("set web.app block tcp/8080", &none).unwrap();
        assert_eq!(run_args("list", &none).unwrap(), "web.app block tcp/8080\n");
        let seen = RefCell::new(String::new());
        let capture = |s: &str| -> Result<(), String> {
            *seen.borrow_mut() = s.to_string();
            Ok(())
        };
        assert_eq!(run_args("apply", &capture).unwrap(), "applied 1 policies\n");
        assert!(seen.borrow().contains("tcp dport 8080 drop comment \"web.app\""));
        assert!(run_args("show", &none).unwrap().contains("policy accept"));
    }

    #[test]
    fn a_conflicting_set_is_refused_and_the_file_stays() {
        let file = tmp("conflict");
        let none = |_: &str| -> Result<(), String> { Ok(()) };
        let go = |a: &str| match parse(&args(&format!("-f {file} {a}"))).unwrap() {
            Parsed::Run(o) => run(&o, &none),
            Parsed::Help => unreachable!(),
        };
        go("set a.one block tcp/22").unwrap();
        assert!(go("set b.two allow tcp/22").is_err());
        assert_eq!(go("list").unwrap(), "a.one block tcp/22\n");
        assert!(go("remove nobody").is_err());
        go("remove a.one").unwrap();
        assert_eq!(go("list").unwrap(), "");
    }

    #[test]
    fn a_missing_file_applies_the_empty_ruleset_and_a_bad_file_applies_nothing() {
        let file = tmp("missing");
        let calls = RefCell::new(0);
        let count = |_: &str| -> Result<(), String> {
            *calls.borrow_mut() += 1;
            Ok(())
        };
        let opts = |f: &str| Options { file: f.to_string(), action: Action::Apply };
        assert!(run(&opts(&file), &count).is_ok());
        assert_eq!(*calls.borrow(), 1);
        std::fs::write(&file, "x.y maybe -\n").unwrap();
        assert!(run(&opts(&file), &count).is_err());
        assert_eq!(*calls.borrow(), 1, "nft is not called for a bad file");
    }

    #[test]
    fn clear_loads_the_delete_script() {
        let seen = RefCell::new(String::new());
        let capture = |s: &str| -> Result<(), String> {
            *seen.borrow_mut() = s.to_string();
            Ok(())
        };
        run(&Options { file: "/nonexistent".into(), action: Action::Clear }, &capture).unwrap();
        assert!(seen.borrow().contains("delete table"));
    }
}
