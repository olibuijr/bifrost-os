"""The pure parts of midget.py's computer-use commands (type, click, drag, scroll, shot, seq): no machine is contacted."""
import importlib.util, pathlib, struct, unittest, zlib
spec = importlib.util.spec_from_file_location("midget", pathlib.Path(__file__).with_name("midget.py"))
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)

SIZE = (2560, 1440)


class Keys(unittest.TestCase):
    def test_chords_and_repeats(self):
        self.assertEqual(m.key_codes("cmd+shift+4"), [125, 42, 5])
        self.assertEqual(m.key_codes("alt+f4"), [56, 62])
        self.assertEqual(m.key_codes("cmd+minus"), [125, 12])
        self.assertIsNone(m.key_codes("cmd+nope"))
        self.assertEqual(m.key_steps(["down*3"]), [[108]] * 3)
        self.assertEqual(m.key_steps(["tab", "cmd+o", "enter*2"]), [[15], [125, 24], [28], [28]])
        for bad in (["down*0"], ["down*x"], ["nope"], ["down*999"], []):
            self.assertIsNone(m.key_steps(bad))

    def test_punctuation_keys_by_name_and_as_the_character(self):
        names = {"minus": 12, "equal": 13, "bracketleft": 26, "bracketright": 27, "semicolon": 39, "apostrophe": 40,
                 "grave": 41, "backslash": 43, "comma": 51, "period": 52, "slash": 53}
        chars = {"-": 12, "=": 13, "[": 26, "]": 27, ";": 39, "'": 40, "`": 41, "\\": 43, ",": 51, ".": 52, "/": 53}
        for name, code in names.items():
            self.assertEqual(m.key_codes(name), [code], name)
            self.assertEqual(m.key_codes("cmd+" + name), [125, code], name)
        for ch, code in chars.items():
            self.assertEqual(m.key_codes(ch), [code], ch)
            self.assertEqual(m.key_codes("cmd+" + ch), [125, code], ch)
        # the chords the browser and Files shortcuts need
        self.assertEqual(m.key_codes("cmd+["), [125, 26])
        self.assertEqual(m.key_codes("cmd+]"), [125, 27])
        self.assertEqual(m.key_codes("cmd+,"), [125, 51])
        self.assertEqual(m.key_steps(["cmd+[", "cmd+]*2", "cmd+shift+."]), [[125, 26], [125, 27], [125, 27], [125, 42, 52]])
        self.assertIsNone(m.key_codes("cmd+bracket"))

    def test_text_uses_us_layout_with_shift(self):
        self.assertEqual(m.type_events("aB-_9")[0], [(False, 30), (True, 48), (False, 12), (True, 12), (False, 10)])
        self.assertEqual(m.type_events("!@#")[0], [(True, 2), (True, 3), (True, 4)])
        self.assertEqual(m.type_events("a b")[0], [(False, 30), (False, 57), (False, 48)])
        self.assertEqual(m.type_events('{}:"|<>?~')[0][:2], [(True, 26), (True, 27)])

    def test_newline_tab_and_unsupported_characters(self):
        seq, skipped = m.type_events("a\n\tb\rå☃")
        self.assertEqual(seq, [(False, 30), (False, 28), (False, 15), (False, 48)])
        self.assertEqual(skipped, ["å", "☃"])

    def test_escapes(self):
        self.assertEqual(m.unescape_text("a\\nb\\tc\\\\d\\x"), "a\nb\tc\\d\\x")

    def test_every_printable_ascii_char_has_a_key(self):
        text = "".join(chr(c) for c in range(32, 127))
        self.assertEqual(m.type_events(text)[1], [])


