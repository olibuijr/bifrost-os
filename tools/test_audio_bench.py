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
        # quantum 48 at 48 kHz = 1 ms; limit 1 + 0.5 + 0.5 = 2.0
        r = res("midi-latency", p99=2.0, extra={"output_latency_ms": 0.5})
        self.assertEqual(ab.budget_breaches(r), [])
        r["p99_ms"] = 2.01
        self.assertEqual(len(ab.budget_breaches(r)), 1)

    def test_roundtrip_budget(self):
        # one frame at 48 kHz = 0.0208 ms
        self.assertEqual(ab.budget_breaches(res("audio-roundtrip", p50=5.01, reported_ms=5.0)), [])
        self.assertEqual(len(ab.budget_breaches(res("audio-roundtrip", p50=5.03, reported_ms=5.0))), 1)
        self.assertEqual(ab.budget_breaches(res("audio-roundtrip", p50=9.0)), [])

    def test_sweep_budget(self):
        ok = res("xrun-sweep", extra={"min_stable_quantum": 64})
        bad = res("xrun-sweep", extra={"min_stable_quantum": 128})
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


if __name__ == "__main__":
    unittest.main()
