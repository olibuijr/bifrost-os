//! Browser memory preferences, kept separate from process startup and feature selection.
//! Module separation follows the approach in <https://github.com/mxmgorin/retsurf/blob/main/src/browser/memory.rs>;
//! its hardware tiers and GC settings are not copied into this embedder.
//!
//! Contracts are checked against Servo 0.7.0:
//! - <https://github.com/servo/servo/blob/v0.7.0/components/config/prefs.rs>
//! - <https://github.com/servo/servo/blob/v0.7.0/components/script/runtime/script_runtime.rs>
//! - <https://github.com/servo/servo/blob/v0.7.0/components/net/http_cache.rs>
//!
//! These limits cover individual engine subsystems, not total process memory. The matched
//! ten-navigation workload has not shown a whole-process memory improvement from this policy.

use servo::Preferences;

/// History entries on each side of the current one whose document stays alive (Servo's back-forward cache); an entry
/// further away is evicted and reloads when it is reached. A live document comes back exactly as it was left, with its
/// scroll offset: Servo 0.7.0 has no scroll restoration of its own (`history.scrollRestoration` is not implemented), so a
/// reloaded entry starts at the top. At 0 every entry was evicted and Back always returned to the top of the page, while
/// the ten-navigation workload measured no whole-process memory difference from it (README, "Performance budgets"), so a
/// few entries are kept. Navigation entries themselves are retained at any distance.
const LIVE_HISTORY_DISTANCE: i64 = 3;
/// MiB passed to the JavaScript runtime's JSGC_MAX_BYTES parameter. Servo accepts 1..256,
/// with an exclusive upper bound: 256 selects the unlimited fallback instead of a 256 MiB limit.
const JS_HEAP_MAX_MIB: i64 = 255;
/// HTTP memory-cache weight using Servo's UnitWeighter, not a byte or whole-profile limit.
const HTTP_CACHE_WEIGHT: u64 = 256;

pub(super) fn apply(preferences: &mut Preferences) {
    preferences.session_history_max_length = LIVE_HISTORY_DISTANCE;
    preferences.js_mem_max = JS_HEAP_MAX_MIB;
    preferences.network_http_cache_size = HTTP_CACHE_WEIGHT;
    // Preserve upstream GC modes and thresholds, including its disabled incremental GC.
    // The pinned source warns that enabling incremental GC can corrupt memory.
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Back and Forward restore the scroll offset only of an entry whose document is alive, and Servo evicts every
    /// entry when the distance is 0 (the page then reloads at the top).
    #[test]
    fn back_and_forward_keep_a_few_live_documents() {
        let mut preferences = Preferences::default();
        apply(&mut preferences);
        assert!((1..=20).contains(&preferences.session_history_max_length));
    }
}
