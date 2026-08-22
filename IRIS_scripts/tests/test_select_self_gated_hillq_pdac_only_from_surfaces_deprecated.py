import importlib.util
from pathlib import Path
import sys
import unittest

import numpy as np
import pandas as pd


SCRIPT_DIR = Path(__file__).parents[1]
sys.path.insert(0, str(SCRIPT_DIR))
MODULE_PATH = SCRIPT_DIR / "select_self_gated_hillq_pdac_only_from_surfaces_deprecated.py"
SPEC = importlib.util.spec_from_file_location(
    "select_self_gated_hillq_pdac_only_from_surfaces", MODULE_PATH
)
analysis = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(analysis)


class PdacOnlySelectionTests(unittest.TestCase):
    def test_selection_uses_only_pdac_objectives(self):
        frame = pd.DataFrame(
            {
                "joint_grid_index": [0, 1],
                "q_order": [0, 1],
                "q_id": ["q0", "q2"],
                "q": [0.0, 2.0],
                "grid_index": [0, 0],
                "c": [-1.0, -1.0],
                "kappa": [0.5, 0.5],
                "pdac_fractional_rank": [0.0, 1.0],
                "pdac_regret": [0.0, 0.2],
                "covid_spike_fractional_rank": [1.0, 0.0],
                "covid_nonspike_fractional_rank": [1.0, 0.0],
            }
        )
        selected, diagnostics = analysis.select_pdac_row(frame, 0.01)
        self.assertEqual(int(selected["joint_grid_index"]), 0)
        self.assertEqual(diagnostics["near_optimal_q_ids"], "q0")

    def test_metric_diagnostics_use_average_tie_ranks(self):
        result = analysis.metric_diagnostics(np.asarray([0.8, 0.8, 0.5]), 1)
        self.assertEqual(result["value"], 0.8)
        self.assertEqual(result["rank"], 1.5)
        self.assertEqual(result["fractional_rank"], 0.25)
        self.assertEqual(result["regret"], 0.0)


if __name__ == "__main__":
    unittest.main()
