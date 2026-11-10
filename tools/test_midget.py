"""Check package-update restart boundaries without contacting a machine."""
import contextlib
import importlib.util
import io
import os
import shlex
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import unittest
from pathlib import Path
from unittest.mock import patch
os.environ["MIDGET_SESSION"] = "0"  # no input-session daemon from a test: plans go through the (patched) one-shot `run`
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

class ShowInFiles(unittest.TestCase):
    """The remote script runs here against a stand-in for the shell's control socket."""

    def setUp(self):
        self.dir = tempfile.mkdtemp(prefix='midget-sif-')
        self.addCleanup(shutil.rmtree, self.dir, True)
        self.folder = os.path.join(self.dir, 'My Files')
        os.mkdir(self.folder)
        self.file = os.path.join(self.folder, 'Q1 report.pdf')
        Path(self.file).write_text('x')
        self.sock = os.path.join(self.dir, 'c.sock')
        self.lines = []

    def serve(self, replies):
        """Answers one connection per reply, in order, and records each request line."""
        server = socket.socket(socket.AF_UNIX)
        server.bind(self.sock)
        server.listen(4)
        server.settimeout(10)

        def loop():
            with server:
                for reply in replies:
                    try:
                        conn, _ = server.accept()
                    except OSError:
                        return
                    with conn:
                        self.lines.append(conn.recv(1024).decode())
                        conn.sendall(reply.encode())
        thread = threading.Thread(target=loop, daemon=True)
        thread.start()
        return thread

    def script(self, path):
        return subprocess.run([sys.executable, '-c', midget.show_in_files_script(self.sock), path], capture_output=True, text=True, timeout=30)

    def test_a_file_is_revealed_selected_in_its_folder(self):
        thread = self.serve(['ok\n'])
        r = self.script(self.file)
        thread.join(10)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(self.lines, [f'reveal-file {midget.FILES_APP_ID} {self.file}\n'])
        self.assertEqual(r.stdout, f'ok: Files shows {self.folder} with Q1 report.pdf selected\n')

    def test_a_folder_is_opened_itself(self):
        thread = self.serve(['ok\n'])
        r = self.script(self.folder + '/')
        thread.join(10)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(self.lines, [f'open-folder {midget.FILES_APP_ID} {self.folder}\n'])
        self.assertEqual(r.stdout, f'ok: Files shows {self.folder}\n')

    def test_an_older_shell_shows_the_folder_without_a_selection(self):
        thread = self.serve(['error: unknown command Some("reveal-file")\n', 'ok\n'])
        r = self.script(self.file)
        thread.join(10)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(self.lines, [f'reveal-file {midget.FILES_APP_ID} {self.file}\n', f'open-folder {midget.FILES_APP_ID} {self.folder}\n'])
        self.assertIn('the file is not selected', r.stdout)

    def test_a_refusal_from_the_shell_fails_with_its_reason(self):
        thread = self.serve(['error: App not installed\n'])
        r = self.script(self.file)
        thread.join(10)
        self.assertEqual(r.returncode, 1)
        self.assertIn('error: App not installed', r.stderr)
        self.assertEqual(r.stdout, '')

    def test_a_missing_path_asks_the_shell_nothing(self):
        r = self.script(os.path.join(self.folder, 'gone.txt'))
        self.assertEqual(r.returncode, 1)
        self.assertIn('no such file or folder', r.stderr)

    def test_a_path_too_long_for_one_control_line_is_refused(self):
        long = os.path.join(self.folder, 'a' * 250, 'b' * 250)
        os.makedirs(os.path.dirname(long))
        Path(long).write_text('x')
        r = self.script(long)
        self.assertEqual(r.returncode, 1)
        self.assertIn('too long', r.stderr)

    def test_the_command_sends_the_script_and_the_quoted_path_over_ssh(self):
        done = subprocess.CompletedProcess([], 0, 'ok: Files shows /home/me\n', '')
        with patch.object(midget, 'run', return_value=done) as run, contextlib.redirect_stdout(io.StringIO()) as out:
            self.assertEqual(midget.cmd_show_in_files(['/home/me/My Files/a b.txt']), 0)
        remote = run.call_args.args[0][-1]
        self.assertEqual(remote, 'python3 -c ' + shlex.quote(midget.show_in_files_script()) + " '/home/me/My Files/a b.txt'")
        self.assertIn(repr(midget.CONTROL_SOCKET), remote)
        self.assertIn('reveal-file', remote)
        self.assertEqual(out.getvalue(), 'ok: Files shows /home/me\n')

    def test_open_file_is_one_control_line_with_the_app_and_the_path(self):
        done = subprocess.CompletedProcess([], 0, 'ok\n', '')
        with patch.object(midget, 'run', return_value=done) as run, contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(midget.cmd_control(['open-file', 'is.olibuijr.textedit.desktop', '/home/me/to do.txt']), 0)
        self.assertIn('open-file is.olibuijr.textedit.desktop /home/me/to do.txt', run.call_args.args[0][-1])

    def test_a_relative_path_is_a_usage_error(self):
        with patch.object(midget, 'run') as run, contextlib.redirect_stdout(io.StringIO()) as out:
            self.assertEqual(midget.cmd_show_in_files(['a.txt']), 2)
            self.assertEqual(midget.cmd_show_in_files([]), 2)
            run.assert_not_called()
        self.assertIn('usage: show-in-files', out.getvalue())

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

    def test_escape_and_the_other_spellings_are_the_same_keys(self):
        self.assertEqual(midget.key_codes('escape'), midget.key_codes('esc'))
        self.assertEqual(midget.key_codes('Escape'), [1])
        self.assertEqual(midget.key_codes('return'), midget.key_codes('enter'))
        self.assertEqual(midget.key_codes('command+period'), [125, 52])
        self.assertEqual(midget.key_steps(['escape*2', 'return']), [[1], [1], [28]])
        for alias, name in midget.KEY_ALIASES.items():
            self.assertEqual(midget.KEY_CODES[alias], midget.KEY_CODES[name], alias)

    def test_an_unknown_key_is_named_on_stderr_and_fails(self):
        self.assertEqual(midget.unknown_keys(['cmd+o', 'escap', 'hyper+o*2', 'escap']), ['escap', 'hyper'])
        self.assertEqual(midget.unknown_keys(['escape', 'cmd+[', 'down*3']), [])
        for command in (midget.cmd_key, midget.cmd_keydown, midget.cmd_keyup):
            err = io.StringIO()
            with patch.object(midget, 'run') as run, contextlib.redirect_stderr(err), contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(command(['escap']), 2)
                run.assert_not_called()
            self.assertIn("unknown key name 'escap'", err.getvalue())
            self.assertIn('nothing was pressed', err.getvalue())

    def test_a_combination_presses_in_order_and_releases_in_reverse(self):
        with patch.object(midget, 'run', return_value=subprocess.CompletedProcess([], 0, '', '')) as run:
            self.assertEqual(midget.cmd_key(['cmd+o']), 0)
            script = run.call_args.args[0][-1]
            self.assertIn("('key', [125, 24], 0.05)", script.replace('\'"\'"\'', "'"))
            self.assertIn('for c in reversed(o[1])', script)


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
