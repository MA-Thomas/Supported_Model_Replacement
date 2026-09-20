#!/usr/bin/env python3
"""Reproducible single-context ProteinGym AP assessments and graph reductions."""
from __future__ import annotations

import argparse
import csv
import hashlib
import itertools
import json
import math
import os
from pathlib import Path
import platform
import subprocess
import sys
from datetime import datetime, timezone

STUDY = Path(__file__).resolve().parent
ROOT = STUDY.parents[1]
sys.path.insert(0, str(ROOT))
from domain_study.search_evidence import effect_bounds, effect_columns, evidence_columns, status, validate_summary

PARENT = STUDY.parent / 'proteingym' / 'outputs'
MODELS = ['score_eve_ensemble', 'score_esm1v_ensemble', 'score_esm2_650m']
LABELS = dict(zip(MODELS, ['EVE ensemble', 'ESM-1v ensemble', 'ESM-2 650M']))
PAIRS = [(MODELS[1], MODELS[0]), (MODELS[2], MODELS[1]), (MODELS[2], MODELS[0])]
SLUGS = ['esm1v_ensemble_vs_eve_ensemble', 'esm2_650m_vs_esm1v_ensemble', 'esm2_650m_vs_eve_ensemble']
PROFILES = {
    'quick': dict(replications=20, grid_points=3, tolerance=1e-6, max_iterations=64, refinement_draws=4),
    'publication': dict(replications=200, grid_points=3, tolerance=1e-8, max_iterations=128, refinement_draws=8),
}


def sha256(path):
    h = hashlib.sha256()
    with Path(path).open('rb') as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b''):
            h.update(block)
    return h.hexdigest()


def read_csv(path):
    with Path(path).open(newline='') as handle:
        return list(csv.DictReader(handle))


def save_json(path, obj):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(path.suffix + '.tmp')
    temporary.write_text(json.dumps(obj, indent=2, allow_nan=False) + '\n')
    temporary.replace(path)


def save_csv(path, rows):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    if not rows:
        raise ValueError('table schema must not be inferred from an empty collection')
    with path.open('w', newline='') as handle:
        writer = csv.DictWriter(handle, list(rows[0]))
        writer.writeheader()
        writer.writerows(rows)


def source_maximal(models, edges):
    """The current observed gate forces an acyclic graph; reject violations."""
    vertices = set(models)
    edges = set(edges)
    if any(a not in vertices or b not in vertices or a == b for a, b in edges):
        raise ValueError('invalid graph edge')
    remaining = set(vertices)
    while remaining:
        roots = {b for b in remaining if not any(v == b and u in remaining for u, v in edges)}
        if not roots:
            raise ValueError('cycle contradicts the common observed gate')
        remaining -= roots
    return vertices - {b for _, b in edges}


def reduce_graphs(models, graphs):
    if not graphs:
        raise ValueError('at least one completed context graph is required')
    maximal = {e: source_maximal(models, edges) for e, edges in graphs.items()}
    candidate = set.intersection(*maximal.values())
    shared = set.intersection(*(set(edges) for edges in graphs.values()))
    replacement = source_maximal(models, shared)
    if not candidate <= replacement:
        raise ValueError('acyclic graph reductions violated set inclusion')
    return {'candidate_conservative': sorted(candidate), 'replacement_conservative': sorted(replacement),
            'common_edges': [list(edge) for edge in sorted(shared)],
            'contexts_admissible': {m: sum(m in x for x in maximal.values()) for m in models}}, maximal


def anchored_summary(anchor, effects, order, floor, delta, gamma):
    """Independent ordered-subset formula used to validate Rust output."""
    n = len(effects)
    if n < order or order < 1 or not all(math.isfinite(x) for x in [anchor] + list(effects)):
        raise ValueError('invalid finite effect collection')
    denominator = math.comb(n, order)
    values = sorted(effects)
    magnitude = math.fsum(min(anchor, value) * math.comb(n-i-1, order-1) / denominator
                          for i, value in enumerate(values[:n-order+1]))
    count = sum(x > floor for x in values)
    survival = (math.comb(count, order) / denominator if count >= order and anchor > floor else 0.0)
    return dict(magnitude=magnitude, survival=survival, supported=magnitude > delta and survival > gamma)


