#!/usr/bin/env python3
"""Preflight and submit exact survivor computations with unchanged Runner CLI."""
import argparse
import os
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT))
import survivor_grid as grid
import validate_complete_f_stage2_inputs as inputs


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--dataset', choices=['all', 'PDAC', 'COVID_SPIKE', 'COVID_NONSPIKE'], default='all')
    p.add_argument('--construction', choices=['all', 'full', 'focal', 'mono'], default='all')
    p.add_argument('--run-id-prefix', required=True)
    p.add_argument('--input-package', type=Path, default=ROOT / 'complete_f_input_package')
    p.add_argument('--mode', choices=['all', 'pn', 'qpi'], default='all')
    p.add_argument('--dry-run', action='store_true')
    p.add_argument('--missing-only', action='store_true')
    p.add_argument('--pn-peptides-per-shard', type=int, default=4000)
    p.add_argument('--array-concurrency', type=int, default=11)
    p.add_argument('--pn-cpus-per-task', type=int, default=16)
    p.add_argument('--qpi-cpus-per-task', type=int, default=8)
    p.add_argument('--pn-mem', default='60G')
    p.add_argument('--qpi-mem', default='30G')
    p.add_argument('--cpus-per-task', type=int, help='Override CPUs for both computation modes')
    p.add_argument('--mem', help='Override memory for both computation modes')
    p.add_argument('--partition', default='componc_cpu')
    p.add_argument('--target-env-ids')
    a = p.parse_args()
    grid.require(re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9._-]*', a.run_id_prefix), 'invalid run ID prefix')
    grid.require(min(a.pn_peptides_per_shard, a.array_concurrency,
                     a.pn_cpus_per_task, a.qpi_cpus_per_task,
                     a.cpus_per_task if a.cpus_per_task is not None else 1) > 0,
                 'resource counts must be positive')
    package = a.input_package.resolve()
    manifest = grid.json_read(package / 'manifest.json')
    datasets = ['PDAC', 'COVID_SPIKE', 'COVID_NONSPIKE'] if a.dataset == 'all' else [a.dataset]
    kinds = ['full', 'focal', 'mono'] if a.construction == 'all' else [a.construction]
    modes = ['qpi', 'pn'] if a.mode == 'all' else [a.mode]
    commands = []
    for dataset in datasets:
        audit = manifest['datasets'][dataset]
        prefix = dataset.lower()
        for kind in kinds:
            rep, scope = ('mono', 'all') if kind == 'mono' else ('full', 'focal' if kind == 'focal' else 'all')
            validation = inputs.validate(package, dataset, rep)
            gpath = inputs.verify_manifest_output(package, manifest, f'{kind}_grid.json')
            exact, _, _ = grid.load(gpath, rep, scope)
            # Validate every installed cohort file used in construction/assembly.
            for name in inputs.collect_dataset_outputs(audit):
                inputs.verify_manifest_output(package, manifest, name)
            envs = audit['mono_environment_rows' if rep == 'mono' else 'full_environment_rows']
            base = ['bash', os.environ.get('SHARED_SUBMIT', str(ROOT / 'submit_stage2_new.sh')),
                    '--dataset', dataset, '--run-id', f'{a.run_id_prefix}_{kind}',
                    '--grid-profile', 'survivor-exact', '--regime-manifest', str(gpath),
                    '--q-model-config', validation['config'], '--external-validation-input-root', str(package),
                    '--hla-environment-representation', rep, '--pn-hla-scope', scope,
                    '--total-env-ids', str(envs), '--env-chunks', str(envs),
                    '--array-concurrency', str(a.array_concurrency), '--partition', a.partition]
            if rep == 'mono':
                base += ['--representation-crosswalk', str(package / f'{prefix}_full_to_mono_observation_crosswalk.csv')]
            if a.target_env_ids:
                base += ['--target-env-ids', a.target_env_ids]
            if a.missing_only:
                base += ['--missing-only']
            for mode in modes:
                cpus = a.cpus_per_task if a.cpus_per_task is not None else getattr(a, f'{mode}_cpus_per_task')
                memory = a.mem if a.mem is not None else getattr(a, f'{mode}_mem')
                command = base + ['--mode', mode, '--cpus-per-task', str(cpus), '--mem', memory]
                if mode == 'pn':
                    command += ['--pn-peptides-per-shard', str(a.pn_peptides_per_shard)]
                commands.append(command)
            print(f'{dataset}/{kind}: {exact["n_regimes"]} exact regimes, {len(exact["geometries"])} geometries, {envs} environments', flush=True)
    # Complete every preflight before the first sbatch call.
    for command in commands:
        subprocess.run(command + ['--dry-run'], check=True)
    if not a.dry_run:
        for command in commands:
            subprocess.run(command, check=True)


if __name__ == '__main__':
    main()
