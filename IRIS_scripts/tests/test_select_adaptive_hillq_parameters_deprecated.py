import importlib.util
import math
from pathlib import Path
import sys
import unittest

import numpy as np
import pandas as pd


SCRIPT_DIR = Path(__file__).parents[1]
sys.path.insert(0, str(SCRIPT_DIR))
MODULE_PATH = SCRIPT_DIR / "select_adaptive_hillq_parameters_deprecated.py"
SPEC = importlib.util.spec_from_file_location("select_adaptive_hillq_parameters", MODULE_PATH)
selection = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(selection)


class AdaptiveHillQSelectionTests(unittest.TestCase):
    def test_q2_offer_matches_hill2_definition(self):
        rosters = [
            np.asarray([], dtype=float),
            np.asarray([-2.0]),
            np.asarray([-2.0, -2.5, -4.0]),
        ]
        maxima, offer = selection.hill_components(rosters, 2.0)
        expected_maxima = np.asarray([selection.FLOOR, -2.0, -2.0])
        np.testing.assert_allclose(maxima, expected_maxima)
        values = rosters[2]
        relative = np.exp(values - values.max())
        probabilities = relative / relative.sum()
        expected = np.log(relative.sum()) * (1.0 - np.sum(probabilities**2))
        self.assertAlmostEqual(offer[2], expected)

    def test_uniform_plateau_has_same_offer_for_all_orders(self):
        roster = [np.asarray([-2.0, -2.0, -2.0])]
        expected = (2.0 / 3.0) * np.log(3.0)
        for q in (0.0, 0.5, 1.0, 2.0, 4.0, math.inf):
            _, offer = selection.hill_components(roster, q)
            self.assertAlmostEqual(offer[0], expected)

    def test_q_boundaries_have_expected_inverse_hill_numbers(self):
        values = np.asarray([0.0, -1.0, -2.0])
        relative = np.exp(values - values.max())
        probabilities = relative / relative.sum()
        bonus = np.log(relative.sum())
        _, offer_q0 = selection.hill_components([values], 0.0)
        _, offer_qinf = selection.hill_components([values], math.inf)
        self.assertAlmostEqual(offer_q0[0], bonus * (1.0 - 1.0 / 3.0))
        self.assertAlmostEqual(offer_qinf[0], bonus * (1.0 - probabilities.max()))

    def test_parse_q_values_rejects_duplicates_and_negative_orders(self):
        self.assertEqual(selection.parse_q_values(["0", "1", "inf"]), [0.0, 1.0, math.inf])
        with self.assertRaises(selection.SelectionError):
            selection.parse_q_values(["1", "1.0"])
        with self.assertRaises(selection.SelectionError):
            selection.parse_q_values(["-0.5"])

    def test_self_gated_scores_satisfy_equation_and_bounds(self):
        maxima = np.asarray([-2.0, 0.0, selection.FLOOR])
        offer = np.asarray([1.5, 0.25, 0.0])
        c = np.asarray([-1.0, 0.5])
        kappa = np.asarray([0.5, 0.0001])
        scores = selection.self_gated_scores(
            maxima,
            offer,
            c,
            kappa,
            absolute_tolerance=1e-12,
            max_iterations=64,
        )
        rhs = maxima[None, :] + offer[None, :] * selection.expit(
            (c[:, None] - scores) / kappa[:, None]
        )
        np.testing.assert_allclose(scores, rhs, atol=2e-9, rtol=0.0)
        self.assertTrue(np.all(scores >= maxima[None, :] - 1e-12))
        self.assertTrue(np.all(scores <= maxima[None, :] + offer[None, :] + 1e-12))
        np.testing.assert_allclose(scores[:, 2], selection.FLOOR, atol=1e-12)

    def test_self_gated_scores_reproduce_memo_examples(self):
        rosters = [
            np.asarray([0.0, -5.0, -5.0]),
            np.asarray([-2.0, -2.0, -2.0]),
            np.asarray([0.0, 0.0, 0.0]),
            np.repeat(-2.0, 10),
        ]
        maxima, offer = selection.hill_components(rosters, 2.0)
        scores = selection.self_gated_scores(
            maxima,
            offer,
            np.asarray([-1.0]),
            np.asarray([0.5]),
            absolute_tolerance=1e-12,
            max_iterations=64,
        )[0]
        np.testing.assert_allclose(
            scores,
            np.asarray([0.00004, -1.473, 0.076, -0.982]),
            atol=6e-4,
            rtol=0.0,
        )

    def test_self_gated_solver_enforces_iteration_contract(self):
        with self.assertRaises(selection.SelectionError):
            selection.self_gated_scores(
                np.asarray([-2.0]),
                np.asarray([1.0]),
                np.asarray([-1.0]),
                np.asarray([0.5]),
                absolute_tolerance=1e-12,
                max_iterations=1,
            )

    def test_joint_triple_is_selected_separately_by_group(self):
        joint_grid = pd.DataFrame(
            {
                "joint_grid_index": [0, 1],
                "q_order": [0, 1],
                "q_id": ["q0", "qinf"],
                "q": [0.0, math.inf],
                "grid_index": [0, 0],
                "c": [-1.0, -1.0],
                "kappa": [0.5, 0.5],
            }
        )
        q0_values = {
            cohort: np.asarray([1.0, 0.0]) for cohort in selection.SELECTION_COHORTS
        }
        qinf_values = {
            cohort: np.asarray([0.0, 1.0]) for cohort in selection.SELECTION_COHORTS
        }
        q0_ranking = selection.rank_joint_group("model_a", "pr", joint_grid, q0_values)
        qinf_ranking = selection.rank_joint_group("model_b", "pr", joint_grid, qinf_values)
        q0_selected = selection.selected_group_parameters(
            "model_a", "pr", q0_ranking, q0_values, 0.0
        )
        qinf_selected = selection.selected_group_parameters(
            "model_b", "pr", qinf_ranking, qinf_values, 0.0
        )
        self.assertEqual(q0_selected["q_id"], "q0")
        self.assertEqual(qinf_selected["q_id"], "qinf")
        self.assertTrue(q0_selected["q_is_zero_or_infinity"])
        self.assertTrue(qinf_selected["q_is_zero_or_infinity"])

    def test_leave_one_cohort_out_repeats_joint_selection(self):
        joint_grid = pd.DataFrame(
            {
                "joint_grid_index": [0, 1],
                "q_order": [0, 1],
                "q_id": ["q0", "q2"],
                "q": [0.0, 2.0],
                "grid_index": [0, 0],
                "c": [-1.0, -1.0],
                "kappa": [0.5, 0.5],
            }
        )
        values = {
            cohort: np.asarray([1.0, 0.0]) for cohort in selection.SELECTION_COHORTS
        }
        ranking = selection.rank_joint_group("model_a", "roc", joint_grid, values)
        full = selection.selected_group_parameters(
            "model_a", "roc", ranking, values, 0.0
        )
        diagnostics = selection.leave_one_cohort_out_diagnostics(
            "model_a", "roc", ranking, full
        )
        self.assertEqual(len(diagnostics), len(selection.SELECTION_COHORTS))
        self.assertTrue(diagnostics["matches_full_selection"].all())
        self.assertEqual(set(diagnostics["q_id"]), {"q0"})


if __name__ == "__main__":
    unittest.main()
