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

/// Reload documents reached through Back/Forward instead of retaining live document pipelines.
/// Navigation entries remain available; this trades retained page state for reloads.
const LIVE_HISTORY_DISTANCE: i64 = 0;
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
