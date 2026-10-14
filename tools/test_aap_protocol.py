import importlib.util
import pathlib
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location("aap", ROOT / "springboard/tools/aap.py")
aap = importlib.util.module_from_spec(spec)
spec.loader.exec_module(aap)


class SdkProtocol(unittest.TestCase):
    def test_reads_the_constant_from_the_proto_crate(self):
        with tempfile.TemporaryDirectory() as d:
            f = pathlib.Path(d) / "lib.rs"
            f.write_text("pub const SDK_VERSION: u32 = 7;\n")
            self.assertEqual(aap.sdk_protocol(str(f)), 7)
        self.assertGreaterEqual(aap.sdk_protocol(), 1)

    def test_stamp_adds_a_top_level_key_before_any_table(self):
        out = aap.stamp_protocol('id = "a.b"\n[[intent]]\nx = "y"\n', 2)
        self.assertEqual(aap.parse_manifest(out)["sdk_protocol"], "2")
        self.assertTrue(out.startswith("sdk_protocol = 2\n"))

    def test_stamp_replaces_an_old_value(self):
        out = aap.stamp_protocol('sdk_protocol = 1\nid = "a.b"\n', 2)
        self.assertEqual(out.count("sdk_protocol"), 1)
        self.assertEqual(aap.parse_manifest(out)["sdk_protocol"], "2")


if __name__ == "__main__":
    unittest.main()
