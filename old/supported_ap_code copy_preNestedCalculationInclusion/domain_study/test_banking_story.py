"""Small deterministic checks for the Python study layer."""

from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

from domain_study import banking_story


class StudyOrchestrationTests(unittest.TestCase):
    def test_raw_sources_have_the_declared_checksums(self) -> None:
        self.assertEqual(
            banking_story.sha256(banking_story.ARCHIVE_PATH),
            banking_story.ARCHIVE_SHA256,
        )
        self.assertEqual(
            banking_story.sha256(banking_story.CSV_PATH),
            banking_story.CSV_SHA256,
        )

    def test_auroc_projected_design_tracks_the_temporal_holdout_mix(self) -> None:
        frame = banking_story.load_data()
        split = int(len(frame) * banking_story.TRAIN_FRACTION)
        holdout = frame.iloc[split:]
        self.assertGreater(int(holdout["label"].sum()), 0)
        self.assertGreater(int((1 - holdout["label"]).sum()), 0)

    def test_auroc_command_makes_every_assessment_choice_explicit(self) -> None:
        profile = banking_story.PROFILES["quick"]
        command = banking_story.auroc_rust_command(
            Path("supported_ap"),
            Path("predictions.csv"),
            Path("auroc.json"),
            profile,
            "candidate",
            "incumbent",
            resampling_seed=17,
        )

        def value_after(option: str) -> str:
            return command[command.index(option) + 1]

        self.assertEqual(command[:3], ["supported_ap", "auroc", "projected"])
        self.assertEqual(value_after("--computational-order"), "2")
        self.assertNotIn("--replication-positive-count", command)
        self.assertNotIn("--replication-negative-count", command)
        self.assertEqual(value_after("--computational-replications"), "24")
        self.assertEqual(value_after("--survival-requirement"), "0.81")
        self.assertIn("--reference-limitation", command)
        self.assertEqual(value_after("--solver-threads"), "1")
        self.assertEqual(
            value_after("--optimization-time-limit-seconds"),
            str(banking_story.AUROC_OPTIMIZATION_TIME_LIMIT_SECONDS),
        )
        self.assertNotIn("--confidence-level", command)
        self.assertNotIn("--outer-replicates", command)

    def test_ap_command_delegates_all_ap_and_support_work_to_rust(self) -> None:
        profile = banking_story.PROFILES["quick"]
        command = banking_story.paired_rust_command(
            Path("supported_ap"),
            Path("predictions.csv"),
            Path("ap.json"),
            profile,
            "candidate",
            "incumbent",
            resampling_seed=17,
            retain_trace=True,
        )

        def value_after(option: str) -> str:
            return command[command.index(option) + 1]

        self.assertEqual(command[:3], ["supported_ap", "ap", "projected"])
        self.assertEqual(value_after("--computational-order"), "2")
        self.assertEqual(value_after("--computational-replications"), "80")
        self.assertIn("--retain-replication-profiles", command)

    def test_auroc_only_mode_never_invokes_the_ap_analysis(self) -> None:
        profile = banking_story.PROFILES["quick"]
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            predictions = root / "predictions"
            reports = root / "reports"
            predictions.mkdir()
            reports.mkdir()
            (predictions / "temporal_holdout_predictions.csv").write_text(
                "label,score_random_forest,score_full_logistic,"
                "score_demographic_logistic\n0,0.1,0.2,0.3\n"
            )
            retained = {"result": {"trace": []}}
            for name in (
                "random_forest_vs_full_logistic.json",
                "full_vs_demographic_logistic.json",
            ):
                (reports / name).write_text(json.dumps(retained))
            (root / "manifest.json").write_text(
                json.dumps({"profile": "quick", "study_context": {}})
            )
            args = SimpleNamespace(profile="quick", cli=Path("supported_ap"))
            with (
                mock.patch.object(banking_story, "OUTPUTS", root),
                mock.patch.object(banking_story, "PREDICTIONS", predictions),
                mock.patch.object(banking_story, "REPORTS", reports),
                mock.patch.object(banking_story, "validate_rust_report"),
                mock.patch.object(banking_story, "validate_auroc_report"),
                mock.patch.object(
                    banking_story, "run_auroc_rust", return_value={"result": {}}
                ) as run_auroc,
                mock.patch.object(banking_story, "render_figures_and_tables"),
                mock.patch.object(banking_story, "configure_plots"),
                mock.patch.object(banking_story, "run_rust") as run_ap,
                mock.patch.object(banking_story, "fit_fresh_predictions") as refit,
            ):
                banking_story.run_auroc_only(args, profile)
            self.assertEqual(run_auroc.call_count, 2)
            run_ap.assert_not_called()
            refit.assert_not_called()
            refreshed = json.loads((root / "manifest.json").read_text())
            self.assertFalse(refreshed["auroc_refresh"]["ap_analysis_rerun"])


if __name__ == "__main__":
    unittest.main()
