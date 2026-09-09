import importlib.util
from pathlib import Path
import sys
import unittest

import numpy as np
import pandas as pd


SCRIPT_DIR = Path(__file__).parents[1]
sys.path.insert(0, str(SCRIPT_DIR))
MODULE_PATH = SCRIPT_DIR / "select_adaptive_hill2_parameters_deprecated.py"
SPEC = importlib.util.spec_from_file_location("select_adaptive_hill2_parameters", MODULE_PATH)
selection = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(selection)


class AdaptiveHill2SelectionTests(unittest.TestCase):
    def test_empty_singleton_and_equal_plateau_components(self):
        maxima, admitted = selection.hill2_components(
            [
                np.asarray([], dtype=float),
                np.asarray([-2.0]),
                np.asarray([-2.0, -2.0, -2.0]),
            ]
        )
        self.assertEqual(maxima[0], selection.FLOOR)
        self.assertEqual(admitted[0], 0.0)
        self.assertEqual(maxima[1], -2.0)
        self.assertEqual(admitted[1], 0.0)
        self.assertAlmostEqual(admitted[2], (2.0 / 3.0) * np.log(3.0))

    def test_adaptive_score_lies_between_max_and_lse(self):
        candidates = [np.asarray([-2.0, -2.0, -2.0])]
        maxima, admitted = selection.hill2_components(candidates)
        scores = selection.adaptive_scores(
            maxima,
            admitted,
            np.asarray([-1.0]),
            np.asarray([0.5]),
        )
        self.assertGreaterEqual(scores[0, 0], maxima[0])
        self.assertLessEqual(scores[0, 0], maxima[0] + np.log(3.0))

    def test_ap_and_auc_include_tie_blocks(self):
        labels = np.asarray([1, 0], dtype=np.int8)
        scores = np.asarray(
            [
                [2.0, 1.0],
                [1.0, 2.0],
                [1.0, 1.0],
            ]
        )
        ap, auc = selection.ap_auc_from_score_matrix(labels, scores)
        np.testing.assert_allclose(ap, [1.0, 0.5, 0.5])
        np.testing.assert_allclose(auc, [1.0, 0.0, 0.5])

    def test_equal_cohort_rank_selection(self):
        grid = pd.DataFrame(
            {
                "grid_index": [0, 1, 2],
                "c": [-2.0, -1.0, 0.0],
                "kappa": [0.5, 0.5, 0.5],
            }
        )
        values = {
            "pdac": np.asarray([0.9, 0.8, 0.7]),
            "covid_spike": np.asarray([0.7, 0.9, 0.8]),
            "covid_nonspike": np.asarray([0.7, 0.9, 0.8]),
        }
        _, summary = selection.rank_group("full_hla", "pr", grid, values)
        self.assertEqual(summary["grid_index"], 1)


if __name__ == "__main__":
    unittest.main()