class Pointer(unittest.TestCase):
    def test_pixels_equal_absolute_values_and_clamp(self):
        self.assertEqual(m.px_to_abs(0, 0, SIZE), (0, 0))
        self.assertEqual(m.px_to_abs(1280, 720, SIZE), (1280, 720))
        self.assertEqual(m.px_to_abs(2559, 1439, SIZE), (2559, 1439))
        self.assertEqual(m.px_to_abs(9999, -5, SIZE), (2559, 0))
        self.assertEqual(m.px_to_abs(10.6, 3.4, SIZE), (11, 3))

    def test_small_image_coordinates_scale_back(self):
        scale, small = m.shot_size(SIZE, 1280)
        self.assertEqual((scale, small), (2.0, (1280, 720)))
        self.assertEqual(m.scale_point(100, 50, scale), (200.0, 100.0))
        self.assertEqual(m.shot_size((1920, 1080), 1280), (1.5, (1280, 720)))
        self.assertEqual(m.shot_size((1024, 768), 1280), (1.0, (1024, 768)))

    def test_click_plan(self):
        plan = m.click_plan(100, 200, SIZE)
        self.assertIn(("move", 100, 200), plan)
        self.assertEqual([o for o in plan if o[0] == "btn"], [("btn", 0x110, 1), ("btn", 0x110, 0)])
        right = [o for o in m.click_plan(1, 2, SIZE, right=True, double=True) if o[0] == "btn"]
        self.assertEqual(right, [("btn", 0x111, 1), ("btn", 0x111, 0)] * 2)

    def test_modifiers_are_held_around_the_click(self):
        plan = m.click_plan(1, 2, SIZE, mods=[125, 42])
        self.assertEqual([o for o in plan if o[0] in ("kd", "ku")], [("kd", 125), ("kd", 42), ("ku", 42), ("ku", 125)])
        self.assertLess(plan.index(("kd", 42)), plan.index(("btn", 0x110, 1)))
        self.assertGreater(plan.index(("ku", 125)), plan.index(("btn", 0x110, 0)))

    def test_drag_interpolates(self):
        pts = m.drag_points(0, 0, 100, 50, 4)
        self.assertEqual(pts, [(0, 0), (25, 12.5), (50, 25), (75, 37.5), (100, 50)])
        plan = m.drag_plan(10, 10, 110, 10, SIZE, 320)
        moves = [o for o in plan if o[0] == "move"]
        self.assertEqual((moves[0], moves[-1]), (("move", 10, 10), ("move", 110, 10)))
        self.assertEqual(len(moves), 21)
        kinds = [o[0] for o in plan if o[0] in ("btn", "move")]
        self.assertEqual(kinds[1], "btn")  # press after reaching the start
        self.assertEqual(kinds[-1], "btn")  # release last

    def test_scroll_directions(self):
        wheel = lambda **k: [o for o in m.scroll_plan(5, 5, SIZE, **k) if o[0] == "wheel"]
        self.assertEqual(wheel(dy=2), [("wheel", 8, -1)] * 2)  # down = wheel toward the user = negative
        self.assertEqual(wheel(dy=-1), [("wheel", 8, 1)])
        self.assertEqual(wheel(dy=0, dx=3), [("wheel", 6, 1)] * 3)
        self.assertEqual(wheel(dy=0, dx=-1), [("wheel", 6, -1)])

    def test_device_script_compiles_for_every_op(self):
        plan = (m.click_plan(1, 2, SIZE, mods=[125]) + m.drag_plan(0, 0, 9, 9, SIZE, 100) + m.scroll_plan(1, 1, SIZE, 1, 1)
                + [("key", [125, 24], 0.05), ("type", [(True, 30)], 0.015), ("sleep", 0.1)])
        src = m.uinput_plan_script(plan, SIZE)
        compile(src, "plan", "exec")
        self.assertIn("SIZE=(2560, 1440)", src)
        self.assertIn("BifrOSt remote pointer", src)
        compile(m.uinput_plan_script([("key", [28], 0.05)]), "k", "exec")
        self.assertGreater(m.plan_seconds(plan), 1.0)


class Flags(unittest.TestCase):
    def test_flags_and_positionals(self):
        pos, fl = m.split_flags(["10", "20", "--right", "--mods", "cmd+shift", "--scale", "2"])
        self.assertEqual((pos, fl), (["10", "20"], {"--right": True, "--mods": "cmd+shift", "--scale": "2"}))
        self.assertEqual(m.split_flags(["-5", "7"])[0], ["-5", "7"])
        with self.assertRaises(ValueError):
            m.split_flags(["--nope"])
        with self.assertRaises(ValueError):
            m.split_flags(["1", "--mods"])

    def test_size_parsing(self):
        self.assertEqual(m.parse_size("2560x1440"), (2560, 1440))
        self.assertEqual(m.parse_size(" 800 X 600 "), (800, 600))
        self.assertIsNone(m.parse_size("0x5"))
        self.assertIsNone(m.parse_size("nonsense"))
        self.assertIsNone(m.parse_size(None))

    def test_png_header_size(self):
        png = m.png_encode(3, 2, 2, [bytes(9)] * 2)
        self.assertEqual(m.png_size(png), (3, 2))
        self.assertIsNone(m.png_size(b"not a png at all, really........"))


class Resize(unittest.TestCase):
    def test_pure_python_nearest_downscale(self):
        # 4x2 RGB: left half red, right half blue
        row = bytes([255, 0, 0] * 2 + [0, 0, 255] * 2)
        small = m.png_resize_nearest(m.png_encode(4, 2, 2, [row, row]), 2)
        self.assertEqual(m.png_size(small), (2, 1))
        w, h, ch, rows = m.png_decode(small)
        self.assertEqual((w, h, ch, bytes(rows[0])), (2, 1, 3, bytes([255, 0, 0, 0, 0, 255])))

    def test_filtered_rows_decode(self):
        # two RGB pixels per row; row 2 uses the Up filter (2), so its raw bytes are the difference
        raw = b"\x00" + bytes([10, 20, 30, 40, 50, 60]) + b"\x02" + bytes([1, 1, 1, 1, 1, 1])
        chunk = lambda t, d: struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d))
        png = (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 2, 2, 8, 2, 0, 0, 0))
               + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b""))
        self.assertEqual([bytes(r) for r in m.png_decode(png)[3]],
                         [bytes([10, 20, 30, 40, 50, 60]), bytes([11, 21, 31, 41, 51, 61])])

    def test_unsupported_png_is_none(self):
        self.assertIsNone(m.png_resize_nearest(b"junk", 10))


class Seq(unittest.TestCase):
    def test_parse_skips_comments_and_blanks(self):
        text = '# a flow\n\nclick 10 20 --double  # open it\ntype "hello world\\n"\nsleep 0.5\n   \nshot /tmp/a.png\n'
        self.assertEqual(m.parse_seq(text), [
            (3, ["click", "10", "20", "--double"]),
            (4, ["type", "hello world\\n"]),
            (5, ["sleep", "0.5"]),
            (7, ["shot", "/tmp/a.png"]),
        ])

    def test_unbalanced_quote_names_the_line(self):
        with self.assertRaisesRegex(ValueError, "line 2"):
            m.parse_seq('sleep 1\ntype "oops\n')

    def test_every_listed_op_exists(self):
        ops = m.seq_ops()
        for name in ("type", "click", "move", "drag", "scroll", "key", "shot", "screen", "tap-window"):
            self.assertIn(name, ops)


if __name__ == "__main__":
    unittest.main()
