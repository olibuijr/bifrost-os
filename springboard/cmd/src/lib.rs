//! Shared helpers for the BifrOSt command-line tools. Each tool is its own binary in `src/bin/<tool>.rs` and keeps
//! its logic in a module here (`pub mod <tool>;`) so it can be unit tested.

pub mod hdiutil;
pub mod open;
pub mod pasteboard;

/// Prints `msg` to stderr prefixed with the tool name and exits with `code`.
pub fn die(tool: &str, msg: &str, code: i32) -> ! {
    eprintln!("{tool}: {msg}");
    std::process::exit(code)
}
