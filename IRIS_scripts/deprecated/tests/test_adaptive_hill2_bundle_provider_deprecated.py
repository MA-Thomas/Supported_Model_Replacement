import importlib.util
from pathlib import Path
from types import SimpleNamespace
import sys
import unittest


SCRIPT_DIR = Path(__file__).parents[1]
sys.path.insert(0, str(SCRIPT_DIR))
MODULE_PATH = SCRIPT_DIR / "iris_adaptive_hill2_bundle_provider_deprecated.py"
SPEC = importlib.util.spec_from_file_location("iris_adaptive_hill2_bundle_provider", MODULE_PATH)
bundle_provider = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(bundle_provider)


class AdaptiveHill2BundleProviderTests(unittest.TestCase):
    @unittest.skipUnless(
        bundle_provider.DEFAULT_BASE_BUNDLES.is_dir(),
        "deprecated historical threshold-zero fixture is not installed",
    )
    def test_frozen_selection_validates_to_five_additional_systems(self):
        for metric in ("pr", "roc"):
            result = bundle_provider.build_bundle(
                SimpleNamespace(
                    command="validate",
                    metric=metric,
                    contract=bundle_provider.DEFAULT_CONTRACT,
                    base_bundle_root=bundle_provider.DEFAULT_BASE_BUNDLES,
                    organizer=bundle_provider.DEFAULT_ORGANIZER,
                    output=None,
                )
            )
            self.assertEqual(result["systems"], 65)
            self.assertEqual(result["evaluations"], 3)
            self.assertEqual(result["expected_matches"], 6240)


if __name__ == "__main__":
    unittest.main()
