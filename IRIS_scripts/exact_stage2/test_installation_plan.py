import hashlib
import json
from pathlib import Path
import unittest
import importlib.util
import tempfile
from unittest.mock import patch

ROOT = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location('stage2_installer', ROOT / 'install.py')
INSTALLER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(INSTALLER)


class InstallationPlanTests(unittest.TestCase):
    def test_revised_plan_preserves_original_preconditions_and_pins_current_sources(self):
        root = Path(__file__).resolve().parent
        original_bytes = (root / 'installation_plan.json').read_bytes()
        original = json.loads(original_bytes)
        revised = json.loads((root / 'installation_plan.rust_compat.json').read_text())
        self.assertEqual(revised['supersedes_plan_sha256'], hashlib.sha256(original_bytes).hexdigest())
        self.assertEqual(revised['replace_directories'], original['replace_directories'])
        self.assertEqual([(f['path'], f['before_sha256']) for f in revised['files']],
                         [(f['path'], f['before_sha256']) for f in original['files']])
        for entry in revised['files']:
            with self.subTest(path=entry['path']):
                self.assertEqual(entry['after_sha256'], hashlib.sha256(
                    (root / 'runtime' / entry['path']).read_bytes()).hexdigest())

    def test_incremental_plan_updates_only_four_files_from_installed_state(self):
        original_bytes = (ROOT / 'installation_plan.json').read_bytes()
        original = {e['path']: e for e in json.loads(original_bytes)['files']}
        revised = json.loads((ROOT / 'installation_plan.rust_compat_incremental.json').read_text())
        self.assertEqual(revised['base_installation_plan_sha256'], hashlib.sha256(original_bytes).hexdigest())
        self.assertEqual(revised['replace_directories'], {})
        self.assertEqual({e['path'] for e in revised['files']}, {
            f'evaluation_code_{cohort}/src/{file}' for cohort in ['COVID', 'PDAC']
            for file in ['lib.rs', 'bin/assemble_tensor_longpep.rs']})
        for entry in revised['files']:
            self.assertEqual(entry['before_sha256'], original[entry['path']]['after_sha256'])
            self.assertEqual(entry['after_sha256'], INSTALLER.grid.sha(ROOT / 'runtime' / entry['path']))


class IncrementalInstallerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        root = Path(self.temp.name)
        self.here, self.destination = root / 'installer', root / 'deployed'
        self.runtime = self.here / 'runtime'
        self.package = self.destination / 'complete_f_input_package'
        self.runtime.mkdir(parents=True)
        self.package.mkdir(parents=True)
        (self.destination / 'complete_f_stage2').mkdir()
        (self.destination / 'complete_f_stage2' / 'launch.py').write_text('preserve launcher')
        (self.destination / 'keep.sh').write_text('preserve script')
        (self.here / 'numerical_source_hashes.json').write_text('{}')
        (self.here / 'installation_receipt.json').write_text('historical receipt')
        self.entries = []
        for name in ['a.rs', 'b.rs']:
            (self.destination / name).write_text('before ' + name)
            (self.runtime / name).write_text('after ' + name)
            self.entries.append(dict(path=name, before_sha256=INSTALLER.grid.sha(self.destination / name),
                                     after_sha256=INSTALLER.grid.sha(self.runtime / name)))
        self.plan = self.here / 'installation_plan.incremental.json'
        self.plan.write_text(json.dumps(dict(files=self.entries, replace_directories={},
            unchanged_files={'keep.sh': INSTALLER.grid.sha(self.destination / 'keep.sh')})))
        (self.package / 'input.csv').write_text('unchanged input')
        (self.package / 'manifest.json').write_text(json.dumps({'output_files': {'input.csv': {
            'bytes': (self.package / 'input.csv').stat().st_size,
            'sha256': INSTALLER.grid.sha(self.package / 'input.csv')}}}))
        for target, value in [('HERE', self.here), ('RUNTIME', self.runtime)]:
            mock = patch.object(INSTALLER, target, value)
            mock.start(); self.addCleanup(mock.stop)
        mock = patch.object(INSTALLER.grid, 'load')
        mock.start(); self.addCleanup(mock.stop)

    def run_install(self, apply=True):
        return INSTALLER.install(self.destination, self.package, self.plan, apply)

    def test_dry_run_does_not_write(self):
        before = INSTALLER.inventory(self.destination)
        self.run_install(False)
        self.assertEqual(before, INSTALLER.inventory(self.destination))
        self.assertFalse((self.destination / 'exact_stage2_backups').exists())

    def test_apply_preserves_directories_and_history_and_backs_up_old_files(self):
        package_before = INSTALLER.inventory(self.package)
        receipt_path = self.run_install()
        receipt = json.loads(receipt_path.read_text())
        for entry in self.entries:
            self.assertEqual(INSTALLER.grid.sha(self.destination / entry['path']), entry['after_sha256'])
            self.assertEqual(INSTALLER.grid.sha(Path(receipt['backup']) / entry['path']), entry['before_sha256'])
        self.assertEqual(INSTALLER.inventory(self.package), package_before)
        self.assertEqual((self.destination / 'complete_f_stage2' / 'launch.py').read_text(), 'preserve launcher')
        self.assertEqual((self.here / 'installation_receipt.json').read_text(), 'historical receipt')
        with self.assertRaisesRegex(ValueError, 'receipt already exists'):
            self.run_install()

    def test_stale_destination_is_rejected_before_writes(self):
        (self.destination / 'a.rs').write_text('concurrent edit')
        with self.assertRaisesRegex(ValueError, 'destination changed'):
            self.run_install()
        self.assertFalse((self.destination / 'exact_stage2_backups').exists())

    def test_failure_restores_previously_replaced_files(self):
        replace = INSTALLER.os.replace
        def fail_second(source, target):
            if Path(target).resolve() == (self.destination / 'b.rs').resolve():
                raise OSError('simulated replacement failure')
            return replace(source, target)
        with patch.object(INSTALLER.os, 'replace', side_effect=fail_second):
            with self.assertRaisesRegex(OSError, 'simulated'):
                self.run_install()
        for entry in self.entries:
            self.assertEqual(INSTALLER.grid.sha(self.destination / entry['path']), entry['before_sha256'])
        self.assertFalse((self.here / 'installation_receipt.incremental.json').exists())


if __name__ == '__main__':
    unittest.main()
