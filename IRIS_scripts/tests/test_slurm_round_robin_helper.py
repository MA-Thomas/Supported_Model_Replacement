import importlib.util
import json
from pathlib import Path
from types import SimpleNamespace
import sys
import tempfile
import unittest
from unittest import mock


MODULE_PATH = Path(__file__).parents[1] / "shared/slurm_round_robin_helper.py"
SPEC = importlib.util.spec_from_file_location("slurm_round_robin_helper", MODULE_PATH)
helper = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(helper)


class SlurmHelperTests(unittest.TestCase):
    def test_frozen_hybrid_bundle_contract_requires_one_hybrid_per_model(self):
        with tempfile.TemporaryDirectory() as temporary:
            bundle = Path(temporary)
            models = [
                "full_hla",
                "focal_hla",
                "old_monoallelic",
                "mono_q_full_pn",
                "full_q_mono_pn",
            ]
            fixed_suffixes = [
                "max", "mean", "median", "logsumexp", "logmeanexp",
                "top_frac_mean_frac0p01", "top_frac_mean_frac0p02",
                "top_frac_mean_frac0p05", "top_k_mean_k2", "top_k_mean_k3",
                "top_k_logsumexp_k2", "top_k_logsumexp_k10",
            ]
            systems = [
                {
                    "system_id": f"{model}__{suffix}",
                    "score_column": f"score_{model}__{suffix}",
                }
                for model in models
                for suffix in fixed_suffixes
            ]
            systems.extend(
                {
                    "system_id": f"{model}__endpoint_local_epitope_second_hla_hybrid_v1",
                    "score_column": (
                        f"score_{model}__endpoint_local_epitope_second_hla_hybrid_v1"
                    ),
                }
                for model in models
            )
            (bundle / "systems.json").write_text(
                json.dumps({"systems": systems}), encoding="utf-8"
            )
            (bundle / "bundle_manifest.json").write_text(
                json.dumps(
                    {
                        "score_provider_identity":
                            "iris-rust-fullroster-frozen-hybrid-provider-v1"
                    }
                ),
                encoding="utf-8",
            )
            (bundle / "tournament_spec.json").write_text(
                json.dumps(
                    {
                        "evaluations": ["pdac", "covid_spike", "covid_nonspike"],
                        "annotations": {
                            "scientific_contract": {
                                "systems_per_component_model": 13,
                                "component_models": models,
                                "frozen_adaptive_l2": {
                                    "method": "endpoint_local_epitope_second_hla_hybrid",
                                    "epitope_gate_center": -2.2,
                                    "epitope_gate_width": 0.13,
                                    "second_hla_threshold": -6.45,
                                    "second_hla_gate_width": 0.02,
                                    "hla_bonus": 1.0,
                                    "hla_weight": 0.12,
                                    "solver_absolute_tolerance": 1e-10,
                                    "solver_max_iterations": 64,
                                },
                            }
                        }
                    }
                ),
                encoding="utf-8",
            )
            helper.check_frozen_hybrid_bundle(SimpleNamespace(bundle=bundle))
            systems.pop()
            (bundle / "systems.json").write_text(
                json.dumps({"systems": systems}), encoding="utf-8"
            )
            with self.assertRaisesRegex(helper.HelperError, "exactly 65 systems"):
                helper.check_frozen_hybrid_bundle(SimpleNamespace(bundle=bundle))

    def test_bundle_dimensions_are_derived_from_registry_and_evaluations(self):
        with tempfile.TemporaryDirectory() as temporary:
            bundle = Path(temporary)
            (bundle / "systems.json").write_text(
                json.dumps(
                    {
                        "systems": [
                            {"system_id": f"s{index}", "score_column": f"score_s{index}"}
                            for index in range(65)
                        ]
                    }
                ),
                encoding="utf-8",
            )
            (bundle / "tournament_spec.json").write_text(
                json.dumps({"evaluations": ["pdac", "covid_spike", "covid_nonspike"]}),
                encoding="utf-8",
            )
            with mock.patch("builtins.print") as output:
                helper.bundle_dimensions(SimpleNamespace(bundle=bundle))
            output.assert_called_once_with("6240\t3\t65")

    def test_measures_child_without_external_time_executable(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            stdout = root / "stdout.json"
            measurement = root / "measurement.json"
            exit_code = helper.run_measured(
                SimpleNamespace(
                    stdout_file=stdout,
                    time_file=measurement,
                    command=[sys.executable, "-c", 'print("{\\\"ok\\\": true}")'],
                )
            )
            self.assertEqual(exit_code, 0)
            self.assertEqual(stdout.read_text(encoding="utf-8"), '{"ok": true}\n')
            elapsed, maximum_rss = helper.parse_time_file(measurement)
            self.assertIsNotNone(elapsed)
            self.assertGreaterEqual(elapsed, 0.0)
            self.assertIsNotNone(maximum_rss)
            self.assertGreater(maximum_rss, 0)

    def test_prepares_and_installs_relative_prebuilt_bundles(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "packaged" / "pr"
            source.mkdir(parents=True)
            (source / "bundle_manifest.json").write_text("{}\n", encoding="utf-8")
            config = root / "config.json"
            config.write_text(
                "{\n"
                '  "schema_version": 1,\n'
                '  "configuration_kind": "prebuilt_bundles",\n'
                '  "prebuilt_bundle_root": "packaged",\n'
                '  "covid_spike_label_specification": {"id": "threshold_zero"}\n'
                "}\n",
                encoding="utf-8",
            )
            effective = root / "run" / "effective.json"
            helper.prepare_config(
                SimpleNamespace(
                    source=config,
                    output=effective,
                    organizer=root / "organizer",
                    label_set="threshold_zero",
                )
            )
            prepared = helper.read_json(effective)
            self.assertEqual(prepared["prebuilt_bundle_root"], str((root / "packaged").resolve()))
            output = root / "run" / "bundles" / "pr"
            helper.install_prebuilt_bundle(SimpleNamespace(source=source, output=output))
            self.assertEqual((output / "bundle_manifest.json").read_text(encoding="utf-8"), "{}\n")

    def test_parses_gnu_time_report(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "time.txt"
            path.write_text(
                "\tElapsed (wall clock) time (h:mm:ss or m:ss): 1:02.50\n"
                "\tMaximum resident set size (kbytes): 12345\n",
                encoding="utf-8",
            )
            self.assertEqual(helper.parse_time_file(path), (62.5, 12345))

    def test_parses_macos_time_report(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "time.txt"
            path.write_text(
                "       12.34 real         1.00 user         0.25 sys\n"
                "  2097152  maximum resident set size\n",
                encoding="utf-8",
            )
            self.assertEqual(helper.parse_time_file(path), (12.34, 2048))


if __name__ == "__main__":
    unittest.main()
