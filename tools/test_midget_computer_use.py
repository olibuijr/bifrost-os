"""The pure parts of midget.py's computer-use commands (type, click, drag, scroll, shot, zoom, holds, the click guard, the
relative mouse, the input session's plumbing, seq): no machine is contacted."""
import importlib.util, os, pathlib, struct, tempfile, unittest, zlib
from unittest.mock import patch
os.environ["MIDGET_SESSION"] = "0"  # a test never starts the input-session daemon (it would ssh to the lab machine)
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


class Layouts(unittest.TestCase):
    """`type` follows the session's keyboard layout (Icelandic on Midget) instead of assuming US."""

    def test_icelandic_symbols_use_the_keys_of_that_layout(self):
        # US Shift+2 is '@'; on Icelandic it types '"', and '@' is AltGr+q (RIGHTALT held around code 16)
        self.assertEqual(m.type_events("@", "is")[0], [(False, 16, True)])
        self.assertEqual(m.type_events('"', "is")[0], [(True, 3, False)])
        self.assertEqual(m.type_events("@", "us")[0], [(True, 3)])
        self.assertEqual(m.type_events("@")[0], [(True, 3)], "no layout is the US table")
        for ch, press in m.ICELANDIC_SYMBOLS.items():
            self.assertEqual(m.type_events(ch, "is")[0], [press], ch)

    def test_newline_tab_and_space_work_on_every_layout(self):
        seq, skipped = m.type_events("a b\n\t", "is")
        self.assertEqual(seq, [(False, 30, False), (False, 57, False), (False, 48, False), (False, 28), (False, 15)])
        self.assertEqual(skipped, [])

    def test_unmapped_characters_are_reported(self):
        self.assertEqual(m.type_events("a☃b", "is")[1], ["☃"])
        self.assertEqual(m.type_events("é", "us")[1], ["é"])

    def test_unknown_layout_is_an_error(self):
        with self.assertRaises(ValueError):
            m.type_events("a", "xx")

    def test_selftest_round_trips_ascii_on_us_and_is(self):
        self.assertTrue(m.layout_selftest())

    def test_layout_is_read_from_the_env_text(self):
        table = m.installer_layout_xkb()
        self.assertEqual(m.layout_from_env_text("XKB_DEFAULT_LAYOUT=is\nXKB_DEFAULT_VARIANT=\n", table), "is")
        self.assertEqual(m.layout_from_env_text("XKB_DEFAULT_LAYOUT=us\nXKB_DEFAULT_VARIANT=intl\n", table), "us-intl")
        self.assertIsNone(m.layout_from_env_text("XKB_DEFAULT_LAYOUT=xx\n", table))
        self.assertIsNone(m.layout_from_env_text("", table))

    def test_session_layout_prefers_the_session_environment_over_the_installed_file(self):
        out = "XKB_DEFAULT_LAYOUT=de\nXKB_DEFAULT_VARIANT=\n---\nXKB_DEFAULT_LAYOUT=is\nXKB_DEFAULT_VARIANT=\n"
        with patch.object(m, "ssh", return_value=out):
            self.assertEqual(m.session_layout(), "de")
        with patch.object(m, "ssh", return_value="---\nXKB_DEFAULT_LAYOUT=is\nXKB_DEFAULT_VARIANT=\n"):
            self.assertEqual(m.session_layout(), "is")
        with patch.object(m, "ssh", return_value=""):
            self.assertIsNone(m.session_layout())

    def test_cmd_type_reads_the_layout_once_and_override_skips_the_probe(self):
        with patch.object(m, "session_layout", return_value="is") as probe, patch.object(m, "run_plan", return_value=0) as run:
            self.assertEqual(m.cmd_type(["a@"]), 0)
            probe.assert_called_once()
            self.assertEqual(run.call_args.args[0], [("type", [(False, 30, False), (False, 16, True)], 0.015)])
        with patch.object(m, "session_layout", return_value="is") as probe, patch.object(m, "run_plan", return_value=0) as run:
            self.assertEqual(m.cmd_type(["--layout", "us", "a@"]), 0)
            probe.assert_not_called()
            self.assertEqual(run.call_args.args[0], [("type", [(False, 30), (True, 3)], 0.015)])
            self.assertEqual(m.cmd_type(["--layout=us", "a"]), 0)

    def test_cmd_type_falls_back_to_us_with_a_warning(self):
        with patch.object(m, "session_layout", return_value=None), patch.object(m, "run_plan", return_value=0) as run, \
                patch("sys.stderr") as err:
            self.assertEqual(m.cmd_type(["@"]), 0)
            self.assertEqual(run.call_args.args[0], [("type", [(True, 3)], 0.015)])
            self.assertIn("typing as us", "".join(c.args[0] for c in err.write.call_args_list))

    def test_cmd_type_rejects_an_unknown_layout(self):
        with patch.object(m, "run_plan") as run, patch("sys.stderr"):
            self.assertEqual(m.cmd_type(["--layout", "xx", "a"]), 2)
            run.assert_not_called()

    def test_device_script_holds_altgr_around_the_key(self):
        src = m.uinput_plan_script([("type", [(False, 16, True), (True, 3, False)], 0.01)])
        compile(src, "plan", "exec")
        self.assertIn("if ag: kb.put(1, 100, 1)", src)
        self.assertIn("if ag: kb.put(1, 100, 0)", src)
        self.assertIn(" 86,", src, "the extra key of non-US boards is on the virtual keyboard")


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
                + [("key", [125, 24], 0.05), ("type", [(True, 30), (False, 16, True)], 0.015), ("sleep", 0.1)])
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
        for name in ("type", "click", "move", "drag", "scroll", "key", "shot", "screen", "tap-window", "down", "up", "keydown",
                     "keyup", "zoom", "press", "wait", "ax"):
            self.assertIn(name, ops)

    def test_a_hold_flow_parses(self):
        lines = m.parse_seq("down 100 200\nshot /tmp/mid-drag.png\nmove 300 200\nup\nkeydown cmd\nclick 10 10\nkeyup cmd\n")
        self.assertEqual([a[0] for _, a in lines], ["down", "shot", "move", "up", "keydown", "click", "keyup"])


