"""Pure protocol tests for the release VM harness's QGA response handling."""
import base64
import importlib.util
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name("release-vm-test.py")
SPEC = importlib.util.spec_from_file_location("release_vm_test", SCRIPT)
release_vm_test = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(release_vm_test)


class FakeQga:
    def __init__(self, statuses):
        self.statuses = list(statuses)
        self.calls = []

    def execute(self, name, **arguments):
        self.calls.append((name, arguments))
        if name == "guest-exec":
            return {"pid": 73}
        if name == "guest-exec-status":
            return self.statuses.pop(0)
        raise AssertionError(f"unexpected QGA command: {name}")


class GuestExecTests(unittest.TestCase):
    def test_exec_polls_pid_then_decodes_base64_output(self):
        fake = FakeQga([
            {"exited": False},
            {"exited": True, "exitcode": 0, "out-data": base64.b64encode(b"ready\n").decode(),
             "err-data": base64.b64encode(b"diagnostic\n").decode()},
        ])
        self.assertEqual(release_vm_test.Qga.run(fake, "echo ready", timeout=1, poll_interval=0), "ready\n")
        self.assertEqual([call[0] for call in fake.calls], ["guest-exec", "guest-exec-status", "guest-exec-status"])
        self.assertEqual(fake.calls[0][1]["path"], "/bin/sh")
        self.assertTrue(fake.calls[0][1]["capture-output"])
        self.assertEqual(fake.calls[1][1], {"pid": 73})

    def test_exec_reports_decoded_guest_error_on_nonzero_exit(self):
        fake = FakeQga([{"exited": True, "exitcode": 19, "out-data": "", "err-data": base64.b64encode(b"denied").decode()}])
        with self.assertRaisesRegex(RuntimeError, "exited 19.*denied"):
            release_vm_test.Qga.run(fake, "false", timeout=1, poll_interval=0)

    def test_exec_times_out_for_a_still_running_pid(self):
        fake = FakeQga([{"exited": False}])
        with self.assertRaisesRegex(TimeoutError, "process 73 did not finish"):
            release_vm_test.Qga.run(fake, "sleep 20", timeout=0, poll_interval=0)

    def test_status_rejects_invalid_base64(self):
        with self.assertRaisesRegex(RuntimeError, "invalid base64"):
            release_vm_test.decode_guest_exec_status({"exited": True, "exitcode": 0, "out-data": "not base64!"})

    def test_status_keeps_unfinished_process_pending(self):
        self.assertEqual(release_vm_test.decode_guest_exec_status({"exited": False}), (False, None, "", ""))


if __name__ == "__main__":
    unittest.main()
