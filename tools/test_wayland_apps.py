"""Application launch privilege and argument checks; no display is needed."""
import json
import runpy
import unittest
from pathlib import Path
from unittest.mock import patch

SCRIPT = Path(__file__).resolve().parents[1] / 'packaging/arch/wayland/apps'
class Executed(Exception):
    pass

class AppLauncherTests(unittest.TestCase):
    def run_launcher(self, args, uid=0):
        def read(path, *a, **kw):
            if path.name == 'account': return 'username=tester\n'
            return json.dumps({'WAYLAND_DISPLAY':'wayland-0','DISPLAY':':9','XDG_RUNTIME_DIR':'/run/bifrost/user','LD_PRELOAD':'bad'})
        with patch('sys.argv',['bifrost-apps',*args]), patch.object(Path,'read_text',read), patch.object(Path,'is_file',return_value=True), patch('os.geteuid',return_value=uid), patch('os.execvp',side_effect=Executed) as execute:
            with self.assertRaises(Executed): runpy.run_path(str(SCRIPT),run_name='__main__')
        return execute.call_args.args

    def test_root_launch_drops_to_account_and_filters_environment(self):
        executable,cmd = self.run_launcher(['firefox'])
        self.assertEqual(executable,'runuser')
        self.assertEqual(cmd[:4],['runuser','-u','tester','--'])
        self.assertIn('DISPLAY=:9',cmd)
        self.assertFalse(any('LD_PRELOAD' in arg for arg in cmd))
        self.assertEqual(cmd[-3:],['gio','launch','/usr/share/applications/firefox.desktop'])

    def test_menu_discovers_desktop_entries_as_user(self):
        _,cmd = self.run_launcher([])
        self.assertEqual(cmd[-3:],['fuzzel','--prompt','Applications: '])

    def test_screenshot_uses_real_compositor(self):
        _,cmd = self.run_launcher(['--screenshot'])
        self.assertEqual(cmd[-2:],['grim','-'])

    def test_path_traversal_is_rejected_before_exec(self):
        with patch('sys.argv',['bifrost-apps','../../bin/bash']), patch.object(Path,'read_text',side_effect=['username=tester\n','{}']), patch('os.execvp') as execute:
            with self.assertRaises(SystemExit): runpy.run_path(str(SCRIPT),run_name='__main__')
            execute.assert_not_called()

    def open_file(self, answer):
        """Runs `--open /home/tester/a.txt` against a shell that answers `answer` (an OSError: no shell). Returns
        the exit code (None when the launcher went on to exec), the request sent and the exec arguments."""
        def read(path, *a, **kw):
            if path.name == 'account': return 'username=tester\n'
            return json.dumps({'WAYLAND_DISPLAY':'wayland-0','XDG_RUNTIME_DIR':'/run/bifrost/user'})
        with patch('sys.argv',['bifrost-apps','--open','/home/tester/a.txt']), patch.object(Path,'read_text',read), patch.object(Path,'is_file',return_value=True), patch.object(Path,'is_dir',return_value=False), patch.object(Path,'resolve',lambda self,*a,**kw:self), patch('os.geteuid',return_value=0), patch('os.execvp',side_effect=Executed) as execute, patch('socket.socket') as sock:
            client = sock.return_value.__enter__.return_value
            if isinstance(answer,Exception): client.connect.side_effect = answer
            else: client.recv.return_value = answer
            code = None
            try: runpy.run_path(str(SCRIPT),run_name='__main__')
            except Executed: pass
            except SystemExit as e: code = e.code
            sent = client.sendall.call_args.args[0] if client.sendall.called else None
            return code, sent, (execute.call_args.args if execute.called else None)

    def test_open_file_asks_the_shell_for_the_default_app(self):
        code,sent,executed = self.open_file(b'ok\n')
        self.assertEqual((code,sent,executed),(0,b'open-document /home/tester/a.txt\n',None))

    def test_open_file_a_shell_refusal_is_final(self):
        code,_,executed = self.open_file(b'error: The file is not in the home folder\n')
        self.assertEqual(code,'error: The file is not in the home folder')
        self.assertIsNone(executed)

    def test_open_file_falls_back_when_no_shell_answers_or_it_is_an_old_one(self):
        for answer in (ConnectionRefusedError(), FileNotFoundError(), b'error: unknown command Some("open-document")\n'):
            code,_,executed = self.open_file(answer)
            self.assertIsNone(code)
            self.assertEqual(executed[1][-3:],['gio','open','/home/tester/a.txt'])

if __name__ == '__main__': unittest.main()
