from __future__ import annotations

import math
from pathlib import Path

import numpy as np
import pytest

from oracle import (
    assert_close,
    cnap,
    cnap_from_ap,
    cnap_tolerance,
    metric_for,
    paired_for,
    prior_standardized_ap,
    run_cli,
)


PREVALENCES = (0.001, 0.05, 0.2, 0.5, 0.9, 0.999)


EDGE_CASES = (
    (
        "perfect",
        [True, True, True, False, False, False],
        [0.9, 0.8, 0.7, 0.3, 0.2, 0.1],
    ),
    (
        "reversed",
        [True, True, True, False, False, False],
        [0.1, 0.2, 0.3, 0.7, 0.8, 0.9],
    ),
    (
        "ties",
        [True, False, True, False, True, False, False, True],
        [3.0, 3.0, 2.0, 2.0, 2.0, 1.0, 1.0, 0.0],
    ),
    (
        "constant",
        [True, False, True, False, False, True],
        [7.25, 7.25, 7.25, 7.25, 7.25, 7.25],
    ),
    (
        "unbounded",
        [False, True, False, True, False, True],
        [-1e100, -3.5, -3.5, 0.0, 17.0, 1e100],
    ),
    (
        "one_positive",
        [True, False, False, False, False, False, False, False],
        [0.31, 0.9, 0.8, 0.7, 0.6, 0.5, 0.4, 0.2],
    ),
)


@pytest.mark.parametrize("name,labels,scores", EDGE_CASES)
def test_cli_matches_sklearn_on_deterministic_edge_cases(
    cli_path: Path,
    tmp_path: Path,
    name: str,
    labels: list[bool],
    scores: list[float],
) -> None:
    run = run_cli(
        cli_path,
        tmp_path,
        labels,
        {"score_main": scores},
        PREVALENCES,
        bootstrap_replicates=24,
        permutations=32,
    )
    assert run.report["failures"] == 0
    assert run.report["schema_version"] == 8

    for prevalence in PREVALENCES:
        metric = metric_for(run.report, "score_main", prevalence)
        expected_ap = prior_standardized_ap(labels, scores, prevalence)
        expected_cnap = cnap(labels, scores, prevalence)
        context = {
            "case": name,
            "prevalence": prevalence,
            "labels": labels,
            "scores": scores,
            "report": run.report,
        }
        assert_close(metric["prior_standardized_ap"], expected_ap, context=context)
        assert_close(
            metric["chance_normalized_ap"],
            expected_cnap,
            tolerance=cnap_tolerance(prevalence),
            context=context,
        )
        assert_close(
            metric["chance_normalized_ap"],
            cnap_from_ap(metric["prior_standardized_ap"], prevalence),
            tolerance=2e-15,
            context=context,
        )


def test_observed_reference_prevalence_matches_sklearn(
    cli_path: Path, tmp_path: Path
) -> None:
    labels = [True, False, False, True, False, False, False]
    scores = [0.4, 0.8, 0.1, 0.7, 0.3, 0.2, 0.6]
    prevalence = sum(labels) / len(labels)
    run = run_cli(
        cli_path,
        tmp_path,
        labels,
        {"score_main": scores},
        ["observed"],
        bootstrap_replicates=32,
        permutations=32,
    )
    metric = metric_for(run.report, "score_main", prevalence)
    assert metric["reference_prevalence_mode"] == "observed"
    assert_close(
        metric["prior_standardized_ap"],
        prior_standardized_ap(labels, scores, prevalence),
    )
    assert_close(
        metric["chance_normalized_ap"],
        cnap(labels, scores, prevalence),
        tolerance=cnap_tolerance(prevalence),
    )


def _random_scores(
    rng: np.random.Generator, labels: np.ndarray
) -> dict[str, np.ndarray]:
    signal = labels.astype(float)
    continuous = 0.8 * signal + rng.normal(0.0, 1.0, len(labels))
    discrete = rng.integers(-2, 4, len(labels)).astype(float) + signal
    reversed_scores = -continuous
    constant = np.full(len(labels), rng.normal())
    return {
        "score_continuous": continuous,
        "score_discrete": discrete,
        "score_reversed": reversed_scores,
        "score_constant": constant,
    }


