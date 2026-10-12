import importlib.util, os, unittest

spec = importlib.util.spec_from_file_location("ab", os.path.join(os.path.dirname(os.path.abspath(__file__)), "audio-bench.py"))
ab = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ab)


def res(bench, q=48, p50=1.0, p99=2.0, mx=3.0, **kw):
    r = {"bench": bench, "host": "h", "driver": "loopback", "rate": 48000, "quantum": q, "samples": 10,
         "p50_ms": p50, "p99_ms": p99, "max_ms": mx, "min_ms": 0.5, "jitter_ms": p99 - p50, "reported_ms": None,
         "xruns": 0, "dropped": 0, "extra": {}}
    r.update(kw)
    return r


class T(unittest.TestCase):
    def test_parse_takes_last_json(self):
        self.assertEqual(ab.parse_result('log\n{"bench":"a","x":1}\n\n')["x"], 1)
        with self.assertRaises(ValueError):
            ab.parse_result("nothing\n{broken")

    def test_cards(self):
        t = "card 0: PCH [HDA Intel PCH], device 0: ALC\ncard 0: PCH [HDA Intel PCH], device 3: HDMI\ncard 1: Loopback [Loopback], device 0: x\n"
        self.assertEqual(ab.parse_cards(t), ["PCH", "Loopback"])
        self.assertEqual(ab.pick_card(["Loopback", "PCH"]), "PCH")
        self.assertEqual(ab.pick_card(["Loopback"]), "Loopback")
        self.assertIsNone(ab.pick_card([]))

    def test_midi_budget(self):
        # reported (output latency + safety + quantum) 1.5 ms; limit 1.5 + 0.5 = 2.0
        r = res("midi-latency", p99=2.0, reported_ms=1.5)
        self.assertEqual(ab.budget_breaches(r), [])
        r["p99_ms"] = 2.01
        self.assertEqual(len(ab.budget_breaches(r)), 1)
        # no reported latency: one quantum (48 at 48 kHz = 1 ms) + 0.5
        self.assertEqual(ab.budget_breaches(res("midi-latency", p99=1.5)), [])
        self.assertEqual(len(ab.budget_breaches(res("midi-latency", p99=1.51))), 1)

    def test_roundtrip_budget(self):
        # one frame at 48 kHz = 0.0208 ms
        self.assertEqual(ab.budget_breaches(res("audio-roundtrip", p50=5.01, reported_ms=5.0)), [])
        self.assertEqual(len(ab.budget_breaches(res("audio-roundtrip", p50=5.03, reported_ms=5.0))), 1)
        self.assertEqual(ab.budget_breaches(res("audio-roundtrip", p50=9.0)), [])

    def test_sweep_budget(self):
        ok = res("xrun-sweep", extra={"smallest_clean_quantum": 64})
        bad = res("xrun-sweep", extra={"smallest_clean_quantum": 128})
        self.assertEqual(ab.budget_breaches(ok, True), [])
        self.assertEqual(len(ab.budget_breaches(bad, True)), 1)
        self.assertEqual(ab.budget_breaches(bad, False), [])

    def test_history_line(self):
        h = ab.history_line(res("midi-latency"), "d", "abc")
        self.assertEqual((h["bench"], h["git"], h["date"]), ("audio-midi-latency", "abc", "d"))

    def test_compare(self):
        prev = ab.history_line(res("midi-latency", p99=2.0), "d", "a")
        lines = ab.compare_table([res("midi-latency", p99=1.0), res("midi-latency", q=64)], [prev]).splitlines()
        self.assertEqual(len(lines), 3)
        self.assertIn("2.000 (-50.0%)", lines[1])  # q48 sorts before q64
        self.assertIn("(no previous)", lines[2])

    def test_all_zero_result_is_a_problem(self):
        self.assertIsNotNone(ab.result_problem(res("midi-latency", p50=0.0, p99=0.0, mx=0.0)))
        self.assertIsNone(ab.result_problem(res("midi-latency")))
        self.assertIsNone(ab.result_problem(res("xrun-sweep", p50=0.0, p99=0.0, mx=0.0)))

    def test_failed_bench_is_a_failure_not_a_result(self):
        calls = []

        def fake(argv, bench, driver, quantum, timeout=120, live=False):
            calls.append((bench, quantum))
            if quantum == 32:
                raise RuntimeError("exit 2: cannot reach bifrost-audiod")
            if quantum == 64:
                return res(bench, q=64, p50=0.0, p99=0.0, mx=0.0)
            return res(bench, q=quantum)

        orig, ab.run_one = ab.run_one, fake
        try:
            results, failures = ab.collect(["x"], "loopback", ["midi-latency"], [16, 32, 64], live=True)
        finally:
            ab.run_one = orig
        self.assertEqual([r["quantum"] for r in results], [16])
        self.assertEqual(len(failures), 2)
        self.assertIn("cannot reach", failures[0])
        self.assertIn("no measurements", failures[1])

    def test_pick_card_aloop(self):
        self.assertEqual(ab.pick_card(["PCH", "Loopback"], prefer_loopback=True), "Loopback")
        self.assertEqual(ab.pick_card(["PCH"], prefer_loopback=True), "PCH")

    def test_insane_latency_is_a_problem(self):
        self.assertIsNotNone(ab.result_problem(res("midi-latency", p50=60740487042.663, p99=1.0, mx=2.0)))
        self.assertIsNotNone(ab.result_problem(res("midi-latency", p50=-1.0)))
        self.assertIsNone(ab.result_problem(res("midi-latency", p50=1.0, p99=1000.0, mx=1000.0)))

    def test_no_loopback_is_skipped_not_failed(self):
        def fake(argv, bench, driver, quantum, timeout=120, live=False):
            if bench == "audio-roundtrip":
                raise ab.Skipped("no loopback: connect line out to line in, or run with --aloop")
            return res(bench, q=quantum)

        orig, ab.run_one = ab.run_one, fake
        skipped = []
        try:
            results, failures = ab.collect(["x"], "alsa:PCH", ["midi-latency", "audio-roundtrip"], [16, 32], live=True, skipped=skipped)
        finally:
            ab.run_one = orig
        self.assertEqual([(r["bench"], r["quantum"]) for r in results], [("midi-latency", 16), ("midi-latency", 32)])
        self.assertEqual(failures, [])
        self.assertEqual(len(skipped), 2)
        self.assertIn("audio-roundtrip q16", skipped[0])

    def test_exit_code_4_raises_skipped(self):
        class P:
            returncode, stdout, stderr = 4, "", "bifrost-audio-bench: no loopback: connect line out to line in, or run with --aloop"

        orig, ab.subprocess.run = ab.subprocess.run, lambda *a, **k: P()
        try:
            with self.assertRaises(ab.Skipped):
                ab.run_one(["x"], "audio-roundtrip", "alsa:PCH", 64, live=True)
        finally:
            ab.subprocess.run = orig

    def test_failed_run_exits_nonzero_and_appends_nothing(self):
        import io, contextlib, tempfile, sys
        appended = []
        saved = (ab.run_mode, ab.append_history, ab.load_history, sys.argv)
        ab.run_mode = lambda *a, **k: ([res("midi-latency")], [], ["audio-roundtrip q64: exit 2"])
        ab.append_history = lambda results: appended.append(results)
        ab.load_history = lambda *a, **k: []
        sys.argv = ["audio-bench.py", "midget", "--append"]
        try:
            with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
                with self.assertRaises(SystemExit) as cm:
                    ab.main()
        finally:
            ab.run_mode, ab.append_history, ab.load_history, sys.argv = saved
        self.assertEqual(cm.exception.code, 1)
        self.assertEqual(appended, [])


if __name__ == "__main__":
    unittest.main()
