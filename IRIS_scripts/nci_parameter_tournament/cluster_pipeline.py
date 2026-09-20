#!/usr/bin/env python3
"""Portable NCI Slurm DAG. Only `submit` calls Slurm; stages are also locally testable."""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys

DEFAULT_PROJECT = Path('/data1/lukszam/Marcus/Supported_Model_Replacement')
MODELS = ['full_hla', 'focal_hla', 'old_monoallelic', 'mono_q_full_pn', 'full_q_mono_pn']
BRANCHES = [(model, metric) for model in MODELS for metric in ['pr', 'roc']]


def digest(path):
    h = hashlib.sha256()
    with open(path, 'rb') as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b''):
            h.update(chunk)
    return h.hexdigest()


def read(path):
    return json.loads(Path(path).read_text())


def write(path, value):
    path = Path(path)
    temporary = path.with_name(path.name + '.tmp')
    temporary.write_text(json.dumps(value, indent=2) + '\n')
    temporary.replace(path)


def run(*args):
    command = list(map(str, args))
    print(shlex.join(command), flush=True)
    subprocess.run(command, check=True)


def stages(settings):
    return [('prepare', 1), ('targets', 10 * settings['target_shards']),
            ('merge', 10), ('complete', 10 * settings['completion_shards']),
            ('branch-finalize', 10), ('finalist-prepare', 1),
            ('finalist-run', 2), ('publish', 1)]


def sbatch(settings, root, name, count, dependency=None):
    command = ['sbatch', '--parsable', '--nodes=1', '--ntasks=1',
               '--job-name=nci_' + name, '--partition=' + settings['partition'],
               '--cpus-per-task=' + str(settings['threads']), '--mem=' + settings['mem'],
               '--time=' + settings['time'], '--output=' + str(root / (name + '_%A_%a.out')),
               '--error=' + str(root / (name + '_%A_%a.err')),
               '--export=ALL,NCI_CLUSTER_DRIVER=' + str(root / 'workflow/cluster_pipeline.py')
               + ',NCI_CLUSTER_RUN=' + str(root) + ',NCI_CLUSTER_STAGE=' + name]
    if settings['account']:
        command.append('--account=' + settings['account'])
    if count > 1:
        command.append('--array=0-' + str(count - 1) + '%' + str(settings['max_concurrent']))
    if dependency:
        command.extend(['--dependency=afterok:' + dependency, '--kill-on-invalid-dep=yes'])
    command.append(str(root / 'workflow/run_nci_cluster_stage.slurm'))
    return command


