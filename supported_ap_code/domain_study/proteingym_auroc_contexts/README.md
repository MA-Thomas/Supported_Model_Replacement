# ProteinGym single-context AUROC study

This is the ordinary-AUROC companion to `../proteingym_contexts/`. Each of
91 assays receives its own observed gate and, for passing directions, an
observed-anchored computational assessment. Candidate-conservative graph
reduction is primary; replacement-conservative reduction is also reported.
There is no joint empirical aggregation across assays.

The study uses EVE ensemble, ESM-1v ensemble and ESM-2 650M, with the same
aligned observations and position-cluster draws as the CNAP companion:
J=200, K_C=2, δ=d=0, γ=0.81, seed 20260917. A J=100 prefix provides a
finite-list stability check. Both requirements remain conjunctive during
the computational challenge.

AUROC uses half credit for ties at Γ=1. It has no prevalence parameter, and
this study does not perform adversarial within-class concentration searches.
The matched comparison therefore concerns ordinary AUROC versus CNAP over
1%–90% prevalence, under identical computational draws and decision thresholds.

Run from the repository root:

```sh
RAYON_NUM_THREADS=8 sh supported_ap_code/domain_study/proteingym_auroc_contexts/run.sh publication
```

This builds the separate Rust example offline and uses the existing
`domain_study/.venv` Python environment. No downloads or model fitting are
required. `quick` uses 20 draws and writes `outputs_quick/`; `render` validates
and redraws completed publication results. A different `--seed`,
`--replications`, or implementation requires a new `--output` directory.
An identical run resumes its checkpoints. The matched comparison is emitted
only when both studies' computational settings agree.

`outputs/REPORT.md` contains results and their interpretation;
`outputs/cnap_comparison.json` and `outputs/tables/cnap_auroc_pair_counts.csv`
contain the matched comparison. All 546 directed assessments, their eligible
computational lists, context survivors, figures, policy and artifact hashes
are saved. Existing CNAP, joint empirical studies, and manuscript files are
separate.

Validation combines exhaustive weighted/tied synthetic AUROC checks,
comparison with the library and saved parent effects, expanded-rank checks
on observed data and selected draws, independent finite-subset summaries,
graph invariants, and end-to-end row-order/thread/resume tests:

```sh
cargo test --manifest-path supported_ap_code/Cargo.toml --example proteingym_auroc_contexts --offline
cd supported_ap_code
domain_study/.venv/bin/python -B -m unittest domain_study.proteingym_auroc_contexts.test_auroc_story domain_study.proteingym_contexts.test_context_story -v
```

Results are conditional on the finite assays and declared computational
design. Computational resampling adds no empirical evidence about unseen
populations; γ is not a confidence level. An empty candidate-conservative set
does not negate improvements supported in particular contexts.
