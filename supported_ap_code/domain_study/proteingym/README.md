# ProteinGym observed-evaluation study

This study compares released ProteinGym scores without fitting or tuning any
model. Each retained deep-mutational-scanning assay is one observed evaluation
in the V17 observed empirical path.

The three model comparisons are:

1. ESM-1v ensemble versus EVE ensemble;
2. ESM-2 650M versus ESM-1v ensemble;
3. ESM-2 650M versus EVE ensemble.

All three are treated as primary comparisons on the same assay cohort. No
additional model is fitted for any comparison.

Only manual-cutoff assays and single amino-acid substitutions are retained.
Experimentally nonfunctional/deleterious variants are the positive class. The
released fitness-oriented scores are negated so that larger values favor that
class. Every retained row must have the same mutation identifier, label, and
finite predictions from all compared models in the two official archives.

The declared AP assessment uses a 1%-90% deleterious-variant prevalence range,
empirical order `K_E = 2`, magnitude threshold `delta = 0`, survival floor `d = 0`,
and literal distinct-subset survival requirement `gamma = 0.81`. Orders 1, 5,
and 10 are reported as prespecified sensitivity analyses. No label
exchangeability law is asserted.

The observed-only AUROC companion applies the same order-2 magnitude and
literal-survival policy to the 91 paired assay AUROC differences at `Gamma = 1`.
All declared comparisons fail that empirical gate, so the study performs no
within-assay computational resampling and no case-mix breakdown search. This is
the `M = 91, K_C = 0` boundary; it is not presented as a full nested example.

## Reproduce

The study reuses the parent domain-study environment:

```sh
python3 -m venv --system-site-packages domain_study/.venv
domain_study/.venv/bin/python -m pip install -r domain_study/proteingym/requirements.txt
sh domain_study/proteingym/run.sh download
sh domain_study/proteingym/run.sh publication
```

The download is approximately 1.95 GB. Inputs are pinned and verified by
SHA-256 before every analysis. Clean runs replace
`domain_study/proteingym/outputs/`.

To redraw retained publication artifacts without reading the raw archives or
rerunning Rust:

```sh
sh domain_study/proteingym/run.sh render
```

To migrate or refresh the Rust reports from the retained aligned score table
without rereading the source archives:

```sh
sh domain_study/proteingym/run.sh rust
```

To add or refresh only the observed AUROC companion while retaining the AP
reports and aligned score table:

```sh
sh domain_study/proteingym/run.sh auroc
```

Python performs archive validation, row alignment, orchestration, descriptive
assay-level AUROC extraction, and rendering. The V17 Rust CLI performs AP,
CNAP, prevalence minimization, observed AUROC support, literal survival, and the
replacement verdict.
