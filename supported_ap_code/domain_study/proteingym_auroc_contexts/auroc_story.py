#!/usr/bin/env python3
"""Separate single-context AUROC assessments and matched CNAP comparison."""
from __future__ import annotations
import argparse
from datetime import datetime, timezone
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import subprocess
import sys

STUDY = Path(__file__).resolve().parent
ROOT = STUDY.parents[1]
sys.path.insert(0, str(ROOT))
from domain_study.proteingym_contexts.context_story import (
    MODELS, LABELS, PAIRS, SLUGS, PARENT, anchored_summary, reduce_graphs,
    read_csv, save_csv, save_json, sha256, names, render,
)


def validate_cnap_reference(cnap_root):
    cnap = json.loads((cnap_root/'plan.json').read_text())
    manifest = json.loads((cnap_root/'manifest.json').read_text())
    summary = json.loads((cnap_root/'summary.json').read_text())
    if manifest['run_id'] != cnap['run_id'] or summary['run_id'] != cnap['run_id']:
        raise ValueError('CNAP plan, summary and manifest identities differ')
    for name in ['plan.json', 'summary.json', 'tables/directed_assessments.csv']:
        if sha256(cnap_root/name) != manifest['artifacts_sha256'].get(name):
            raise ValueError('CNAP artifact hash mismatch: ' + name)
    if not summary.get('graph_result_is_resolved', True):
        raise ValueError('matched CNAP comparison has unresolved decisions')
    return cnap


def prepare_plan(args):
    cnap_root = args.cnap_output.resolve()
    sources = {'scores': PARENT/'predictions/proteingym_observed_scores.csv',
               'audit': PARENT/'tables/assay_audit.csv', 'parent_manifest': PARENT/'manifest.json',
               'saved_effects': PARENT/'tables/observed_auroc_assay_effects.csv',
               'cnap_plan': cnap_root/'plan.json', 'cnap_summary': cnap_root/'summary.json',
               'cnap_directions': cnap_root/'tables/directed_assessments.csv'}
    audit = read_csv(sources['audit']); ids = sorted(x['assay_id'] for x in audit)
    parent = json.loads(sources['parent_manifest'].read_text())
    cnap = validate_cnap_reference(cnap_root)
    if parent['profile'] != 'publication' or len(ids) != 91 or len(set(ids)) != 91:
        raise ValueError('requires the retained publication cohort')
    if ids != cnap['context_ids'] or MODELS != cnap['models']:
        raise ValueError('CNAP cohort/roster mismatch')
    for key in ['scores', 'audit']:
        if sha256(sources[key]) != cnap['sources'][key]['sha256']:
            raise ValueError('CNAP aligned inputs changed')
    # Verify identical parsing, canonical row/cluster order, RNG and redraw logic.
    old = (ROOT/'examples/proteingym_contexts.rs').read_text()
    new = (ROOT/'examples/proteingym_auroc_contexts.rs').read_text()
    for source in ['examples/proteingym_contexts.rs', 'Cargo.lock']:
        if sha256(ROOT/source) != cnap['numerical_source_sha256'][source]:
            raise ValueError('saved CNAP sampler or dependency versions changed')
    for start, old_end, new_end in [('fn read_contexts(', 'fn policy(', 'fn gate('),
                                    ('fn stable_seed(', '#[cfg(test)]', '#[cfg(test)]')]:
        if old[old.index(start):old.index(old_end)] != new[new.index(start):new.index(new_end)]:
            raise ValueError('AUROC and CNAP context resampling implementations differ')
    p = dict(replications=args.replications if args.replications is not None else (200 if args.profile=='publication' else 20),
             seed=args.seed, computational_order=2, delta=0.0, floor=0.0, gamma=0.81, check_draws=8)
    if p['replications'] < 4:
        raise ValueError('full and prefix lists require at least two computational draws')
    matched = all(p[k] == cnap['parameters'][k] for k in ['replications','seed','computational_order','delta','floor','gamma'])
    implementation = [ROOT/'examples/proteingym_auroc_contexts.rs', Path(__file__),
        STUDY.parent/'proteingym_contexts/context_story.py', ROOT/'Cargo.toml', ROOT/'Cargo.lock'] + sorted((ROOT/'src').rglob('*.rs'))
    core = dict(schema_version=1, analysis='proteingym_single_context_anchored_auroc_graphs',
        metric='ordinary paired AUROC with half credit for score ties', concentration_factor=1.0,
        concentration_scope='Gamma=1 only; no adversarial within-class concentration search',
        profile=args.profile, parameters=p, models=MODELS, context_ids=ids,
        primary_reduction='candidate_conservative', comparison_reduction='replacement_conservative',
        empirical_aggregation=None, resampling=cnap['resampling'], matched_cnap_design=matched,
        sources={k:dict(path=os.path.relpath(v, ROOT),sha256=sha256(v)) for k,v in sources.items()},
        numerical_source_sha256={str(v.relative_to(ROOT)):sha256(v) for v in implementation},
        interpretation='Retrospective illustration conditional on the represented assays and the declared computational design; no unseen-population, confidence, or deployment guarantee.',
        numerical_checks=dict(observed='all pairs checked against saved ordinary AUROC differences and expanded average ranks',
            computational='eight deterministically spaced draws per passing pair checked using expanded average ranks',
            strict_zero='integer twice-concordance differences before floating division',
            replication_count='J/2 prefix versus full finite list; not an independent seed'))
    plan = dict(core,run_id=hashlib.sha256(json.dumps(core,sort_keys=True).encode()).hexdigest())
    path = args.output/'plan.json'
    if path.exists():
        if json.loads(path.read_text())['run_id'] != plan['run_id']:
            raise ValueError('different inputs/configuration/implementation: use a new --output')
    elif args.render_only:
        raise ValueError('render requires a saved completed run')
    else:
        save_json(path,dict(plan,declared_at_utc=datetime.now(timezone.utc).isoformat()))
    return plan,audit,sources


