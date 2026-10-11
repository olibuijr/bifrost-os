#!/usr/bin/env python3
"""Guard tests for midget-wipe.py: refusals must happen before anything touches midget."""
import argparse, importlib.util, os, sys, tempfile, unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
spec = importlib.util.spec_from_file_location("midget_wipe", HERE / "midget-wipe.py")
w = importlib.util.module_from_spec(spec)
spec.loader.exec_module(w)


def args(**kw):
    base = dict(disk="/dev/nvme0n1", confirm="WIPE 192.168.20.140 /dev/nvme0n1", dry_run=True,
                user="tester", password_env="MIDGET_WIPE_PASSWORD", timeout=5)
    base.update(kw)
    return argparse.Namespace(**base)


class WipeGuards(unittest.TestCase):
    def setUp(self):
        self.calls = []
        self._ssh, self._state, self._host = w.m.ssh, w.m.state, w.m.HOST
        w.m.HOST = "192.168.20.140"  # midget.py picks the reachable address at import; the tokens below name this one
        w.m.ssh = self.fake_ssh(disks="/dev/nvme0n1 512110190592 disk 0")
        w.m.state = lambda: self.calls.append("state") or "installed"
        self._repo = w.REPO
        w.REPO = tempfile.mkdtemp()

    def fake_ssh(self, disks):
        def ssh(cmd, *a, **k):
            self.calls.append(cmd)
            if "lsblk" in cmd:
                return disks
            if "address" in cmd:
                return "00:23:24:dc:c8:40 00:00:00:00:00:00"
            if "findmnt" in cmd:
                return "/dev/sdb2"
            return ""
        return ssh

    def tearDown(self):
        w.m.ssh, w.m.state, w.REPO, w.m.HOST = self._ssh, self._state, self._repo, self._host

    def test_wrong_token_refused_before_any_ssh(self):
        for bad in ("", "WIPE 192.168.20.140 /dev/sda", "wipe 192.168.20.140 /dev/nvme0n1", "WIPE 192.168.20.140 /dev/nvme0n1 "):
            self.calls.clear()
            self.assertEqual(w.run(args(confirm=bad)), 1, bad)
            self.assertEqual(self.calls, [], bad)

    def test_dry_run_with_exact_token_touches_nothing_destructive(self):
        self.assertEqual(w.run(args()), 0)
        destructive = [c for c in self.calls if isinstance(c, str) and any(x in c for x in ("nohup", "BIFROST_AUTO", "reboot", "bootnext"))]
        self.assertEqual(destructive, [])

    def test_wrong_disk_refused(self):
        w.m.ssh = self.fake_ssh(disks="/dev/nvme0n1 512110190592 disk 0\n/dev/sda 64000000000 disk 0")
        self.assertEqual(w.run(args(disk="/dev/sda", confirm="WIPE 192.168.20.140 /dev/sda")), 1)
        self.assertFalse(any("nohup" in c for c in self.calls if isinstance(c, str)))


if __name__ == "__main__":
    unittest.main()
