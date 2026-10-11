"""The by-name driving helpers of midget.py (ax list/press/wait), with the control socket faked."""
import sys, unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).parent))
import midget

LISTING = (
    "os/settings/general/software-update\tpage\tSoftware Update\t\tenabled\n"
    "os/settings/general/software-update/download-and-install\trow\tDownload and Install\t\tenabled\n"
    "os/settings/general/software-update/automatic-updates\ttoggle\tAutomatic Updates\ton\tenabled\n"
    "os/settings/alert\talert\tAlert\tDownload Update?\tenabled\n"
    "os/settings/alert/download\tbutton\tDownload\t\tdisabled\n"
)


class Ax(unittest.TestCase):
    def test_rows_parse_tab_separated_lines(self):
        with mock.patch.object(midget, "control_text", return_value=LISTING):
            rows = midget.ax_rows()
        self.assertEqual(len(rows), 5)
        self.assertEqual(rows[2], {"address": "os/settings/general/software-update/automatic-updates", "role": "toggle",
                                   "name": "Automatic Updates", "value": "on", "enabled": True})
        self.assertFalse(rows[4]["enabled"])

    def test_no_answer_and_errors_are_none(self):
        with mock.patch.object(midget, "control_text", return_value=None):
            self.assertIsNone(midget.ax_rows())
        with mock.patch.object(midget, "control_text", return_value="error: unknown command\n"):
            self.assertIsNone(midget.ax_rows())

    def test_find_takes_an_address_or_a_unique_suffix(self):
        with mock.patch.object(midget, "control_text", return_value=LISTING):
            rows = midget.ax_rows()
        self.assertEqual(midget.ax_find(rows, "os/settings/alert")["role"], "alert")
        self.assertEqual(midget.ax_find(rows, "download-and-install")["name"], "Download and Install")
        self.assertIsNone(midget.ax_find(rows, "missing"))

    def test_press_reports_ok_or_the_error(self):
        with mock.patch.object(midget, "control_text", return_value="ok os/apps/settings\n"):
            self.assertEqual(midget.ax_press("os/apps/settings"), (True, "ok os/apps/settings"))
        with mock.patch.object(midget, "control_text", return_value='error: no element "x"\n'):
            self.assertEqual(midget.ax_press("x"), (False, 'error: no element "x"'))

    def test_wait_returns_when_the_value_matches_and_times_out_otherwise(self):
        with mock.patch.object(midget, "control_text", return_value=LISTING):
            self.assertEqual(midget.cmd_wait(["automatic-updates", "--value", "on", "--timeout", "2"]), 0)
            self.assertEqual(midget.cmd_wait(["automatic-updates", "--value", "off", "--timeout", "1"]), 1)
            self.assertEqual(midget.cmd_wait(["download", "--enabled", "--timeout", "1"]), 1)
            self.assertEqual(midget.cmd_wait(["nothing", "--gone", "--timeout", "2"]), 0)
        self.assertEqual(midget.cmd_wait([]), 2)
        self.assertEqual(midget.cmd_wait(["x", "--bogus"]), 2)


if __name__ == "__main__":
    unittest.main()
