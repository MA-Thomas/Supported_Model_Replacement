import importlib.util
import json
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).parents[1] / "validate_complete_f_stage2_inputs.py"
SPEC = importlib.util.spec_from_file_location("complete_f_stage2", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


class CompleteFStage2ValidationTest(unittest.TestCase):
    def fixture(self):
        temp = tempfile.TemporaryDirectory()
        bundle = Path(temp.name)
        paths = {}
        for representation in ("full", "mono"):
            query = bundle / f"test_{representation}_query.csv"
            query.write_text(
                "peptide,HLA-RE,env_id,TCGA_EXPR_TYPE\nABCDEFGHI,A0101,0,TYPE\n",
                encoding="utf-8",
            )
            config = bundle / f"test_{representation}.toml"
            config.write_text(
                "query_peptide_input_tuples_file = "
                f'"${{EXTERNAL_VALIDATION_INPUT_ROOT}}/{query.name}"\n',
                encoding="utf-8",
            )
            paths[representation] = (query, config)
        query, config = paths["full"]
        output_files = {
            path.name: {"bytes": path.stat().st_size, "sha256": MODULE.sha256_file(path)}
            for pair in paths.values()
            for path in pair
        }
        manifest = {
            "datasets": {
                "TEST": {
                    "floor_candidate_rows": 0,
                    "input_audit_status": "pass",
                    "full_query_rows": 1,
                    "mono_query_rows": 1,
                    "output_files": {
                    "full_deduplicated_query": query.name,
                    "full_deduplicated_config": config.name,
                    "mono_query": paths["mono"][0].name,
                    "mono_config": paths["mono"][1].name,
                    },
                }
            },
            "output_files": output_files,
        }
        (bundle / "manifest.json").write_text(json.dumps(manifest), encoding="utf-8")
        return temp, bundle, query

    def test_accepts_exact_manifest_bound_complete_query(self):
        temp, bundle, _ = self.fixture()
        self.addCleanup(temp.cleanup)
        report = MODULE.validate(bundle, "TEST", "full")
        self.assertEqual(report["query_rows"], 1)
        self.assertTrue(report["manifest_exact_match"])

    def test_rejects_query_mutation(self):
        temp, bundle, query = self.fixture()
        self.addCleanup(temp.cleanup)
        query.write_text(query.read_text() + "JKLMNOPQR,B0702,1,TYPE\n")
        with self.assertRaisesRegex(MODULE.ContractError, "hash/size mismatch"):
            MODULE.validate(bundle, "TEST", "full")

    def test_installs_dataset_outputs_directly_and_refuses_overwrite(self):
        temp, bundle, _ = self.fixture()
        self.addCleanup(temp.cleanup)
        with tempfile.TemporaryDirectory() as destination:
            report = MODULE.install(
                bundle,
                "TEST",
                work_data_root=Path(destination),
                destination=Path(destination),
            )
            self.assertEqual(report["status"], "installed")
            self.assertTrue((Path(destination) / "test_full_query.csv").is_file())
            self.assertTrue(
                (Path(destination) / "test_complete_f_inputs_manifest.json").is_file()
            )
            with self.assertRaisesRegex(MODULE.ContractError, "refusing to overwrite"):
                MODULE.install(
                    bundle,
                    "TEST",
                    work_data_root=Path(destination),
                    destination=Path(destination),
                )
            report = MODULE.install(
                bundle,
                "TEST",
                work_data_root=Path(destination),
                destination=Path(destination),
                archive_existing=True,
            )
            self.assertTrue(report["archived_files"])
            self.assertTrue(
                (Path(destination) / "test_full_query_deprecated.csv").is_file()
            )


if __name__ == "__main__":
    unittest.main()
