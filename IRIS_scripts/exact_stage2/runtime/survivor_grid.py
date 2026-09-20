"""Exact survivor-derived Stage 2 input contract (no numerical calculations)."""
import argparse
import csv
from decimal import Decimal
import hashlib
import json
from pathlib import Path

TAUS = ['1000', '50052', '91691', '120604', '156295', '200000']
GEOMETRY = ['d_pos', 'd_neg', 'steepness_pos', 'steepness_neg']
CONSUMERS = {'full': ['full_hla', 'mono_q_full_pn'], 'focal': ['focal_hla'],
             'mono': ['old_monoallelic', 'full_q_mono_pn']}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def decimal(value):
    result = Decimal(str(value))
    require(result.is_finite() and result > 0, 'nonpositive/nonfinite grid value')
    return format(result.normalize(), 'f')


def sha(path):
    h = hashlib.sha256()
    with Path(path).open('rb') as f:
        for chunk in iter(lambda: f.read(1048576), b''):
            h.update(chunk)
    return h.hexdigest()


def json_read(path):
    return json.loads(Path(path).read_text())


def record(path, root):
    return {'path': str(path.relative_to(root)), 'sha256': sha(path), 'bytes': path.stat().st_size}


def checked_file(root, rec):
    path = (root / rec['path']).resolve()
    require(path.is_relative_to(root.resolve()), 'grid file escapes manifest directory')
    require(path.is_file() and path.stat().st_size == rec['bytes'] and sha(path) == rec['sha256'],
            f'grid input hash/size mismatch: {path}')
    return path


def csv_rows(path, headers):
    with path.open(newline='') as f:
        r = csv.DictReader(f)
        require(r.fieldnames == headers, f'bad header: {path}')
        return [[row[h] for h in headers] for row in r]


def load(path, representation=None, scope=None):
    path = Path(path).resolve()
    d = json_read(path)
    require(d.get('kind') == 'survivor_exact_stage2_grid' and d.get('schema_version') == 1,
            'expected survivor_exact_stage2_grid v1')
    kind = d['construction']
    require(kind in CONSUMERS, 'unknown construction')
    expected_rep = 'mono' if kind == 'mono' else 'full'
    expected_scope = 'focal' if kind == 'focal' else 'all'
    require(d['hla_environment_representation'] == expected_rep and d['pn_hla_scope'] == expected_scope,
            'construction representation/scope mismatch')
    require(representation in (None, expected_rep) and scope in (None, expected_scope),
            'requested representation/scope does not match survivor grid')
    require(d['tau_values_str'] == TAUS, 'all six frozen tau values are required')
    param = checked_file(path.parent, d['parameter_file'])
    union = checked_file(path.parent, d['mn_union_file'])
    expected = set()
    ids = set()
    for row in d['survivors']:
        require(row['model_id'] in CONSUMERS[kind] and row['metric'] in ['pr', 'roc'], 'wrong survivor consumer')
        require(row['system_id'] not in ids, 'duplicate survivor identity')
        ids.add(row['system_id'])
        g = tuple(decimal(row[k]) for k in GEOMETRY)
        m, n = row['M'], row['N']
        require(type(m) is int and type(n) is int and m > 0 and n > 0, 'invalid M/N')
        expected.add((*g, m, n))
    require(expected, 'empty survivor grid')
    observed = set()
    params = []
    seen_g = set()
    for i, g in enumerate(d['geometries']):
        values = tuple(g['geometry_params_str'])
        require(g['geometry_idx'] == i and len(values) == 4 and values not in seen_g, 'invalid geometry indexing')
        require(tuple(decimal(v) for v in values) == values, 'noncanonical geometry')
        seen_g.add(values)
        mn = checked_file(path.parent, g['mn_file'])
        pairs = [tuple(map(int, r)) for r in csv_rows(mn, ['at_least_M', 'at_most_N'])]
        require(pairs == sorted(set(pairs)) and pairs == [tuple(p) for p in g['mn_pairs']], 'M/N file mismatch')
        for m, n in pairs:
            require(m > 0 and n > 0, 'nonpositive M/N')
            observed.add((*values, m, n))
        params.extend([*values, tau] for tau in TAUS)
    require(observed == expected, 'computed regimes must equal the exact survivor union')
    require(csv_rows(param, GEOMETRY + ['tau_thymus']) == params, 'parameter CSV must be geometry-major, six taus each')
    pairs = sorted({r[-2:] for r in expected})
    require(csv_rows(union, ['at_least_M', 'at_most_N']) == [[str(m), str(n)] for m, n in pairs], 'M/N union mismatch')
    require(d['n_regimes'] == len(expected), 'incorrect regime count')
    return d, param, union


def mode_grid(mode_path, parameter_start, parameter_stop):
    """Resolve one geometry's M/N file against the frozen mode fingerprint."""
    mode = json_read(mode_path)
    comparable = {k: v for k, v in mode.items() if k not in ('run_id', 'mode_fingerprint')}
    digest = hashlib.sha256(json.dumps(comparable, sort_keys=True, separators=(',', ':')).encode()).hexdigest()
    require(digest == mode['mode_fingerprint'], 'mode fingerprint mismatch')
    entries = [x for x in mode['inputs'] if x['role'] == 'survivor_grid']
    require(len(entries) == 1, 'missing exact survivor grid in mode manifest')
    entry = entries[0]
    require(sha(entry['path']) == entry['sha256'], 'survivor grid changed after submission')
    d, param, _ = load(entry['path'])
    require(parameter_start % 6 == 0 and parameter_stop == parameter_start + 6,
            'each P/N task must compute one geometry with six taus')
    index = parameter_start // 6
    require(0 <= index < len(d['geometries']), 'geometry index out of range')
    return checked_file(Path(entry['path']).parent, d['geometries'][index]['mn_file'])


def main():
    p = argparse.ArgumentParser()
    sub = p.add_subparsers(dest='command', required=True)
    desc = sub.add_parser('describe')
    desc.add_argument('--manifest', required=True)
    desc.add_argument('--representation', required=True)
    desc.add_argument('--scope', required=True)
    task = sub.add_parser('task-mn')
    task.add_argument('--mode-manifest', required=True)
    task.add_argument('--start', type=int, required=True)
    task.add_argument('--stop', type=int, required=True)
    a = p.parse_args()
    if a.command == 'describe':
        d, param, union = load(a.manifest, a.representation, a.scope)
        print(f'{len(d["geometries"])*6}\t{len(d["geometries"])}\t{param}\t{union}')
    else:
        print(mode_grid(a.mode_manifest, a.start, a.stop))


if __name__ == '__main__':
    main()
