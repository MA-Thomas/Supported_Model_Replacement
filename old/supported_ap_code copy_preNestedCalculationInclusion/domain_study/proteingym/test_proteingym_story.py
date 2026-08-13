"""Deterministic checks for the ProteinGym study layer."""

from __future__ import annotations

import tempfile
import unittest
import zipfile
from pathlib import Path

import pandas as pd

from domain_study.proteingym import proteingym_story


def metadata_fixture() -> pd.DataFrame:
    common = {
        "UniProt_ID": "P_TEST",
        "taxon": "Human",
        "source_organism": "Homo sapiens",
        "first_author": "Author",
        "title": "Fixture assay",
        "year": 2024,
        "coarse_selection_type": "Activity",
        "selection_assay": "activity",
        "DMS_total_number_mutants": 4,
        "DMS_number_single_mutants": 3,
        "DMS_number_multiple_mutants": 1,
    }
    return pd.DataFrame(
        [
            {
                **common,
                "DMS_id": "manual_assay",
                "DMS_filename": "manual_assay.csv",
                "DMS_binarization_method": "manual",
            },
            {
                **common,
                "DMS_id": "median_assay",
                "DMS_filename": "median_assay.csv",
                "DMS_binarization_method": "median",
            },
        ]
    )


SOURCE_CSV = """mutant,DMS_score_bin
A1V,1
B2C,0
C3D,1
A1V:B2C,0
"""

SCORE_CSV = """mutant,DMS_score_bin,EVE_ensemble,ESM1v_single,ESM1v_ensemble,ESM2_650M
A1V,1,0.5,0.6,0.7,0.8
B2C,0,-0.5,-0.6,-0.7,-0.8
C3D,1,0.4,0.5,0.6,0.7
A1V:B2C,0,-0.2,-0.3,-0.4,-0.5
"""


class ProteinGymStudyTests(unittest.TestCase):
    def test_three_model_comparison_set_is_complete(self) -> None:
        self.assertEqual(
            [comparison.slug for comparison in proteingym_story.COMPARISONS],
            [
                "esm1v_ensemble_vs_eve_ensemble",
                "esm2_650m_vs_esm1v_ensemble",
                "esm2_650m_vs_eve_ensemble",
            ],
        )

    def test_build_observed_frame_aligns_rows_and_orients_damage_scores(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            dms_path = root / "dms.zip"
            score_path = root / "scores.zip"
            with zipfile.ZipFile(dms_path, "w") as archive:
                archive.writestr(
                    "DMS_ProteinGym_substitutions/manual_assay.csv", SOURCE_CSV
                )
                archive.writestr(
                    "DMS_ProteinGym_substitutions/median_assay.csv", SOURCE_CSV
                )
            with zipfile.ZipFile(score_path, "w") as archive:
                archive.writestr("manual_assay.csv", SCORE_CSV)
                archive.writestr("median_assay.csv", SCORE_CSV)

            observed, audit = proteingym_story.build_observed_frame(
                metadata_fixture(), dms_path, score_path
            )

        self.assertEqual(audit["assay_id"].tolist(), ["manual_assay"])
        self.assertEqual(observed["variant_id"].tolist(), ["A1V", "B2C", "C3D"])
        self.assertEqual(observed["label"].tolist(), [0, 1, 0])
        self.assertEqual(observed["score_eve_ensemble"].tolist(), [-0.5, 0.5, -0.4])
        self.assertEqual(int(audit.loc[0, "deleterious_count"]), 1)
        self.assertEqual(int(audit.loc[0, "functional_count"]), 2)

    def test_rust_command_uses_observed_evidence_without_resampling(self) -> None:
        command = proteingym_story.observed_ap_command(
            Path("supported_ap"),
            Path("paired.csv"),
            Path("report.json"),
            proteingym_story.PROFILES["quick"],
            proteingym_story.COMPARISONS[0],
            2,
        )

        def value_after(option: str) -> str:
            return command[command.index(option) + 1]

        self.assertEqual(command[:3], ["supported_ap", "ap", "observed"])
        self.assertEqual(value_after("--evaluation-id-col"), "assay_id")
        self.assertEqual(value_after("--empirical-order"), "2")
        self.assertEqual(value_after("--interval-lower"), "0.01")
        self.assertEqual(value_after("--interval-upper"), "0.9")
        self.assertIn("--reference-limitation", command)
        self.assertNotIn("--computational-replications", command)
        self.assertNotIn("--resampling-seed", command)

    def test_auroc_command_uses_observed_evidence_without_resampling(self) -> None:
        command = proteingym_story.observed_auroc_command(
            Path("supported_ap"),
            Path("paired.csv"),
            Path("auroc.json"),
            proteingym_story.COMPARISONS[0],
        )

        def value_after(option: str) -> str:
            return command[command.index(option) + 1]

        self.assertEqual(command[:3], ["supported_ap", "auroc", "observed"])
        self.assertEqual(value_after("--evaluation-id-col"), "assay_id")
        self.assertEqual(value_after("--empirical-order"), "2")
        self.assertEqual(value_after("--survival-requirement"), "0.81")
        self.assertIn("--reference-limitation", command)
        self.assertNotIn("--computational-replications", command)
        self.assertNotIn("--resampling-seed", command)
        self.assertNotIn("--resampling-unit", command)

    def test_report_validation_requires_explicit_assay_ids(self) -> None:
        report = {
            "schema_version": 17,
            "manuscript_version": "v17",
            "analysis": "ap_observed_empirical_gate",
            "evaluation_ids": ["a", "b"],
            "result": {
                "evidence": "observed",
                "evaluation_count": 2,
                "empirical_order": 2,
                "forward": {"monte_carlo_standard_error": None},
            },
        }
        proteingym_story.validate_report(report, ["a", "b"], 2)
        with self.assertRaises(RuntimeError):
            proteingym_story.validate_report(report, ["b", "a"], 2)


if __name__ == "__main__":
    unittest.main()
