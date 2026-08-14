import csv
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest


MODULE_PATH = Path(__file__).parents[1] / "iris_score_provider.py"
SPEC = importlib.util.spec_from_file_location("iris_score_provider", MODULE_PATH)
provider = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(provider)


class ProviderUnitTests(unittest.TestCase):
    def test_pdac_mutation_is_rejected(self):
        config = base_config()
        config["evaluations"]["pdac"] = evaluation(
            ["patient_id", "mutation", "long_peptide"], label_field="long_peptide_label"
        )
        with self.assertRaisesRegex(provider.ProviderError, "PDAC.*must not contain mutation"):
            provider.validate_config(config)

    def test_spike_identity_retains_mutation(self):
        fields = ["patient_id", "mutation", "long_peptide"]
        first = provider.endpoint_id("covid_spike", fields, {
            "patient_id": "p1", "mutation": "D614G", "long_peptide": "PEPTIDE"
        })
        second = provider.endpoint_id("covid_spike", fields, {
            "patient_id": "p1", "mutation": "WT", "long_peptide": "PEPTIDE"
        })
        self.assertNotEqual(first, second)

    def test_prediction_slice_never_collapses_spike_mutations(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "predictions.csv"
            with path.open("w", newline="", encoding="utf-8") as stream:
                writer = csv.DictWriter(stream, fieldnames=[
                    "selector", "l3_variant", "is_count_baseline", "patient_id",
                    "mutation", "long_peptide", "label", "score",
                ])
                writer.writeheader()
                writer.writerows([
                    {"selector": "nci_best_pr", "l3_variant": "max", "is_count_baseline": "false",
                     "patient_id": "p1", "mutation": "D614G", "long_peptide": "PEP", "label": "1", "score": "0.8"},
                    {"selector": "nci_best_pr", "l3_variant": "max", "is_count_baseline": "false",
                     "patient_id": "p1", "mutation": "WT", "long_peptide": "PEP", "label": "0", "score": "0.7"},
                ])
            rows, identities = provider.read_prediction_slice(
                path, "covid_spike", evaluation(["patient_id", "mutation", "long_peptide"]),
                "nci_best_pr", "max",
            )
            self.assertEqual(len(rows), 2)
            self.assertEqual({item["mutation"] for item in identities.values()}, {"D614G", "WT"})

    def test_count_like_aggregation_is_rejected(self):
        config = base_config()
        config["aggregations"] = ["log_count"]
        with self.assertRaisesRegex(provider.ProviderError, "count-like aggregation"):
            provider.validate_config(config)

    def test_spike_label_specification_must_match_spike(self):
        config = base_config()
        config["evaluations"]["covid_spike"]["threshold"] = 0.53
        with self.assertRaisesRegex(provider.ProviderError, "covid_spike threshold does not match"):
            provider.validate_config(config)

    def test_named_higher_spike_threshold_does_not_change_nonspike(self):
        config = base_config()
        config["covid_spike_label_specification"].update({
            "id": "higher_threshold_0p53",
            "description": "COVID SPIKE response is positive only above 0.53.",
            "threshold": 0.53,
        })
        config["evaluations"]["covid_spike"]["threshold"] = 0.53
        provider.validate_config(config)

    def test_nonspike_rejects_spike_higher_threshold(self):
        config = base_config()
        config["evaluations"]["covid_nonspike"]["threshold"] = 0.53
        with self.assertRaisesRegex(provider.ProviderError, "covid_nonspike label contract must remain"):
            provider.validate_config(config)

    def test_mapping_audit_records_but_does_not_collapse_spike_error(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "mapping.csv"
            with path.open("w", newline="", encoding="utf-8") as stream:
                writer = csv.DictWriter(stream, fieldnames=[
                    "patient_id", "mutation", "long_peptide", "response",
                ])
                writer.writeheader()
                writer.writerows([
                    {"patient_id": "p1", "mutation": "WT", "long_peptide": "PEP", "response": "0"},
                    {"patient_id": "p1", "mutation": "D614G", "long_peptide": "PEP", "response": "1"},
                ])
            labels, _, audit = provider.mapping_roster(
                path, "covid_spike", evaluation(["patient_id", "mutation", "long_peptide"])
            )
            self.assertEqual(len(labels), 2)
            self.assertEqual(audit["coarser_units_crossing_binary_threshold"], 1)
            self.assertTrue(audit["coarser_unit_is_not_analysis_unit"])


def evaluation(endpoint_fields, label_field=None, response_field="response"):
    return {
        "transfer_name": "fixture", "evaluation_dir": "/fixture", "run_prefix": "run",
        "mapping_source": "/mapping.csv", "endpoint_fields": endpoint_fields,
        "response_field": None if label_field else response_field, "label_field": label_field,
        "threshold": None if label_field else 0.0,
        "comparison_operator": None if label_field else ">",
        "measurement_error_policy": "preserve measurements",
    }


def base_config():
    models = {model: {"label": model, "nci_summary": "/nci.json"} for model in provider.MODEL_IDS}
    return {
        "schema_version": 3, "provider_identity": "test", "organizer_binary": "/organizer",
        "transfer_root": "/transfers", "run_inputs_root": "/inputs", "models": models,
        "evaluations": {
            "covid_spike": evaluation(["patient_id", "mutation", "long_peptide"]),
            "covid_nonspike": evaluation(
                ["patient_id", "long_peptide"], response_field="cd8_TNFa_IFNg_dmso_adj"
            ),
            "pdac": evaluation(["patient_id", "long_peptide"], label_field="long_peptide_label"),
        },
        "aggregations": ["max"],
        "policies": {"shared": {}, "pr": {}, "roc": {}},
        "accepted_measurement_error": {},
        "covid_spike_label_specification": {
            "id": "threshold_zero",
            "description": "COVID SPIKE response is positive above zero.",
            "threshold": 0.0,
            "comparison_operator": ">",
            "response_field": "response",
        },
        "_config_dir": "/",
    }


if __name__ == "__main__":
    unittest.main()
