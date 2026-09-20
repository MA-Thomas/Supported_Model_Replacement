import argparse
import contextlib
import csv
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

import stage2_run_contract as contract
import survivor_grid as grid

RUNTIME = Path(contract.__file__).resolve().parent


def write_csv(path, headers, rows):
    with path.open('w', newline='') as f:
        w = csv.writer(f, lineterminator='\n'); w.writerow(headers); w.writerows(rows)


def fixture_grid(root):
    params = root / 'params.csv'; union = root / 'union.csv'
    geometries = [('3','2','1','1'), ('4','2','1','1')]
    pairs = [(1,1),(2,1)]
    write_csv(params, grid.GEOMETRY+['tau_thymus'], [[*g,t] for g in geometries for t in grid.TAUS])
    write_csv(union, ['at_least_M','at_most_N'], pairs)
    groups=[]; survivors=[]
    for i,(g,pair) in enumerate(zip(geometries,pairs)):
        mn=root/f'mn_{i}.csv';write_csv(mn,['at_least_M','at_most_N'],[pair])
        groups.append(dict(geometry_idx=i, geometry_params_str=g,mn_pairs=[pair],mn_file=grid.record(mn,root)))
        survivors.append(dict(system_id=f'full_hla__pr__regime_{i}',model_id='full_hla',metric='pr',
                              **dict(zip(grid.GEOMETRY,g)),M=pair[0],N=pair[1]))
    manifest=root/'grid.json'
    manifest.write_text(json.dumps(dict(schema_version=1,kind='survivor_exact_stage2_grid',construction='full',
        hla_environment_representation='full',pn_hla_scope='all',n_regimes=2,tau_values_str=grid.TAUS,
        parameter_file=grid.record(params,root),mn_union_file=grid.record(union,root),geometries=groups,survivors=survivors)))
    grid.load(manifest)
    return manifest,params,union


FAKE_RUNNER = '''import csv,json,os,sys
from pathlib import Path
sys.path.insert(0,os.environ['TEST_RUNTIME'])
import stage2_run_contract as c
args=sys.argv[1:]
def value(flag):return args[args.index(flag)+1]
root=Path(value('--outputs-root'));out=root/'results_pn_env_id_0_1';out.mkdir(parents=True)
start,stop=int(value('--start-idx')),int(value('--stop-idx'))
params=c.load_param_rows(Path(value('--parameter-file')),start,stop)
pairs=c.load_mn_order(Path(value('--mn-tuples-file')))
rows=c.load_query(Path(os.environ['QUERY_INPUT_FILE']))
if '--query-shard-index' in args:
 rows=c.select_query_shard(rows,int(value('--query-shard-index')),int(value('--query-shard-count')))
for dp,dn,sp,sn,tau in params:
 name=f'query_peptides_results_dpos_{dp}_dneg_{dn}_steepness_pos_{sp}_steepness_neg_{sn}_fft_size_16384_tau_thymus_{tau}_HLA_A0101_expr_PDAC.csv'
 with (out/name).open('w',newline='') as f:
  w=csv.writer(f,lineterminator='\\n');w.writerow(['peptide','env_id','M','N','p_pos','p_neg'])
  for row in rows:
   for m,n in pairs:w.writerow([row.peptide,row.env_id,m,n,0.2,0.3])
Path(os.environ['CAPTURE']+str(os.environ['SLURM_ARRAY_TASK_ID'])).write_text(json.dumps({'pairs':pairs,'params':params}))
'''