def prepare_plan(args):
    source_paths = {'scores': PARENT/'predictions/proteingym_observed_scores.csv',
                    'audit': PARENT/'tables/assay_audit.csv', 'parent_manifest': PARENT/'manifest.json',
                    'saved_effects': PARENT/'tables/observed_assay_effects.csv'}
    parent = json.loads(source_paths['parent_manifest'].read_text())
    if parent['profile'] != 'publication' or parent['manuscript_version'] != 'v17':
        raise ValueError('requires the retained parent publication study')
    audit = read_csv(source_paths['audit'])
    ids = sorted(x['assay_id'] for x in audit)
    if len(ids) != len(set(ids)) or len(ids) != parent['retained']['assays']:
        raise ValueError('parent assay roster is inconsistent')
    parameters = dict(PROFILES[args.profile])
    if args.replications is not None:
        parameters['replications'] = args.replications
    parameters.update(seed=args.seed, lower=0.01, upper=0.90, computational_order=2, delta=0.0, floor=0.0, gamma=0.81)
    implementation = [ROOT/'examples/proteingym_contexts.rs', ROOT/'Cargo.toml', ROOT/'Cargo.lock'] + sorted((ROOT/'src').rglob('*.rs')) + [Path(__file__), STUDY.parent/'search_evidence.py']
    core = {'schema_version': 2, 'analysis': 'proteingym_single_context_anchored_ap_graphs',
            'profile': args.profile, 'parameters': parameters, 'models': MODELS, 'context_ids': ids,
            'primary_reduction': 'candidate_conservative', 'comparison_reduction': 'replacement_conservative',
            'sources': {k: {'path': str(p.relative_to(ROOT)), 'sha256': sha256(p)} for k, p in source_paths.items()},
            'numerical_source_sha256': {str(p.relative_to(ROOT)): sha256(p) for p in implementation},
            'resampling': {'unit': 'residue_position_within_assay',
                'draw_size': 'observed number of distinct positions; sample positions with replacement',
                'cluster_contents': 'all retained substitutions at each selected position',
                'missing_class': 'redraw complete draw; maximum 10000 attempts',
                'pairing': 'same multiplicities across models, directions, and prevalences within an assay',
                'seed_scheme': 'FNV-1a context-ID hash, SplitMix64, ChaCha8; independent of pair and input row order'},
            'interpretation': 'retrospective illustration conditional on this computational design; no population or deployment guarantee',
            'metric': 'tie-averaged prior-standardized paired CNAP',
            'empirical_aggregation': None,
            'numerical_checks': {'observed': 'bounded adaptive searches; refined searches use twice the initial density and tenfold tighter objective tolerance',
                'computational': 'deterministically spaced replications at the same refined settings',
                'replication_count': 'half-length prefix compared with full finite list; not an independent-seed check'}}
    run_id = hashlib.sha256(json.dumps(core, sort_keys=True).encode()).hexdigest()
    plan = dict(core, run_id=run_id)
    path = args.output/'plan.json'
    if path.exists():
        existing = json.loads(path.read_text())
        if existing['run_id'] != run_id:
            raise ValueError('output belongs to another input/configuration/implementation; select a new --output directory')
    else:
        if args.render_only:
            raise ValueError('render requires a completed run with a saved plan')
        save_json(path, dict(plan, declared_at_utc=datetime.now(timezone.utc).isoformat()))
    return plan, audit, source_paths


