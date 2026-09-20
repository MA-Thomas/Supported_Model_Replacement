# ProteinGym tournament replay — September 18, 2026

Repeated both completed September 17 tournaments using the updated Rust code. The 91 assays, three models, input data, seed (20260917), and all saved numerical parameters were preserved. Each tournament contains 273 pair comparisons (546 directions), with 200 position-cluster bootstrap replications for each direction that passes the observed gate.

| Metric | Supported final edges: previous → replay | Changed decisions across all stages | Unresolved final decisions |
| --- | ---: | ---: | ---: |
| CNAP | 125 → 125 | 0 | 0 |
| AUROC | 181 → 181 | 0 | 0 |

## Model outcomes

| Metric | Model | Admissible assays: previous → replay |
| --- | --- | ---: |
| CNAP | EVE | 56 → 56 |
| CNAP | ESM-1v | 61 → 61 |
| CNAP | ESM-2 650M | 69 → 69 |
| AUROC | EVE | 41 → 41 |
| AUROC | ESM-1v | 49 → 49 |
| AUROC | ESM-2 650M | 54 → 54 |

For both metrics, the candidate-conservative set remains empty, and the replacement-conservative set retains all three models. No single model is admissible in every assay; no model can be removed by one common supported replacement across every assay.

## Numerical differences

AUROC observed effects, bootstrap effects, and observed/prefix/full summaries are exactly unchanged.

- CNAP maximum absolute change in observed retained effect: 2.43044434717e-08.
- CNAP maximum absolute change in individual bootstrap retained effect: 4.43120929438e-06.
- CNAP maximum absolute change in sampled effect (observed and bootstrap): 1.4485213029e-09.
- CNAP maximum absolute change in final supported magnitude: 4.43273485418e-08.
- CNAP maximum absolute change in final literal survival: 8.881784197e-16.

CNAP now reports conservative lower bounds and numerical uncertainty. The replay retained the old 257 initial grid points, tolerance 1e-8, and 128 iterations; under the updated algorithm the tolerance bounds objective error and iterations budget shared bisections. This is a replay under the new numerical semantics, not a timing benchmark using the new three-point default.

Of 44,946 recorded CNAP effect certificates, 151 stopped at the search budget. Their bounds were propagated into the decisions; 0 final directional decisions remain unresolved.

## Verification and files

Both Rust processes exited successfully. The replay validated all 546 report files, independently recomputed the bootstrap summaries, and reconstructed both the baseline and replay tournament graphs. Input hashes, original numerical artifact hashes, and replay source/binary hashes were rechecked after completion. The CNAP example's uncertainty-aware summary changes passed its four tests, the AUROC example's four tests, and strict Clippy for both examples.

Original result directories were preserved. Each metric subdirectory contains its plan, execution record, raw reports, comparison.json, decision_changes.csv, numeric_changes.csv, and context_survivors.csv. The root manifest records SHA-256 hashes for the replay artifacts.
