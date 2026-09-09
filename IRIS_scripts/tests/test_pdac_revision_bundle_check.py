import importlib.util
import json
from pathlib import Path
import tempfile
import unittest


HELPER_PATH = Path(__file__).resolve().parents[1] / "shared/slurm_round_robin_helper.py"
SPEC = importlib.util.spec_from_file_location("slurm_round_robin_helper", HELPER_PATH)
helper = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(helper)


class RevisionBundleCheckTests(unittest.TestCase):
    def test_revision_contract_is_checked(self):
        with tempfile.TemporaryDirectory() as temporary:
            bundle = Path(temporary)
            (bundle / "evaluations/pdac").mkdir(parents=True)
            (bundle / "tournament_spec.json").write_text(json.dumps({
                "evaluations": ["pdac"],
                "annotations": {"context_revision": {
                    "revision_id": "revision_a",
                    "replaced_evaluation": "pdac",
                }},
            }))
            (bundle / "systems.json").write_text(json.dumps({
                "systems": [{"system_id": "a"}, {"system_id": "b"}],
            }))
            (bundle / "evaluations/pdac/endpoints.csv").write_text(
                "endpoint_id,label\ne1,1\ne2,0\ne3,0\n"
            )
            args = type("Args", (), {
                "bundle": bundle,
                "revision_id": "revision_a",
                "evaluation": "pdac",
                "expected_systems": 2,
                "expected_endpoints": 3,
                "expected_positive": 1,
            })()
            helper.check_revision_bundle(args)
            args.revision_id = "revision_b"
            with self.assertRaises(helper.HelperError):
                helper.check_revision_bundle(args)


if __name__ == "__main__":
    unittest.main()
