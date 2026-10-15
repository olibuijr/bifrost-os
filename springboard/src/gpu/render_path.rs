//! Which render path a surface uses: the GPU when there is one, the CPU when it is forced or when the GPU is missing.
//! The choice is a pure function of the `BIFROST_GPU` setting and whether a GPU is present, so a test can drive every
//! case without a device. The compositor and an app read the same rule.

/// The path frames are rendered on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderPath {
    /// Render on the GPU and present through the GPU presenter.
    Gpu,
    /// Render on the CPU into shared memory.
    Cpu,
}

/// Why a path was chosen, so a log line can say whether the fallback was the user's choice or a missing device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    /// The GPU is used because it is present and nothing forbids it.
    GpuAvailable,
    /// The setting asks for the CPU path (`BIFROST_GPU=0`).
    ForcedByUser,
    /// There is no GPU to use, so the CPU path is the only one left.
    GpuMissing,
}

/// A chosen path and the reason for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Choice {
    pub path: RenderPath,
    pub reason: Reason,
}

/// Chooses the render path. `setting` is the value of `BIFROST_GPU`: `Some("0")` forces the CPU; any other value,
/// or none, leaves the choice to the hardware. A forced CPU path never looks at the GPU.
pub fn choose(setting: Option<&str>, gpu_present: bool) -> Choice {
    if setting.map(str::trim) == Some("0") {
        return Choice { path: RenderPath::Cpu, reason: Reason::ForcedByUser };
    }
    if gpu_present {
        Choice { path: RenderPath::Gpu, reason: Reason::GpuAvailable }
    } else {
        Choice { path: RenderPath::Cpu, reason: Reason::GpuMissing }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_present_gpu_is_used_when_nothing_forbids_it() {
        assert_eq!(choose(None, true), Choice { path: RenderPath::Gpu, reason: Reason::GpuAvailable });
        assert_eq!(choose(Some("1"), true).path, RenderPath::Gpu);
    }

    #[test]
    fn the_setting_zero_forces_the_cpu_even_with_a_gpu() {
        assert_eq!(choose(Some("0"), true), Choice { path: RenderPath::Cpu, reason: Reason::ForcedByUser });
        assert_eq!(choose(Some(" 0 "), true).reason, Reason::ForcedByUser);
    }

    #[test]
    fn a_missing_gpu_falls_back_to_the_cpu_and_says_so() {
        assert_eq!(choose(None, false), Choice { path: RenderPath::Cpu, reason: Reason::GpuMissing });
        assert_eq!(choose(Some("1"), false).reason, Reason::GpuMissing);
    }

    #[test]
    fn an_unrecognised_setting_is_not_a_request_for_the_cpu() {
        assert_eq!(choose(Some("false"), true).path, RenderPath::Gpu);
        assert_eq!(choose(Some(""), true).path, RenderPath::Gpu);
    }
}
