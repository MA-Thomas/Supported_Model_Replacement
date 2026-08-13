# Bank Marketing story study

This reproducible study is organized around eight questions that a reader can
understand before learning the supported-AP framework:

1. Why does traditional AP change when outcome prevalence changes?
2. Can matched real-data evaluations have different replication support?
3. Is zero the chance center of supported CNAP across real score designs?
4. Does calibration restore a common meaning for zero in those designs?
5. Where do these distinctions matter in a real rare-outcome prediction task?
6. Does a higher model score support a reproducible model-replacement claim?
7. How does evaluation information change that claim without changing models?
8. Does the supported replacement claim transport across target prevalences?

The primary data are the chronological `bank-additional-full.csv` records from
the UCI Bank Marketing dataset:

- S. Moro, P. Rita, and P. Cortez, *Bank Marketing*, UCI Machine Learning
  Repository, 2014. <https://doi.org/10.24432/C5K306>
- Source page: <https://archive.ics.uci.edu/dataset/222/bank%2B>
- License: CC BY 4.0.

The first 80% of the date-ordered rows train three fixed model specifications:
a full one-hot logistic regression, a full one-hot random forest, and a
demographic-only logistic regression using age, job, marital status, and
education. The final 20% supply an untouched temporal evaluation set and the
empirical class-conditional score pools used in controlled resampling
experiments. `duration` is excluded because a pre-contact targeting decision
cannot know the length of the future call.

The training-period prevalence is the reference prevalence for every experiment.
All CLI-reported supported and calibrated values are produced by the compiled
Rust CLI. Figure 2 uses one retained Python bootstrap stream for both its
displayed distributions and their summaries.

## Figures

- `01-prevalence-transport`: changing prevalence moves traditional AP while
  prior-standardized AP remains on a fixed population scale.
- `02-same-performance-different-support`: matched held-out Bank Marketing
  samples can conceal materially different empirical replication distributions.
- `03-design-specific-chance`: across real logistic-regression score samples,
  the raw supported-CNAP chance center depends on positive count and resolution.
- `04-calibration-restores-zero`: label permutations within those held-out
  score sets show that conditional-null calibration re-anchors chance to zero.
- `05-real-data-sparse-outcomes`: the reporting sequence and its total
  adjustment show where support and calibration matter most in practice.
- `06-supported-model-superiority`: each observed paired advantage is partitioned
  into the share retained as supported superiority and the share discounted by
  the replication-support adjustment.
- `07-evidence-for-model-superiority`: connected observed and supported estimates
  show directly how evaluation information changes the strength of a
  model-replacement claim while both fitted models remain fixed.
- `08-target-prevalence-model-superiority`: a paired prevalence sweep shows
  whether the random-forest replacement claim remains supported in populations
  with different outcome prevalences.

Every figure uses held-out Bank Marketing predictions. Figures 1–5 use the full
logistic model; Figure 2 selects matched real-data subsamples to isolate
replication support, and Figures 3–4 use controlled score quantization and label
permutation within real held-out samples. Figures 6–8 use paired predictions
from the fixed comparison models; Figure 8 changes only the declared
target-population prevalence.

## Run

From the crate directory:

```bash
sh scripts/run-bank-marketing-study.sh publication
```

The first run downloads the official 1 MB UCI archive and verifies its SHA-256
digest. Use `quick` for a short pipeline and visual check. Each run replaces
`domain_study/outputs/`, so obsolete exploratory figures cannot be confused
with the current story.

Outputs include:

- `REPORT.md`: generated numerical findings and interpretation;
- `tables/`: analysis-ready CSV tables;
- `figures/`: publication-sized PNG and PDF figures;
- `reports/`: raw schema-versioned CLI JSON;
- `predictions/`: exact evaluation rows passed to the CLI;
- `manifest.json`: data, software, model, seed, and Monte Carlo metadata.

The publication profile uses finite Monte Carlo counts chosen to remain
runnable on a workstation. CLI Monte Carlo standard errors quantify numerical
uncertainty only; increase the counts before treating individual values as
final manuscript estimates.
