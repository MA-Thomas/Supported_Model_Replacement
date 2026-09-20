# ProteinGym replay with adaptive defaults — September 18, 2026

Repeated both completed September 17 tournaments using the intended defaults of the updated Rust search. CNAP now starts with three prevalence points and adaptively subdivides, with an objective-gap tolerance of 1e-8 and a budget of 128 shared bisections. The search CLI options were omitted so the runner used the library defaults. The input data, 91 assays, three models, seed (20260917), 200 position-bootstrap draws, prevalence interval, support order, and decision thresholds were preserved.

## Runtime

| Metric | Yesterday, estimated compute seconds | Adaptive run, measured seconds | Yesterday / adaptive |
| --- | ---: | ---: | ---: |
| CNAP | 270.129 | 136.360 | 1.981 |
| AUROC | 1.866 | 1.855 | 1.006 |

The new runs executed sequentially, with no forced thread limit; both used the automatic Rayon pool. Elapsed time was measured separately for each process and excludes compilation, post-run validation, and plot rendering. Yesterday's estimates use plan creation and last report timestamps; its thread count and background machine load were not recorded. These are observed run-time comparisons, not a controlled estimate of algorithm-only speedup.

## Decisions

| Metric | Supported final edges: yesterday → adaptive | Changed/unresolved assessments across stages | Unresolved final decisions |
| --- | ---: | ---: | ---: |
| CNAP | 125 → 125 | 0 | 0 |
| AUROC | 181 → 181 | 0 | 0 |

| Metric | Model | Admissible assays: yesterday → adaptive |
| --- | --- | ---: |
| CNAP | EVE | 56 → 56 |
| CNAP | ESM-1v | 61 → 61 |
| CNAP | ESM-2 650M | 69 → 69 |
| AUROC | EVE | 41 → 41 |
| AUROC | ESM-1v | 49 → 49 |
| AUROC | ESM-2 650M | 54 → 54 |

CNAP final assay survivor-set changes: 0. Candidate-conservative survivors: []; replacement-conservative survivors: ['ESM-1v', 'ESM-2 650M', 'EVE']


AUROC final assay survivor-set changes: 0. Candidate-conservative survivors: []; replacement-conservative survivors: ['ESM-1v', 'ESM-2 650M', 'EVE']

## Numerical differences

| Metric | Maximum observed effect change | Maximum bootstrap effect change | Maximum final magnitude change | Maximum final survival change |
| --- | ---: | ---: | ---: | ---: |
| CNAP | 5.38697911287e-06 | 0.000106338152285 | 3.78849682944e-06 | 8.881784197e-16 |
| AUROC | 0 | 0 | 0 | 0 |

Of 44,946 retained CNAP search certificates, 761 exhausted their numerical search budget. Budget-limited effects carry bounds through to the final policy, which can still resolve a decision if both bounds agree. Maximum difference between the newly sampled effects and yesterday's values: 1.68191747004e-07.

## Verification

Both processes exited successfully. All 546 report files were validated; bootstrap summaries were independently recomputed and tournament graphs reconstructed. Inputs, original numerical outputs, and the executed source/binary hashes were verified again after completion. Original result directories and the earlier 257-point replay are preserved. Machine-readable results, timings, individual decision/value comparisons, commands, logs, and raw reports accompany this report.
