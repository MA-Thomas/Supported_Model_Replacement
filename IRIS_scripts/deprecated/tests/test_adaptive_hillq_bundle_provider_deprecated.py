import csv
import hashlib
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest


SCRIPT_DIR = Path(__file__).parents[1]
sys.path.insert(0, str(SCRIPT_DIR))
MODULE_PATH = SCRIPT_DIR / "iris_adaptive_hillq_bundle_provider_deprecated.py"
SPEC = importlib.util.spec_from_file_location("iris_adaptive_hillq_bundle_provider", MODULE_PATH)
bundle_provider = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(bundle_provider)


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


class AdaptiveHillQBundleProviderTests(unittest.TestCase):
    def test_system_ids_are_policy_distinct(self):
        self.assertEqual(
            bundle_provider.system_id("pdac_only", "full_hla"),
            "full_hla__self_gated_hillq_pdac_selected",
        )
        self.assertEqual(
            bundle_provider.system_id("all_contexts_equal_weight", "full_hla"),
            "full_hla__self_gated_hillq_all_contexts_selected",
        )

    def test_loads_exact_dual_policy_report_contract(self):
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            parameter_path = root / "selected_parameters.csv"
            parameter_fields = [
                "selection_policy", "model", "branch", "selection_metric", "q_id", "q",
                "c", "kappa", "c_at_boundary", "kappa_at_boundary", "system_id",
            ]
            parameter_rows = []
            for policy in bundle_provider.POLICIES:
                for branch in ("pr", "roc"):
                    for model in bundle_provider.provider.MODEL_IDS:
                        parameter_rows.append({
                            "selection_policy": policy,
                            "model": model,
                            "branch": branch,
                            "selection_metric": "average_precision" if branch == "pr" else "auroc",
                            "q_id": "q2",
                            "q": "2.0",
                            "c": "-1.0",
                            "kappa": "0.5",
                            "c_at_boundary": "False",
                            "kappa_at_boundary": "False",
                            "system_id": bundle_provider.system_id(policy, model),
                        })
            with parameter_path.open("w", newline="", encoding="utf-8") as stream:
                writer = csv.DictWriter(stream, fieldnames=parameter_fields)
                writer.writeheader()
                writer.writerows(parameter_rows)

            score_path = root / "selected_endpoint_scores.csv"
            score_fields = [
                "selection_policy", "system_id", "model", "branch", "cohort",
                "endpoint_id", "label", "q_id", "q", "c", "kappa",
                "self_gated_hillq_score",
            ]
            with score_path.open("w", newline="", encoding="utf-8") as stream:
                writer = csv.DictWriter(stream, fieldnames=score_fields)
                writer.writeheader()
                for row in parameter_rows:
                    for cohort in bundle_provider.EVALUATIONS:
                        writer.writerow({
                            **{field: row[field] for field in (
                                "selection_policy", "system_id", "model", "branch",
                                "q_id", "q", "c", "kappa",
                            )},
                            "cohort": cohort,
                            "endpoint_id": f"{cohort}.one",
                            "label": "1",
                            "self_gated_hillq_score": "-0.25",
                        })

            manifest = {
                "schema_version": 2,
                "analysis": "component-metric-specific self-gated Hill-q L2 parameter selection",
                "formula": {
                    "score": "S solves S = m + C_q * sigmoid((c-S)/kappa)",
                    "floor_definition": "ln(1e-12)",
                    "empty_roster_score": __import__("math").log(1e-12),
                },
                "generated_files": {
                    parameter_path.name: sha256(parameter_path),
                    score_path.name: sha256(score_path),
                },
                "selected_parameters": [
                    {
                        "selection_policy": row["selection_policy"],
                        "branch": row["branch"],
                        "model": row["model"],
                        "q_id": row["q_id"],
                        "c": float(row["c"]),
                        "kappa": float(row["kappa"]),
                        "system_id": row["system_id"],
                    }
                    for row in parameter_rows
                ],
            }
            (root / "selection_manifest.json").write_text(
                json.dumps(manifest), encoding="utf-8"
            )

            _, parameters, scores, _ = bundle_provider.load_selection(root, "pr")
            self.assertEqual(len(parameters), 20)
            self.assertEqual(len(scores), 30)
            registry = {
                "systems": [
                    {
                        "system_id": f"{model}__max",
                        "display_label": f"{model} / max",
                        "score_column": f"score_{model}__max",
                        "annotations": {"l3_aggregation_id": "max"},
                    }
                    for model in bundle_provider.provider.MODEL_IDS
                ]
            }
            migrated = bundle_provider.l2_registry(registry)
            self.assertTrue(
                all("l2_aggregation_id" in item["annotations"] for item in migrated["systems"])
            )
            self.assertTrue(
                all("l3_aggregation_id" not in item["annotations"] for item in migrated["systems"])
            )
            additions = bundle_provider.adaptive_systems(
                migrated,
                parameters,
                "pr",
                {"manifest": "a" * 64, "selected_parameters": "b" * 64},
            )
            self.assertEqual(len(additions), 10)
            self.assertTrue(
                all("l2_aggregation_id" in item["annotations"] for item in additions)
            )
            self.assertTrue(
                all("l3_aggregation_id" not in item["annotations"] for item in additions)
            )


if __name__ == "__main__":
    unittest.main()
