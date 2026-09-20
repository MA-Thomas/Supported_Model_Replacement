#!/usr/bin/env python3
"""Install the reviewed exact Stage 2 overlay, refusing concurrent source edits."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import shutil
import sys
import tempfile

HERE = Path(__file__).resolve().parent
RUNTIME = HERE / 'runtime'
sys.path.insert(0, str(RUNTIME))
import survivor_grid as grid


def inventory(root):
    return {str(p.relative_to(root)): grid.sha(p) for p in root.rglob('*')
            if p.is_file() and p.name != '.DS_Store' and '__pycache__' not in p.parts}


def install(destination, package, plan_path, apply=False):
    destination, package, plan_path = destination.resolve(), package.resolve(), plan_path.resolve()
    plan = grid.json_read(plan_path)
    receipt_name = plan_path.name.replace('installation_plan', 'installation_receipt', 1)
    grid.require(plan_path.name.startswith('installation_plan'), 'installation plan filename must start with installation_plan')
    receipt_path = HERE / receipt_name
    if apply:
        grid.require(not receipt_path.exists(), f'installation receipt already exists: {receipt_path}')
    for entry in plan['files']:
        target = destination / entry['path']
        expected = entry['before_sha256']
        grid.require((target.is_file() and grid.sha(target) == expected) if expected else not target.exists(),
                     f'destination changed since review: {target}')
        grid.require(grid.sha(RUNTIME / entry['path']) == entry['after_sha256'], 'staged source changed since review')
    for name, expected in plan.get('unchanged_files', {}).items():
        grid.require(grid.sha(destination / name) == expected, f'unchanged source differs: {name}')
    for name, expected in plan['replace_directories'].items():
        grid.require(inventory(destination / name) == expected, f'{name} changed since review')
    for path, expected in grid.json_read(HERE / 'numerical_source_hashes.json').items():
        grid.require(grid.sha(path) == expected, f'numerical source changed: {path}')
    manifest = grid.json_read(package / 'manifest.json')
    for name, rec in manifest['output_files'].items():
        file = package / name
        grid.require(file.is_file() and file.stat().st_size == rec['bytes'] and grid.sha(file) == rec['sha256'],
                     f'invalid staged input package file: {file}')
    for kind in ['full', 'focal', 'mono']:
        grid.load(package / f'{kind}_grid.json')
    print(f'Reviewed {len(plan["files"])} file updates and {len(plan["replace_directories"])} directory replacements.', flush=True)
    if not apply:
        return

    stamp = datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%S%fZ')
    backup = destination / 'exact_stage2_backups' / stamp
    backup.mkdir(parents=True, exist_ok=False)
    replaced = []
    installed = []
    try:
        # Stage every replacement before changing any live files.
        with tempfile.TemporaryDirectory(prefix='.exact_stage2_install_', dir=destination) as staging:
            staging = Path(staging)
            directory_sources = {'complete_f_stage2': RUNTIME / 'complete_f_stage2',
                                 'complete_f_input_package': package}
            for name in plan['replace_directories']:
                shutil.copytree(directory_sources[name], staging / name,
                                ignore=shutil.ignore_patterns('__pycache__', '*.pyc'))
            for entry in plan['files']:
                staged = staging / 'file_updates' / entry['path']
                staged.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(RUNTIME / entry['path'], staged)
                grid.require(grid.sha(staged) == entry['after_sha256'], 'source changed during staging')
            for name in plan['replace_directories']:
                os.replace(destination / name, backup / name)
                replaced.append(name)
                os.replace(staging / name, destination / name)
            for entry in plan['files']:
                rel = entry['path']
                target = destination / rel
                if target.exists():
                    old = backup / rel
                    old.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copy2(target, old)
                target.parent.mkdir(parents=True, exist_ok=True)
                os.replace(staging / 'file_updates' / rel, target)
                installed.append(rel)
        for entry in plan['files']:
            grid.require(grid.sha(destination / entry['path']) == entry['after_sha256'], 'installed file mismatch')
        for name in plan['replace_directories']:
            grid.require(inventory(destination / name) == inventory(directory_sources[name]), f'installed directory mismatch: {name}')
        for name, expected in plan.get('unchanged_files', {}).items():
            grid.require(grid.sha(destination / name) == expected, f'unchanged source differs: {name}')
    except BaseException:
        for rel in reversed(installed):
            old, target = backup / rel, destination / rel
            if old.exists():
                shutil.copy2(old, target)
            elif target.exists():
                target.unlink()
        for name in reversed(replaced):
            target = destination / name
            if target.exists():
                shutil.rmtree(target)
            os.replace(backup / name, target)
        raise
    receipt = dict(destination=str(destination), backup=str(backup), package_manifest_sha256=grid.sha(package/'manifest.json'),
                   installation_plan=str(plan_path), installation_plan_sha256=grid.sha(plan_path),
                   files=plan['files'], numerical_source_files_unchanged=True)
    with receipt_path.open('x') as handle:
        handle.write(json.dumps(receipt, indent=2) + '\n')
    print(f'Installed exact survivor Stage 2. Backup: {backup}')
    return receipt_path


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--destination', type=Path, required=True)
    p.add_argument('--package', type=Path, required=True)
    p.add_argument('--apply', action='store_true')
    p.add_argument('--plan', type=Path, default=HERE / 'installation_plan.rust_compat_incremental.json')
    a = p.parse_args()
    install(a.destination, a.package, a.plan, a.apply)


if __name__ == '__main__':
    main()
