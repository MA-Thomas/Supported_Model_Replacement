"""Independent Python and scikit-learn oracles for the CLI test suite."""

from __future__ import annotations

import csv
import itertools
import json
import math
import subprocess
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable, Mapping, Sequence

import numpy as np
from sklearn.metrics import average_precision_score


ABS_TOLERANCE = 2e-12


def prior_standardized_ap(
    labels: Sequence[bool] | np.ndarray,
    scores: Sequence[float] | np.ndarray,
    reference_prevalence: float,
) -> float:
    """Use sklearn's non-interpolated weighted AP as the external oracle."""
    y = np.asarray(labels, dtype=bool)
    values = np.asarray(scores, dtype=float)
    positives = int(y.sum())
    negatives = int((~y).sum())
    if positives == 0 or negatives == 0:
        raise ValueError("both classes are required")
    weights = np.where(
        y,
        reference_prevalence / positives,
        (1.0 - reference_prevalence) / negatives,
    )
    return float(average_precision_score(y, values, sample_weight=weights))


def cnap_from_ap(ap: float, reference_prevalence: float) -> float:
    if ap == 1.0:
        return 1.0
    if ap == reference_prevalence:
        return 0.0
    return (ap - reference_prevalence) / (1.0 - reference_prevalence)


def cnap(
    labels: Sequence[bool] | np.ndarray,
    scores: Sequence[float] | np.ndarray,
    reference_prevalence: float,
) -> float:
    return cnap_from_ap(
        prior_standardized_ap(labels, scores, reference_prevalence),
        reference_prevalence,
    )


def cnap_tolerance(reference_prevalence: float, *, operands: int = 1) -> float:
    """Roundoff allowance for the ill-conditioned AP-to-CNAP transformation."""
    propagated = (
        operands
        * 32.0
        * np.finfo(float).eps
        / (1.0 - reference_prevalence)
    )
    return max(ABS_TOLERANCE, float(propagated))


def inverted_cdf(values: Sequence[float], probability: float) -> float:
    ordered = np.sort(np.asarray(values, dtype=float))
    rank = math.ceil(len(ordered) * probability)
    index = min(len(ordered) - 1, max(0, rank - 1))
    return float(ordered[index])


def iid_minimum_expectation(values: Sequence[float], order: int) -> float:
    """E[min(X_1, ..., X_K)] for iid draws from an empirical distribution."""
    ordered = np.sort(np.asarray(values, dtype=float))
    size = len(ordered)
    total = 0.0
    for index, value in enumerate(ordered):
        survival_here = ((size - index) / size) ** order
        survival_next = ((size - index - 1) / size) ** order
        total += float(value) * (survival_here - survival_next)
    return total


def enumerate_stratified_bootstrap(
    labels: Sequence[bool] | np.ndarray,
    *score_vectors: Sequence[float] | np.ndarray,
    reference_prevalence: float,
    replication_positive_count: int | None = None,
    replication_negative_count: int | None = None,
) -> list[tuple[float, ...]]:
    """Enumerate all ordered draws within each observed class."""
    y = np.asarray(labels, dtype=bool)
    scores = [np.asarray(vector, dtype=float) for vector in score_vectors]
    positive_indices = np.flatnonzero(y).tolist()
    negative_indices = np.flatnonzero(~y).tolist()
    positive_draw_count = (
        len(positive_indices)
        if replication_positive_count is None
        else replication_positive_count
    )
    negative_draw_count = (
        len(negative_indices)
        if replication_negative_count is None
        else replication_negative_count
    )
    effects: list[tuple[float, ...]] = []

    positive_draws = itertools.product(
        positive_indices, repeat=positive_draw_count
    )
    for sampled_positive in positive_draws:
        for sampled_negative in itertools.product(
            negative_indices, repeat=negative_draw_count
        ):
            sampled = np.asarray(sampled_positive + sampled_negative, dtype=int)
            sampled_labels = y[sampled]
            effects.append(
                tuple(
                    cnap(
                        sampled_labels,
                        model_scores[sampled],
                        reference_prevalence,
                    )
                    for model_scores in scores
                )
            )
    return effects


def enumerate_conditional_null(
    scores: Sequence[float] | np.ndarray,
    positive_count: int,
    reference_prevalence: float,
    support_order: int,
) -> list[float]:
    """Exact supported CNAP for every distinct fixed-score label allocation."""
    values = np.asarray(scores, dtype=float)
    out: list[float] = []
    for positive_indices in itertools.combinations(range(len(values)), positive_count):
        labels = np.zeros(len(values), dtype=bool)
        labels[list(positive_indices)] = True
        replicates = enumerate_stratified_bootstrap(
            labels,
            values,
            reference_prevalence=reference_prevalence,
        )
        effects = [row[0] for row in replicates]
        out.append(iid_minimum_expectation(effects, support_order))
    return out


