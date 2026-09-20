"""Independent graph and support checks using synthetic data."""
import itertools
import unittest
from domain_study.proteingym_contexts.context_story import anchored_summary, reduce_graphs, source_maximal

class ContextStudyTests(unittest.TestCase):
    def test_opponent_switching(self):
        result, maximal = reduce_graphs(['A','B','C'], {'X':{('B','A')}, 'Y':{('C','A')}})
        self.assertEqual(result['candidate_conservative'], ['B','C'])
        self.assertEqual(result['replacement_conservative'], ['A','B','C'])
        self.assertEqual(maximal['X'], {'B','C'})

    def test_empty_intersection_is_valid(self):
        result,_ = reduce_graphs(['A','B'], {'X':{('A','B')},'Y':{('B','A')}})
        self.assertEqual(result['candidate_conservative'], [])
        self.assertEqual(result['replacement_conservative'], ['A','B'])

    def test_invalid_graphs_are_not_no_verdicts(self):
        with self.assertRaises(ValueError): reduce_graphs(['A'], {})
        with self.assertRaises(ValueError): source_maximal(['A','B','C'], {('A','B'),('B','C'),('C','A')})
        with self.assertRaises(ValueError): source_maximal(['A','B'], {('C','A')})

    def test_anchoring_and_strict_thresholds_match_enumeration(self):
        for anchor in [-0.1,0,.15,.9]:
            for order in [1,2,3]:
                effects = [.1,.2,-.05]
                challenges = [min([anchor]+list(x)) for x in itertools.combinations(effects,order)]
                expected_magnitude = sum(challenges)/len(challenges)
                expected_survival = sum(x>0 for x in challenges)/len(challenges)
                actual = anchored_summary(anchor,effects,order,0,0,.81)
                self.assertAlmostEqual(actual['magnitude'],expected_magnitude)
                self.assertAlmostEqual(actual['survival'],expected_survival)
        self.assertFalse(anchored_summary(1,[1,0],1,0,0,.5)['supported'])

    def test_stronger_computational_test_can_restore_admissibility(self):
        observed,_ = reduce_graphs(['A','B'],{'X':{('A','B')},'Y':{('B','A')}})
        full,_ = reduce_graphs(['A','B'],{'X':set(),'Y':{('B','A')}})
        self.assertEqual(observed['candidate_conservative'],[])
        self.assertEqual(full['candidate_conservative'],['B'])


class ExecutableTests(unittest.TestCase):
    def test_parallel_row_order_and_resume_invariance(self):
        import csv
        import json
        import os
        from pathlib import Path
        import subprocess
        import tempfile
        binary = Path(__file__).resolve().parents[2] / 'target/release/examples/proteingym_contexts'
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
                    '--run-id',run_id,'--replications','4','--grid-points','9','--refinement-draws','2'],
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
