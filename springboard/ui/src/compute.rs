//! Where a model's operations run (`MLComputeDevice`, `MLComputePolicy`): the devices present, the devices a policy
//! allows, and the device one operation is placed on. This only decides placement. The GPU presenter draws the
//! screen and is not used for model work here. This platform has no Neural Engine, so none is offered.

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Device {
    Cpu,
    Gpu,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Policy {
    /// Every device present, the GPU first and the CPU as the fallback.
    #[default]
    Automatic,
    /// The CPU only, even when a GPU is present.
    CpuOnly,
}

impl Policy {
    /// The devices this policy may use, in order of preference, given the devices present.
    pub fn allowed(self, available: &[Device]) -> Vec<Device> {
        let order: &[Device] = match self {
            Policy::Automatic => &[Device::Gpu, Device::Cpu],
            Policy::CpuOnly => &[Device::Cpu],
        };
        order.iter().copied().filter(|d| available.contains(d)).collect()
    }
}

/// The device for one operation: the first device the policy allows that the operation can run on.
pub fn place(policy: Policy, available: &[Device], supported: &[Device]) -> Option<Device> {
    policy.allowed(available).into_iter().find(|d| supported.contains(d))
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOTH: [Device; 2] = [Device::Cpu, Device::Gpu];

    #[test]
    fn cpu_only_never_allows_the_gpu() {
        assert_eq!(Policy::CpuOnly.allowed(&BOTH), vec![Device::Cpu]);
        assert_eq!(Policy::CpuOnly.allowed(&[Device::Gpu]), Vec::<Device>::new());
    }

    #[test]
    fn automatic_prefers_the_gpu_and_falls_back_to_the_cpu() {
        assert_eq!(Policy::Automatic.allowed(&BOTH), vec![Device::Gpu, Device::Cpu]);
        assert_eq!(Policy::Automatic.allowed(&[Device::Cpu]), vec![Device::Cpu]);
    }

    #[test]
    fn nothing_present_allows_nothing() {
        assert!(Policy::Automatic.allowed(&[]).is_empty());
        assert_eq!(place(Policy::Automatic, &[], &BOTH), None);
    }

    #[test]
    fn an_operation_goes_to_the_first_allowed_device_it_supports() {
        assert_eq!(place(Policy::Automatic, &BOTH, &BOTH), Some(Device::Gpu));
        assert_eq!(
            place(Policy::Automatic, &BOTH, &[Device::Cpu]),
            Some(Device::Cpu),
            "an operation without GPU support falls back to the CPU"
        );
    }

    #[test]
    fn an_operation_no_allowed_device_supports_is_not_placed() {
        assert_eq!(place(Policy::CpuOnly, &BOTH, &[Device::Gpu]), None);
        assert_eq!(place(Policy::CpuOnly, &BOTH, &[Device::Cpu]), Some(Device::Cpu));
    }
}
