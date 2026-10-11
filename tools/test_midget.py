"""Check package-update restart boundaries without contacting a machine."""
import importlib.util
import subprocess
import unittest
from pathlib import Path
from unittest.mock import patch
spec = importlib.util.spec_from_file_location('midget', Path(__file__).with_name('midget.py'))
midget = importlib.util.module_from_spec(spec)
spec.loader.exec_module(midget)

class UpdateLifecycle(unittest.TestCase):
    def result(self, code=0):
        return subprocess.CompletedProcess([], code, '', '')

    def test_failed_upgrade_never_restarts(self):
        with patch.object(midget, 'run', return_value=self.result(1)) as run:
            self.assertEqual(midget.cmd_update(['--restart']), 1)
            self.assertEqual(run.call_count, 2)  # status, install

    def test_default_leaves_running_shell(self):
        with patch.object(midget, 'run', return_value=self.result()) as run:
            self.assertEqual(midget.cmd_update([]), 0)
            self.assertEqual(run.call_count, 2)  # status, install

    def test_explicit_restart_follows_successful_upgrade(self):
        with patch.object(midget, 'run', return_value=self.result()) as run:
            self.assertEqual(midget.cmd_update(['--restart']), 0)
            self.assertEqual(run.call_count, 3)  # status, install, apply
            self.assertEqual(run.call_args.args[0][-1], 'bifrost-update-guard apply')

    def test_multiple_control_requests_are_rejected(self):
        with patch.object(midget, 'run') as run:
            self.assertEqual(midget.cmd_control(['state\nreboot']), 2)
            run.assert_not_called()

class KeyCombos(unittest.TestCase):
    def test_names_letters_and_combinations_map_to_evdev_codes(self):
        self.assertEqual(midget.key_codes('enter'), [28])
        self.assertEqual(midget.key_codes('cmd+o'), [125, 24])
        self.assertEqual(midget.key_codes('Ctrl+Shift+Z'), [29, 42, 44])
        self.assertIsNone(midget.key_codes('cmd+nope'))
        self.assertIsNone(midget.key_codes(''))

    def test_an_unknown_key_runs_nothing(self):
        with patch.object(midget, 'run') as run:
            self.assertEqual(midget.cmd_key(['hyper+o']), 2)
            run.assert_not_called()

    def test_a_combination_presses_in_order_and_releases_in_reverse(self):
        with patch.object(midget, 'run', return_value=subprocess.CompletedProcess([], 0, '', '')) as run:
            self.assertEqual(midget.cmd_key(['cmd+o']), 0)
            script = run.call_args.args[0][-1]
            self.assertIn('codes=[125, 24]', script)
            self.assertIn('for c in reversed(codes)', script)


class RebootWaits(unittest.TestCase):
    def test_down_midget_is_reported_not_waited_for(self):
        with patch.object(midget, 'state', return_value='down'), patch.object(midget, 'schedule_power') as power:
            self.assertEqual(midget.cmd_reboot([]), 1)
            power.assert_not_called()

    def test_pinging_midget_is_waited_for_and_reports_recovery(self):
        states = iter(['up', 'up', 'live'])
        with patch.object(midget, 'state', side_effect=lambda: next(states)), \
             patch.object(midget, 'wait', return_value=True) as wait, \
             patch.object(midget, 'cmd_recovery', return_value=0) as recovery, \
             patch.object(midget, 'schedule_power') as power:
            self.assertEqual(midget.cmd_reboot([]), 0)
            power.assert_not_called()
            wait.assert_called_once()
            recovery.assert_called_once()


class UnlockTyping(unittest.TestCase):
    def test_lab_passwords_type_on_a_us_keyboard_with_shift_where_needed(self):
        # token_urlsafe passwords: letters, digits, '-' and '_'
        self.assertEqual(midget.text_keys('aB-_9'), [(False, 30), (True, 48), (False, 12), (True, 12), (False, 10)])
        self.assertIsNone(midget.text_keys('å'), 'a character with no key is refused, not mistyped')

    def test_unlock_types_the_password_then_enter_and_never_prints_it(self):
        with patch.object(midget, 'login_password', return_value='pw-1'), \
             patch.object(midget, 'run', return_value=subprocess.CompletedProcess([], 0, '', 'oops pw-1')) as run:
            self.assertEqual(midget.cmd_unlock(), 0)
            script = run.call_args[0][0][-1]
            self.assertNotIn('pw-1', script, 'only key codes cross ssh')
            self.assertIn('28)]', script, 'Enter comes last')


if __name__ == '__main__':
    unittest.main()