class RelativeMouse(unittest.TestCase):
    """`--relative`: the mouse travels by deltas that land exactly, at a pace the window server does not accelerate."""

    def test_steps_add_up_exactly_and_stay_under_the_step(self):
        for dx, dy in ((0, 0), (1, 0), (-7, 3), (100, -37), (2560, 1440), (-5, -5), (3.4, 2.6)):
            steps = m.rel_steps(dx, dy)
            self.assertEqual((sum(a for a, _ in steps), sum(b for _, b in steps)), (round(dx), round(dy)), (dx, dy))
            self.assertTrue(all(abs(a) <= 2 and abs(b) <= 2 for a, b in steps), (dx, dy, steps))
        self.assertEqual(m.rel_steps(0, 0), [(0, 0)])
        self.assertEqual(len(m.rel_steps(5, 0)), 3)  # three steps of at most 2 counts
        self.assertEqual(len(m.rel_steps(40, 10, step=4)), 10)

    def test_pace_sits_on_the_unaccelerated_plateau(self):
        speed = m.INPUT_STEP / (m.INPUT_TICK * 1000)  # counts per ms
        self.assertGreater(speed, 0.07, "below this the window server decelerates")
        self.assertLess(speed, 0.4, "from here it accelerates")
        self.assertLessEqual(m.INPUT_STEP, 2, "the shell's own acceleration knee")

    def test_relative_plans_travel_and_use_the_mouse_buttons(self):
        plan = m.click_plan(100, 200, SIZE, relative=True)
        self.assertEqual(plan[0], ("travel", 100, 200))
        self.assertEqual([o for o in plan if o[0] in ("btn", "mbtn")], [("mbtn", 0x110, 1), ("mbtn", 0x110, 0)])
        self.assertNotIn("move", [o[0] for o in plan])
        drag = m.drag_plan(10, 10, 110, 10, SIZE, relative=True)
        self.assertEqual([o[0] for o in drag if o[0] != "sleep"], ["travel", "mbtn", "travel", "mbtn"])
        self.assertEqual(m.goto(5, 6, SIZE), ("move", 5, 6))
        self.assertEqual(m.goto(-5, 6, SIZE, True), ("travel", 0, 6))

    def test_travel_time_is_budgeted(self):
        self.assertGreater(m.plan_seconds([("travel", 1, 1)], SIZE), m.plan_seconds([("move", 1, 1)], SIZE) + 5)


