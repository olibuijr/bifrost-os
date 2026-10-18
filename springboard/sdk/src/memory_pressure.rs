//! `bifrost_app::memory_pressure`: the level bifrostd last published (`/run/bifrost/memory-pressure`), for code that
//! reads it when it needs it. To be told about changes, register a handler with `DispatchSource::memory_pressure`
//! (dispatch.rs): that runs on the in-process PSI watcher and works without the daemon.

pub use bifrost_render::purgeable::MemoryPressure;

/// The file bifrostd writes the level to (`springboard/bifrostd/src/pager.rs`, `LEVEL_PATH`).
pub const LEVEL_PATH: &str = "/run/bifrost/memory-pressure";

/// Parses the word bifrostd writes: `normal`, `warn` or `critical` (surrounding whitespace is ignored).
pub fn parse_level(text: &str) -> Option<MemoryPressure> {
    match text.trim() {
        "normal" => Some(MemoryPressure::Normal),
        "warn" => Some(MemoryPressure::Warning),
        "critical" => Some(MemoryPressure::Critical),
        _ => None,
    }
}

/// The level bifrostd last published. Normal when the file is missing (bifrostd not running, or no pressure yet).
pub fn published() -> MemoryPressure {
    std::fs::read_to_string(LEVEL_PATH).ok().and_then(|t| parse_level(&t)).unwrap_or(MemoryPressure::Normal)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_published_words_map_to_levels() {
        assert_eq!(parse_level("normal\n"), Some(MemoryPressure::Normal));
        assert_eq!(parse_level("warn\n"), Some(MemoryPressure::Warning));
        assert_eq!(parse_level(" critical "), Some(MemoryPressure::Critical));
        assert_eq!(parse_level("warning"), None, "bifrostd writes warn, not warning");
        assert_eq!(parse_level(""), None);
    }
}
