import importlib.util
from pathlib import Path
import unittest

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("app_compliance", HERE / "check-app-compliance.py")
compliance = importlib.util.module_from_spec(spec)
spec.loader.exec_module(compliance)


class ResidualWakeDetection(unittest.TestCase):
    def test_files_drag_and_debounce_deadlines_are_not_worker_polling(self):
        files = HERE.parent / "springboard/apps/files/src/main.rs"
        lines = files.read_text().splitlines()
        start = next(i for i, line in enumerate(lines) if "fn next_wake(" in line)
        body = compliance.rust_function_body(lines, start)
        source = "\n".join(p.read_text() for p in (files.parent / "ops.rs", files.parent / "search.rs"))

        self.assertIn("next_deadline", body)
        self.assertIn("t + 0.1", body)
        self.assertFalse(compliance.res02_required(body, source, []))

    def test_pending_receiver_poll_is_reported_despite_unrelated_wake_facilities(self):
        body = """fn next_wake(&self, t: f64) -> Option<f64> {
            if self.job.is_some() { Some(t + 0.05) } else { None }
        }"""
        source = """
            use std::sync::mpsc::Receiver;
            fn wake_fd(&self) -> Option<i32> { self.monitor_fd }
            DispatchQueue::main().async_(|| {});
        """

        self.assertTrue(compliance.res02_required(body, source, []))


if __name__ == "__main__":
    unittest.main()