class Holds(unittest.TestCase):
    """`down`/`move`/`up` and `keydown`/`keyup` leave a button or key down between commands; they need the session."""

    def test_plans(self):
        self.assertEqual(m.hold_plan(10, 20, SIZE), [("move", 10, 20), ("sleep", 0.15), ("btn", 0x110, 1), ("sleep", 0.05)])
        self.assertEqual(m.hold_plan(10, 20, SIZE, right=True, relative=True)[2], ("mbtn", 0x111, 1))
        self.assertEqual(m.release_plan()[0], ("btn", 0x110, 0))
        self.assertEqual(m.release_plan(right=True)[0], ("btn", 0x111, 0))

    def test_commands_ask_for_a_held_device(self):
        with patch.object(m, "run_plan", return_value=0) as run, patch.object(m, "output_size", return_value=SIZE):
            self.assertEqual(m.cmd_down(["10", "20", "--right"]), 0)
            self.assertEqual(run.call_args.kwargs, {"hold": True})
            self.assertEqual(run.call_args.args[0][2], ("btn", 0x111, 1))
            self.assertEqual(m.cmd_up([]), 0)
            self.assertEqual(run.call_args.args[0][0], ("btn", 0x110, 0))
            self.assertEqual(m.cmd_keydown(["cmd+shift"]), 0)
            self.assertEqual(run.call_args.args[0][:2], [("kd", 125), ("kd", 42)])
            self.assertEqual(m.cmd_keyup(["cmd+shift"]), 0)
            self.assertEqual(run.call_args.args[0][:2], [("ku", 42), ("ku", 125)])
        with patch.object(m, "run_plan") as run:
            self.assertEqual(m.cmd_keydown(["nope"]), 2)
            self.assertEqual(m.cmd_keyup([]), 2)
            self.assertEqual(m.cmd_up(["5"]), 2)
            run.assert_not_called()

    def test_a_hold_refuses_the_one_shot_device(self):
        # MIDGET_SESSION=0 (set above): a one-shot helper would release the button when it exits, so a hold is refused
        with patch.object(m, "run") as run, patch("sys.stdout"):
            self.assertEqual(m.run_plan([("btn", 0x110, 1)], SIZE, hold=True), 2)
            run.assert_not_called()

    def test_one_shot_runs_the_helper_with_the_plan_embedded(self):
        import subprocess
        with patch.object(m, "run", return_value=subprocess.CompletedProcess([], 0, "", "")) as run:
            self.assertEqual(m.run_plan([("key", [28], 0.05)]), 0)
            self.assertIn("PLAN=[('key', [28], 0.05)]", run.call_args.args[0][-1].replace('\'"\'"\'', "'"))


class Session(unittest.TestCase):
    """The plumbing of the input session (the daemon and the helper are not started here)."""

    def test_helper_source_compiles_in_both_modes(self):
        serve = m.helper_script()
        compile(serve, "helper", "exec")
        self.assertIn("PLAN=None", serve)
        self.assertIn("def rel_steps(", serve, "the stepping is the same function the tests cover")
        self.assertIn("def serve(", serve)
        self.assertIn("BifrOSt remote mouse", serve)
        self.assertIn("IDLE", serve)
        one = m.helper_script([("sleep", 0.1)], (10, 10), idle=5)
        self.assertIn("PLAN=[('sleep', 0.1)]", one)
        self.assertIn("SIZE=(10, 10)", one)
        self.assertIn(" 5.0,", one)

    def test_enabled_by_default_and_off_with_zero(self):
        with patch.dict(os.environ, {"MIDGET_SESSION": "0"}):
            self.assertFalse(m.session_enabled())
        with patch.dict(os.environ, {}, clear=False):
            os.environ.pop("MIDGET_SESSION", None)
            self.assertTrue(m.session_enabled())
        os.environ["MIDGET_SESSION"] = "0"

    def test_recv_line_reads_one_line_from_chunks(self):
        class Sock:
            def __init__(self, chunks):
                self.chunks = list(chunks)

            def recv(self, n):
                return self.chunks.pop(0) if self.chunks else b""
        self.assertEqual(m.recv_line(Sock([b"ok 3", b"ms pos=1,2\nextra"])), "ok 3ms pos=1,2")
        self.assertIsNone(m.recv_line(Sock([b"no newline"])))
        self.assertIsNone(m.recv_line(Sock([])))

    def test_no_daemon_means_no_reply(self):
        with patch.object(m, "SESSION_SOCK", os.path.join(tempfile.gettempdir(), "bifrost-midget-none.sock")):
            self.assertIsNone(m.session_request({"op": "status"}, 0.2))

    def test_session_command_usage(self):
        with patch.object(m, "session_request", return_value=None), patch("sys.stdout"):
            self.assertEqual(m.cmd_session(["status"]), 1)
            self.assertEqual(m.cmd_session(["stop"]), 0)
            self.assertEqual(m.cmd_session(["bogus"]), 2)
            self.assertEqual(m.cmd_session(["start"]), 2, "MIDGET_SESSION=0 refuses to start one")


