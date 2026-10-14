//! Swap growth plan: how many bytes of extra swap to add when memory is short and the swap already in use is nearly full.
//!
//! The rule is pure. It reports growth only when less than a tenth of RAM is available and at most a quarter of
//! the swap is free, and never grows swap past a cap. Creating and enabling the swap file is the caller's job.

/// A snapshot of memory and swap, in bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SwapState {
    /// Physical memory installed.
    pub ram_total: u64,
    /// Memory the system can give to applications without swapping.
    pub ram_available: u64,
    /// Swap space configured.
    pub swap_total: u64,
    /// Swap space not in use.
    pub swap_free: u64,
}

/// How many bytes of swap to add now. `chunk` is the size of one new swap file and `cap` is the most swap the
/// system may hold in total. Returns zero when memory is not short, when swap still has room, or when the cap is
/// already reached.
pub fn growth_bytes(state: SwapState, chunk: u64, cap: u64) -> u64 {
    let ram_short = state.ram_available.saturating_mul(10) < state.ram_total;
    let swap_short = state.swap_free.saturating_mul(4) <= state.swap_total;
    if !(ram_short && swap_short) {
        return 0;
    }
    chunk.min(cap.saturating_sub(state.swap_total))
}