def test_randomized_cli_differential_matrix(
    cli_path: Path,
    tmp_path: Path,
    randomized_case_count: int,
    profile: str,
) -> None:
    master_seed = 0xA0B5_C1A5
    master_rng = np.random.default_rng(master_seed)
    checked = 0

    for case_index in range(randomized_case_count):
        case_seed = int(master_rng.integers(0, 2**63))
        rng = np.random.default_rng(case_seed)
        if profile == "stress":
            sample_size = int(rng.integers(4, 501))
        else:
            sample_size = int(rng.integers(4, 81))
        positive_count = int(rng.integers(1, sample_size))
        labels = np.zeros(sample_size, dtype=bool)
        labels[rng.choice(sample_size, positive_count, replace=False)] = True
        score_columns = _random_scores(rng, labels)

        case_directory = tmp_path / f"case_{case_index:04d}"
        case_directory.mkdir()
        run = run_cli(
            cli_path,
            case_directory,
            labels.tolist(),
            {name: values.tolist() for name, values in score_columns.items()},
            PREVALENCES,
            bootstrap_replicates=12,
            permutations=12,
            paired_baseline="score_continuous",
            bootstrap_seed=case_seed,
            permutation_seed=case_seed ^ 0x5EED,
        )

        for score_name, scores in score_columns.items():
            for prevalence in PREVALENCES:
                metric = metric_for(run.report, score_name, prevalence)
                context = {
                    "master_seed": master_seed,
                    "case_index": case_index,
                    "case_seed": case_seed,
                    "labels": labels.tolist(),
                    "score_name": score_name,
                    "scores": scores.tolist(),
                    "prevalence": prevalence,
                    "report": run.report,
                }
                assert_close(
                    metric["prior_standardized_ap"],
                    prior_standardized_ap(labels, scores, prevalence),
                    context=context,
                )
                assert_close(
                    metric["chance_normalized_ap"],
                    cnap(labels, scores, prevalence),
                    tolerance=cnap_tolerance(prevalence),
                    context=context,
                )
                assert_close(
                    metric["chance_normalized_ap"],
                    cnap_from_ap(metric["prior_standardized_ap"], prevalence),
                    tolerance=2e-15,
                    context=context,
                )
                checked += 1

        baseline = score_columns["score_continuous"]
        for score_name, scores in score_columns.items():
            if score_name == "score_continuous":
                continue
            for prevalence in PREVALENCES:
                paired = paired_for(
                    run.report, score_name, "score_continuous", prevalence
                )
                expected = cnap(labels, scores, prevalence) - cnap(
                    labels, baseline, prevalence
                )
                assert_close(
                    paired["observed_cnap_difference"],
                    expected,
                    tolerance=cnap_tolerance(prevalence, operands=2),
                    context={
                        "master_seed": master_seed,
                        "case_index": case_index,
                        "case_seed": case_seed,
                        "model_a": score_name,
                        "prevalence": prevalence,
                        "report": run.report,
                    },
                )

    expected_checked = randomized_case_count * len(PREVALENCES) * 4
    assert checked == expected_checked


def test_strictly_increasing_score_transforms_are_invariant(
    cli_path: Path, tmp_path: Path
) -> None:
    labels = [True, False, True, False, False, True, False, True]
    scores = np.asarray([-2.0, -1.0, -0.5, 0.0, 0.2, 0.7, 2.0, 3.0])
    run = run_cli(
        cli_path,
        tmp_path,
        labels,
        {
            "score_original": scores,
            "score_affine": 17.0 * scores - 103.0,
            "score_exp": np.exp(scores),
        },
        PREVALENCES,
        bootstrap_replicates=24,
        permutations=24,
    )
    for prevalence in PREVALENCES:
        original = metric_for(run.report, "score_original", prevalence)
        for transformed_name in ("score_affine", "score_exp"):
            transformed = metric_for(run.report, transformed_name, prevalence)
            for field in ("prior_standardized_ap", "chance_normalized_ap"):
                assert_close(transformed[field], original[field])


def test_shared_and_ragged_row_modes_match_filtered_sklearn_inputs(
    cli_path: Path, tmp_path: Path
) -> None:
    labels = [True, False, True, False, True, False]
    score_a: list[object] = [0.9, 0.7, "", 0.3, 0.8, 0.1]
    score_b: list[object] = [0.8, "", 0.6, 0.4, 0.9, 0.2]
    prevalence = 0.2

    shared_dir = tmp_path / "shared"
    shared_dir.mkdir()
    shared = run_cli(
        cli_path,
        shared_dir,
        labels,
        {"score_a": score_a, "score_b": score_b},
        [prevalence],
        bootstrap_replicates=24,
        permutations=24,
    )
    shared_mask = np.asarray([True, False, False, True, True, True])
    for name, scores in (("score_a", score_a), ("score_b", score_b)):
        values = np.asarray(scores, dtype=object)[shared_mask].astype(float)
        metric = metric_for(shared.report, name, prevalence)
        assert_close(
            metric["prior_standardized_ap"],
            prior_standardized_ap(np.asarray(labels)[shared_mask], values, prevalence),
        )

    ragged_dir = tmp_path / "ragged"
    ragged_dir.mkdir()
    ragged = run_cli(
        cli_path,
        ragged_dir,
        labels,
        {"score_a": score_a, "score_b": score_b},
        [prevalence],
        bootstrap_replicates=24,
        permutations=24,
        allow_ragged_rows=True,
    )
    for name, scores in (("score_a", score_a), ("score_b", score_b)):
        mask = np.asarray([value != "" for value in scores])
        values = np.asarray(scores, dtype=object)[mask].astype(float)
        metric = metric_for(ragged.report, name, prevalence)
        assert_close(
            metric["prior_standardized_ap"],
            prior_standardized_ap(np.asarray(labels)[mask], values, prevalence),
        )


def test_all_documented_cli_label_spellings_agree(
    cli_path: Path, tmp_path: Path
) -> None:
    labels: list[object] = ["1", "0.0", "TRUE", "f", "yes", "N", "t", "y"]
    boolean_labels = [True, False, True, False, True, False, True, True]
    scores = [0.9, 0.8, 0.7, 0.6, 0.5, 0.4, 0.3, 0.2]
    prevalence = 0.4
    run = run_cli(
        cli_path,
        tmp_path,
        labels,
        {"score_main": scores},
        [prevalence],
        bootstrap_replicates=24,
        permutations=24,
    )
    metric = metric_for(run.report, "score_main", prevalence)
    assert_close(
        metric["prior_standardized_ap"],
        prior_standardized_ap(boolean_labels, scores, prevalence),
    )
    assert math.isfinite(metric["calibrated_supported_cnap"])
