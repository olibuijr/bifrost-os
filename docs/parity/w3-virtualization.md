# Parity wave 3: Virtualization (gap rows)

Branch `parity/w3-virtualization`. Pure Rust, std only, in `springboard/ui/src` (bifrost-ui). No builds were run;
the lead runs the integration build and tests.

## What was built

Three pure models for the guest virtual machine features that the matrix lists as gaps. None of them starts, pauses,
exports or mounts anything. The hypervisor layer drives them later.

- `springboard/ui/src/vm_state.rs` (`VirtualMachine`, `VmState`, `VmOperation`, `VmStateError`): the execution state
  of one guest. Start and restore need a stopped guest; pause, stop and request-stop need a running guest (stop also
  accepts paused); save needs a paused guest. Transitional states (starting, pausing, resuming, stopping, saving,
  restoring) wait for `complete(ok)`; a failed transition enters `Error`, from which start is allowed again.
- `springboard/ui/src/vm_share.rs` (`FileSystemDevice`, `DirectoryShare`, `SharedDirectory`, `validate_tag`,
  `validate_share`, `ShareError`): a device tag plus one share, either one directory or named directories, each
  read-only or read-write. A refused share leaves the previous share in place.
- `springboard/ui/src/vm_error.rs` (`VmErrorCode`, `VmError`): the 25 error codes of the official enumeration, their
  official one-sentence descriptions, the restore and device groupings, and a `Display` that appends detail text.

## Official API names mirrored

- Virtualization `VZVirtualMachine.State` (stopped, running, paused, error, starting, pausing, stopping, resuming,
  restoring, saving) and the lifecycle calls start, pause, resume, stop, request stop, save and restore state.
  Source: https://developer.apple.com/documentation/virtualization/vzvirtualmachine/state-swift.enum
- `VZError.Code` (25 cases listed in the official enumeration). Source:
  https://developer.apple.com/documentation/virtualization/vzerror/code
- `VZVirtioFileSystemDeviceConfiguration` (`init(tag:)`, `tag`, `share`, single and multiple directory shares).
  Source: https://developer.apple.com/documentation/virtualization/vzvirtiofilesystemdeviceconfiguration

The transition rules, the tag rule (1 to 36 bytes, ASCII letters, digits, `-`, `_`, `.`, no leading `-` or `.`) and the
name rule are BifrOSt's own model. The official pages do not state the tag or name rules, and they are not claimed as
Apple behaviour. The restore-and-save completion targets (both end paused) are also BifrOSt's rule.

## Official DocC and REA evidence

- `python3 -I tools/parity/docc.py virtualization/vzvirtualmachine/state-swift.enum` (official state list)
- `python3 -I tools/parity/docc.py virtualization/vzerror` and `virtualization/vzerror/code` (error codes)
- `python3 -I tools/parity/docc.py virtualization/vzvirtiofilesystemdeviceconfiguration` (tag and shares)
- REA evidence of the rendered pages:
  - `ev_9475857ad84702564651741157b918f6aeee1c4e242e661f18477714bfe8bc50` (VZVirtualMachine.State)
  - `ev_b9d9e3734b22652b46ca78a46fe3a9156fb7fb7479d04a888b2adeaa15fdce82` (VZError.Code)
  - `ev_9329dccdb755151ea949a0f066f4d744da722df971466bdafaf7aaf5a913e821` (VZVirtioFileSystemDeviceConfiguration)

## Matrix rows closed

Framework slug `virtualization`. Status moves to partial: each model covers the state, share or error part only, not the
guest runtime, display, devices or sockets.

| Framework slug | Feature (as written in data/virtualization.json) | New status | BifrOSt path |
|---|---|---|---|
| virtualization | VM runtime (start, pause, resume, state) | partial | springboard/ui/src/vm_state.rs |
| virtualization | Shared directories | partial | springboard/ui/src/vm_share.rs |
| virtualization | Errors and enumerations | partial | springboard/ui/src/vm_error.rs |

## Remaining in this framework

Running a guest from an app, the guest display view, device configuration (storage, network, entropy, balloon, serial),
clipboard bridging, sockets, audio and USB stay gaps. Each needs the hypervisor or a guest driver, which pure Rust
cannot provide here.
