from __future__ import annotations

import math
from pathlib import Path

import numpy as np

from oracle import (
    assert_close,
    cnap,
    cnap_tolerance,
    enumerate_conditional_null,
    enumerate_stratified_bootstrap,
    iid_minimum_expectation,
    inverted_cdf,
    metric_for,
    paired_for,
    run_cli,
)


def _mean_standard_error(values: list[float], replicates: int) -> float:
    return float(np.std(values, ddof=0) / math.sqrt(replicates))


def test_cli_bootstrap_and_paired_estimators_match_exact_enumeration(
    cli_path: Path, tmp_path: Path
) -> None:
    labels = [True, False, True, False, False]
    model_a = [0.9, 0.8, 0.4, 0.3, 0.1]
    model_b = [0.7, 0.9, 0.8, 0.2, 0.1]
    prevalence = 0.2
    support_order = 2
    bootstrap_replicates = 30_000

    exact_rows = enumerate_stratified_bootstrap(
        labels,
        model_a,
        model_b,
        reference_prevalence=prevalence,
    )
    effects_a = [row[0] for row in exact_rows]
    effects_b = [row[1] for row in exact_rows]
    differences = [left - right for left, right in exact_rows]

    run = run_cli(
        cli_path,
        tmp_path,
        labels,
        {"score_a": model_a, "score_b": model_b},
        [prevalence],
        bootstrap_replicates=bootstrap_replicates,
        permutations=2,
        support_order=support_order,
        paired_baseline="score_b",
        bootstrap_seed=0xB0057,
        permutation_seed=0xC0FFEE,
    )

    for name, effects in (("score_a", effects_a), ("score_b", effects_b)):
        metric = metric_for(run.report, name, prevalence)
        exact_mean = float(np.mean(effects))
        exact_supported = iid_minimum_expectation(effects, support_order)
        mean_tolerance = max(
            7.0 * _mean_standard_error(effects, bootstrap_replicates), 2e-12
        )
        supported_tolerance = max(
            7.0 * metric["supported_cnap_mc_se"], 2e-12
        )
        assert_close(
            metric["replicate_mean"],
            exact_mean,
            tolerance=mean_tolerance,
            context={"effect_distribution": effects, "report": run.report},
        )
        assert_close(
            metric["supported_cnap"],
            exact_supported,
            tolerance=supported_tolerance,
            context={"effect_distribution": effects, "report": run.report},
        )
        assert_close(
            metric["support_penalty"],
            metric["replicate_mean"] - metric["supported_cnap"],
            tolerance=2e-12,
        )

        probability = 1.0 - 0.5 ** (1.0 / support_order)
        # DKW gives a distribution-free 99.9999% band for the empirical CDF.
        cdf_error = math.sqrt(
            math.log(2.0 / 1e-6) / (2.0 * bootstrap_replicates)
        )
        lower = inverted_cdf(effects, max(0.0, probability - cdf_error))
        upper = inverted_cdf(effects, min(1.0, probability + cdf_error))
        assert lower - 2e-12 <= metric["survival_level"] <= upper + 2e-12

    paired = paired_for(run.report, "score_a", "score_b", prevalence)
    exact_difference_mean = float(np.mean(differences))
    exact_supported_difference = iid_minimum_expectation(
        differences, support_order
    )
    assert_close(
        paired["observed_cnap_difference"],
        cnap(labels, model_a, prevalence) - cnap(labels, model_b, prevalence),
        tolerance=cnap_tolerance(prevalence, operands=2),
    )
    assert_close(
        paired["replicate_mean_cnap_difference"],
        exact_difference_mean,
        tolerance=max(
            7.0 * _mean_standard_error(differences, bootstrap_replicates),
            2e-12,
        ),
        context={"difference_distribution": differences, "report": run.report},
    )
    assert_close(
        paired["estimated_supported_superiority"],
        exact_supported_difference,
        tolerance=max(
            7.0 * paired["estimated_supported_superiority_mc_se"], 2e-12
        ),
        context={"difference_distribution": differences, "report": run.report},
    )