def load_and_validate(output, plan, audit, parent_effects):
    reports = []
    context_ids = plan['context_ids']; p = plan['parameters']
    expected_paths = {f'context-{i:03}-pair-{j}.json' for i in range(len(context_ids)) for j in range(3)}
    actual_paths = {f.name for f in (output/'reports').glob('*.json')}
    if actual_paths != expected_paths:
        raise ValueError(f'incomplete or unexpected report set: {len(actual_paths)}/{len(expected_paths)}')
    by_id = {x['assay_id']: x for x in audit}
    parent = {(x['assay_id'], x['comparison']): x for x in parent_effects}
    max_parent_difference = 0.0
    max_legacy_difference = 0.0
    for i, context in enumerate(context_ids):
        for j, pair in enumerate(PAIRS):
            report = json.loads((output/'reports'/f'context-{i:03}-pair-{j}.json').read_text())
            if report['run_id'] != plan['run_id'] or report['context_id'] != context or report['pair_index'] != j:
                raise ValueError('checkpoint identity mismatch')
            if report['observation_count'] != int(by_id[context]['single_variant_count']) or report['positive_count'] != int(by_id[context]['deleterious_count']):
                raise ValueError('context cohort changed')
            for key, orientation, field in [('forward', pair, 'retained_cnap_difference'), ('reverse', pair[::-1], 'reverse_retained_cnap_difference')]:
                d = report[key]
                if (d['candidate'], d['incumbent']) != orientation:
                    raise ValueError('direction identity mismatch')
                anchor = d['observed']['value']
                if not math.isfinite(anchor):
                    raise ValueError('invalid observed effect')
                max_parent_difference = max(max_parent_difference, abs(anchor-float(parent[(context, SLUGS[j])][field])))
                max_legacy_difference = max(max_legacy_difference, abs(d['legacy_observed']['value']-float(parent[(context, SLUGS[j])][field])))
                low, high = effect_bounds(d['observed'])
                validate_summary(d['observed_gate'], (low, float(low > p['floor'])),
                                 (high, float(high > p['floor'])), p)
                gate = status(d['observed_gate']) == 'verified_pass'
                if gate:
                    effects = [x['value'] for x in d['computational']]
                    if len(effects) != p['replications']:
                        raise ValueError('missing computational effects for passing gate')
                    for stage, values in [('full', effects), ('prefix', effects[:p['replications']//2])]:
                        independent = anchored_summary(anchor, values, p['computational_order'], p['floor'], p['delta'], p['gamma'])
                        recorded = d[stage]
                        selected = d['computational'] if stage == 'full' else d['computational'][:p['replications']//2]
                        upper = anchored_summary(high, [effect_bounds(x)[1] for x in selected],
                                                 p['computational_order'], p['floor'], p['delta'], p['gamma'])
                        validate_summary(recorded, (independent['magnitude'], independent['survival']),
                                         (upper['magnitude'], upper['survival']), p)
                    if d['final_supported'] != d['full']['supported']:
                        raise ValueError('final verdict mismatch')
                elif d['computational'] or d['full'] is not None or d['prefix'] is not None or d['final_supported']:
                    raise ValueError('failed observed gate must stop that direction')
            if report['forward']['observed']['value'] + report['reverse']['observed']['value'] > 1e-10:
                raise ValueError('paired orientation bound violated')
            reports.append(report)
    if plan.get('schema_version', 1) < 2 and plan['profile'] == 'publication' and max_legacy_difference > 1e-7:
        raise ValueError(f'unmodified library anchors differ from original publication results by {max_legacy_difference}')
    return reports, max_parent_difference


def summarize(output, plan, audit, reports, parent_difference):
    p = plan['parameters']; ids = plan['context_ids']
    possible_graphs = {stage: {e: set() for e in ids} for stage in ['observed', 'prefix', 'full']}
    unresolved = {stage: 0 for stage in possible_graphs}
    graphs = {stage: {e: set() for e in ids} for stage in ['observed', 'prefix', 'full']}
    directions = []
    for r in reports:
        for key in ['forward', 'reverse']:
            d = r[key]; full = d['full']; prefix = d['prefix']; edge = (d['candidate'], d['incumbent'])
            if d['observed_gate']['supported']: graphs['observed'][r['context_id']].add(edge)
            if prefix and prefix['supported']: graphs['prefix'][r['context_id']].add(edge)
            if d['final_supported']: graphs['full'][r['context_id']].add(edge)
            for stage, assessment in [('observed', d['observed_gate']), ('prefix', prefix), ('full', full)]:
                assessment = assessment or d['observed_gate']
                decision = status(assessment)
                if decision != 'verified_failure': possible_graphs[stage][r['context_id']].add(edge)
                unresolved[stage] += decision == 'unresolved'
            directions.append(dict(**effect_columns(d['observed'], 'observed_'),
                **evidence_columns(full or d['observed_gate']), context_id=r['context_id'], candidate=d['candidate'], incumbent=d['incumbent'],
                observed_effect=d['observed']['value'], limiting_prevalence=d['observed']['limiting_prevalence'],
                observed_gate=d['observed_gate']['supported'], replications=len(d['computational']),
                supported_magnitude=full['magnitude'] if full else '', literal_survival=full['survival'] if full else '',
                full_supported=d['final_supported'], prefix_magnitude=prefix['magnitude'] if prefix else '',
                prefix_survival=prefix['survival'] if prefix else '', prefix_supported=prefix['supported'] if prefix else False))
    reductions = {}; maximal = {}
    for stage, stage_graphs in graphs.items():
        reductions[stage], maximal[stage] = reduce_graphs(MODELS, stage_graphs)
        reductions[stage]['supported_edges'] = sum(map(len, stage_graphs.values()))
    for e in ids:
        if not graphs['full'][e] <= graphs['observed'][e] or not maximal['observed'][e] <= maximal['full'][e]:
            raise ValueError('anchoring/acyclic admissibility invariant violated')
    by_id = {x['assay_id']: x for x in audit}
    contexts = [dict(context_index=i+1, context_id=e, selection_type=by_id[e]['selection_type'],
        units=int(by_id[e]['single_variant_count']), observed_survivors=';'.join(sorted(maximal['observed'][e])),
        full_survivors=';'.join(sorted(maximal['full'][e])),
        observed_edges=len(graphs['observed'][e]), full_edges=len(graphs['full'][e])) for i,e in enumerate(ids)]
    admissibility = [dict(stage=stage, model=m, contexts_admissible=reductions[stage]['contexts_admissible'][m],
                         contexts_total=len(ids), candidate_conservative=m in reductions[stage]['candidate_conservative'],
                         replacement_conservative=m in reductions[stage]['replacement_conservative'])
                     for stage in graphs for m in MODELS]
    diagnostics = dict(maximum_parent_anchor_difference=parent_difference,
        maximum_refinement_difference=max(r['maximum_refinement_difference'] for r in reports),
        refinement_gate_disagreements=sum(r['refinement_gate_disagreements'] for r in reports),
        refinement_floor_disagreements=sum(r['refinement_floor_disagreements'] for r in reports),
        refined_computational_draws=sum(r['refined_computational_draws'] for r in reports),
        prefix_edge_disagreements=sum(len(graphs['prefix'][e] ^ graphs['full'][e]) for e in ids),
        prefix_context_set_disagreements=sum(maximal['prefix'][e] != maximal['full'][e] for e in ids),
        prefix_candidate_set_unchanged=reductions['prefix']['candidate_conservative']==reductions['full']['candidate_conservative'],
        prefix_replacement_set_unchanged=reductions['prefix']['replacement_conservative']==reductions['full']['replacement_conservative'],
        pair_contexts_computed=sum(r['replications']>0 for r in reports),
        rejected_draws_across_pair_jobs=sum(r['rejected_missing_class_draws'] for r in reports))
    if plan.get('schema_version', 1) < 2 and (diagnostics['refinement_gate_disagreements'] or diagnostics['refinement_floor_disagreements'] or diagnostics['maximum_refinement_difference'] > 1e-7):
        raise ValueError(f'numerical refinement needs investigation before reduction is reported: {diagnostics}')
    definite_reductions = {}
    for stage, edges in possible_graphs.items():
        # Possible edges may form cycles; no-incoming candidates remain a safe inner set.
        per_context = {e: set(MODELS) - {b for _, b in v} for e, v in edges.items()}
        common = set.intersection(*edges.values())
        definite_reductions[stage] = dict(candidate_conservative=sorted(set.intersection(*per_context.values())),
            replacement_conservative=sorted(set(MODELS) - {b for _, b in common}))
    summary = dict(schema_version=2, unresolved_decisions=unresolved,
        graph_result_is_resolved=not any(unresolved.values()), definite_reductions=definite_reductions,
        reductions_semantics='possible survivor sets from verified edges; definite_reductions also accounts for unresolved edges', run_id=plan['run_id'], policy=p, context_count=len(ids),
        model_count=len(MODELS), directed_assessments=len(directions),
        computational_direction_assessments=sum(d['observed_gate'] for d in directions),
        reductions=reductions, diagnostics=diagnostics,
        interpretation=plan['interpretation'], numerical_qualification='finite computational sample; continuous prevalence effects enclosed numerically; unresolved policy decisions retained')
    save_csv(output/'tables/directed_assessments.csv', directions)
    save_csv(output/'tables/context_survivors.csv', contexts)
    save_csv(output/'tables/model_admissibility.csv', admissibility)
    save_json(output/'summary.json', summary)
    return summary, directions, contexts, maximal


def names(models):
    return ', '.join(LABELS[m] for m in models) if models else 'empty set'


def write_report(output, plan, summary):
    p = plan['parameters']; reductions = summary['reductions']; diag = summary['diagnostics']
    rows = '\n'.join(f'| {LABELS[m]} | {reductions["observed"]["contexts_admissible"][m]} | '
                     f'{reductions["full"]["contexts_admissible"][m]} |' for m in MODELS)
    text = f'''# ProteinGym: separate context assessments and graph reduction

This retrospective AP/CNAP study treats each of the {summary['context_count']} retained assays as one empirical context. It reuses the existing three-model roster, aligned scores, labels, and 1%–90% target-prevalence range. It fits no models and changes no cohort membership. The original joint empirical study and conditional nested illustration remain separate.

## Declared policy and computational design

- Candidate-conservative reduction is primary; replacement-conservative reduction is a comparison.
- Magnitude threshold δ={p['delta']:g}, survival floor d={p['floor']:g}, literal-survival requirement γ={p['gamma']:g}.
- Both orientations of all three pairs are assessed in every context: {summary['directed_assessments']} directed assessments.
- Each observed effect is the lowest paired tie-averaged CNAP advantage across the declared prevalence interval. The observed gate requires an effect strictly above max(δ,d). Its binary survival is insensitive to γ within (0,1).
- Passing directions receive J={p['replications']} computational evaluations, with order K_C={p['computational_order']}. Every challenge includes its observed anchor. Both supported magnitude >δ and literal survival >γ must hold.
- Each draw samples the observed number of residue positions with replacement, carrying all retained substitutions at each selected position. A complete draw lacking either outcome class is redrawn, up to 10,000 attempts. The design probes dependence on represented positions; it is not an assertion that variants or assays are independent samples from an unseen population.
- A stable context-ID seed supplies the same draws across models and directions within an assay. Seed: {p['seed']}.
- There is no empirical subset order across assays. The Rust library is used only in its exact one-row/order-one boundary to calculate each single-context anchored summary.
- This computational design is adopted for this retrospective illustration, not established prospectively as a ProteinGym deployment regime. Results are conditional on it. No label-exchangeability reference law is asserted.

## Results

| Model | Admissible contexts, observed only | Admissible contexts, full assessment |
|---|---:|---:|
{rows}

Counts are contexts with no incoming supported replacement claim, not wins, accuracy, or a ranking score. Contexts receive no observation-count weights.

- Observed-only candidate-conservative set: **{names(reductions['observed']['candidate_conservative'])}**.
- Full candidate-conservative set S₀: **{names(reductions['full']['candidate_conservative'])}**.
- Observed-only replacement-conservative set: **{names(reductions['observed']['replacement_conservative'])}**.
- Full replacement-conservative set: **{names(reductions['full']['replacement_conservative'])}**.
- Supported context-specific arrows: {reductions['observed']['supported_edges']} observed; {reductions['full']['supported_edges']} after the full challenge.
- Computational assessments: {summary['computational_direction_assessments']} directions; all remaining directions stopped at the observed gate.

An empty candidate-conservative set means no model remained admissible in every declared context. It does not establish that every model is inadequate for every context. Retention by the replacement-conservative rule instead means no shared replacement path excludes that model. These are different claims. Additional computational criticism may remove arrows and enlarge context-specific survivor sets; the observed-only result is not substituted for the full result. No operational tie-break is applied to an empty primary set.

## Numerical and computational checks

- Each saved anchored magnitude and survival is independently recomputed in Python from the retained computational effects and checked against Rust. All planned comparisons must be present and valid before reduction.
- The unmodified library search reproduces the existing publication anchors. Explicit endpoint-cell minimization can lower an anchor: maximum change from the saved parent values is {diag['maximum_parent_anchor_difference']:.3g}. Original parent artifacts are retained unchanged.
- Observed profiles use {p['grid_points']} initial points followed by adaptive interval subdivision; every pair is checked at {2*p['grid_points']-1} points and tenfold tighter tolerance. The same check is applied to {p['refinement_draws']} deterministically spaced computational draws per computed pair/context ({diag['refined_computational_draws']} draws total).
- Largest checked retained-effect change: {diag['maximum_refinement_difference']:.3g}; observed-gate disagreements: {diag['refinement_gate_disagreements']}; computational-floor disagreements: {diag['refinement_floor_disagreements']}.
- A J={p['replications']//2} prefix is compared with J={p['replications']}. Directed-edge disagreements: {diag['prefix_edge_disagreements']}; context-survivor-set disagreements: {diag['prefix_context_set_disagreements']}.
- Candidate-conservative set unchanged across those list sizes: {diag['prefix_candidate_set_unchanged']}; replacement-conservative set unchanged: {diag['prefix_replacement_set_unchanged']}.
- Prefix comparison shares draws and is not an independent-seed replication. Neither numerical refinement nor finite-J agreement proves exactness under the ideal computational law. No statistical confidence interpretation is attached to γ.
- Missing-class redraws summed across pair jobs: {diag['rejected_draws_across_pair_jobs']}; counts repeat when the same assay draws are reused for multiple pairs.

## Saved artifacts

- `plan.json`: policy recorded before computation, cohort, source hashes, numerical implementation hashes, and seed design.
- `reports/`: one resumable JSON checkpoint per assay/unordered pair, including both observed directions, eligible computational effects, and refinement diagnostics.
- `summary.json`: reductions and numerical checks.
- `tables/directed_assessments.csv`: all directed gates, supported magnitudes, survival fractions, and verdicts.
- `tables/context_survivors.csv`: context-index mapping and survivor sets.
- `tables/model_admissibility.csv`: counts and membership under both reductions.
- `figures/`: context admissibility, aggregate comparison, and the computational two-part rule, in PDF and PNG.

Input provenance and the inherited assay dependence/multiplicity records are in the parent study and referenced by hash. NCI is not part of this study.
'''
    if not summary.get('graph_result_is_resolved', True):
        text = ('# Pending numerical resolution\n\nThe survivor sets below are possible sets, not a finalized selection. '
                'See summary.json for unresolved decisions and definite survivor sets.\n\n') + text
    (output/'REPORT.md').write_text(text)


def render(output, plan, summary, directions, contexts, maximal):
    os.environ.setdefault('MPLCONFIGDIR', str(STUDY/'.mplconfig'))
    os.environ.setdefault('MPLBACKEND', 'Agg')
    import matplotlib.pyplot as plt
    from matplotlib.colors import ListedColormap
    import numpy as np
    directory = output/'figures'; directory.mkdir(exist_ok=True)
    plt.rcParams.update({'font.size': 10, 'pdf.fonttype': 42, 'ps.fonttype': 42})
    ids = plan['context_ids']
    def save(fig, stem):
        fig.savefig(directory/f'{stem}.pdf', bbox_inches='tight')
        fig.savefig(directory/f'{stem}.png', dpi=160, bbox_inches='tight')
        plt.close(fig)
    fig, axes = plt.subplots(1, 2, figsize=(9, 16), sharey=True, constrained_layout=True)
    for ax, stage, title in zip(axes, ['observed', 'full'], ['Observed gate only', 'Full anchored assessment']):
        matrix = np.array([[int(m in maximal[stage][e]) for m in MODELS] for e in ids])
        ax.imshow(matrix, aspect='auto', interpolation='nearest', cmap=ListedColormap(['#dddddd', '#0072B2']), vmin=0, vmax=1)
        ax.set_xticks(range(3), [LABELS[m] for m in MODELS], rotation=25, ha='right')
        ax.set_yticks(range(0,len(ids),2), [str(i+1) for i in range(0,len(ids),2)])
        ax.set_title(title)
    axes[0].set_ylabel('Context index (mapping in context_survivors.csv)')
    fig.suptitle('Admissibility within each assay\nBlue: no incoming supported loss; gray: excluded', fontsize=13)
    save(fig, '01-context-admissibility')
    fig, ax = plt.subplots(figsize=(8,4.8), constrained_layout=True)
    x = np.arange(3); width = .34
    for offset, stage, color, label in [(-width/2,'observed','#999999','Observed only'),(width/2,'full','#0072B2','Full assessment')]:
        values = [summary['reductions'][stage]['contexts_admissible'][m] for m in MODELS]
        bars = ax.bar(x+offset, values, width, color=color, label=label); ax.bar_label(bars)
    ax.axhline(len(ids), color='#D55E00', linestyle='--', label='Every context required for primary S₀')
    ax.set_ylim(0,len(ids)+10); ax.set_xticks(x,[LABELS[m] for m in MODELS]); ax.set_ylabel('Contexts in which model remains admissible')
    ax.set_title('Candidate-conservative reduction requires survival everywhere')
    ax.legend(loc='upper left', bbox_to_anchor=(0, .88), frameon=False, fontsize=9)
    save(fig, '02-cross-context-admissibility')
    fig, ax = plt.subplots(figsize=(7,5), constrained_layout=True)
    evaluated = [d for d in directions if d['observed_gate']]
    for m,color in zip(MODELS,['#009E73','#0072B2','#E69F00']):
        rows = [d for d in evaluated if d['candidate']==m]
        ax.scatter([d['supported_magnitude'] for d in rows],[d['literal_survival'] for d in rows],s=20,alpha=.65,color=color,label=LABELS[m])
    ax.axvline(plan['parameters']['delta'],color='black',linestyle='--')
    ax.axhline(plan['parameters']['gamma'],color='black',linestyle='--')
    ax.set(xlabel='Observed-anchored supported magnitude',ylabel='Literal survival above d = 0',ylim=(-.03,1.04),
           title='Both magnitude and survival requirements must pass')
    ax.legend(title='Candidate in oriented claim',frameon=False,fontsize=9)
    save(fig, '03-computational-two-part-rule')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--profile', choices=PROFILES, default='publication')
    parser.add_argument('--output', type=Path)
    parser.add_argument('--example', type=Path, default=ROOT/'target/release/examples/proteingym_contexts')
    parser.add_argument('--seed', type=int, default=20260917)
    parser.add_argument('--replications', type=int)
    parser.add_argument('--render-only', action='store_true')
    args = parser.parse_args()
    args.output = (args.output or STUDY/('outputs' if args.profile=='publication' else 'outputs_quick')).resolve()
    plan,audit,sources = prepare_plan(args)
    if not args.render_only:
        command = [str(args.example.resolve()), '--input', str(sources['scores']), '--output-dir', str(args.output/'reports'), '--run-id',plan['run_id']]
        for key,value in plan['parameters'].items():
            command.extend(['--'+key.replace('_','-'),str(value)])
        print('Policy and provenance saved:',args.output/'plan.json',flush=True)
        subprocess.run(command,check=True)
    reports,difference = load_and_validate(args.output,plan,audit,read_csv(sources['saved_effects']))
    summary,directions,contexts,maximal = summarize(args.output,plan,audit,reports,difference)
    write_report(args.output,plan,summary)
    if summary['graph_result_is_resolved']:
        render(args.output,plan,summary,directions,contexts,maximal)
    artifact_hashes = {str(p.relative_to(args.output)):sha256(p) for p in sorted(args.output.rglob('*')) if p.is_file() and p.name != 'manifest.json' and not p.name.endswith('.tmp')}
    save_json(args.output/'manifest.json',dict(run_id=plan['run_id'],completed_at_utc=datetime.now(timezone.utc).isoformat(),
        python=sys.version,platform=platform.platform(),driver_sha256=sha256(__file__),artifacts_sha256=artifact_hashes))
    print(json.dumps(summary,indent=2),flush=True)

if __name__ == '__main__':
    main()