def rgb_png(w, h, color, patch_box=None, patch_color=(255, 255, 255)):
    """A solid RGB PNG, optionally with a rectangle (x0, y0, x1, y1) in another colour."""
    rows = []
    for y in range(h):
        row = bytearray(bytes(color) * w)
        if patch_box and patch_box[1] <= y < patch_box[3]:
            for x in range(patch_box[0], patch_box[2]):
                row[x * 3:x * 3 + 3] = bytes(patch_color)
        rows.append(bytes(row))
    return m.png_encode(w, h, 2, rows)


class Guard(unittest.TestCase):
    """`click --expect-unchanged`: the square around the target must still match the caller's screenshot."""

    def test_boxes_clamp_to_the_screen_and_leave_a_pointer_hole(self):
        box, hole = m.guard_boxes(100, 100, (2560, 1440), 40)
        self.assertEqual(box, (60, 60, 140, 140))
        self.assertEqual(hole, (98, 98, 130, 140))
        box, hole = m.guard_boxes(5, 1435, (2560, 1440), 40)
        self.assertEqual(box, (0, 1395, 45, 1440))
        self.assertEqual(hole, (3, 1433, 35, 1440))

    def test_diff_fraction(self):
        a = (4, 2, 3, [bytes(12), bytes(12)])
        self.assertEqual(m.region_diff(a, a), 0.0)
        b = (4, 2, 3, [bytes([0] * 9 + [200, 0, 0]), bytes(12)])
        self.assertAlmostEqual(m.region_diff(a, b), 1 / 8)
        self.assertEqual(m.region_diff(a, b, hole=(3, 0, 4, 1)), 0.0, "a changed pixel inside the hole does not count")
        noise = (4, 2, 3, [bytes([10] * 12), bytes([10] * 12)])
        self.assertEqual(m.region_diff(a, noise), 0.0, "small differences are within the tolerance")
        self.assertEqual(m.region_diff(a, (3, 2, 3, [bytes(9)] * 2)), 1.0)

    def test_check_refuses_a_changed_target_and_passes_an_unchanged_one(self):
        size = (200, 100)
        with tempfile.TemporaryDirectory() as d:
            ref = os.path.join(d, "ref.png")
            open(ref, "wb").write(rgb_png(*size, (20, 20, 20), (50, 40, 150, 60)))
            moved = rgb_png(*size, (20, 20, 20), (50, 50, 150, 70))  # the row moved 10 px down
            same = rgb_png(*size, (20, 20, 20), (50, 40, 150, 60))

            def fake_screen(fresh):
                open(fresh, "wb").write(fake_screen.png)
                return 0
            with patch.object(m, "cmd_screen", fake_screen), patch.object(m, "runtime_dir", return_value=d), patch.object(m, "KEY", os.path.join(d, "k")):
                fake_screen.png = moved
                ok, note = m.guard_check(100, 50, size, ref)
                self.assertFalse(ok)
                self.assertIn("refused", note)
                fake_screen.png = same
                ok, note = m.guard_check(100, 50, size, ref)
                self.assertTrue(ok, note)
                self.assertFalse(m.guard_check(100, 50, size, os.path.join(d, "missing.png"))[0])
                small = os.path.join(d, "small.png")
                open(small, "wb").write(rgb_png(100, 50, (0, 0, 0)))
                self.assertIn("not a full", m.guard_check(100, 50, size, small)[1])

    def test_click_stops_before_the_plan_when_refused(self):
        with patch.object(m, "output_size", return_value=SIZE), patch.object(m, "run_plan", return_value=0) as run, \
                patch.object(m, "guard_check", return_value=(False, "refused: moved")), patch("sys.stdout"):
            self.assertEqual(m.cmd_click(["10", "20", "--expect-unchanged", "--from", "/tmp/x.png"]), 3)
            run.assert_not_called()
        with patch.object(m, "output_size", return_value=SIZE), patch.object(m, "run_plan", return_value=0) as run, \
                patch.object(m, "guard_check", return_value=(True, "guard: 0.0% changed")) as guard, patch("sys.stdout"):
            self.assertEqual(m.cmd_click(["10", "20", "--expect-unchanged", "--from", "/tmp/x.png", "--radius", "24"]), 0)
            self.assertEqual(guard.call_args.args[3:], ("/tmp/x.png", 24))
            run.assert_called_once()
        with patch.object(m, "output_size", return_value=SIZE), patch.object(m, "run_plan", return_value=0) as run, patch("sys.stdout"):
            self.assertEqual(m.cmd_click(["10", "20"]), 0, "without the flag nothing is compared")

    def test_last_shot_is_remembered_with_its_scale(self):
        with tempfile.TemporaryDirectory() as d:
            png = os.path.join(d, "a.png")
            open(png, "wb").write(rgb_png(4, 4, (0, 0, 0)))
            with patch.object(m, "LAST_SHOT", os.path.join(d, "last-shot")):
                self.assertIsNone(m.last_shot())
                m.remember_shot(png, 2.0)
                self.assertEqual(m.last_shot(), (png, 2.0))
                os.unlink(png)
                self.assertIsNone(m.last_shot(), "a deleted screenshot is not a reference")


