//! Shared helpers for the BifrOSt command-line tools. Each tool is its own binary in `src/bin/<tool>.rs` and keeps
//! its logic in a module here (`pub mod <tool>;`) so it can be unit tested.

pub mod network;
pub mod networksetup;
pub mod open;
pub mod pasteboard;
pub mod profile;
pub mod scutil;
pub mod sw_vers;

/// Prints `msg` to stderr prefixed with the tool name and exits with `code`.
pub fn die(tool: &str, msg: &str, code: i32) -> ! {
    eprintln!("{tool}: {msg}");
    std::process::exit(code)
}
