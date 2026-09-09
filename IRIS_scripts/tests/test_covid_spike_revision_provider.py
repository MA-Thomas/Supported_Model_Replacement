import importlib.util
from pathlib import Path
import sys
import unittest


IRIS_SCRIPTS_ROOT = Path(__file__).resolve().parents[1]
PROVIDER_PATH = (
    IRIS_SCRIPTS_ROOT
    / "revisions/covid_spike/iris_covid_spike_revision_provider.py"
)
SPEC = importlib.util.spec_from_file_location(
    "iris_covid_spike_revision_provider", PROVIDER_PATH
)
revision = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(revision)


class CovidSpikeRevisionProviderTests(unittest.TestCase):
    def test_exact_higher_threshold_label_contract_is_accepted(self):
        revision.validate_label_specification(
            {
                "id": "higher_threshold_0p53",
                "description": (
                    "COVID SPIKE response is positive when cd8_IFNg_dmso_adj "
                    "is strictly greater than 0.53."
                ),
                "threshold": 0.53,
                "comparison_operator": ">",
                "response_field": "cd8_IFNg_dmso_adj",
            },
            revision.REVISION_LABEL_SET_ID,
            revision.EXPECTED_THRESHOLD,
        )

    def test_non_strict_higher_threshold_contract_is_rejected(self):
        with self.assertRaises(revision.provider.ProviderError):
            revision.validate_label_specification(
                {
                    "id": "higher_threshold_0p53",
                    "description": (
                        "COVID SPIKE response is positive when cd8_IFNg_dmso_adj "
                        "is strictly greater than 0.53."
                    ),
                    "threshold": 0.53,
                    "comparison_operator": ">=",
                    "response_field": "cd8_IFNg_dmso_adj",
                },
                revision.REVISION_LABEL_SET_ID,
                revision.EXPECTED_THRESHOLD,
            )


if __name__ == "__main__":
    unittest.main()
