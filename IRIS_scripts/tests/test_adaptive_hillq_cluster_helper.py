import argparse
import importlib.util
import json
import tempfile
import unittest
from pathlib import Path


HELPER = (
    Path(__file__).resolve().parents[1]
    / "adaptive_selection"
    / "adaptive_hillq_cluster_helper.py"
)
SPEC = importlib.util.spec_from_file_location("adaptive_hillq_cluster_helper", HELPER)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


class CheckPlanGridTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.plan = Path(self.temporary.name)
        (self.plan / "plan.json").write_text(
            json.dumps(
                {
                    "grid": {
                        "alpha_values": ["0.5", "1.0", "2.0", "4.0", "inf"],
                        "q_values": ["0.5", "1.0", "2.0", "inf"],
                        "c_min": -11.0,
                        "c_max": 3.2,
                        "c_step": 0.2,
                        "kappa_min": 0.02,
                        "kappa_max": 4.0,
                        "kappa_points": 6,
                    }
                }
            ),
            encoding="utf-8",
        )

    def tearDown(self):
        self.temporary.cleanup()

    def arguments(self, **overrides):
        values = {
            "plan": self.plan,
            "alpha_values": ["0.5", "1", "2", "4", "infinity"],
            "q_values": ["0.5", "1", "2", "inf"],
            "c_min": -11.0,
            "c_max": 3.2,
            "c_step": 0.2,
            "kappa_min": 0.02,
            "kappa_max": 4.0,
            "kappa_points": 6,
        }
        values.update(overrides)
        return argparse.Namespace(**values)

    def test_accepts_equivalent_explicit_compact_grid(self):
        MODULE.check_plan_grid(self.arguments())

    def test_rejects_reused_plan_with_different_grid(self):
        with self.assertRaisesRegex(ValueError, "existing plan c_min differs"):
            MODULE.check_plan_grid(self.arguments(c_min=-9.5))


if __name__ == "__main__":
    unittest.main()
