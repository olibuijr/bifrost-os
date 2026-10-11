import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(__file__))
import fps

LOG = """springboard: frame draw 4.0 present 1.0 damage 100% gap 16.7
springboard: frame draw 6.0 present 2.0 damage 40% gap 16.7
springboard: frame draw 0.0 present 0.0 damage 0% gap 0.0
springboard: frame draw 20.0 present 1.0 damage 100% gap 33.4
springboard: frame draw 5.0 present 1.0 damage 100% gap 900.0
springboard: frame draw 5.0 present 1.0 damage 100% gap inf
"""

class FpsTests(unittest.TestCase):
    def test_parse_skips_idle_and_infinite_gaps(self):
        self.assertEqual(fps.parse_frames(LOG), [(4.0, 1.0, 16.7), (6.0, 2.0, 16.7), (20.0, 1.0, 33.4), (5.0, 1.0, 900.0)])

    def test_summary_ignores_pauses_between_gestures(self):
        s = fps.summarize(fps.parse_frames(LOG))
        self.assertEqual(s["frames"], 3)
        self.assertAlmostEqual(s["fps"], 1000 / ((16.7 + 16.7 + 33.4) / 3), places=3)
        self.assertEqual(s["over_16_7"], 1)
        self.assertEqual(s["draw_max"], 20.0)
        self.assertIsNone(fps.summarize([]))

    def test_percentile(self):
        self.assertEqual(fps.percentile([1, 2, 3, 4, 5, 6, 7, 8, 9, 10], 0.95), 10)
        self.assertEqual(fps.percentile([], 0.5), 0.0)


if __name__ == "__main__":
    unittest.main()
