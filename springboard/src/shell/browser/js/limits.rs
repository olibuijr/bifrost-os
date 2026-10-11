//! Runtime limits for a tab's Boa context, so a runaway script cannot eat the
//! CPU or the stack.
//!
//! Boa 0.22 exposes `RuntimeLimits` with exactly four knobs: loop iteration
//! limit, recursion limit, stack size limit and backtrace limit. All of the
//! first three are applied here. There is NO heap or allocation limit in the
//! engine, and `boa_gc`'s allocated-byte counter is private, so
//! `heap_bytes_estimate` returns None; the budget code estimates JS memory
//! from script and DOM size instead (`budget::TabMem::js_bytes`).

use boa_engine::Context;

/// Limits applied to a context.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Iterations of any single loop before a RuntimeLimit error is thrown
    /// (`u64::MAX` disables).
    pub loop_iterations: u64,
    /// Maximum JS call depth.
    pub recursion: usize,
    /// Maximum VM value-stack size (Boa's unit: stack slots).
    pub stack: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { loop_iterations: 5_000_000, recursion: 400, stack: 1024 * 10 }
    }
}

/// Applies `limits` to `ctx` and keeps the backtrace short (it is allocated per error).
pub fn apply_limits(ctx: &mut Context, limits: Limits) {
    let rl = ctx.runtime_limits_mut();
    if limits.loop_iterations == u64::MAX {
        rl.disable_loop_iteration_limit();
    } else {
        rl.set_loop_iteration_limit(limits.loop_iterations);
    }
    rl.set_recursion_limit(limits.recursion);
    rl.set_stack_size_limit(limits.stack);
    rl.set_backtrace_limit(10);
}

/// Engine heap size in bytes. Boa 0.22 offers no public accessor, so None.
pub fn heap_bytes_estimate(_ctx: &Context) -> Option<usize> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use boa_engine::Source;

    #[test]
    fn limits_are_applied() {
        let mut ctx = Context::default();
        apply_limits(&mut ctx, Limits { loop_iterations: 123, recursion: 45, stack: 6789 });
        let rl = ctx.runtime_limits();
        assert_eq!((rl.loop_iteration_limit(), rl.recursion_limit(), rl.stack_size_limit()), (123, 45, 6789));
    }

    #[test]
    fn runaway_loop_is_stopped() {
        let mut ctx = Context::default();
        apply_limits(&mut ctx, Limits { loop_iterations: 1000, ..Limits::default() });
        assert!(ctx.eval(Source::from_bytes("for (;;) {}")).is_err());
        assert!(ctx.eval(Source::from_bytes("var n = 0; for (var i = 0; i < 100; i++) n += i; n")).is_ok());
    }

    #[test]
    fn runaway_recursion_is_stopped() {
        let mut ctx = Context::default();
        apply_limits(&mut ctx, Limits::default());
        assert!(ctx.eval(Source::from_bytes("function f() { return f() + 1; } f()")).is_err());
    }

    #[test]
    fn no_heap_accessor() {
        assert_eq!(heap_bytes_estimate(&Context::default()), None);
    }
}
