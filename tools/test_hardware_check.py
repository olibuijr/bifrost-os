"""Fault-injection mocks, not tests of Samsung silicon or a compiled kernel."""
import unittest
from dataclasses import replace
from unittest.mock import Mock

from hardware_check import CHECKS, Observation, release_blockers, validate


class HardwareContractTests(unittest.TestCase):
    def setUp(self):
        self.probe = Mock()
        self.probe.observe.side_effect = lambda subsystem: Observation(
            'mock', 'SM-X200', 'a' * 64, True, True, True,
            dict.fromkeys(CHECKS[subsystem], True))

    def test_every_subsystem_accepts_successful_mock(self):
        for subsystem in CHECKS:
            with self.subTest(subsystem=subsystem):
                self.assertEqual(validate(subsystem, self.probe.observe(subsystem)), [])

    def test_each_behavioral_failure_is_detected(self):
        for subsystem, checks in CHECKS.items():
            for check in checks:
                with self.subTest(subsystem=subsystem, check=check):
                    obs = self.probe.observe(subsystem)
                    obs.checks[check] = False
                    self.assertIn(check, validate(subsystem, obs))

    def test_missing_measurements_and_string_booleans_fail_closed(self):
        for subsystem in CHECKS:
            obs = self.probe.observe(subsystem)
            for value in ({}, dict.fromkeys(CHECKS[subsystem], 'false')):
                with self.subTest(subsystem=subsystem, value=value):
                    self.assertTrue(validate(subsystem, replace(obs, checks=value)))

    def test_driver_firmware_and_module_faults(self):
        for subsystem in CHECKS:
            for field in ('driver_ready', 'firmware_ready', 'module_abi_matches'):
                with self.subTest(subsystem=subsystem, field=field):
                    obs = replace(self.probe.observe(subsystem), **{field: False})
                    self.assertIn(field, validate(subsystem, obs))

    def test_virgl_cannot_stand_in_for_native_mali(self):
        obs = self.probe.observe('gpu')
        obs.checks['native_egl'] = False
        obs.checks['virgl_egl'] = True
        self.assertIn('native_egl', validate('gpu', obs))

    def test_mock_and_emulator_passes_never_clear_release(self):
        for source in ('mock', 'emulator'):
            observations = {s: replace(self.probe.observe(s), source=source,
                                      evidence='mock-log') for s in CHECKS}
            blockers = release_blockers(observations)
            self.assertEqual(sum('physical device' in e for e in blockers), len(CHECKS))

    def test_missing_hardware_blocks_release(self):
        self.assertEqual(len(release_blockers({})), len(CHECKS))

    def test_mixed_artifacts_and_missing_logs_block_release(self):
        observations = {s: replace(self.probe.observe(s), source='device',
                                   evidence='fixture-log') for s in CHECKS}
        # Synthetic device-labelled records exercise the gate, not real hardware.
        self.assertEqual(release_blockers(observations), [])
        observations['gpu'] = replace(observations['gpu'], artifact_sha256='b'*64, evidence='')
        blockers = release_blockers(observations)
        self.assertIn('results refer to different candidate artifacts', blockers)
        self.assertIn('gpu: missing evidence reference', blockers)

    def test_wrong_model_and_bad_hash_rejected(self):
        obs = replace(self.probe.observe('display'), model='SM-X205', artifact_sha256='unknown')
        self.assertIn('wrong device model', validate('display', obs))
        self.assertIn('missing/invalid artifact SHA256', validate('display', obs))


if __name__ == '__main__':
    unittest.main()
