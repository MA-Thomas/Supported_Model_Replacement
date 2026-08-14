import importlib.util
from pathlib import Path
import tempfile
import unittest


MODULE_PATH = Path(__file__).parents[1] / "slurm_round_robin_helper.py"
SPEC = importlib.util.spec_from_file_location("slurm_round_robin_helper", MODULE_PATH)
helper = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(helper)


class SlurmHelperTests(unittest.TestCase):
    def test_parses_gnu_time_report(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "time.txt"
            path.write_text(
                "\tElapsed (wall clock) time (h:mm:ss or m:ss): 1:02.50\n"
                "\tMaximum resident set size (kbytes): 12345\n",
                encoding="utf-8",
            )
            self.assertEqual(helper.parse_time_file(path), (62.5, 12345))

    def test_parses_macos_time_report(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "time.txt"
            path.write_text(
                "       12.34 real         1.00 user         0.25 sys\n"
                "  2097152  maximum resident set size\n",
                encoding="utf-8",
            )
            self.assertEqual(helper.parse_time_file(path), (12.34, 2048))


if __name__ == "__main__":
    unittest.main()
