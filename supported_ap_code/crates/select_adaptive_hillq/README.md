# select_adaptive_hillq

Rust implementation of complete-F, self-gated power-mean-anchor Hill-q L2
aggregation and cross-cohort parameter selection.

This crate replaces the former max-anchored selector. It does not implement a
legacy mode. Its input contract requires a computed Level-1 score for every
declared 9--12-mer/HLA tuple. A `mapping_status` other than `scoreable` is an
error: an omitted tensor value is missing data, not evidence of non-presentation.

## Aggregation rule

For candidate log scores `z_i >= ln(1e-12)`, define the explicitly selected
power-mean anchor

```text
A_alpha = (1/alpha) log(mean(exp(alpha*z_i)))  alpha > 0
A_0     = mean(z_i)
A_inf   = max(z_i).
```

Thus `alpha=0`, `alpha=1`, and `alpha=inf` recover mean, log-mean-exp, and max
on the log-score scale. There is intentionally no default alpha grid.

The floor-aware Level-1 signal is `F_i = max(exp(z_i)-1e-12, 0)`. For positive
total signal, normalize `p_i = F_i/sum(F)` and calculate the Hill number `N_q`.
The corroboration offer is

```text
U             = log(1e-12 + sum(F_i))
D_q           = 1 - 1/N_q
C_{alpha,q}   = D_q * max(U - A_alpha, 0)
S             = A_alpha + C_{alpha,q} * sigmoid((c-S)/kappa).
```

If all signals are zero, the endpoint score is exactly `ln(1e-12)` and the
offer is zero. The implicit equation has one root on
`[A_alpha, A_alpha + C_{alpha,q}]` and is solved by certified bisection.

## Selection contract

Every `(alpha, q, c, kappa)` tuple is evaluated jointly. The all-context policy
minimizes worst-cohort fractional rank first, then mean fractional rank and mean
metric regret. The PDAC-only policy uses the same ordering on PDAC alone; COVID
is transport evidence for that policy.

For PR, eligibility still requires staged paired-CNAP support over the same
component model's fixed-max reference in every cohort used by the policy. The
parameter-selection challenge uses its separately declared finite replication
roster; the downstream candidate-conservative tournament remains a distinct
test. ROC selection uses AUROC ranks with the same worst-cohort-first ordering.

The current omission-floor full-roster results are not valid production inputs.
Rerun the Level-1 F tensors for the complete COVID and PDAC tuple roster, rebuild
the transfers, and re-diagnose the cohort preferences before freezing an alpha
grid or selecting production parameters. The CLI requires `--alpha-values` to
make that non-default explicit.

## Build and test

From `supported_ap_code/`:

```bash
cargo build --release -p select_adaptive_hillq --bins
cargo test -p select_adaptive_hillq
```

Rust 1.85+ is required.

## Inputs

`--source-root` must be a validated Rust transfer package containing
`manifest.json` and the 30 selection-eligible primary tasks:

```text
<pdac|covid_spike|covid_nonspike>/<model>/<pr|roc>/
  long_peptide_predictions.csv
  target_tau_selection_by_observation.csv
  summary.json
```

Each `summary.json` names the corresponding full-roster mapping CSV. After the
9--12-mer length filter, every mapping row must have `mapping_status=scoreable`
and must resolve exactly once to the tau table. Exact duplicate biological
tuples are removed by endpoint + n-mer + normalized HLA; environment and
observation identifiers are provenance, not multiplicity.

`--bundle-root` supplies authoritative endpoint identities, labels, COVID label
contracts, fixed-max reference scores, and the PR tournament contract.
Reconstructed `max` and `logsumexp` baselines are audited before selection.
Secondary transfer views are never added to the worst-cohort objective. After
primary-only parameters are selected, the same frozen tuples are applied to
every manifest-declared secondary view and written under
`secondary_views/<view_id>/`. These files are conditional post-selection
diagnostics and are not read by the production bundle builder.

## Cluster execution

The dense PR grid uses an immutable, restartable lifecycle:

```text
select_adaptive_hillq_cluster plan --alpha-values <explicit orders> ...
select_adaptive_hillq_cluster run-shard ...
select_adaptive_hillq_cluster status ...
select_adaptive_hillq_cluster audit ...
select_adaptive_hillq --alpha-values <same orders> --cnap-plan ... --cnap-results ...
```

Planning deduplicates exact score rankings and tie blocks within each
model/cohort. Result shards are bound to the plan, input hashes, selection
contract, and executable. The final selector audits the complete shard set
before reconstructing surfaces and applying the two policies.

No alpha example here is designated as the production grid. Freeze it only
after complete-F reruns and clean-data diagnostics.

## Principal outputs

- `alpha_grid.csv`
- `q_grid.csv`
- `joint_parameter_grid.csv`
- `selected_parameters.csv`
- `selected_parameters_pdac_only.csv`
- `selected_parameters_all_contexts.csv`
- `selected_endpoint_scores.csv`
- `leave_one_cohort_out.csv`
- `baseline_reproduction.csv`
- `surfaces/<branch>__<model>/*.csv.gz`
- `selection_manifest.json`
- `secondary_views/<view_id>/selected_endpoint_scores.csv`
- `secondary_views/<view_id>/metrics.csv`
- `secondary_views/<view_id>/manifest.json`

The output directory must not already exist. A selected `c` or `kappa` on a
grid boundary fails closed and requests a wider grid.