def validate(output,plan,audit,parent_effects):
    ids=plan['context_ids']; p=plan['parameters']
    expected={f'context-{i:03}-pair-{j}.json' for i in range(len(ids)) for j in range(3)}
    if {f.name for f in (output/'reports').glob('*.json')} != expected:
        raise ValueError('incomplete or unexpected checkpoint set')
    parents={(r['assay_id'],r['comparison']):float(r['paired_auroc_difference']) for r in parent_effects}
    cohort={r['assay_id']:r for r in audit}; reports=[]; max_parent=0.0
    for i,e in enumerate(ids):
        for j,pair in enumerate(PAIRS):
            r=json.loads((output/'reports'/f'context-{i:03}-pair-{j}.json').read_text())
            if (r['run_id'],r['context_id'],r['pair_index']) != (plan['run_id'],e,j):
                raise ValueError('checkpoint identity mismatch')
            if r['observation_count']!=int(cohort[e]['single_variant_count']) or r['positive_count']!=int(cohort[e]['deleterious_count']):
                raise ValueError('cohort changed')
            for key,orientation,sign in [('forward',pair,1),('reverse',pair[::-1],-1)]:
                d=r[key]; anchor=d['observed']['value']
                if (d['candidate'],d['incumbent'])!=orientation or not math.isfinite(anchor):
                    raise ValueError('invalid direction or effect')
                max_parent=max(max_parent,abs(anchor-sign*parents[e,SLUGS[j]]))
                gate=anchor>max(p['delta'],p['floor'])
                if d['observed_gate']!=dict(magnitude=anchor,survival=float(anchor>p['floor']),supported=gate):
                    raise ValueError('invalid observed gate')
                if gate:
                    values=[x['value'] for x in d['computational']]
                    if len(values)!=p['replications'] or any(not math.isfinite(x) or abs(x)>1 for x in values):
                        raise ValueError('invalid computational list')
                    for stage,effects in [('full',values),('prefix',values[:p['replications']//2])]:
                        independent=anchored_summary(anchor,effects,p['computational_order'],p['floor'],p['delta'],p['gamma'])
                        if any(abs(independent[k]-d[stage][k])>1e-12 for k in ['magnitude','survival']) or independent['supported']!=d[stage]['supported']:
                            raise ValueError('anchored summary disagrees with independent subset formula')
                    if d['final_supported']!=d['full']['supported']:
                        raise ValueError('final verdict mismatch')
                elif d['computational'] or d['full'] is not None or d['prefix'] is not None or d['final_supported']:
                    raise ValueError('failed observed gate must stop')
            if r['forward']['observed']['value'] != -r['reverse']['observed']['value']:
                raise ValueError('ordinary AUROC orientation identity failed')
            if r['maximum_check_difference']>1e-12 or r['check_gate_disagreements'] or r['check_floor_disagreements']:
                raise ValueError('independent AUROC check failed')
            reports.append(r)
    if max_parent>1e-12: raise ValueError('saved parent AUROC differs')
    return reports,max_parent


def summarize(output,plan,audit,reports,parent_difference):
    ids=plan['context_ids']; graphs={s:{e:set() for e in ids} for s in ['observed','prefix','full']}
    directions=[]
    for r in reports:
        for key in ['forward','reverse']:
            d=r[key]; f=d['full']; p=d['prefix']; edge=(d['candidate'],d['incumbent'])
            if d['observed_gate']['supported']: graphs['observed'][r['context_id']].add(edge)
            if p and p['supported']: graphs['prefix'][r['context_id']].add(edge)
            if d['final_supported']: graphs['full'][r['context_id']].add(edge)
            directions.append(dict(context_id=r['context_id'],candidate=edge[0],incumbent=edge[1],
                observed_effect=d['observed']['value'],observed_gate=d['observed_gate']['supported'],
                replications=len(d['computational']),supported_magnitude=f['magnitude'] if f else '',
                literal_survival=f['survival'] if f else '',full_supported=d['final_supported'],
                prefix_magnitude=p['magnitude'] if p else '',prefix_survival=p['survival'] if p else '',
                prefix_supported=p['supported'] if p else False))
    reductions={}; maximal={}
    for stage in graphs:
        reductions[stage],maximal[stage]=reduce_graphs(MODELS,graphs[stage])
        reductions[stage]['supported_edges']=sum(map(len,graphs[stage].values()))
    for e in ids:
        if not graphs['full'][e]<=graphs['observed'][e] or not maximal['observed'][e]<=maximal['full'][e]:
            raise ValueError('anchoring invariant failed')
    by_id={r['assay_id']:r for r in audit}
    contexts=[dict(context_index=i+1,context_id=e,selection_type=by_id[e]['selection_type'],
        units=int(by_id[e]['single_variant_count']),observed_survivors=';'.join(sorted(maximal['observed'][e])),
        full_survivors=';'.join(sorted(maximal['full'][e])),observed_edges=len(graphs['observed'][e]),
        full_edges=len(graphs['full'][e])) for i,e in enumerate(ids)]
    diagnostics=dict(maximum_parent_anchor_difference=parent_difference,
        maximum_independent_auc_difference=max(r['maximum_check_difference'] for r in reports),
        independently_checked_draws=sum(r['independently_checked_draws'] for r in reports),
        check_gate_disagreements=sum(r['check_gate_disagreements'] for r in reports),
        check_floor_disagreements=sum(r['check_floor_disagreements'] for r in reports),
        prefix_edge_disagreements=sum(len(graphs['prefix'][e]^graphs['full'][e]) for e in ids),
        prefix_context_set_disagreements=sum(maximal['prefix'][e]!=maximal['full'][e] for e in ids),
        prefix_candidate_set_unchanged=reductions['prefix']['candidate_conservative']==reductions['full']['candidate_conservative'],
        prefix_replacement_set_unchanged=reductions['prefix']['replacement_conservative']==reductions['full']['replacement_conservative'],
        rejected_draws_across_pair_jobs=sum(r['rejected_missing_class_draws'] for r in reports))
    summary=dict(schema_version=1,run_id=plan['run_id'],policy=plan['parameters'],concentration_factor=1,
        context_count=len(ids),model_count=3,directed_assessments=len(directions),
        computational_direction_assessments=sum(d['observed_gate'] for d in directions),
        reductions=reductions,diagnostics=diagnostics,interpretation=plan['interpretation'])
    save_csv(output/'tables/directed_assessments.csv',directions)
    save_csv(output/'tables/context_survivors.csv',contexts)
    save_csv(output/'tables/model_admissibility.csv',[dict(stage=s,model=m,
        contexts_admissible=reductions[s]['contexts_admissible'][m],contexts_total=len(ids),
        candidate_conservative=m in reductions[s]['candidate_conservative'],
        replacement_conservative=m in reductions[s]['replacement_conservative']) for s in graphs for m in MODELS])
    save_json(output/'summary.json',summary)
    return summary,directions,contexts,maximal


def compare_cnap(output,plan,summary,directions):
    if not plan['matched_cnap_design']: return None
    cnap_summary=json.loads((ROOT/plan['sources']['cnap_summary']['path']).read_text())
    cnap={(r['context_id'],r['candidate'],r['incumbent']):r for r in read_csv(ROOT/plan['sources']['cnap_directions']['path'])}
    keys={(r['context_id'],r['candidate'],r['incumbent']) for r in directions}
    if keys!=set(cnap): raise ValueError('matched comparison requires identical directed roster')
    rows=[]
    for r in directions:
        c=cnap[r['context_id'],r['candidate'],r['incumbent']]
        rows.append(dict(context_id=r['context_id'],candidate=r['candidate'],incumbent=r['incumbent'],
            cnap_supported=c['full_supported']=='True',auroc_supported=r['full_supported'],
            cnap_magnitude=c['supported_magnitude'],auroc_magnitude=r['supported_magnitude'],
            cnap_survival=c['literal_survival'],auroc_survival=r['literal_survival']))
    counts=[]
    for a in MODELS:
        for b in MODELS:
            if a==b: continue
            group=[r for r in rows if r['candidate']==a and r['incumbent']==b]
            counts.append(dict(candidate=a,incumbent=b,
                cnap_supported=sum(r['cnap_supported'] for r in group),auroc_supported=sum(r['auroc_supported'] for r in group),
                both_supported=sum(r['cnap_supported'] and r['auroc_supported'] for r in group),
                cnap_only=sum(r['cnap_supported'] and not r['auroc_supported'] for r in group),
                auroc_only=sum(r['auroc_supported'] and not r['cnap_supported'] for r in group)))
    comparison=dict(cnap_run_id=cnap_summary['run_id'],auroc_run_id=plan['run_id'],same_contexts_models_draws_thresholds=True,
        scope='CNAP minimizes paired effects over prevalence 0.01–0.90; AUROC is ordinary Gamma=1 with no concentration search.',
        cnap_reductions=cnap_summary['reductions']['full'],auroc_reductions=summary['reductions']['full'],
        directed_counts=counts,both_supported=sum(r['cnap_supported'] and r['auroc_supported'] for r in rows),
        cnap_only=sum(r['cnap_supported'] and not r['auroc_supported'] for r in rows),
        auroc_only=sum(r['auroc_supported'] and not r['cnap_supported'] for r in rows))
    save_csv(output/'tables/cnap_auroc_directed_comparison.csv',rows)
    save_csv(output/'tables/cnap_auroc_pair_counts.csv',counts)
    save_json(output/'cnap_comparison.json',comparison)
    return comparison


def write_report(output,plan,summary,comparison):
    p=plan['parameters']; red=summary['reductions']; diag=summary['diagnostics']
    table='\n'.join(f"| {LABELS[m]} | {red['observed']['contexts_admissible'][m]} | {red['full']['contexts_admissible'][m]} |" for m in MODELS)
    paired=''
    if comparison:
        pairtable='\n'.join(f"| {LABELS[r['candidate']]} → {LABELS[r['incumbent']]} | {r['cnap_supported']} | {r['auroc_supported']} | {r['both_supported']} |" for r in comparison['directed_counts'])
        paired=f'''
## Matched CNAP–AUROC comparison

Both studies use identical contexts, models, labels, scores, position draws, J, K_C, δ, d, and γ. CNAP retains the minimum paired effect across 1%–90% prevalence; ordinary AUROC has no prevalence parameter. This AUROC run uses Γ=1 only and makes no claim about survival under additional adversarial case-mix concentration. The studies therefore compare two declared metric assessments, not identical metric uncertainty sets. The previous joint empirical AUROC summaries are not used as these verdicts.

| Supported context-specific replacement | CNAP | AUROC | Both |
|---|---:|---:|---:|
{pairtable}

Across all directed assessments: {comparison['both_supported']} pass both, {comparison['cnap_only']} pass only CNAP, and {comparison['auroc_only']} pass only AUROC. Counts describe context-specific claims; they are not context weights or a model ranking.

CNAP candidate-conservative set: **{names(comparison['cnap_reductions']['candidate_conservative'])}**. AUROC candidate-conservative set: **{names(comparison['auroc_reductions']['candidate_conservative'])}**. A claim of no general supported replacement must be distinguished from the supported improvements found in particular contexts. EVE is itself a deep generative evolutionary model; this comparison concerns the two protein language models versus EVE, not deep learning versus a non-deep-learning baseline.
'''
    (output/'REPORT.md').write_text(f'''# ProteinGym: single-context AUROC and conservative replacement

Each of the 91 retained assays is assessed separately. The three models are EVE ensemble, ESM-1v ensemble, and ESM-2 650M. This run uses the existing aligned observations and fits no models. The manuscript has not been updated.

## Policy

- Ordinary paired AUROC, with half credit for score ties, at concentration factor Γ=1. There is no concentration search.
- δ={p['delta']:g}, d={p['floor']:g}, γ={p['gamma']:g}, K_C={p['computational_order']}, J={p['replications']}; seed {p['seed']}.
- The observed gate requires z_obs>δ and 1{{z_obs>d}}>γ. Passing directions undergo an observed-anchored computational challenge requiring both supported magnitude>δ and literal survival>γ. Failed observed directions stop.
- Each draw samples the observed number of residue positions with replacement, carrying all retained substitutions at each selected position. Draws missing either class are redrawn completely, at most 10,000 attempts. Canonical ordering and context-specific seeds give identical draws across pairs, directions, and the CNAP companion.
- Each challenge is the minimum of its observed anchor and K_C distinct computational effects. Magnitude averages these finite challenges; survival is their fraction strictly above d. There is no empirical aggregation across assays. The library's one-row/order-one identity implements each single-context assessment.
- Primary candidate-conservative reduction intersects the context-specific admissible sets. Replacement-conservative reduction intersects the directed edges first. All graphs are verified acyclic; admissible models have no incoming supported replacement claim.

## Results

| Model | Admissible contexts, observed gate | Admissible contexts, full assessment |
|---|---:|---:|
{table}

- Full candidate-conservative set S₀: **{names(red['full']['candidate_conservative'])}**.
- Full replacement-conservative set: **{names(red['full']['replacement_conservative'])}**.
- {summary['directed_assessments']} directions assessed; {summary['computational_direction_assessments']} pass the observed gate and receive computational assessments.
- Supported arrows: {red['observed']['supported_edges']} observed; {red['full']['supported_edges']} after the anchored computational challenge.

Admissibility counts are not wins or performance scores. An empty candidate-conservative set means no model avoids a supported loss in every declared context. It does not mean each model fails everywhere. A replacement-conservative survivor has no incoming claim shared by every context; its survival does not imply universal superiority. No tie-break is applied to an empty primary set.
{paired}
## Validation and interpretation

All 273 pair/context checkpoints must be complete before reporting reductions. All observed differences reproduce the saved ordinary AUROC values within {diag['maximum_parent_anchor_difference']:.3g}. Production AUROC uses integer twice-concordance counts before division, preserving exact zero effects. Every observed pair and {diag['independently_checked_draws']} computational draws are independently checked by expanded average ranks; maximum difference {diag['maximum_independent_auc_difference']:.3g}. Every anchored magnitude and survival is independently recomputed in Python using the finite-subset formula.

The J={p['replications']//2} prefix versus J={p['replications']} changes {diag['prefix_edge_disagreements']} directed edges and {diag['prefix_context_set_disagreements']} context survivor sets. Candidate-conservative set unchanged: {diag['prefix_candidate_set_unchanged']}; replacement-conservative set unchanged: {diag['prefix_replacement_set_unchanged']}. This shares draws and is not an independent-seed replication. Missing-class redraws summed across pair jobs: {diag['rejected_draws_across_pair_jobs']} (shared draws can be counted more than once).

This is a retrospective illustration conditional on the finite assays and the declared computational design. Resampling probes dependence on represented positions; it supplies computational criticism, not new empirical evidence. It does not establish independent sampling of variants, assays, proteins, or publications from an unseen population. γ is a survival requirement, not a confidence level. Numerical checks and agreement between finite draw counts do not establish an exact ideal-law verdict. No population or deployment guarantee, multiplicity-adjusted inferential claim, or general ranking of model families is asserted.

## Artifacts

`plan.json` records policy, cohort, implementation and input hashes before computation. `reports/` retains observed effects, eligible computational effects, both orientations and independent checks. `summary.json`, `tables/` and `figures/` give graph reductions and diagnostic plots. `cnap_comparison.json` and the comparison tables retain the matched metric comparison when the computational settings agree. `manifest.json` hashes all saved artifacts. Original CNAP and joint empirical AUROC outputs remain separate.
''')


def render_comparison(output,comparison):
    if not comparison: return
    import matplotlib.pyplot as plt
    import numpy as np
    fig,ax=plt.subplots(figsize=(9,5),constrained_layout=True)
    x=np.arange(3); width=.34
    for offset,metric,color in [(-width/2,'cnap','#0072B2'),(width/2,'auroc','#009E73')]:
        values=[comparison[f'{metric}_reductions']['contexts_admissible'][m] for m in MODELS]
        bars=ax.bar(x+offset,values,width,label='CNAP (1%–90% prevalence)' if metric=='cnap' else 'AUROC (Γ = 1)',color=color)
        ax.bar_label(bars)
    ax.axhline(91,color='#D55E00',linestyle='--',label='All 91 required for candidate-conservative set')
    ax.set(ylim=(0,104),ylabel='Contexts with no incoming supported loss',
           title='Full single-context assessments: matched resampling and thresholds')
    ax.set_xticks(x,[LABELS[m] for m in MODELS]); ax.legend(loc='upper left',bbox_to_anchor=(0,.87),frameon=False,fontsize=9)
    for ext in ['png','pdf']: fig.savefig(output/f'figures/04-cnap-auroc-comparison.{ext}',dpi=160,bbox_inches='tight')
    plt.close(fig)


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cnap-output', type=Path, required=True, help='Completed CNAP run to match, including its provenance manifest')
    parser.add_argument('--profile',choices=['publication','quick'],default='publication')
    parser.add_argument('--output',type=Path)
    parser.add_argument('--example',type=Path,default=ROOT/'target/release/examples/proteingym_auroc_contexts')
    parser.add_argument('--seed',type=int,default=20260917)
    parser.add_argument('--replications',type=int)
    parser.add_argument('--render-only',action='store_true')
    args=parser.parse_args()
    args.output=(args.output or STUDY/('outputs' if args.profile=='publication' else 'outputs_quick')).resolve()
    plan,audit,sources=prepare_plan(args)
    if not args.render_only:
        cmd=[str(args.example.resolve()),'--input',str(sources['scores']),'--output-dir',str(args.output/'reports'),'--run-id',plan['run_id']]
        for k,v in plan['parameters'].items(): cmd.extend(['--'+k.replace('_','-'),str(v)])
        print('Policy and provenance saved:',args.output/'plan.json',flush=True)
        subprocess.run(cmd,check=True)
    reports,difference=validate(args.output,plan,audit,read_csv(sources['saved_effects']))
    summary,directions,contexts,maximal=summarize(args.output,plan,audit,reports,difference)
    comparison=compare_cnap(args.output,plan,summary,directions)
    write_report(args.output,plan,summary,comparison)
    os.environ.setdefault('MPLCONFIGDIR',str(STUDY/'.mplconfig'))
    render(args.output,plan,summary,directions,contexts,maximal)
    render_comparison(args.output,comparison)
    hashes={str(f.relative_to(args.output)):sha256(f) for f in sorted(args.output.rglob('*')) if f.is_file() and f.name!='manifest.json' and not f.name.endswith('.tmp')}
    save_json(args.output/'manifest.json',dict(run_id=plan['run_id'],completed_at_utc=datetime.now(timezone.utc).isoformat(),
        python=sys.version,platform=platform.platform(),artifacts_sha256=hashes))
    print(json.dumps(summary,indent=2),flush=True)

if __name__=='__main__': main()
