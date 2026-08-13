"""Deterministic checks for the isolated conditional nested-study layer."""

from __future__ import annotations

import math
import unittest
from pathlib import Path

import numpy as np
import pandas as pd

from domain_study.proteingym_nested import conditional_nested_story as story


class ConditionalNestedStoryTests(unittest.TestCase):
    def test_position_parser_groups_substitutions_by_residue(self) -> None:
        variants = pd.Series(["A1V", "A1G", "L27P", "W103*"])
        self.assertEqual(story.position_from_variant(variants).tolist(), [1, 1, 27, 103])
        with self.assertRaises(RuntimeError):
            story.position_from_variant(pd.Series(["A1V:B2C"]))

    def test_rust_command_declares_the_full_conditional_design(self) -> None:
        command = story.rust_command(
            Path("proteingym_nested"),
            Path("input.csv"),
            Path("report.json"),
            story.PROFILES["quick"],
        )

        def value_after(option: str) -> str:
            return command[command.index(option) + 1]

        self.assertEqual(value_after("--cluster-id-col"), "position_id")
        self.assertEqual(value_after("--interval-lower"), "0.4")
        self.assertEqual(value_after("--interval-upper"), "0.5")
        self.assertEqual(value_after("--empirical-order"), "2")
        self.assertEqual(value_after("--computational-order"), "2")
        self.assertEqual(value_after("--survival-requirement"), "0.6")
        self.assertEqual(value_after("--computational-replications"), "16")

    def test_nested_survival_curve_matches_order_two_formula(self) -> None:
        report = {
            "result": {
                "forward": {
                    "observed_gate": {},
                    "full_assessment": {
                        "retained_effect_rows": [
                            {
                                "observed": {"value": 0.3},
                                "computational": [
                                    {"value": 0.2},
                                    {"value": 0.1},
                                    {"value": -0.1},
                                ],
                            },
                            {
                                "observed": {"value": 0.2},
                                "computational": [
                                    {"value": 0.4},
                                    {"value": 0.1},
                                    {"value": 0.05},
                                ],
                            },
                        ]
                    },
                }
            }
        }
        curve = story.build_survival_curve(report, np.array([0.0]))
        self.assertEqual(curve.loc[0, "observed_pair_survival"], 1.0)
        expected = (math.comb(2, 2) / math.comb(3, 2)) * 1.0
        self.assertAlmostEqual(curve.loc[0, "anchored_nested_survival"], expected)

    def test_output_directory_is_disjoint_from_existing_studies(self) -> None:
        self.assertNotEqual(story.OUTPUTS, story.PARENT_STUDY / "outputs")
        self.assertNotEqual(story.OUTPUTS, story.ROOT / "domain_study" / "outputs")


if __name__ == "__main__":
    unittest.main()
