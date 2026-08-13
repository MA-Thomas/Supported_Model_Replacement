# Conditional ProteinGym full-nesting illustration

This isolated workflow uses the retained publication artifacts from the parent
[`proteingym/`](../proteingym/) study to show the complete V17 nested
calculation with `M > 1`, `K_E = 2`, and `K_C = 2`.

It is explicitly a pedagogical conditional calculation, not an additional
ProteinGym model-replacement conclusion. The calculation asks what would follow
**if** all of the following were scientifically appropriate:

- ESM-1v ensemble versus ESM-1v single were the replacement comparison;
- 40%–50% deleterious prevalence were the applicable target range;
- residue position were the actionable within-assay resampling unit; and
- the illustrative policy required literal survival greater than `gamma=0.60`.

The observed empirical gate uses `K_E=2`. Within every assay, a computational
replication samples the observed number of residue positions with replacement
and carries every retained substitution at a selected position. A complete
draw lacking either outcome class is redrawn. Both model scores and the entire
prevalence profile use the same cluster multiplicities.

The workflow never writes into the parent banking or ProteinGym output
directories. Its artifacts live only under this directory's `outputs/`.

## Reproduce

First generate the retained parent ProteinGym publication artifacts. Then run:

```sh
sh domain_study/proteingym_nested/run.sh quick
```

For the publication replication count and diagnostic grid:

```sh
sh domain_study/proteingym_nested/run.sh publication
```

To redraw the retained publication artifacts without rerunning Rust:

```sh
sh domain_study/proteingym_nested/run.sh render
```

The study-specific Rust program is an example target rather than a new command
in the main `supported_ap` CLI. It performs residue-position resampling and
passes the resulting multiplicities through the library's exact tie-averaged
AP, continuous prevalence minimization, and observed-anchored nested support.

## Figure sequence

1. **Condition challenge to assay anchors.** Shows each assay's paired CNAP
   profile over 40%–50% and the infimum retained from that complete range.
2. **Observed order-2 gate.** Shows all 4,095 assay-pair minima, the reduction
   from mean assay effect to order-2 support, and literal pair survival.
3. **Observed-anchored full nested result.** Shows the position-bootstrap
   effects within every empirical row, observed versus full survival curves,
   and the two-stage conditional result.