def submit(args):
    root = args.run_root.resolve()
    if ',' in str(root) or '\n' in str(root):
        raise ValueError('Run path cannot contain commas/newlines (Slurm export syntax)')
    pipeline = (args.pipeline or args.project_root / 'supported_ap_code/target/release/iris_nci_parameter_tournament').resolve()
    organizer = (args.organizer or args.project_root / 'supported_ap_code/target/release/directed_round_robin_organizer').resolve()
    package = (args.input_package or args.project_root / 'IRIS_scripts/nci_parameter_tournament/input_package').resolve()
    settings = {key: getattr(args, key) for key in ['threads', 'target_shards', 'completion_shards', 'batch_size',
                'max_concurrent', 'partition', 'account', 'mem', 'time']}
    if args.dry_run:
        dependency = None
        for name, count in stages(settings):
            print(shlex.join(sbatch(settings, root, name, count, dependency)))
            dependency = '<' + name + '_job_id>'
        return
    for binary in [pipeline, organizer]:
        if not binary.is_file() or not os.access(binary, os.X_OK):
            raise ValueError('Missing executable: ' + str(binary))
    if not shutil.which('sbatch'):
        raise ValueError('sbatch is unavailable; submit on the IRIS login node')
    run(pipeline, 'audit-inputs', '--package', package)
    # Fixed array sizes allow the complete DAG to be submitted on the login node.
    # Reject oversized arrays before creating the first job when the limit is visible.
    if shutil.which('scontrol'):
        result = subprocess.run(['scontrol', 'show', 'config'], capture_output=True, text=True)
        if result.returncode == 0:
            for line in result.stdout.splitlines():
                if line.strip().startswith('MaxArraySize'):
                    limit = int(line.split('=', 1)[1].strip())
                    if max(count for _, count in stages(settings)) > limit:
                        raise ValueError('Array exceeds MaxArraySize; reduce --target-shards/--completion-shards')
    root.mkdir(parents=True, exist_ok=True)
    with open(root / '.submit.lock', 'a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        if (root / 'jobs.json').exists():
            previous = read(root / 'jobs.json')
            if not args.resume:
                raise ValueError('Run already submitted; use --resume after previous jobs have stopped')
            ids = [item['job_id'] for item in previous['jobs']]
            if ids:
                check = subprocess.run(['squeue', '--noheader', '--jobs=' + ','.join(ids)], capture_output=True, text=True, check=True)
                if check.stdout.strip():
                    raise ValueError('Previous jobs are still queued/running; refusing duplicate submission')
        expected = dict(settings, payload={
            'bin/iris_nci_parameter_tournament': digest(pipeline),
            'bin/directed_round_robin_organizer': digest(organizer),
            'input_package/manifest.json': digest(package / 'manifest.json'),
            'workflow/cluster_pipeline.py': digest(Path(__file__)),
            'workflow/run_nci_cluster_stage.slurm': digest(Path(__file__).with_name('run_nci_cluster_stage.slurm'))})
        if (root / 'run.json').exists():
            if read(root / 'run.json') != expected:
                raise ValueError('Run settings, input package, or executables changed; use a new run root')
            for path, hash_ in expected['payload'].items():
                if digest(root / path) != hash_:
                    raise ValueError('Frozen runtime payload changed: ' + path)
        else:
            if any(p.name != '.submit.lock' for p in root.iterdir()):
                raise ValueError('Run directory is nonempty without run.json; choose a new run root')
            (root / 'bin').mkdir()
            (root / 'workflow').mkdir()
            shutil.copy2(pipeline, root / 'bin/iris_nci_parameter_tournament')
            shutil.copy2(organizer, root / 'bin/directed_round_robin_organizer')
            shutil.copytree(package, root / 'input_package')
            shutil.copy2(__file__, root / 'workflow/cluster_pipeline.py')
            shutil.copy2(Path(__file__).with_name('run_nci_cluster_stage.slurm'), root / 'workflow/run_nci_cluster_stage.slurm')
            for path, hash_ in expected['payload'].items():
                if digest(root / path) != hash_:
                    raise ValueError('Runtime payload changed during snapshot: ' + path)
            run(root / 'bin/iris_nci_parameter_tournament', 'audit-inputs', '--package', root / 'input_package')
            write(root / 'run.json', expected)
        jobs = {'jobs': [], 'status': 'submitting'}
        if (root / 'jobs.json').exists():
            history = read(root / 'submission_history.json') if (root / 'submission_history.json').exists() else []
            history.append(read(root / 'jobs.json'))
            write(root / 'submission_history.json', history)
        write(root / 'jobs.json', jobs)
        dependency = None
        try:
            for name, count in stages(settings):
                command = sbatch(settings, root, name, count, dependency)
                result = subprocess.run(command, capture_output=True, text=True, check=True)
                job_id = result.stdout.strip().split(';')[0]
                if not job_id.isdigit():
                    raise ValueError('Unexpected sbatch response: ' + result.stdout)
                jobs['jobs'].append({'stage': name, 'job_id': job_id})
                write(root / 'jobs.json', jobs)
                dependency = job_id
                print(name + ': ' + job_id, flush=True)
            jobs['status'] = 'submitted'
            write(root / 'jobs.json', jobs)
        except BaseException:
            jobs['status'] = 'submission_failed'
            write(root / 'jobs.json', jobs)
            if jobs['jobs']:
                subprocess.run(['scancel'] + [j['job_id'] for j in jobs['jobs']], check=False)
            raise
    print('Final result: ' + str(root / 'component_winners.json'))


def stage(args):
    root = args.run_root.resolve()
    settings = read(root / 'run.json')
    for path, expected in settings['payload'].items():
        if digest(root / path) != expected:
            raise ValueError('Frozen runtime payload changed: ' + path)
    threads = int(os.environ.get('SLURM_CPUS_PER_TASK', settings['threads']))
    if threads <= 0:
        raise ValueError('Invalid CPU allocation')
    os.environ.update(RAYON_NUM_THREADS=str(threads), OMP_NUM_THREADS='1', MKL_NUM_THREADS='1', OPENBLAS_NUM_THREADS='1')
    pipe = root / 'bin/iris_nci_parameter_tournament'
    org = root / 'bin/directed_round_robin_organizer'
    config = root / 'input_package/config.json'
    prepared = root / 'prepared'
    counts = dict(stages(settings))
    if args.task < 0 or args.task >= counts[args.stage]:
        raise ValueError('Task ID is outside the stage assignment')
    if args.stage == 'prepare':
        run(pipe, 'audit-inputs', '--package', root / 'input_package')
        if read(config)['tournament']['selection_strategy'] != 'candidate_conservative':
            raise ValueError('Distributed accelerated execution requires candidate_conservative selection')
        if not prepared.exists():
            run(pipe, 'prepare', '--config', config, '--output', prepared, '--threads', threads)
        run(pipe, 'audit-prepared', '--package', prepared, '--config', config)
        for model, metric in BRANCHES:
            bundle = prepared / model / metric / 'bundle'
            plan = root / 'plans' / model / metric
            plan.parent.mkdir(parents=True, exist_ok=True)
            if not plan.exists():
                run(org, 'accelerated', 'plan', '--bundle', bundle, '--output', plan, '--batch-size', settings['batch_size'])
            recipe = read(plan / 'accelerated_plan.json')
            if recipe['options']['batch_size'] != settings['batch_size'] or recipe['options']['output_scope'] != 'survivor_set_and_operational_inputs':
                raise ValueError('Existing accelerated plan options differ')
        return
    if args.stage in ['targets', 'merge', 'complete', 'branch-finalize']:
        count = settings['target_shards'] if args.stage in ['targets', 'merge'] else settings['completion_shards']
        branch_id, shard = divmod(args.task, count) if args.stage in ['targets', 'complete'] else (args.task, 0)
        model, metric = BRANCHES[branch_id]
        bundle = prepared / model / metric / 'bundle'
        plan = root / 'plans' / model / metric
        results = root / 'results' / model / metric
        survivors = root / 'survivors' / model / metric
        selection = root / 'selections' / model / metric
        phase = {'targets': 'targets', 'merge': 'merge', 'complete': 'complete', 'branch-finalize': 'finish'}[args.stage]
        receipt_type = 'target_receipts' if phase in ['targets', 'merge'] else 'completion_receipts'
        for directory in [survivors, selection]:
            directory.parent.mkdir(parents=True, exist_ok=True)
        command = [org, 'accelerated', 'distributed', '--stage', phase, '--bundle', bundle, '--plan', plan,
                   '--results', results, '--receipts', root / receipt_type / model / metric,
                   '--output', survivors if phase == 'merge' else selection, '--shard-count', count,
                   '--shard-id', shard, '--threads', threads]
        if phase in ['complete', 'finish']:
            command.extend(['--survivors', survivors])
        run(*command)
        if phase == 'finish':
            output = root / 'version2' / model / metric
            finalize(pipe, config, bundle, plan, results, selection, output, threads)
        return
    if args.stage == 'finalist-prepare':
        if not (root / 'finalists').exists():
            run(pipe, 'prepare-finalists', '--config', config, '--prepared', prepared,
                '--version2', root / 'version2', '--output', root / 'finalists')
        run(pipe, 'audit-finalists', '--package', root / 'finalists')
    elif args.stage == 'finalist-run':
        metric = ['pr', 'roc'][args.task]
        run(pipe, 'audit-finalists', '--package', root / 'finalists')
        branch = read(root / 'finalists/manifest.json')['branches'][metric]
        if branch['status'] != 'tournament':
            return
        bundle = root / 'finalists' / metric / 'bundle'
        plan = root / 'finalist_plans' / metric
        results = root / 'finalist_results' / metric
        selection = root / 'finalist_selections' / metric
        for directory in [plan, selection]:
            directory.parent.mkdir(parents=True, exist_ok=True)
        if not plan.exists():
            run(org, 'accelerated', 'plan', '--bundle', bundle, '--output', plan, '--batch-size', settings['batch_size'])
        if not selection.exists():
            run(org, 'accelerated', 'run', '--bundle', bundle, '--plan', plan, '--results', results,
                '--output', selection, '--threads', threads)
        run(org, 'accelerated', 'audit', '--bundle', bundle, '--plan', plan, '--results', results, '--selection', selection)
        finalize(pipe, config, bundle, plan, results, selection, root / 'final_version2' / metric, threads)
    elif args.stage == 'publish':
        run(pipe, 'summarize-components', '--config', config, '--finalists', root / 'finalists',
            '--selections', root / 'final_version2', '--output', root / 'component_winners.json')


def finalize(pipe, config, bundle, plan, results, selection, output, threads):
    output.parent.mkdir(parents=True, exist_ok=True)
    if not output.exists():
        run(pipe, 'finalize-version2-accelerated', '--config', config, '--bundle', bundle, '--plan', plan,
            '--results', results, '--selection', selection, '--output', output, '--threads', threads)
    run(pipe, 'audit-version2', '--output', output, '--config', config, '--bundle', bundle)


def positive(value):
    result = int(value)
    if result < 1:
        raise argparse.ArgumentTypeError('must be positive')
    return result


def main():
    if sys.version_info < (3, 8):
        raise RuntimeError('The cluster driver requires Python 3.8 or newer')
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    submit_parser = commands.add_parser('submit')
    submit_parser.add_argument('--project-root', type=Path, default=DEFAULT_PROJECT)
    submit_parser.add_argument('--run-root', type=Path, required=True)
    for name in ['input-package', 'pipeline', 'organizer']:
        submit_parser.add_argument('--' + name, type=Path)
    for name, default in [('threads', 4), ('target-shards', 100), ('completion-shards', 16), ('batch-size', 1), ('max-concurrent', 100)]:
        submit_parser.add_argument('--' + name, type=positive, default=default)
    for name, default in [('partition', 'componc_cpu'), ('account', 'lukszam'), ('mem', '20G'), ('time', '2-00:00:00')]:
        submit_parser.add_argument('--' + name, default=default)
    submit_parser.add_argument('--dry-run', action='store_true')
    submit_parser.add_argument('--resume', action='store_true')
    stage_parser = commands.add_parser('stage')
    stage_parser.add_argument('--run-root', type=Path, required=True)
    stage_parser.add_argument('--stage', choices=[s for s, _ in stages({'target_shards': 1, 'completion_shards': 1})], required=True)
    stage_parser.add_argument('--task', type=int, default=0)
    args = parser.parse_args()
    (submit if args.command == 'submit' else stage)(args)


if __name__ == '__main__':
    main()
