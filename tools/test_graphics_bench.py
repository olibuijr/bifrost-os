"""Tests for tools/graphics_bench.py: run with `python3 -I -m unittest tools/test_graphics_bench.py`."""
import os, sys, unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import graphics_bench as gb  # noqa: E402


def frame(seq, cpu=1000, gpu=2000, lat=16000, missed=0, dmg=100, t=None):
    f = {"seq": seq, "cpu_us": cpu, "gpu_us": gpu, "latency_us": lat, "missed": missed, "damaged_px": dmg}
    if t is not None:
        f["t_us"] = t
    return f


class Percentile(unittest.TestCase):
    def test_nearest_rank_on_ten_samples(self):
        s = list(range(1, 11))  # 1..10
        self.assertEqual(gb.percentile(s, 50), 5)   # rank ceil(5) = 5
        self.assertEqual(gb.percentile(s, 95), 10)  # rank ceil(9.5) = 10
        self.assertEqual(gb.percentile(s, 99), 10)

    def test_exact_product_does_not_round_up(self):
        s = list(range(1, 21))  # 0.95 * 20 = 19 exactly
        self.assertEqual(gb.percentile(s, 95), 19)

    def test_clamps_and_empty(self):
        self.assertIsNone(gb.percentile([], 50))
        self.assertEqual(gb.percentile([3, 1, 2], 0), 1)    # rank clamped to 1
        self.assertEqual(gb.percentile([3, 1, 2], 150), 3)  # clamped to 100

    def test_unsorted_input(self):
        self.assertEqual(gb.percentile([9, 1, 5, 3, 7], 50), 5)


class Parse(unittest.TestCase):
    def test_skips_partial_and_bad_lines(self):
        text = ('{"seq":1,"cpu_us":10,"gpu_us":20,"latency_us":5,"missed":0,"damaged_px":3}\n'
                '\n'
                'not json\n'
                '{"seq":2,"cpu_us":10,"gpu_us":2')  # trailing partial line
        frames = gb.parse_frames(text)
        self.assertEqual([f["seq"] for f in frames], [1])

    def test_requires_core_keys(self):
        self.assertEqual(gb.parse_frames('{"seq":1}\n[1,2]\n'), [])

    def test_keeps_optional_timestamp(self):
        f = gb.parse_frames('{"seq":4,"cpu_us":1,"gpu_us":1,"t_us":99}\n')[0]
        self.assertEqual(f["t_us"], 99)


class Dropped(unittest.TestCase):
    def test_ratio_counts_missed_frames(self):
        fs = [frame(1), frame(2, missed=1), frame(3), frame(4, missed=2)]
        self.assertAlmostEqual(gb.dropped_ratio(fs), 0.5)

    def test_empty_is_none(self):
        self.assertIsNone(gb.dropped_ratio([]))

    def test_busy_frames_filter_on_damage(self):
        fs = [frame(1, dmg=0), frame(2, dmg=5), frame(3, dmg=0, missed=1)]
        self.assertEqual([f["seq"] for f in gb.busy_frames(fs)], [2])

    def test_idle_desktop_falls_back_to_all_frames(self):
        fs = [frame(1, dmg=0), frame(2, dmg=0, missed=1)]
        res = gb.summarize(fs)
        self.assertEqual(res["gfx_busy_frames"], 0)
        self.assertAlmostEqual(res["gfx_dropped_ratio"], 0.5)


class Photon(unittest.TestCase):
    def test_first_frame_after_click_is_the_photon(self):
        fs = [frame(1, t=1_000_000), frame(2, t=1_010_000), frame(3, t=1_030_000)]
        # click bracketed by 1_000_000 and 1_005_000: the first frame at or after 1_005_000 is at 1_010_000
        self.assertEqual(gb.photon_latencies_ms([(1_000_000, 1_005_000)], fs), [10.0])

    def test_frame_exactly_at_click_counts(self):
        fs = [frame(1, t=2_000)]
        self.assertEqual(gb.photon_latencies_ms([(1_000, 2_000)], fs), [1.0])

    def test_no_later_frame_drops_the_click(self):
        fs = [frame(1, t=500)]
        self.assertEqual(gb.photon_latencies_ms([(1_000, 2_000)], fs), [])

    def test_frames_without_timestamp_are_ignored(self):
        fs = [frame(1), frame(2, t=5_000)]
        self.assertEqual(gb.photon_latencies_ms([(0, 1_000)], fs), [5.0])

    def test_several_clicks_each_match_their_own_frame(self):
        fs = [frame(1, t=10_000), frame(2, t=30_000)]
        self.assertEqual(gb.photon_latencies_ms([(0, 5_000), (15_000, 20_000)], fs), [10.0, 15.0])


class Summarize(unittest.TestCase):
    def test_compose_and_gpu_percentiles_in_ms(self):
        fs = [frame(i, cpu=i * 100, gpu=i * 1000) for i in range(1, 11)]
        res = gb.summarize(fs)
        self.assertEqual(res["frames"], 10)
        self.assertAlmostEqual(res["gfx_cpu_p95_ms"], 1.0)   # max cpu 1000 us
        self.assertAlmostEqual(res["gfx_gpu_p50_ms"], 5.0)
        self.assertAlmostEqual(res["gfx_gpu_p99_ms"], 10.0)

    def test_photon_keys_only_with_events(self):
        res = gb.summarize([frame(1, t=2_000)])
        self.assertNotIn("gfx_photon_p95_ms", res)
        res = gb.summarize([frame(1, t=2_000)], [(1_000, 1_500)])
        self.assertAlmostEqual(res["gfx_photon_p95_ms"], 1.0)
        self.assertEqual(res["gfx_photon_clicks"], 1)


class Gate(unittest.TestCase):
    GOOD = {"gfx_cpu_p95_ms": 0.5, "gfx_gpu_p95_ms": 3.9, "gfx_dropped_ratio": 0.0, "gfx_photon_p95_ms": 20.0}

    def test_within_budget_has_no_breach(self):
        self.assertEqual(gb.gate(self.GOOD), [])

    def test_cpu_over_budget(self):
        res = dict(self.GOOD, gfx_cpu_p95_ms=1.2)
        self.assertEqual(len(gb.gate(res)), 1)

    def test_gpu_exactly_at_limit_passes_max(self):
        res = dict(self.GOOD, gfx_gpu_p95_ms=4.0)
        self.assertEqual(gb.gate(res), [])

    def test_photon_exactly_at_limit_fails_strict(self):
        res = dict(self.GOOD, gfx_photon_p95_ms=25.0)
        self.assertEqual(len(gb.gate(res)), 1)

    def test_dropped_ratio_over_one_percent(self):
        res = dict(self.GOOD, gfx_dropped_ratio=0.02)
        breaches = gb.gate(res)
        self.assertEqual(len(breaches), 1)
        self.assertIn("gfx_dropped_ratio", breaches[0])

    def test_missing_measure_is_a_breach(self):
        res = {k: v for k, v in self.GOOD.items() if k != "gfx_photon_p95_ms"}
        breaches = gb.gate(res)
        self.assertEqual(len(breaches), 1)
        self.assertIn("not measured", breaches[0])


if __name__ == "__main__":
    unittest.main()
