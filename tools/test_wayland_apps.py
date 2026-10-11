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

if __name__ == '__main__': unittest.main()
