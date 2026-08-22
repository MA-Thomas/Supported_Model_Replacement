import importlib.util
import json
from pathlib import Path
from types import SimpleNamespace
import sys
import tempfile
import unittest
from unittest import mock


MODULE_PATH = Path(__file__).parents[1] / "slurm_round_robin_helper.py"
SPEC = importlib.util.spec_from_file_location("slurm_round_robin_helper", MODULE_PATH)
helper = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(helper)


class SlurmHelperTests(unittest.TestCase):
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
