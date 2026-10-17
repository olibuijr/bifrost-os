//! Shared helpers for the BifrOSt command-line tools. Each tool is its own binary in `src/bin/<tool>.rs` and keeps
//! its logic in a module here (`pub mod <tool>;`) so it can be unit tested.

pub mod defaults;
pub mod ditto;
pub mod b64;
pub mod bzegrep;
pub mod caffeinate;
pub mod chflags;
pub mod control;
pub mod cu;
pub mod network;
pub mod notifyutil;
pub mod networksetup;
pub mod newsyslog;
pub mod diskutil;
pub mod launchctl;
pub mod leave;
pub mod lastcomm;
pub mod mdfind;
pub mod mdutil;
pub mod neighbors;
pub mod mtree;
pub mod hdiutil;
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
pub mod profile;
pub mod scutil;
pub mod sw_vers;
pub mod softwareupdate;
pub mod textutil;
pub mod tiff2icns;
pub mod tmutil;
pub mod units;
pub mod vis;
pub mod what;

/// Prints `msg` to stderr prefixed with the tool name and exits with `code`.
pub fn die(tool: &str, msg: &str, code: i32) -> ! {
    eprintln!("{tool}: {msg}");
    std::process::exit(code)
}
