#!/usr/bin/env python3
"""Derive exact computational grids from merged NCI certificates and registry."""
import argparse
import csv
import json
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent / 'runtime'))
import survivor_grid as grid


def write_json(path, payload):
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + '\n')


def write_csv(path, headers, rows):
    with path.open('w', newline='') as f:
        w = csv.writer(f, lineterminator='\n')
        w.writerow(headers)
        w.writerows(rows)


def build(run, output):
    output.mkdir(parents=True, exist_ok=True)
    records, sources = [], []
    for model in sum(grid.CONSUMERS.values(), []):
        for metric in ['pr', 'roc']:
            cert = run / 'survivors' / model / metric / 'selection_certificate.json'
            roster = run / 'prepared' / model / metric / 'candidate_roster.json'
            c, r = grid.json_read(cert), grid.json_read(roster)
            grid.require(c['output_scope'] == 'survivor_set', 'expected merged survivor certificate')
            lookup = {x['system_id']: x for x in r['candidates']}
            grid.require(len(lookup) == len(r['candidates']), 'duplicate registry identity')
            grid.require(len(set(c['survivors'])) == len(c['survivors']), 'duplicate certified survivor')
            for sid in c['survivors']:
                row = lookup[sid]
                grid.require(row['model_id'] == model and row['metric'] == metric, 'registry branch mismatch')
                records.append({k: row[k] for k in ['system_id', 'model_id', 'metric', 'regime_idx'] + grid.GEOMETRY + ['M', 'N']})
            sources.append({'model_id': model, 'metric': metric, 'plan_id': c['plan_id'],
                            'certificate': {'path': str(cert.resolve()), 'sha256': grid.sha(cert)},
                            'registry': {'path': str(roster.resolve()), 'sha256': grid.sha(roster)}})
    for kind, consumers in grid.CONSUMERS.items():
        survivors = [r for r in records if r['model_id'] in consumers]
        wanted = {(*(grid.decimal(r[k]) for k in grid.GEOMETRY), r['M'], r['N']) for r in survivors}
        geometries = sorted({r[:4] for r in wanted})
        param = output / f'{kind}_param_sets.csv'
        union = output / f'{kind}_mn_union.csv'
        write_csv(param, grid.GEOMETRY + ['tau_thymus'], [[*g, t] for g in geometries for t in grid.TAUS])
        write_csv(union, ['at_least_M', 'at_most_N'], sorted({r[-2:] for r in wanted}))
        groups = []
        for i, g in enumerate(geometries):
            pairs = sorted({r[-2:] for r in wanted if r[:4] == g})
            mn = output / f'{kind}_mn_g{i:04d}.csv'
            write_csv(mn, ['at_least_M', 'at_most_N'], pairs)
            groups.append({'geometry_idx': i, 'geometry_params_str': g,
                           'mn_pairs': pairs, 'mn_file': grid.record(mn, output)})
        manifest = output / f'{kind}_grid.json'
        write_json(manifest, {'schema_version': 1, 'kind': 'survivor_exact_stage2_grid',
            'construction': kind, 'hla_environment_representation': 'mono' if kind == 'mono' else 'full',
            'pn_hla_scope': 'focal' if kind == 'focal' else 'all', 'tau_values_str': grid.TAUS,
            'n_regimes': len(wanted), 'parameter_file': grid.record(param, output),
            'mn_union_file': grid.record(union, output), 'geometries': groups, 'survivors': survivors,
            'sources': [s for s in sources if s['model_id'] in consumers]})
        grid.load(manifest)
        print(f'{kind}: {len(wanted)} regimes; {len(geometries)} geometries; {len(wanted)*6} cells per observation')


def main():
    p = argparse.ArgumentParser()
    p.add_argument('--nci-run', type=Path, required=True)
    p.add_argument('--output', type=Path, required=True)
    p.add_argument('--base-config', type=Path)
    p.add_argument('--package-config', type=Path)
    a = p.parse_args()
    build(a.nci_run, a.output)
    if a.package_config:
        d = grid.json_read(a.base_config)
        d['passthrough_files'] = [{'path': str(x.resolve()), 'expected_sha256': grid.sha(x)}
                                  for x in sorted(a.output.iterdir()) if x.suffix in ('.csv', '.json')]
        write_json(a.package_config, d)


if __name__ == '__main__':
    main()
