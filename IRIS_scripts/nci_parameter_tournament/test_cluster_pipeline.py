"""Submission tests use fake Slurm commands; no cluster is contacted."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
DRIVER = HERE / 'cluster_pipeline.py'
FAKE = '''#!{python}
import json, os, pathlib, sys
name = pathlib.Path(sys.argv[0]).name
log = pathlib.Path(os.environ['FAKE_LOG'])
with log.open('a') as stream: stream.write(json.dumps([name] + sys.argv[1:]) + '\\n')
if name == 'sbatch':
    if os.environ.get('FAKE_FAIL') and any('nci_merge' in a for a in sys.argv): sys.exit(1)
    count = sum(json.loads(line)[0] == 'sbatch' for line in log.read_text().splitlines())
    print(1000 + count)
elif name == 'squeue' and os.environ.get('FAKE_BUSY'): print('RUNNING')
elif name == 'scontrol': print('MaxArraySize = 1001')
'''


class ClusterSubmission(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.run_root = self.root / 'run with spaces'
        binaries = self.root / 'fake-bin'
        binaries.mkdir()
        for name in ['sbatch', 'squeue', 'scontrol', 'scancel', 'pipeline', 'organizer']:
            path = binaries / name
            path.write_text(FAKE.format(python=sys.executable))
            path.chmod(0o755)
        package = self.root / 'inputs'
        package.mkdir()
        (package / 'manifest.json').write_text('{}')
        self.log = self.root / 'calls.jsonl'
        self.env = dict(os.environ, PATH=str(binaries) + os.pathsep + os.environ['PATH'], FAKE_LOG=str(self.log))
        self.command = [sys.executable, str(DRIVER), 'submit', '--run-root', str(self.run_root),
                        '--pipeline', str(binaries / 'pipeline'), '--organizer', str(binaries / 'organizer'),
                        '--input-package', str(package), '--target-shards', '2', '--completion-shards', '2']

    def invoke(self, *extra, env=None):
        return subprocess.run(self.command + list(extra), env=env or self.env, capture_output=True, text=True)

    def calls(self):
        return [json.loads(line) for line in self.log.read_text().splitlines()]

    def test_complete_dag_and_duplicate_submission_guard(self):
        result = self.invoke()
        self.assertEqual(result.returncode, 0, result.stderr)
        calls = [c for c in self.calls() if c[0] == 'sbatch']
        self.assertEqual(len(calls), 8)
        self.assertFalse(any(a.startswith('--dependency') for a in calls[0]))
        for index, call in enumerate(calls[1:], 1):
            self.assertIn('--dependency=afterok:' + str(1000 + index), call)
        self.assertIn('--array=0-19%100', calls[1])
        self.assertIn('--array=0-1%100', calls[6])
        self.assertNotEqual(self.invoke().returncode, 0)
        self.assertNotEqual(self.invoke('--resume', env=dict(self.env, FAKE_BUSY='1')).returncode, 0)
        self.assertEqual(len([c for c in self.calls() if c[0] == 'sbatch']), 8)
        result = self.invoke('--resume')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(json.loads((self.run_root / 'submission_history.json').read_text())), 1)

    def test_submission_failure_cancels_already_submitted_jobs(self):
        result = self.invoke(env=dict(self.env, FAKE_FAIL='1'))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(['scancel', '1001', '1002'], self.calls())
        self.assertEqual(json.loads((self.run_root / 'jobs.json').read_text())['status'], 'submission_failed')

    def test_dry_run_is_complete_and_does_not_prepare_or_submit(self):
        result = self.invoke('--dry-run')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(result.stdout.splitlines()), 8)
        self.assertFalse(self.run_root.exists())
        self.assertFalse(self.log.exists())

    def test_worker_detects_changed_binary_before_running(self):
        result = self.invoke()
        self.assertEqual(result.returncode, 0, result.stderr)
        (self.run_root / 'bin/directed_round_robin_organizer').write_text('changed')
        result = subprocess.run([sys.executable, str(DRIVER), 'stage', '--run-root', str(self.run_root),
                                 '--stage', 'targets', '--task', '0'], env=self.env, capture_output=True, text=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('Frozen runtime payload changed', result.stderr)


if __name__ == '__main__':
    unittest.main()
