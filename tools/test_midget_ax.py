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

    def test_scope_names_one_window(self):
        rows = [{"address": a, "role": "button", "name": "Save", "value": "", "enabled": True} for a in (
            "os/settings/general/save", "app/is.olibuijr.mail.desktop/button/save", "app/is.olibuijr.notes.desktop/button/save")]
        self.assertEqual(midget.ax_scope(rows, "mail"), ("app/is.olibuijr.mail.desktop", []))
        self.assertEqual(midget.ax_scope(rows, "os/settings"), ("os/settings", []))
        self.assertEqual(midget.ax_scope(rows, "Settings"), ("os/settings", []))
        scope, cands = midget.ax_scope(rows, "olibuijr")
        self.assertIsNone(scope)
        self.assertEqual(cands, ["app/is.olibuijr.mail.desktop", "app/is.olibuijr.notes.desktop"])
        scope, cands = midget.ax_scope(rows, "nothing")
        self.assertIsNone(scope)
        self.assertEqual(len(cands), 3, "nothing matched: every scope is offered")

    def test_match_by_address_suffix_or_name_with_an_optional_role(self):
        rows = [
            {"address": "app/x/button/save", "role": "button", "name": "Save", "value": "", "enabled": True},
            {"address": "app/x/list/mail/save", "role": "row", "name": "Save", "value": "", "enabled": True},
            {"address": "app/x/button/cancel", "role": "button", "name": "Cancel", "value": "", "enabled": True},
        ]
        self.assertEqual([r["address"] for r in midget.ax_match(rows, "save")], ["app/x/button/save", "app/x/list/mail/save"])
        self.assertEqual([r["address"] for r in midget.ax_match(rows, "save", role="button")], ["app/x/button/save"])
        self.assertEqual([r["address"] for r in midget.ax_match(rows, "app/x/button/save")], ["app/x/button/save"])
        self.assertEqual([r["address"] for r in midget.ax_match(rows, "cancel")], ["app/x/button/cancel"])
        self.assertEqual([r["address"] for r in midget.ax_match(rows, "CANCEL")], ["app/x/button/cancel"])
        self.assertEqual(midget.ax_match(rows, "nope"), [])

    def test_press_scoped_to_a_window_presses_the_one_address_or_explains(self):
        listing = ("os/settings/general/about\trow\tAbout\t\tenabled\n"
                   "app/is.olibuijr.mail.desktop/list/inbox/hello\trow\tHello\t\tenabled\n"
                   "app/is.olibuijr.mail.desktop/button/reply\tbutton\tReply\t\tenabled\n"
                   "app/is.olibuijr.mail.desktop/menu/reply\tmenuitem\tReply\t\tenabled\n"
                   "app/is.olibuijr.notes.desktop/button/reply\tbutton\tReply\t\tenabled\n")
        sent = []

        def control(line, timeout=15):
            sent.append(line)
            return listing if line.startswith("ax list") else "ok " + line.split(" ", 2)[2] + "\n"
        with mock.patch.object(midget, "control_text", control), mock.patch("sys.stdout"):
            self.assertEqual(midget.cmd_press(["Hello", "--in", "mail"]), 0)
            self.assertEqual(sent[-1], "ax press app/is.olibuijr.mail.desktop/list/inbox/hello")
            self.assertEqual(midget.cmd_press(["Reply", "--in", "mail"]), 1, "two Reply elements in Mail: ambiguous")
            self.assertEqual(midget.cmd_press(["Reply", "--in", "mail", "--role", "button"]), 0)
            self.assertEqual(sent[-1], "ax press app/is.olibuijr.mail.desktop/button/reply")
            self.assertEqual(midget.cmd_press(["Reply", "--in", "desktop"]), 2, "two apps match the scope word")
            self.assertEqual(midget.cmd_press(["Nope", "--in", "mail"]), 1)
            self.assertEqual(midget.cmd_press([]), 2)
            self.assertEqual(midget.cmd_press(["About", "--bogus"]), 2)
        with mock.patch.object(midget, "cmd_control", return_value=0) as plain, mock.patch("sys.stdout"):
            self.assertEqual(midget.cmd_press(["General > Software Update"]), 0)
            plain.assert_called_once_with(["ax", "press", "General > Software Update"])

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
