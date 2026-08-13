# Python differential tests

These tests treat the `supported_ap_metrics` executable as a black box and
compare its JSON report with an independent Python implementation.

The deterministic prior-standardized AP oracle is scikit-learn's
non-interpolated `average_precision_score` with class-constant sample weights:

```text
positive weight = reference_prevalence / positive_count
negative weight = (1 - reference_prevalence) / negative_count
```

The weights make the total positive and negative mass equal to the declared
reference prevalence and its complement. CNAP is then calculated from that AP
using the package's documented normalization.

For small datasets, the suite independently enumerates every ordered
class-stratified bootstrap resample. It also enumerates every distinct label
allocation for a conditional permutation null. Those exact distributions are
used as targets for the CLI's Monte Carlo estimates.

## Run

From the crate directory:

```bash
python3 -m venv .venv-python-tests
.venv-python-tests/bin/python -m pip install -r python_tests/requirements.txt
cargo build --bin supported_ap_metrics
.venv-python-tests/bin/python -m pytest python_tests --profile extensive
```

Profiles:

- `smoke`: deterministic edge cases and 8 randomized datasets.
- `extensive`: deterministic, exhaustive, and 40 randomized datasets.
- `stress`: deterministic, exhaustive, and 250 randomized datasets.

Set `SUPPORTED_AP_CLI` to test a different executable. Every randomized failure
reports its master seed, case seed, labels, scores, reference prevalence,
expected value, and complete CLI report, so it can be converted directly into
a permanent regression fixture.
