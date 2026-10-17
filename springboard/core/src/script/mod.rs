//! BifrOSt scripting: the scripting dictionaries apps declare in their manifest (`dictionary`), and a small
//! AppleScript-style language (`lexer`, `parser`, `ast`, `interp`) that sends dictionary commands to apps as typed
//! events. The `osascript` and `osacompile` tools in `springboard/cmd` are its front ends.

pub mod dictionary;
