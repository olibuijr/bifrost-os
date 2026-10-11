"""SM-X200 hardware acceptance contract; consumes observations, never touches devices.

Mock observations exercise the contract only. Device adapters remain to be built.
"""
from dataclasses import dataclass

# Measurements required by docs/arch-hardware.md. No probe or stimulus runs here.
CHECKS = {
    'cpu_memory': ('eight_cores', 'memory_stable', 'frequency_changes'),
    'display': ('mode_1200x1920', 'brightness', 'blank_resume'),
    'touch_keys': ('full_panel', 'multitouch', 'power_volume', 'wake'),
    'gpu': ('native_egl', 'gpu_workload', 'allocation_stable'),
    'storage': ('emmc_checksums', 'sd_checksums', 'sd_hotplug', 'root_mount'),
    'usb': ('gadget', 'otg', 'reconnect', 'charge_data_transition'),
    'wifi': ('scan', 'wpa', 'traffic', 'resume'),
    'bluetooth': ('discovery', 'pairing', 'hid', 'resume'),
    'audio': ('four_speakers', 'headphone_route', 'jack_detection', 'microphone'),
    'camera_media': ('front_capture', 'rear_capture', 'native_pipeline', 'decode_encode'),
    'sensors': ('populated_devices_identified', 'native_samples', 'calibration'),
    'battery_thermal': ('charge_reporting', 'charge_cycle', 'thermal_throttling', 'reconnect'),
    'suspend_rtc': ('repeated_wake', 'peripheral_resume', 'idle_drain', 'rtc'),
    'recovery_update': ('usb_restore', 'update', 'rollback'),
}


@dataclass(frozen=True)
class Observation:
    source: str  # mock, emulator or device
    model: str
    artifact_sha256: str
    driver_ready: bool
    firmware_ready: bool
    module_abi_matches: bool
    checks: dict
    evidence: str = ''


def validate(subsystem, observation):
    """Return blockers. Strict booleans prevent strings like 'false' passing."""
    required = CHECKS[subsystem]
    errors = []
    if observation.model != 'SM-X200':
        errors.append('wrong device model')
    if observation.source not in ('mock', 'emulator', 'device'):
        errors.append('unknown evidence source')
    digest = observation.artifact_sha256
    if len(digest) != 64 or any(c not in '0123456789abcdef' for c in digest):
        errors.append('missing/invalid artifact SHA256')
    for field in ('driver_ready', 'firmware_ready', 'module_abi_matches'):
        if getattr(observation, field) is not True:
            errors.append(field)
    for check in required:
        if observation.checks.get(check) is not True:
            errors.append(check)
    return errors


def release_blockers(observations):
    """Only complete, artifact-consistent physical results can clear this gate.

Evidence paths are references for human review, not proof of authenticity.
"""
    errors = []
    digests = set()
    for subsystem in CHECKS:
        observation = observations.get(subsystem)
        if observation is None:
            errors.append(f'{subsystem}: untested')
            continue
        errors.extend(f'{subsystem}: {e}' for e in validate(subsystem, observation))
        if observation.source != 'device':
            errors.append(f'{subsystem}: physical device evidence required')
        if not observation.evidence.strip():
            errors.append(f'{subsystem}: missing evidence reference')
        digests.add(observation.artifact_sha256)
    if len(digests) > 1:
        errors.append('results refer to different candidate artifacts')
    return errors