def test_paired_prospective_counts_match_exact_enumeration(
    cli_path: Path, tmp_path: Path
) -> None:
    labels = [True, False, True, False, False]
    model_a = [0.9, 0.8, 0.4, 0.3, 0.1]
    model_b = [0.7, 0.9, 0.8, 0.2, 0.1]
    prevalence = 0.2
    support_order = 2
    bootstrap_replicates = 30_000
    replication_positive_count = 1
    replication_negative_count = 2

    exact_rows = enumerate_stratified_bootstrap(
        labels,
        model_a,
        model_b,
        reference_prevalence=prevalence,
        replication_positive_count=replication_positive_count,
        replication_negative_count=replication_negative_count,
    )
    differences = [left - right for left, right in exact_rows]

    run = run_cli(
        cli_path,
        tmp_path,
        labels,
        {"score_a": model_a, "score_b": model_b},
        [prevalence],
        bootstrap_replicates=bootstrap_replicates,
        permutations=2,
        support_order=support_order,
        paired_baseline="score_b",
        replication_positive_count=replication_positive_count,
        replication_negative_count=replication_negative_count,
        bootstrap_seed=0xF07ECA57,
        permutation_seed=0xC0FFEE,
    )
    paired = paired_for(run.report, "score_a", "score_b", prevalence)

    assert paired["observed_positive_count"] == 2
    assert paired["observed_negative_count"] == 3
    assert (
        paired["replication_positive_count"] == replication_positive_count
    )
    assert (
        paired["replication_negative_count"] == replication_negative_count
    )
    assert_close(
        paired["replicate_mean_cnap_difference"],
        float(np.mean(differences)),
        tolerance=max(
            7.0 * _mean_standard_error(differences, bootstrap_replicates),
            2e-12,
        ),
        context={"difference_distribution": differences, "report": run.report},
    )
    assert_close(
        paired["estimated_supported_superiority"],
        iid_minimum_expectation(differences, support_order),
        tolerance=max(
            7.0 * paired["estimated_supported_superiority_mc_se"], 2e-12
        ),
        context={"difference_distribution": differences, "report": run.report},
    )


def test_cli_conditional_null_matches_all_label_allocations(
    cli_path: Path, tmp_path: Path
) -> None:
    labels = [True, True, False, False, False]
    scores = [3.0, 2.0, 2.0, 1.0, 0.0]
    prevalence = 0.25
    support_order = 2
    bootstrap_replicates = 2_000
    permutations = 2_000

    observed_effects = [
        row[0]
        for row in enumerate_stratified_bootstrap(
            labels, scores, reference_prevalence=prevalence
        )
    ]
    exact_supported = iid_minimum_expectation(observed_effects, support_order)
    exact_null_values = enumerate_conditional_null(
        scores,
        positive_count=sum(labels),
        reference_prevalence=prevalence,
        support_order=support_order,
    )
    exact_null = float(np.mean(exact_null_values))
    exact_calibrated = (exact_supported - exact_null) / (1.0 - exact_null)

    run = run_cli(
        cli_path,
        tmp_path,
        labels,
        {"score_main": scores},
        [prevalence],
        bootstrap_replicates=bootstrap_replicates,
        permutations=permutations,
        support_order=support_order,
        bootstrap_seed=0xDEC0DE,
        permutation_seed=0xBAD5EED,
    )
    metric = metric_for(run.report, "score_main", prevalence)

    assert_close(
        metric["supported_cnap"],
        exact_supported,
        tolerance=max(7.0 * metric["supported_cnap_mc_se"], 2e-12),
        context={"exact_null_values": exact_null_values, "report": run.report},
    )
    assert_close(
        metric["conditional_null_supported_cnap"],
        exact_null,
        tolerance=max(
            7.0 * metric["conditional_null_supported_cnap_mc_se"], 2e-12
        ),
        context={"exact_null_values": exact_null_values, "report": run.report},
    )
    assert_close(
        metric["calibrated_supported_cnap"],
        exact_calibrated,
        tolerance=max(
            8.0 * metric["calibrated_supported_cnap_mc_se"], 2e-12
        ),
        context={"exact_null_values": exact_null_values, "report": run.report},
    )

    expected_quantiles = {
        "conditional_null_q05": inverted_cdf(exact_null_values, 0.05),
        "conditional_null_q50": inverted_cdf(exact_null_values, 0.50),
        "conditional_null_q95": inverted_cdf(exact_null_values, 0.95),
    }
    # The permutation-level statistic also contains finite-bootstrap noise.
    # Quantiles need not equal the discrete exact target, but must preserve
    # their order and remain in a conservative exact-distribution envelope.
    assert (
        metric["conditional_null_q05"]
        <= metric["conditional_null_q50"]
        <= metric["conditional_null_q95"]
    )
    envelope = max(exact_null_values) - min(exact_null_values)
    for field, expected in expected_quantiles.items():
        assert_close(
            metric[field],
            expected,
            tolerance=max(0.20 * envelope, 0.03),
            context={"exact_null_values": exact_null_values, "report": run.report},
        )
