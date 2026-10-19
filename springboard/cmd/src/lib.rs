//! Shared helpers for the BifrOSt command-line tools. Each tool is its own binary in `src/bin/<tool>.rs` and keeps
//! its logic in a module here (`pub mod <tool>;`) so it can be unit tested.

pub mod defaults;
pub mod ditto;
pub mod audioctl;
pub mod b64;
pub mod bzegrep;
pub mod caffeinate;
pub mod certassist;
pub mod chflags;
pub mod control;
pub mod cu;
pub mod network;
pub mod notifyutil;
pub mod networksetup;
pub mod newsyslog;
pub mod diskutil;
pub mod firewall;
pub mod launchctl;
pub mod leaks;
pub mod leave;
pub mod lastcomm;
pub mod mdfind;
pub mod mdutil;
pub mod midictl;
pub mod neighbors;
pub mod mtree;
pub mod hdiutil;
pub mod ifconfig;
pub mod ipconfig;
pub mod iconutil;
pub mod open;
pub mod osascript;
pub mod osacompile;
pub mod plistbuddy;
pub mod plutil;
pub mod pasteboard;
pub mod ping6;
pub mod pmset;
pub mod route;
pub mod purge;
pub mod say;
pub mod shortcuts;
pub mod sips;
pub mod screencapture;
pub mod security;
pub mod smbutil;
pub mod profile;
pub mod scutil;
pub mod sw_vers;
pub mod softwareupdate;
pub mod textutil;
pub mod tiff2icns;
pub mod tiffutil;
pub mod tmutil;
pub mod unitctl;
pub mod units;
pub mod usdextract;
pub mod vis;
pub mod what;

/// Prints `msg` to stderr prefixed with the tool name and exits with `code`.
pub fn die(tool: &str, msg: &str, code: i32) -> ! {
    eprintln!("{tool}: {msg}");
    std::process::exit(code)
}

/// Runs `program` with `args`, then exits with its status (2 when it cannot start or was killed by a signal). Used by
/// the tools that translate their command line into a call of a Linux program (`leaks`, `smbutil`, `tiffutil`).
pub fn forward(tool: &str, program: &str, args: &[String]) -> ! {
    match std::process::Command::new(program).args(args).status() {
        Ok(status) => std::process::exit(status.code().unwrap_or(2)),
        Err(e) => die(tool, &format!("{program} could not run: {e}"), 2),
    }
}
