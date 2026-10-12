import importlib.util, pathlib, unittest
spec = importlib.util.spec_from_file_location("midget", pathlib.Path(__file__).with_name("midget.py"))
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)

SAMPLE = """size=1280 800
screen=account
stage=0 Waiting
percent=0
button=next 540 500 200 44
button=back 560 560 160 30
field=name 380 120 520 52 1
field=password 380 224 520 52 0
bogus line
"""


class QaTests(unittest.TestCase):
    def test_icelandic_name_types_with_dead_keys(self):
        t = m.installer_keymaps()
        self.assertIsNone(m.layout_keys("Ó", "us", t), "U.S. has no key for an accented capital")
        # the acute dead key (40 on Icelandic), then Shift+O
        self.assertEqual(m.fifo_key_lines(m.layout_keys("Ó", "is", t)), ["key 40 1", "key 40 0", "key 42 1", "key 24 1", "key 24 0", "key 42 0"])
        self.assertEqual(m.layout_keys("ab", "is", t), [(False, 30, False), (False, 48, False)])
        self.assertIsNotNone(m.layout_keys("Ólafur Búi Ólafsson", "is", t))

    def test_altgr_is_held_around_the_key(self):
        t = m.installer_keymaps()
        self.assertEqual(m.fifo_key_lines(m.layout_keys("@", "is", t)), ["key 100 1", "key 16 1", "key 16 0", "key 100 0"])

    def test_parse(self):
        s = m.parse_installer_state(SAMPLE)
        self.assertEqual(s["size"], (1280, 800))
        self.assertEqual(s["screen"], "account")
        self.assertEqual(s["buttons"]["next"], (540, 500, 200, 44))
        self.assertEqual(s["fields"]["name"], ((380, 120, 520, 52), True))
        self.assertEqual(s["fields"]["password"][1], False)

    def test_button_lookup(self):
        s = m.parse_installer_state(SAMPLE)
        self.assertEqual(m.find_button(s, "next"), (640, 522))
        self.assertEqual(m.find_button(s, "install", "back"), (640, 575))
        self.assertIsNone(m.find_button(s, "install"))
        s["buttons"]["disk1"] = (0, 0, 10, 10)
        self.assertEqual(m.find_button(s, prefix="disk"), (5, 5))

    def test_pointer_scaling(self):
        self.assertEqual(m.abs_pointer(0, 0, (1280, 800)), (0, 0))
        self.assertEqual(m.abs_pointer(1279, 799, (1280, 800)), (65535, 65535))
        self.assertEqual(m.abs_pointer(640, 400, (1281, 801)), (32768, 32768))
        self.assertEqual(m.abs_pointer(5000, -4, (1280, 800)), (65535, 0))

    def test_scripts_compile(self):
        compile(m.uinput_pointer_script([(1, 2)]), "p", "exec")
        compile(m.uinput_keys_script([(False, 30)]), "k", "exec")


if __name__ == "__main__":
    unittest.main()
