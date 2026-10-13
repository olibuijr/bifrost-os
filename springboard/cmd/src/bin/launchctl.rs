//! `launchctl`: lists and controls the user's background services through systemd's user manager. See
//! `bifrost_cmd::launchctl`.

use bifrost_cmd::die;
use bifrost_cmd::launchctl::{self, Command};
use std::process::Stdio;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = match launchctl::parse(&args) {
        Ok(cmd) => cmd,
        Err(e) => die("launchctl", &e, 1),
    };
    let Some(sub) = launchctl::systemctl_args(&cmd) else {
        println!("{}", launchctl::USAGE);
        return;
    };
    let out = match std::process::Command::new("systemctl").arg("--user").args(&sub).stderr(Stdio::piped()).output() {
        Ok(out) => out,
        Err(e) => die("launchctl", &format!("systemctl could not run: {e}"), 1),
    };
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let text = if stderr.is_empty() { "the service manager refused the request".to_string() } else { stderr };
        die("launchctl", &text, 1);
    }
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    match cmd {
        Command::List => print!("{}", launchctl::list_text(&stdout)),
        Command::Print(label) => match launchctl::print_text(&label, &stdout) {
            Ok(text) => print!("{text}"),
            Err(e) => die("launchctl", &e, 113),
        },
        _ => {}
    }
}