@dataclass(frozen=True)
class CliRun:
    report: dict
    input_path: Path
    output_path: Path
    command: tuple[str, ...]


def run_cli(
    cli_path: Path,
    directory: Path,
    labels: Sequence[object],
    score_columns: Mapping[str, Sequence[object]],
    prevalences: Iterable[float | str],
    *,
    bootstrap_replicates: int = 16,
    permutations: int = 16,
    support_order: int = 2,
    survival_frequency: float = 0.5,
    paired_baseline: str | None = None,
    replication_positive_count: int | None = None,
    replication_negative_count: int | None = None,
    allow_ragged_rows: bool = False,
    bootstrap_seed: int = 1729,
    permutation_seed: int = 2718,
) -> CliRun:
    input_path = directory / "input.csv"
    output_path = directory / "report.json"
    names = list(score_columns)
    row_count = len(labels)
    if any(len(score_columns[name]) != row_count for name in names):
        raise ValueError("all columns must have the same physical CSV length")

    with input_path.open("w", newline="", encoding="utf-8") as handle:
        writer = csv.writer(handle)
        writer.writerow(["label", *names])
        for index, label in enumerate(labels):
            writer.writerow(
                [label, *(score_columns[name][index] for name in names)]
            )

    prevalence_argument = ",".join(str(value) for value in prevalences)
    command = [
        str(cli_path),
        "--input",
        str(input_path),
        "--output",
        str(output_path),
        "--label-col",
        "label",
    ]
    for name in names:
        command.extend(["--score-col", name])
    command.extend(
        [
            "--reference-prevalence",
            prevalence_argument,
            "--bootstrap-replicates",
            str(bootstrap_replicates),
            "--permutations",
            str(permutations),
            "--support-order",
            str(support_order),
            "--survival-frequency",
            str(survival_frequency),
            "--bootstrap-seed",
            str(bootstrap_seed),
            "--permutation-seed",
            str(permutation_seed),
            "--fail-on-error",
        ]
    )
    if paired_baseline is not None:
        command.extend(["--paired-baseline", paired_baseline])
    if replication_positive_count is not None:
        command.extend(
            ["--replication-positive-count", str(replication_positive_count)]
        )
    if replication_negative_count is not None:
        command.extend(
            ["--replication-negative-count", str(replication_negative_count)]
        )
    if allow_ragged_rows:
        command.append("--allow-ragged-rows")

    completed = subprocess.run(command, text=True, capture_output=True)
    if completed.returncode != 0:
        report = (
            json.loads(output_path.read_text(encoding="utf-8"))
            if output_path.exists()
            else None
        )
        raise AssertionError(
            "CLI failed\n"
            f"command: {' '.join(command)}\n"
            f"stdout: {completed.stdout}\n"
            f"stderr: {completed.stderr}\n"
            f"report: {json.dumps(report, indent=2)}"
        )
    report = json.loads(output_path.read_text(encoding="utf-8"))
    return CliRun(report, input_path, output_path, tuple(command))


def metric_for(report: dict, score_col: str, prevalence: float) -> dict:
    matches = [
        metric
        for metric in report["metrics"]
        if metric["score_col"] == score_col
        and math.isclose(
            metric["reference_prevalence"], prevalence, rel_tol=0.0, abs_tol=1e-15
        )
    ]
    if len(matches) != 1:
        raise AssertionError(
            f"expected one metric for {score_col=} and {prevalence=}, got {matches}"
        )
    return matches[0]


def paired_for(
    report: dict, model_a: str, model_b: str, prevalence: float
) -> dict:
    matches = [
        metric
        for metric in report["paired"]
        if metric["model_a"] == model_a
        and metric["model_b"] == model_b
        and math.isclose(
            metric["reference_prevalence"], prevalence, rel_tol=0.0, abs_tol=1e-15
        )
    ]
    if len(matches) != 1:
        raise AssertionError(
            f"expected one paired result for {model_a=}, {model_b=}, "
            f"and {prevalence=}, got {matches}"
        )
    return matches[0]


def assert_close(
    actual: float,
    expected: float,
    *,
    tolerance: float = ABS_TOLERANCE,
    context: object = None,
) -> None:
    if not math.isclose(actual, expected, rel_tol=0.0, abs_tol=tolerance):
        raise AssertionError(
            f"expected {expected:.17g}, received {actual:.17g}, "
            f"absolute error {abs(actual - expected):.3g}, "
            f"tolerance {tolerance:.3g}\ncontext: {context!r}"
        )