class Zoom(unittest.TestCase):
    """`zoom X Y W H`: a full-resolution crop of a region read off the small image."""

    def test_box_scales_and_clamps(self):
        self.assertEqual(m.zoom_box(10, 5, 20, 10, 2.0, (2560, 1440)), (20, 10, 60, 30))
        self.assertEqual(m.zoom_box(1270, 715, 50, 50, 2.0, (2560, 1440)), (2540, 1430, 2560, 1440))
        self.assertIsNone(m.zoom_box(1300, 0, 10, 10, 2.0, (2560, 1440)))

    def test_pure_crop_enlarges(self):
        png = m.png_crop(rgb_png(4, 4, (0, 0, 0), (2, 2, 4, 4), (255, 0, 0)), (2, 2, 4, 4), up=2)
        self.assertEqual(m.png_size(png), (4, 4))
        w, h, ch, rows = m.png_decode(png)
        self.assertEqual(bytes(rows[0]), bytes([255, 0, 0] * 4))
        self.assertIsNone(m.png_crop(b"junk", (0, 0, 1, 1)))

    def test_command_writes_the_crop(self):
        with tempfile.TemporaryDirectory() as d:
            src, out = os.path.join(d, "full.png"), os.path.join(d, "z.png")
            open(src, "wb").write(rgb_png(200, 100, (0, 0, 0), (20, 10, 60, 30), (0, 255, 0)))
            with patch("sys.stdout"):
                self.assertEqual(m.cmd_zoom(["10", "5", "20", "10", out, "--from", src, "--scale", "2"]), 0)
                self.assertEqual(m.png_size(open(out, "rb").read(32)), (80, 40))
                self.assertEqual(m.cmd_zoom(["10", "5", "20", "10", out, "--from", src, "--scale", "2", "--up", "1"]), 0)
                self.assertEqual(m.png_size(open(out, "rb").read(32)), (40, 20))
                self.assertEqual(m.cmd_zoom(["10", "5", "20"]), 2)
                self.assertEqual(m.cmd_zoom(["10", "5", "20", "10", "--from", os.path.join(d, "none.png")]), 2)
                self.assertEqual(m.cmd_zoom(["9999", "5", "20", "10", "--from", src]), 2)
            with patch.object(m, "LAST_SHOT", os.path.join(d, "last-shot")), patch("sys.stdout"):
                m.remember_shot(src, 2.0)
                self.assertEqual(m.cmd_zoom(["10", "5", "20", "10"]), 0, "the last shot and its scale are the defaults")
                self.assertEqual(m.png_size(open(os.path.join(d, "full-zoom.png"), "rb").read(32)), (80, 40))


class Windows(unittest.TestCase):
    """`shot` lists the open windows in small-image pixels."""

    def test_state_lines_parse(self):
        text = "version=1\nscreen=home\nwindow=Settings,557,187,1200,780\nwindow=External(2),0,0,2560,1440\nwindow=Odd,Name,1,2,3,4\nwindow=bad,x,y,1,2\n"
        self.assertEqual(m.state_windows(text), [("Settings", 557, 187, 1200, 780), ("External(2)", 0, 0, 2560, 1440), ("Odd,Name", 1, 2, 3, 4)])
        self.assertEqual(m.windows_in_small(m.state_windows(text)[:1], 2.0), [("Settings", 278, 94, 600, 390)])
        self.assertEqual(m.state_windows(""), [])

    def test_print_lists_windows_and_screen(self):
        import io
        with patch.object(m, "control_text", return_value="screen=home\nwindow=Settings,100,200,400,300\n"), \
                patch("sys.stdout", new_callable=io.StringIO) as out:
            m.print_windows(2.0)
        self.assertIn("screen: home", out.getvalue())
        self.assertIn("Settings at 50,100 200x150", out.getvalue())


if __name__ == "__main__":
    unittest.main()
