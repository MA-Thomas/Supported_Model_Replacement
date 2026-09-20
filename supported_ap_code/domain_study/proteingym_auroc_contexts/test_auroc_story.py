"""Synthetic end-to-end AUROC checks; shared graph tests live in the CNAP study."""
import unittest
from domain_study.proteingym_contexts.context_story import anchored_summary

class ExecutableTests(unittest.TestCase):
    def test_explicit_cnap_reference_rejects_tampering_and_pending_results(self):
        import json
        import tempfile
        from pathlib import Path
        from domain_study.proteingym_auroc_contexts.auroc_story import validate_cnap_reference, sha256
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root/'tables').mkdir()
            (root/'plan.json').write_text(json.dumps({'run_id': 'chosen-run'}))
            (root/'summary.json').write_text(json.dumps({'run_id': 'chosen-run', 'graph_result_is_resolved': True}))
            (root/'tables/directed_assessments.csv').write_text('candidate,incumbent\na,b\n')
            def seal():
                (root/'manifest.json').write_text(json.dumps({'run_id': 'chosen-run', 'artifacts_sha256': {
                    name: sha256(root/name) for name in ['plan.json', 'summary.json', 'tables/directed_assessments.csv']}}))
            seal()
            self.assertEqual(validate_cnap_reference(root)['run_id'], 'chosen-run')
            (root/'tables/directed_assessments.csv').write_text('candidate,incumbent\nb,a\n')
            with self.assertRaisesRegex(ValueError, 'hash mismatch'):
                validate_cnap_reference(root)
            (root/'summary.json').write_text(json.dumps({'run_id': 'chosen-run', 'graph_result_is_resolved': False}))
            seal()
            with self.assertRaisesRegex(ValueError, 'unresolved'):
                validate_cnap_reference(root)

    def test_parallel_row_order_and_resume_invariance(self):
        import csv
        import json
        import os
        from pathlib import Path
        import subprocess
        import tempfile
        binary = Path(__file__).resolve().parents[2] / 'target/release/examples/proteingym_auroc_contexts'
        if not binary.exists():
            self.skipTest('build the release example to run the end-to-end check')
        columns = ['assay_id','variant_id','label','score_eve_ensemble','score_esm1v_ensemble','score_esm2_650m']
        rows = []
        for context in ['X','Y']:
            for i in range(6):
                label = int(i % 2 == 0)
                perfect = [6,1,5,2,4,3][i]
                eve = 7-perfect if context == 'X' else perfect
                esm2 = [4,3,6,1,5,2][i] if context == 'X' else perfect
                rows.append([context,f'A{i//2+1}{"C" if label else "D"}',label,eve,perfect,esm2])
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            def run(name, data, threads, run_id='test'):
                source = root/f'{name}.csv'
                with source.open('w',newline='') as handle:
                    writer = csv.writer(handle); writer.writerow(columns); writer.writerows(data)
                return subprocess.run([str(binary),'--input',str(source),'--output-dir',str(root/name),
                    '--run-id',run_id,'--replications','4','--check-draws','2'],
                    env=dict(os.environ,RAYON_NUM_THREADS=str(threads)),capture_output=True,text=True)
            first = run('first',rows,1); self.assertEqual(first.returncode,0,first.stderr)
            second = run('second',list(reversed(rows)),2); self.assertEqual(second.returncode,0,second.stderr)
            files = sorted((root/'first').glob('*.json')); self.assertEqual(len(files),6)
            for f in files:
                a = json.loads(f.read_text()); b = json.loads((root/'second'/f.name).read_text())
                self.assertEqual(a,b)
                for key in ['forward','reverse']:
                    direction = a[key]
                    if not direction['observed_gate']['supported']:
                        self.assertEqual(direction['computational'],[])
                        self.assertIsNone(direction['full'])
                    else:
                        expected = anchored_summary(direction['observed']['value'],
                            [x['value'] for x in direction['computational']],2,0,0,.81)
                        self.assertAlmostEqual(expected['magnitude'],direction['full']['magnitude'])
                        self.assertAlmostEqual(expected['survival'],direction['full']['survival'])
            rerun = run('first',rows,2); self.assertEqual(rerun.returncode,0,rerun.stderr)
            incompatible = run('first',rows,1,'incompatible')
            self.assertNotEqual(incompatible.returncode,0)
            self.assertIn('incompatible checkpoint',incompatible.stderr)

if __name__ == '__main__': unittest.main()