class ExactTests(unittest.TestCase):
    def setUp(self):
        self.tmp=tempfile.TemporaryDirectory();self.root=Path(self.tmp.name)
        self.manifest,self.params,self.union=fixture_grid(self.root)
    def tearDown(self):self.tmp.cleanup()

    def test_grid_rejects_extra_required_regime_and_modified_file(self):
        d=grid.json_read(self.manifest);d['geometries'][0]['mn_pairs'].append([2,1])
        self.manifest.write_text(json.dumps(d))
        with self.assertRaises(ValueError):grid.load(self.manifest)
        self.manifest,self.params,self.union=fixture_grid(self.root)
        self.params.write_text(self.params.read_text().replace('1000\n','1001\n'))
        with self.assertRaises(ValueError):grid.load(self.manifest)

    def test_wrong_construction_is_rejected(self):
        with self.assertRaises(ValueError):grid.load(self.manifest,'mono','all')
        with self.assertRaises(ValueError):grid.load(self.manifest,'full','focal')

    def test_geometry_tasks_shard_merge_audit_and_optional_assembly(self):
        query=self.root/'query.csv'
        query.write_text('peptide,HLA-RE,PatientID,TCGA_EXPR_TYPE,env_id,gene,count\nPEPTIDEAA,A0101,P1,PDAC,0,G,1\nPEPTIDEAB,A0101,P1,PDAC,0,G,1\n')
        env=self.root/'env.csv';env.write_text('env_id,allele_environment\n0,"A0101,B0702"\n')
        config=self.root/'q.toml';config.write_text(f'query_peptide_input_tuples_file = "{query}"\nhla_env_dict = "{env}"\n')
        mapping=self.root/'mapping.csv';mapping.write_text('patient_id,env_id,long_peptide,nmer,long_peptide_label,gene,cancer_type\nP1,0,LONGPEPTIDEAA,PEPTIDEAA,1,G,PDAC\nP1,0,LONGPEPTIDEAB,PEPTIDEAB,0,G,PDAC\n')
        matrix=self.root/'data/matrices';matrix.mkdir(parents=True);(matrix/'dummy').write_text('fixture')
        pmhc=self.root/'pmhc.csv';pmhc.write_text('fixture')
        runner=self.root/'Runner';runner.write_text(f'#!{sys.executable}\n'+FAKE_RUNNER);runner.chmod(0o755)
        run=self.root/'run';run.mkdir();plan=run/'pn_shard_plan.json'
        contract.write_pn_shard_plan(plan,query,1,1,12,2,1)
        args=dict(run_root=str(run),run_id='fixture',dataset='PDAC',runner_root=str(self.root),runner=str(runner),
             q_model_config=str(config),submit_script=str(RUNTIME/'submit_stage2_new.sh'),run_script=str(RUNTIME/'run_stage_2_new.sh'),
             contract_helper=str(RUNTIME/'stage2_run_contract.py'),total_env_ids=1,env_chunks=1,total_params=12,param_chunks=2,
             param_file=str(self.params),mn_tuples=str(self.union),pmhc_config=str(config),pmhc_parameters_file=str(pmhc),
             peptide_conc=1000.0,pn_hla_scope='all',max_num_ps_values_log2=14,hla_environment_representation='full',
             representation_crosswalk='',pn_peptides_per_shard=1,pn_shard_plan=str(plan),finalize_script=str(RUNTIME/'finalize_stage2_pn_shards.sh'),
             allow_operational_provenance_change=False,regime_manifest=str(self.manifest))
        with contextlib.redirect_stdout(io.StringIO()):
            contract.command_prepare_run(argparse.Namespace(**args,mode='qpi',in_vitro=1))
            contract.command_prepare_run(argparse.Namespace(**args,mode='pn',in_vitro=0))
        mode=grid.json_read(run/'pn_manifest.json');fp=mode['mode_fingerprint']
        environment=os.environ.copy();environment.update({k:str(v) for k,v in dict(DATASET='PDAC',COMPUTE_PN=1,COMPUTE_Q=0,
            COMPUTE_PI=0,COMPUTE_EVAC=0,EXECUTION_MODE='pn',TOTAL_ENV_IDS=1,ENV_CHUNKS=1,TOTAL_PARAMS=12,PARAM_CHUNKS=2,
            PN_HLA_SCOPE='all',MAX_NUM_PS_VALUES_LOG2=14,RUNNER=runner,RUNNER_ROOT=self.root,OUTPUTS_ROOT=run,
            SLURM_JOB_ID=101,SLURM_CPUS_PER_TASK=1,STAGE2_MODE='pn',RUN_ID='fixture',MODE_FINGERPRINT=fp,
            MODE_MANIFEST=run/'pn_manifest.json',QUERY_INPUT_FILE=query,CONTRACT_HELPER=RUNTIME/'stage2_run_contract.py',
            PYTHON_BIN=sys.executable,PARAM_FILE=self.params,MN_TUPLES_FILE=self.union,PARAM_FILE_SHA256=grid.sha(self.params),
            MN_TUPLES_FILE_SHA256=grid.sha(self.union),HLA_ENVIRONMENT_REPRESENTATION='full',Q_MODEL_CONFIG=config,
            PN_SHARD_PLAN=plan,SURVIVOR_GRID_HELPER=RUNTIME/'survivor_grid.py',GRID_PROFILE='survivor-exact',
            TEST_RUNTIME=RUNTIME,CAPTURE=self.root/'capture').items()})
        for task in range(4):
            environment['SLURM_ARRAY_TASK_ID']=str(task)
            subprocess.run(['bash',str(RUNTIME/'run_stage_2_new.sh')],env=environment,check=True,capture_output=True)
            captured=grid.json_read(self.root/f'capture{task}')
            self.assertEqual(captured['pairs'],[[task//2+1,1]])
            self.assertEqual(len(captured['params']),6)
        for parent in range(2):
            environment['SLURM_ARRAY_TASK_ID']=str(parent)
            subprocess.run(['bash',str(RUNTIME/'finalize_stage2_pn_shards.sh')],env=environment,check=True,capture_output=True)
        with contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(contract.command_audit_run(argparse.Namespace(run_root=str(run),mode_manifest=str(run/'pn_manifest.json'),verify_hashes=True)),0)
        files=list((run/'results_pn_env_id_0_1').glob('*.csv'))
        self.assertEqual(len(files),12)
        total = 0
        for file in files:
            with file.open() as handle:
                total += len(list(csv.DictReader(handle)))
        self.assertEqual(total,24)
        # Missing, extra and duplicate data must fail before publication.
        parameters=contract.load_param_rows(self.params,0,6)
        staging=self.root/'bad';staging.mkdir()
        import shutil
        for f in files:
            if '_dpos_3_' in f.name:shutil.copyfile(f,staging/f.name)
        target=next(staging.glob('*.csv'));original=target.read_text()
        for contents in [original+original.splitlines()[1]+'\n', original.replace(',1,1,',',2,1,'), '\n'.join(original.splitlines()[:-1])+'\n']:
            target.write_text(contents)
            with self.assertRaises(contract.ContractError):contract.validate_pn(staging,contract.load_query(query),parameters,{(1,1)})
        # Prepare independently validated Q/Pi outputs for real assembler integration.
        qstage=self.root/'qstage';qd=qstage/'results_qpi_env_id_0_1';qd.mkdir(parents=True)
        (qd/'query_peptides_results_HLA_A0101_q_values.csv').write_text('peptide,env_id,q_value\nPEPTIDEAA,0,0.1\nPEPTIDEAB,0,0.1\n')
        (qd/'query_peptides_results_HLA_A0101_pi_values.csv').write_text('peptide,pi_value\nPEPTIDEAA,0.4\nPEPTIDEAB,0.4\n')
        qfp=grid.json_read(run/'qpi_manifest.json')['mode_fingerprint']
        with contextlib.redirect_stdout(io.StringIO()):
            contract.command_validate_publish(argparse.Namespace(mode='qpi',run_id='fixture',fingerprint=qfp,task_id=0,
                query=str(query),staging_root=str(qstage),final_root=str(run),env_start=0,env_stop=1,
                parameter_start=0,parameter_stop=1,param_file=str(self.params),mn_tuples=str(self.union)))
        for family in ['PDAC','COVID']:
            binary=os.environ.get(f'EXACT_{family}_ASSEMBLER')
            if not binary:continue
            output=self.root/f'{family}.parquet'
            command=[binary,'--results-dirs',str(run),'--dataset','PDAC','--run-id','fixture',
                '--run-manifest',str(run/'run_manifest.json'),'--qpi-manifest',str(run/'qpi_manifest.json'),
                '--pn-manifest',str(run/'pn_manifest.json'),'--mapping',str(mapping),'--query-peptides',str(query),
                '--output',str(output),'--threads','1']
            subprocess.run(command,check=True,capture_output=True)
            metadata=grid.json_read(output.with_suffix('.metadata.json'))
            self.assertEqual(len(metadata['required_parameter_mn_cells']),12)
            self.assertEqual(metadata['n_tensor_rows'],24)
            import pyarrow.parquet as pq
            self.assertEqual(pq.read_table(output).num_rows,24)
        # A changed file is rejected even if the mode fingerprint itself stays unchanged.
        mn=grid.mode_grid(run/'pn_manifest.json',0,6);mn.write_text(mn.read_text()+'2,1\n')
        with self.assertRaises(ValueError):grid.mode_grid(run/'pn_manifest.json',0,6)

if __name__=='__main__':unittest.main()
