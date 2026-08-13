#!/usr/bin/env python3
"""Generate the eight-figure pedagogical story for supported AP metrics."""

from __future__ import annotations

import argparse
import hashlib
import io
import json
import math
import os
import platform
import shutil
import subprocess
import tempfile
import urllib.request
import zipfile
from concurrent.futures import ThreadPoolExecutor
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Mapping

os.environ.setdefault(
    "MPLCONFIGDIR", str(Path(tempfile.gettempdir()) / "supported-ap-matplotlib")
)
import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt
import numpy as np
import pandas as pd
import sklearn
from matplotlib.colors import TwoSlopeNorm
from matplotlib.ticker import PercentFormatter
from sklearn.compose import ColumnTransformer
from sklearn.ensemble import RandomForestClassifier
from sklearn.linear_model import LogisticRegression
from sklearn.metrics import average_precision_score
from sklearn.pipeline import Pipeline
from sklearn.preprocessing import OneHotEncoder, StandardScaler


ROOT = Path(__file__).resolve().parents[1]
STUDY_ROOT = ROOT / "domain_study"
RAW_DIR = STUDY_ROOT / "data" / "raw"
OUTPUT_DIR = STUDY_ROOT / "outputs"
TABLE_DIR = OUTPUT_DIR / "tables"
FIGURE_DIR = OUTPUT_DIR / "figures"
REPORT_DIR = OUTPUT_DIR / "reports"
PREDICTION_DIR = OUTPUT_DIR / "predictions"

DATA_URL = "https://archive.ics.uci.edu/static/public/222/bank%2Bmarketing.zip"
ARCHIVE_SHA256 = "e0bf5f5de5b846e2f18e9d90606637267d46dfa260e0f17bb12e605db5efbeb4"
ARCHIVE_PATH = RAW_DIR / "bank_marketing.zip"
CSV_PATH = RAW_DIR / "bank-additional" / "bank-additional-full.csv"

MASTER_SEED = 20_260_726
TRAIN_FRACTION = 0.80
PREVALENCE_GRID = (0.02, 0.05, 0.10, 0.20, 0.40)
POSITIVE_COUNT_GRID = (5, 10, 25, 50, 100)
RESOLUTION_GRID = (2, 5, 20, 0)
TARGET_PREVALENCE_GRID = tuple(np.geomspace(0.01, 0.50, 33))

BLUE = "#0072B2"
SKY = "#56B4E9"
ORANGE = "#E69F00"
GREEN = "#009E73"
VERMILLION = "#D55E00"
PURPLE = "#CC79A7"
GRAY = "#666666"
LIGHT_GRAY = "#D9D9D9"


@dataclass(frozen=True)
class MonteCarloSettings:
    matched_bootstrap: int
    matched_permutations: int
    grid_bootstrap: int
    grid_permutations: int
    null_repetitions: int
    null_bootstrap: int
    null_permutations: int
    real_repetitions: int
    real_bootstrap: int
    real_permutations: int
    comparison_repetitions: int
    comparison_bootstrap: int
    matched_distribution_bootstrap: int


PROFILES = {
    "quick": MonteCarloSettings(
        600, 30, 300, 30, 40, 180, 15, 6, 180, 15, 4, 180, 600
    ),
    "publication": MonteCarloSettings(
        3_000, 120, 1_200, 100, 160, 600, 60, 18, 600, 60, 12, 500, 5_000
    ),
}


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--profile",
        choices=tuple(PROFILES),
        default="publication",
        help="Monte Carlo effort used by the Rust CLI",
    )
    parser.add_argument(
        "--cli",
        type=Path,
        default=ROOT / "target" / "release" / "supported_ap_metrics",
        help="compiled CLI executable",
    )
    return parser.parse_args()


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def ensure_dataset() -> None:
    RAW_DIR.mkdir(parents=True, exist_ok=True)
    if not ARCHIVE_PATH.exists():
        temporary = ARCHIVE_PATH.with_suffix(".download")
        urllib.request.urlretrieve(DATA_URL, temporary)
        temporary.replace(ARCHIVE_PATH)
    actual_digest = sha256(ARCHIVE_PATH)
    if actual_digest != ARCHIVE_SHA256:
        raise RuntimeError(
            f"UCI archive SHA-256 mismatch: {actual_digest}; "
            f"expected {ARCHIVE_SHA256}"
        )
    if CSV_PATH.exists():
        return
    with zipfile.ZipFile(ARCHIVE_PATH) as outer:
        nested_bytes = outer.read("bank-additional.zip")
    with zipfile.ZipFile(io.BytesIO(nested_bytes)) as nested:
        member = "bank-additional/bank-additional-full.csv"
        destination = RAW_DIR / member
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(nested.read(member))


def prepare_output_directories() -> None:
    # Outputs are fully generated artifacts. Replacing this directory ensures
    # obsolete exploratory figures cannot be mistaken for the current story.
    if OUTPUT_DIR.exists():
        shutil.rmtree(OUTPUT_DIR)
    for directory in (TABLE_DIR, FIGURE_DIR, REPORT_DIR, PREDICTION_DIR):
        directory.mkdir(parents=True, exist_ok=True)


def build_models(frame: pd.DataFrame) -> tuple[dict[str, Pipeline], list[str], list[str]]:
    feature_frame = frame.drop(columns=["y", "duration"])
    categorical = feature_frame.select_dtypes(include=["object"]).columns.tolist()
    numeric = [column for column in feature_frame.columns if column not in categorical]

    def preprocessor(columns: list[str]) -> ColumnTransformer:
        selected_categorical = [column for column in columns if column in categorical]
        selected_numeric = [column for column in columns if column not in categorical]
        return ColumnTransformer(
            [
                ("numeric", StandardScaler(), selected_numeric),
                (
                    "categorical",
                    OneHotEncoder(handle_unknown="ignore", sparse_output=False),
                    selected_categorical,
                ),
            ],
            sparse_threshold=0.0,
        )

    def logistic(columns: list[str]) -> Pipeline:
        return Pipeline(
            [
                ("preprocess", preprocessor(columns)),
                (
                    "model",
                    LogisticRegression(
                        C=1.0,
                        max_iter=2_000,
                        solver="liblinear",
                        random_state=MASTER_SEED,
                    ),
                ),
            ]
        )

    all_columns = feature_frame.columns.tolist()
    demographic_columns = ["age", "job", "marital", "education"]
    models = {
        "full_logistic": logistic(all_columns),
        "random_forest": Pipeline(
            [
                ("preprocess", preprocessor(all_columns)),
                (
                    "model",
                    RandomForestClassifier(
                        n_estimators=300,
                        min_samples_leaf=20,
                        max_features="sqrt",
                        n_jobs=-1,
                        random_state=MASTER_SEED,
                    ),
                ),
            ]
        ),
        "demographic_logistic": logistic(demographic_columns),
    }
    return models, numeric, categorical


def standardized_ap(labels: np.ndarray, scores: np.ndarray, prevalence: float) -> float:
    labels = np.asarray(labels, dtype=bool)
    positive = int(labels.sum())
    negative = len(labels) - positive
    if positive == 0 or negative == 0:
        raise ValueError("both classes are required")
    weights = np.where(
        labels,
        prevalence / positive,
        (1.0 - prevalence) / negative,
    )
    return float(average_precision_score(labels, scores, sample_weight=weights))


def cnap_from_ap(ap: float, prevalence: float) -> float:
    return (ap - prevalence) / (1.0 - prevalence)


def direct_cnap(labels: np.ndarray, scores: np.ndarray, prevalence: float) -> float:
    """CNAP with grouped score ties, used only for fast design search."""
    labels = np.asarray(labels, dtype=bool)
    scores = np.asarray(scores, dtype=float)
    order = np.argsort(-scores, kind="mergesort")
    ordered_labels = labels[order]
    ordered_scores = scores[order]
    group_ends = np.r_[ordered_scores[1:] != ordered_scores[:-1], True]
    cumulative_positive = np.cumsum(ordered_labels)[group_ends]
    cumulative_negative = np.cumsum(~ordered_labels)[group_ends]
    true_positive_rate = cumulative_positive / labels.sum()
    false_positive_rate = cumulative_negative / (~labels).sum()
    denominator = (
        prevalence * true_positive_rate
        + (1.0 - prevalence) * false_positive_rate
    )
    precision = np.divide(
        prevalence * true_positive_rate,
        denominator,
        out=np.zeros_like(denominator, dtype=float),
        where=denominator > 0.0,
    )
    recall_increments = np.diff(np.r_[0.0, true_positive_rate])
    ap = float(np.sum(recall_increments * precision))
    return cnap_from_ap(ap, prevalence)


def support_order_two(values: np.ndarray) -> float:
    ordered = np.sort(np.asarray(values, dtype=float))
    count = len(ordered)
    weights = count - np.arange(count) - 1
    return float(2.0 * np.dot(ordered, weights) / (count * (count - 1)))


def stratified_bootstrap_cnap(
    labels: np.ndarray,
    scores: np.ndarray,
    prevalence: float,
    replicates: int,
    seed: int,
) -> np.ndarray:
    labels = np.asarray(labels, dtype=bool)
    positive_scores = np.asarray(scores)[labels]
    negative_scores = np.asarray(scores)[~labels]
    canonical_labels = np.r_[
        np.ones(len(positive_scores), dtype=bool),
        np.zeros(len(negative_scores), dtype=bool),
    ]
    rng = np.random.default_rng(seed)
    result = np.empty(replicates)
    for index in range(replicates):
        sampled_scores = np.r_[
            rng.choice(positive_scores, len(positive_scores), replace=True),
            rng.choice(negative_scores, len(negative_scores), replace=True),
        ]
        result[index] = direct_cnap(canonical_labels, sampled_scores, prevalence)
    return result


def fixed_metric(report: dict, score_col: str) -> dict:
    matches = [
        row
        for row in report["metrics"]
        if row["score_col"] == score_col
        and row["reference_prevalence_mode"] != "observed"
    ]
    if len(matches) != 1 or matches[0]["status"] != "ok":
        raise RuntimeError(f"expected one successful fixed-prevalence metric: {matches}")
    return matches[0]


def run_cli(
    cli: Path,
    context: str,
    labels: np.ndarray,
    scores: Mapping[str, np.ndarray],
    reference_prevalence: float | tuple[float, ...],
    bootstrap_replicates: int,
    permutations: int,
    seed_offset: int,
    paired_baseline: str | None = None,
    threads: int | None = None,
) -> dict:
    if not cli.is_file():
        raise FileNotFoundError(f"compiled CLI not found: {cli}")
    prediction_path = PREDICTION_DIR / f"{context}.csv"
    report_path = REPORT_DIR / f"{context}.json"
    data: dict[str, np.ndarray] = {"label": np.asarray(labels, dtype=np.uint8)}
    data.update({name: np.asarray(value, dtype=float) for name, value in scores.items()})
    pd.DataFrame(data).to_csv(prediction_path, index=False, float_format="%.17g")

    command = [
        str(cli),
        "--input",
        str(prediction_path),
        "--output",
        str(report_path),
        "--label-col",
        "label",
    ]
    for score_name in scores:
        command.extend(["--score-col", score_name])
    if isinstance(reference_prevalence, tuple):
        prevalence_token = ",".join(
            f"{prevalence:.17g}" for prevalence in reference_prevalence
        )
    else:
        prevalence_token = f"{reference_prevalence:.17g}"
    command.extend(
        [
            "--reference-prevalence",
            prevalence_token,
            "--bootstrap-replicates",
            str(bootstrap_replicates),
            "--permutations",
            str(permutations),
            "--support-order",
            "2",
            "--survival-frequency",
            "0.5",
            "--bootstrap-seed",
            str(MASTER_SEED + seed_offset),
            "--permutation-seed",
            str(MASTER_SEED + 100_000 + seed_offset),
            "--threads",
            str(threads or max(1, min(8, os.cpu_count() or 1))),
            "--fail-on-error",
        ]
    )
    if paired_baseline is not None:
        command.extend(["--paired-baseline", paired_baseline])
    completed = subprocess.run(command, text=True, capture_output=True)
    if completed.returncode != 0:
        raise RuntimeError(
            f"CLI failed for {context}\ncommand: {' '.join(command)}\n"
            f"stdout: {completed.stdout}\nstderr: {completed.stderr}"
        )
    report = json.loads(report_path.read_text(encoding="utf-8"))
    if report["failures"]:
        raise RuntimeError(f"CLI reported failures for {context}: {report['failures']}")
    return report


def flatten_metric(
    context: str,
    report: dict,
    score_col: str,
    labels: np.ndarray,
    scores: np.ndarray,
    reference_prevalence: float,
) -> dict:
    metric = fixed_metric(report, score_col)
    expected = standardized_ap(labels, scores, reference_prevalence)
    if not math.isclose(
        expected,
        metric["prior_standardized_ap"],
        rel_tol=0.0,
        abs_tol=2e-12,
    ):
        raise AssertionError("CLI standardized AP does not match scikit-learn")
    return {
        "context": context,
        "score_col": score_col,
        "n": len(labels),
        "n_positive": int(np.sum(labels)),
        "n_negative": int(len(labels) - np.sum(labels)),
        "observed_prevalence": float(np.mean(labels)),
        "reference_prevalence": reference_prevalence,
        "traditional_ap": float(average_precision_score(labels, scores)),
        "prior_standardized_ap": metric["prior_standardized_ap"],
        "chance_normalized_ap": metric["chance_normalized_ap"],
        "replicate_mean": metric["replicate_mean"],
        "support_penalty": metric["support_penalty"],
        "supported_cnap": metric["supported_cnap"],
        "supported_cnap_mc_se": metric["supported_cnap_mc_se"],
        "conditional_null_supported_cnap": metric[
            "conditional_null_supported_cnap"
        ],
        "conditional_null_supported_cnap_mc_se": metric[
            "conditional_null_supported_cnap_mc_se"
        ],
        "calibrated_supported_cnap": metric["calibrated_supported_cnap"],
        "calibrated_supported_cnap_mc_se": metric[
            "calibrated_supported_cnap_mc_se"
        ],
        "permutation_p_value": metric["permutation_p_value"],
        "conditional_null_q05": metric["conditional_null_q05"],
        "conditional_null_q50": metric["conditional_null_q50"],
        "conditional_null_q95": metric["conditional_null_q95"],
    }


def flatten_paired_metric(
    context: str,
    comparison: str,
    report: dict,
    candidate_col: str,
    baseline_col: str,
) -> dict:
    matches = [
        row
        for row in report["paired"]
        if row["model_a"] == candidate_col
        and row["model_b"] == baseline_col
        and row["reference_prevalence_mode"] != "observed"
    ]
    if len(matches) != 1 or matches[0]["status"] != "ok":
        raise RuntimeError(f"expected one successful paired metric: {matches}")
    row = matches[0]
    return {
        "context": context,
        "comparison": comparison,
        "candidate_col": candidate_col,
        "baseline_col": baseline_col,
        "reference_prevalence": row["reference_prevalence"],
        "observed_cnap_difference": row["observed_cnap_difference"],
        "replicate_mean_cnap_difference": row[
            "replicate_mean_cnap_difference"
        ],
        "estimated_supported_superiority": row[
            "estimated_supported_superiority"
        ],
        "support_penalty": row["support_penalty"],
        "estimated_supported_superiority_mc_se": row[
            "estimated_supported_superiority_mc_se"
        ],
    }


def paired_bootstrap_cnap_differences(
    labels: np.ndarray,
    candidate_scores: np.ndarray,
    baseline_scores: np.ndarray,
    prevalence: float,
    replicates: int,
    seed: int,
) -> np.ndarray:
    labels = np.asarray(labels, dtype=bool)
    positive_positions = np.flatnonzero(labels)
    negative_positions = np.flatnonzero(~labels)
    canonical_labels = np.r_[
        np.ones(len(positive_positions), dtype=bool),
        np.zeros(len(negative_positions), dtype=bool),
    ]
    rng = np.random.default_rng(seed)
    differences = np.empty(replicates)
    for index in range(replicates):
        sampled_positions = np.r_[
            rng.choice(positive_positions, len(positive_positions), replace=True),
            rng.choice(negative_positions, len(negative_positions), replace=True),
        ]
        candidate = direct_cnap(
            canonical_labels, candidate_scores[sampled_positions], prevalence
        )
        baseline = direct_cnap(
            canonical_labels, baseline_scores[sampled_positions], prevalence
        )
        differences[index] = candidate - baseline
    return differences


def save_table(frame: pd.DataFrame, name: str) -> None:
    frame.to_csv(TABLE_DIR / name, index=False, float_format="%.12g")


def configure_plots() -> None:
    plt.rcParams.update(
        {
            "figure.dpi": 130,
            "savefig.dpi": 240,
            "font.family": "DejaVu Sans",
            "font.size": 10,
            "axes.titlesize": 11,
            "axes.labelsize": 10,
            "axes.spines.top": False,
            "axes.spines.right": False,
            "axes.grid": True,
            "axes.axisbelow": True,
            "grid.alpha": 0.20,
            "legend.frameon": False,
        }
    )


def save_figure(figure: plt.Figure, stem: str) -> None:
    figure.savefig(FIGURE_DIR / f"{stem}.png", bbox_inches="tight")
    figure.savefig(FIGURE_DIR / f"{stem}.pdf", bbox_inches="tight")
    plt.close(figure)


def stratified_sample(
    labels: np.ndarray,
    sample_size: int,
    positive_fraction: float,
    seed: int,
) -> np.ndarray:
    positive_count = max(1, min(sample_size - 1, round(sample_size * positive_fraction)))
    negative_count = sample_size - positive_count
    positive_indices = np.flatnonzero(labels)
    negative_indices = np.flatnonzero(~labels)
    if positive_count > len(positive_indices) or negative_count > len(negative_indices):
        raise ValueError("requested stratified sample is larger than the available class")
    rng = np.random.default_rng(seed)
    selected = np.r_[
        rng.choice(positive_indices, positive_count, replace=False),
        rng.choice(negative_indices, negative_count, replace=False),
    ]
    rng.shuffle(selected)
    return selected


def prevalence_transport(
    labels: np.ndarray,
    scores: np.ndarray,
    reference_prevalence: float,
) -> pd.DataFrame:
    rows: list[dict] = []
    for grid_index, target in enumerate(PREVALENCE_GRID):
        for repeat in range(30):
            indices = stratified_sample(
                labels,
                1_200,
                target,
                MASTER_SEED + grid_index * 10_000 + repeat,
            )
            sampled_labels = labels[indices]
            sampled_scores = scores[indices]
            rows.append(
                {
                    "target_prevalence": target,
                    "repeat": repeat,
                    "observed_prevalence": float(sampled_labels.mean()),
                    "traditional_ap": float(
                        average_precision_score(sampled_labels, sampled_scores)
                    ),
                    "prior_standardized_ap": standardized_ap(
                        sampled_labels, sampled_scores, reference_prevalence
                    ),
                }
            )
    return pd.DataFrame(rows)


def plot_prevalence_transport(
    rows: pd.DataFrame, reference_prevalence: float
) -> None:
    summary = (
        rows.groupby("target_prevalence")
        .agg(
            traditional_mean=("traditional_ap", "mean"),
            traditional_q10=("traditional_ap", lambda x: x.quantile(0.10)),
            traditional_q90=("traditional_ap", lambda x: x.quantile(0.90)),
            standardized_mean=("prior_standardized_ap", "mean"),
            standardized_q10=("prior_standardized_ap", lambda x: x.quantile(0.10)),
            standardized_q90=("prior_standardized_ap", lambda x: x.quantile(0.90)),
        )
        .reset_index()
    )
    x = summary["target_prevalence"].to_numpy()
    figure, axis = plt.subplots(figsize=(8.4, 5.0))
    for prefix, color, label in (
        ("traditional", BLUE, "Traditional AP"),
        (
            "standardized",
            ORANGE,
            f"AP standardized to {reference_prevalence:.1%} prevalence",
        ),
    ):
        mean = summary[f"{prefix}_mean"].to_numpy()
        lower = summary[f"{prefix}_q10"].to_numpy()
        upper = summary[f"{prefix}_q90"].to_numpy()
        axis.fill_between(x, lower, upper, color=color, alpha=0.15, linewidth=0)
        axis.plot(x, mean, marker="o", linewidth=2.4, color=color, label=label)
    axis.axvline(
        reference_prevalence,
        color=GRAY,
        linestyle=":",
        linewidth=1.3,
        label="training prevalence",
    )
    axis.set(
        xlabel="Positive prevalence in the evaluation sample",
        ylabel="Average precision",
        title=(
            "The familiar problem: AP moves when prevalence moves\n"
            "Bank Marketing model: logistic regression"
        ),
    )
    axis.xaxis.set_major_formatter(PercentFormatter(1.0))
    axis.set_ylim(bottom=0.0)
    axis.legend(loc="upper left")
    axis.text(
        0.98,
        0.05,
        "Same fitted model and class-conditional test scores\n"
        "Bands: 10th–90th percentile over 30 samples",
        transform=axis.transAxes,
        ha="right",
        va="bottom",
        color=GRAY,
    )
    save_figure(figure, "01-prevalence-transport")


def heldout_design_sample(
    labels: np.ndarray,
    scores: np.ndarray,
    row_indices: np.ndarray,
    n_positive: int,
    reference_prevalence: float,
    seed: int,
) -> tuple[np.ndarray, np.ndarray, np.ndarray]:
    sample_size = round(n_positive / reference_prevalence)
    n_negative = sample_size - n_positive
    positive_positions = np.flatnonzero(labels)
    negative_positions = np.flatnonzero(~labels)
    if n_positive > len(positive_positions) or n_negative > len(negative_positions):
        raise ValueError("requested held-out design exceeds the available class pool")
    rng = np.random.default_rng(seed)
    selected = np.r_[
        rng.choice(positive_positions, n_positive, replace=False),
        rng.choice(negative_positions, n_negative, replace=False),
    ]
    canonical_labels = np.r_[
        np.ones(n_positive, dtype=bool),
        np.zeros(n_negative, dtype=bool),
    ]
    return canonical_labels, scores[selected], row_indices[selected]


def matched_replication_designs(
    labels: np.ndarray,
    scores: np.ndarray,
    row_indices: np.ndarray,
    reference_prevalence: float,
    bootstrap_replicates: int,
) -> tuple[
    np.ndarray,
    dict[str, np.ndarray],
    pd.DataFrame,
    pd.DataFrame,
    pd.DataFrame,
]:
    n = 120
    n_positive = 8
    # Candidate IDs come from a deterministic search over held-out stratified
    # samples. They align observed CNAP and bootstrap mean while separating the
    # order-2 support penalty. Replaying the sampler makes every source row
    # auditable without rerunning the search.
    candidate_to_design = {
        2_051: "stable_pattern",
        1_311: "fragile_pattern",
    }
    positive_positions = np.flatnonzero(labels)
    negative_positions = np.flatnonzero(~labels)
    rng = np.random.default_rng(MASTER_SEED + 71_000 + n)
    score_map: dict[str, np.ndarray] = {}
    selected_map: dict[str, np.ndarray] = {}
    for candidate in range(max(candidate_to_design) + 1):
        selected = np.r_[
            rng.choice(positive_positions, n_positive, replace=False),
            rng.choice(negative_positions, n - n_positive, replace=False),
        ]
        if candidate in candidate_to_design:
            name = candidate_to_design[candidate]
            score_map[name] = scores[selected]
            selected_map[name] = selected

    canonical_labels = np.r_[
        np.ones(n_positive, dtype=bool),
        np.zeros(n - n_positive, dtype=bool),
    ]
    selection_rows: list[dict] = []
    bootstrap_rows: list[dict] = []
    source_rows: list[dict] = []
    for offset, name in enumerate(("stable_pattern", "fragile_pattern")):
        design_scores = score_map[name]
        boot = stratified_bootstrap_cnap(
            canonical_labels,
            design_scores,
            reference_prevalence,
            bootstrap_replicates,
            MASTER_SEED + 30_000 + offset,
        )
        design_bootstrap_mean = float(boot.mean())
        design_supported_cnap = support_order_two(boot)
        selection_rows.append(
            {
                "design": name,
                "design_observed_cnap": direct_cnap(
                    canonical_labels, design_scores, reference_prevalence
                ),
                "design_bootstrap_replicates": bootstrap_replicates,
                "design_bootstrap_mean": design_bootstrap_mean,
                "design_support_penalty": (
                    design_bootstrap_mean - design_supported_cnap
                ),
                "design_supported_cnap": design_supported_cnap,
                "design_bootstrap_sd": float(boot.std(ddof=1)),
                "data_source": "UCI Bank Marketing temporal holdout",
                "model": "logistic regression",
            }
        )
        bootstrap_rows.extend(
            {"design": name, "replicate": index, "cnap": value}
            for index, value in enumerate(boot)
        )
        for within_sample_row, source_position in enumerate(selected_map[name]):
            source_rows.append(
                {
                    "design": name,
                    "within_sample_row": within_sample_row,
                    "source_row_index": row_indices[source_position],
                    "label": int(labels[source_position]),
                    "score_logistic_regression": scores[source_position],
                }
            )
    return (
        canonical_labels,
        score_map,
        pd.DataFrame(selection_rows),
        pd.DataFrame(bootstrap_rows),
        pd.DataFrame(source_rows),
    )


def matched_support_experiment(
    cli: Path,
    settings: MonteCarloSettings,
    labels: np.ndarray,
    scores: np.ndarray,
    row_indices: np.ndarray,
    reference_prevalence: float,
) -> tuple[pd.DataFrame, pd.DataFrame, pd.DataFrame]:
    design_labels, design_scores, selection, bootstrap, sources = (
        matched_replication_designs(
            labels,
            scores,
            row_indices,
            reference_prevalence,
            settings.matched_distribution_bootstrap,
        )
    )
    report = run_cli(
        cli,
        "matched_observed_performance",
        design_labels,
        design_scores,
        reference_prevalence,
        settings.matched_bootstrap,
        settings.matched_permutations,
        2_000,
    )
    rows = [
        {
            **flatten_metric(
                "matched_observed_performance",
                report,
                name,
                design_labels,
                values,
                reference_prevalence,
            ),
            "design": name,
        }
        for name, values in design_scores.items()
    ]
    metrics = pd.DataFrame(rows).merge(selection, on="design")
    return metrics, bootstrap, sources


def plot_matched_support(metrics: pd.DataFrame, bootstrap: pd.DataFrame) -> None:
    order = ["stable_pattern", "fragile_pattern"]
    labels = ["More reproducible", "Less reproducible"]
    colors = [GREEN, VERMILLION]
    figure, axes = plt.subplots(1, 2, figsize=(11.8, 4.8))

    axis = axes[0]
    x = np.arange(2)
    observed = [
        metrics.loc[metrics["design"].eq(name), "chance_normalized_ap"].iloc[0]
        for name in order
    ]
    bars = axis.bar(x, observed, color=colors, width=0.62)
    axis.set_xticks(x, labels)
    axis.set_ylabel("Observed CNAP")
    axis.set_title("A. The observed headline is nearly identical")
    axis.set_ylim(0.0, max(observed) + 0.10)
    for bar, value in zip(bars, observed):
        axis.text(
            bar.get_x() + bar.get_width() / 2,
            value + 0.012,
            f"{value:.3f}",
            ha="center",
            fontweight="bold",
        )
    axis.text(
        0.5,
        0.91,
        "Matched held-out samples: n = 120, 8 positives",
        transform=axis.transAxes,
        ha="center",
        color=GRAY,
        fontsize=9,
    )

    axis = axes[1]
    violin_data = [
        bootstrap.loc[bootstrap["design"].eq(name), "cnap"].to_numpy()
        for name in order
    ]
    violins = axis.violinplot(
        violin_data,
        positions=x,
        widths=0.75,
        showmeans=False,
        showmedians=False,
        showextrema=False,
    )
    for body, color in zip(violins["bodies"], colors):
        body.set_facecolor(color)
        body.set_edgecolor(color)
        body.set_alpha(0.27)
    for position, name, color, values in zip(x, order, colors, violin_data):
        lower, upper = np.quantile(values, [0.10, 0.90])
        axis.vlines(position, lower, upper, color=color, linewidth=3, zorder=3)
        axis.hlines(
            [lower, upper],
            position - 0.10,
            position + 0.10,
            color=color,
            linewidth=2,
            zorder=3,
        )
        row = metrics.loc[metrics["design"].eq(name)].iloc[0]
        axis.scatter(
            position,
            row["design_bootstrap_mean"],
            marker="o",
            s=55,
            color=color,
            edgecolor="white",
            linewidth=0.8,
            zorder=4,
            label="Bootstrap mean" if position == 0 else None,
        )
        axis.scatter(
            position,
            row["design_supported_cnap"],
            marker="D",
            s=58,
            color="black",
            zorder=5,
            label="Supported CNAP" if position == 0 else None,
        )
        axis.annotate(
            f"supported {row['design_supported_cnap']:.3f}",
            (position, row["design_supported_cnap"]),
            xytext=(8 if position == 0 else -8, -18),
            textcoords="offset points",
            ha="left" if position == 0 else "right",
            fontsize=9,
        )
    axis.axhline(0.0, color=GRAY, linewidth=1)
    axis.set_xticks(x, labels)
    axis.set_ylabel("CNAP across empirical replications")
    axis.set_title("B. Replication support separates them")
    axis.legend(loc="upper right")
    figure.suptitle(
        "A point estimate cannot reveal whether performance is reproducible",
        fontsize=13,
        y=1.05,
    )
    figure.text(
        0.5,
        0.985,
        "Bank Marketing model: logistic regression",
        ha="center",
        color=GRAY,
        fontsize=9,
    )
    figure.tight_layout()
    save_figure(figure, "02-same-performance-different-support")


def quantize_real_scores(scores: np.ndarray, levels: int) -> np.ndarray:
    if levels == 0:
        return np.asarray(scores, dtype=float)
    codes = pd.qcut(
        pd.Series(scores),
        q=levels,
        labels=False,
        duplicates="drop",
    )
    return codes.to_numpy(dtype=float)


def conditional_null_grid(
    cli: Path,
    settings: MonteCarloSettings,
    labels: np.ndarray,
    model_scores: np.ndarray,
    row_indices: np.ndarray,
    reference_prevalence: float,
) -> tuple[pd.DataFrame, pd.DataFrame]:
    rows: list[dict] = []
    source_rows: list[dict] = []
    for index, n_positive in enumerate(POSITIVE_COUNT_GRID):
        design_labels, continuous_scores, selected_rows = heldout_design_sample(
            labels,
            model_scores,
            row_indices,
            n_positive,
            reference_prevalence,
            MASTER_SEED + 70_000 + index,
        )
        scores = {
            (
                "continuous" if levels == 0 else f"levels_{levels}"
            ): quantize_real_scores(continuous_scores, levels)
            for levels in RESOLUTION_GRID
        }
        report = run_cli(
            cli,
            f"conditional_null_grid_pos_{n_positive}",
            design_labels,
            scores,
            reference_prevalence,
            settings.grid_bootstrap,
            settings.grid_permutations,
            3_000 + index,
        )
        for levels, (name, values) in zip(RESOLUTION_GRID, scores.items()):
            rows.append(
                {
                    **flatten_metric(
                        f"conditional_null_grid_pos_{n_positive}",
                        report,
                        name,
                        design_labels,
                        values,
                        reference_prevalence,
                    ),
                    "score_levels": levels,
                    "score_resolution": "continuous" if levels == 0 else str(levels),
                }
            )
        for within_sample_row, (label, score, source_row) in enumerate(
            zip(design_labels, continuous_scores, selected_rows)
        ):
            source_rows.append(
                {
                    "n_positive": n_positive,
                    "within_sample_row": within_sample_row,
                    "source_row_index": source_row,
                    "label": int(label),
                    "score_logistic_regression": score,
                }
            )
    return pd.DataFrame(rows), pd.DataFrame(source_rows)


def plot_conditional_null_grid(rows: pd.DataFrame) -> None:
    labels = ["2", "5", "20", "continuous"]
    values = np.array(
        [
            [
                rows.loc[
                    rows["n_positive"].eq(n_positive)
                    & rows["score_resolution"].eq(label),
                    "conditional_null_supported_cnap",
                ].iloc[0]
                for label in labels
            ]
            for n_positive in POSITIVE_COUNT_GRID
        ]
    )
    largest = max(0.005, float(np.max(np.abs(values))) * 1.05)
    figure, axis = plt.subplots(figsize=(9.1, 5.2))
    image = axis.imshow(
        values,
        cmap="PuOr_r",
        norm=TwoSlopeNorm(vmin=-largest, vcenter=0.0, vmax=largest),
        aspect="auto",
    )
    for row_index in range(values.shape[0]):
        for column_index in range(values.shape[1]):
            value = values[row_index, column_index]
            axis.text(
                column_index,
                row_index,
                f"{value:+.3f}",
                ha="center",
                va="center",
                color="white" if abs(value) > largest * 0.52 else "black",
                fontweight="bold" if abs(value) > largest * 0.40 else "normal",
            )
    axis.set_xticks(np.arange(len(labels)), labels)
    axis.set_yticks(np.arange(len(POSITIVE_COUNT_GRID)), POSITIVE_COUNT_GRID)
    axis.set_xlabel("Number of distinct score levels")
    prevalence = rows["reference_prevalence"].iloc[0]
    axis.set_ylabel(f"Positive cases (≈{prevalence:.1%} prevalence)")
    axis.set_title(
        "Raw supported CNAP does not have a universal zero under chance\n"
        "Bank Marketing model: logistic regression"
    )
    colorbar = figure.colorbar(image, ax=axis, fraction=0.05, pad=0.03)
    colorbar.set_label("Mean supported CNAP under the conditional permutation null")
    axis.text(
        1.0,
        -0.18,
        "The chance anchor depends on sample information and score resolution.",
        transform=axis.transAxes,
        ha="right",
        color=GRAY,
    )
    figure.tight_layout()
    save_figure(figure, "03-design-specific-chance")


def null_reanchoring_experiment(
    cli: Path,
    settings: MonteCarloSettings,
    labels: np.ndarray,
    model_scores: np.ndarray,
    row_indices: np.ndarray,
    reference_prevalence: float,
) -> tuple[pd.DataFrame, pd.DataFrame]:
    rows: list[dict] = []
    source_rows: list[dict] = []
    designs = [
        (10, 5, "10 positives · 5 levels"),
        (10, 0, "10 positives · continuous"),
        (100, 5, "100 positives · 5 levels"),
        (100, 0, "100 positives · continuous"),
    ]
    for design_index, (n_positive, levels, label) in enumerate(designs):
        grid_index = POSITIVE_COUNT_GRID.index(n_positive)
        design_labels, continuous_scores, selected_rows = heldout_design_sample(
            labels,
            model_scores,
            row_indices,
            n_positive,
            reference_prevalence,
            MASTER_SEED + 70_000 + grid_index,
        )
        base_scores = quantize_real_scores(continuous_scores, levels)
        rng = np.random.default_rng(MASTER_SEED + 50_000 + design_index)
        permuted_labels = [
            rng.permutation(design_labels)
            for _ in range(settings.null_repetitions)
        ]

        def evaluate_outer_permutation(
            item: tuple[int, np.ndarray],
        ) -> dict:
            repeat, outer_labels = item
            context = (
                f"null_reanchoring_{n_positive}_{levels or 'continuous'}"
                f"_repeat_{repeat:03d}"
            )
            report = run_cli(
                cli,
                context,
                outer_labels,
                {"full_logistic": base_scores},
                reference_prevalence,
                settings.null_bootstrap,
                settings.null_permutations,
                4_000 + design_index * 1_000 + repeat,
                threads=1,
            )
            return {
                **flatten_metric(
                    label,
                    report,
                    "full_logistic",
                    outer_labels,
                    base_scores,
                    reference_prevalence,
                ),
                "design": label,
                "repeat": repeat,
                "score_levels": levels,
            }

        max_workers = max(1, min(8, os.cpu_count() or 1))
        with ThreadPoolExecutor(max_workers=max_workers) as executor:
            rows.extend(
                executor.map(
                    evaluate_outer_permutation,
                    enumerate(permuted_labels),
                )
            )
        for within_sample_row, (observed_label, score, source_row) in enumerate(
            zip(design_labels, continuous_scores, selected_rows)
        ):
            source_rows.append(
                {
                    "design": label,
                    "within_sample_row": within_sample_row,
                    "source_row_index": source_row,
                    "observed_label": int(observed_label),
                    "score_logistic_regression": score,
                }
            )
    return pd.DataFrame(rows), pd.DataFrame(source_rows)


def plot_null_reanchoring(rows: pd.DataFrame) -> None:
    order = list(dict.fromkeys(rows["design"]))
    x = np.arange(len(order))
    colors = [ORANGE, BLUE, PURPLE, GREEN]
    figure, axes = plt.subplots(1, 2, figsize=(12.2, 4.8), sharey=True)
    for axis, column, title in (
        (
            axes[0],
            "supported_cnap",
            "A. Before calibration: chance centers differ",
        ),
        (
            axes[1],
            "calibrated_supported_cnap",
            "B. After calibration: chance is re-centered at zero",
        ),
    ):
        data = [
            rows.loc[rows["design"].eq(design), column].to_numpy()
            for design in order
        ]
        violins = axis.violinplot(
            data,
            positions=x,
            widths=0.78,
            showmeans=False,
            showmedians=False,
            showextrema=False,
        )
        for body, color in zip(violins["bodies"], colors):
            body.set_facecolor(color)
            body.set_edgecolor(color)
            body.set_alpha(0.28)
        for position, values, color in zip(x, data, colors):
            mean = float(np.mean(values))
            mean_standard_error = float(np.std(values, ddof=1) / np.sqrt(len(values)))
            axis.errorbar(
                position,
                mean,
                yerr=1.96 * mean_standard_error,
                fmt="_",
                markersize=20,
                color=color,
                markeredgewidth=2.5,
                elinewidth=1.8,
                capsize=4,
                zorder=3,
            )
        axis.axhline(0.0, color="black", linewidth=1.2, linestyle="--")
        axis.set_xticks(x, [value.replace(" · ", "\n") for value in order])
        axis.set_title(title)
        axis.set_ylabel("Metric value under permuted labels")
    axes[0].text(
        0.02,
        0.03,
        "Raw zero is not the design's chance center",
        transform=axes[0].transAxes,
        color=GRAY,
    )
    axes[1].text(
        0.98,
        0.03,
        "Whiskers: mean ± 1.96 SE\nConditional chance → 0; perfect separation → 1",
        transform=axes[1].transAxes,
        ha="right",
        color=GRAY,
    )
    figure.suptitle(
        "Conditional-null calibration gives zero a consistent interpretation",
        fontsize=13,
        y=1.05,
    )
    figure.text(
        0.5,
        0.985,
        "Bank Marketing logistic-regression scores; labels permuted within held-out samples",
        ha="center",
        color=GRAY,
        fontsize=9,
    )
    figure.tight_layout()
    save_figure(figure, "04-calibration-restores-zero")


def real_sparse_evaluations(
    cli: Path,
    settings: MonteCarloSettings,
    labels: np.ndarray,
    scores: np.ndarray,
    reference_prevalence: float,
) -> pd.DataFrame:
    rows: list[dict] = []
    positive_pool = scores[labels]
    negative_pool = scores[~labels]
    for grid_index, n_positive in enumerate(POSITIVE_COUNT_GRID):
        n = round(n_positive / reference_prevalence)
        n_negative = n - n_positive
        if n_positive > len(positive_pool) or n_negative > len(negative_pool):
            raise ValueError("requested real-data evaluation exceeds temporal test pool")
        canonical_labels = np.r_[
            np.ones(n_positive, dtype=bool),
            np.zeros(n_negative, dtype=bool),
        ]
        rng = np.random.default_rng(MASTER_SEED + 60_000 + grid_index)
        score_map = {
            f"sample_{repeat:02d}": np.r_[
                rng.choice(positive_pool, n_positive, replace=False),
                rng.choice(negative_pool, n_negative, replace=False),
            ]
            for repeat in range(settings.real_repetitions)
        }
        report = run_cli(
            cli,
            f"real_sparse_pos_{n_positive}",
            canonical_labels,
            score_map,
            reference_prevalence,
            settings.real_bootstrap,
            settings.real_permutations,
            5_000 + grid_index,
        )
        for repeat, (name, values) in enumerate(score_map.items()):
            rows.append(
                {
                    **flatten_metric(
                        f"real_sparse_pos_{n_positive}",
                        report,
                        name,
                        canonical_labels,
                        values,
                        reference_prevalence,
                    ),
                    "repeat": repeat,
                }
            )
    return pd.DataFrame(rows)


def quantile_summary(
    rows: pd.DataFrame, column: str
) -> tuple[np.ndarray, np.ndarray, np.ndarray]:
    grouped = rows.groupby("n_positive")[column]
    median = grouped.median().reindex(POSITIVE_COUNT_GRID).to_numpy()
    lower = grouped.quantile(0.10).reindex(POSITIVE_COUNT_GRID).to_numpy()
    upper = grouped.quantile(0.90).reindex(POSITIVE_COUNT_GRID).to_numpy()
    return median, lower, upper


def plot_real_sparse_evaluations(rows: pd.DataFrame) -> None:
    x = np.asarray(POSITIVE_COUNT_GRID, dtype=float)
    figure, axes = plt.subplots(
        1,
        2,
        figsize=(12.2, 4.8),
        gridspec_kw={"width_ratios": (1.05, 1.0)},
    )

    summary = rows.groupby("n_positive").median(numeric_only=True)
    axis = axes[0]
    display_counts = (5, 100)
    y_positions = (1.0, 0.0)
    quantities = (
        ("chance_normalized_ap", BLUE, "o", "Observed CNAP"),
        ("supported_cnap", VERMILLION, "D", "Supported CNAP"),
        ("calibrated_supported_cnap", PURPLE, "s", "Calibrated supported CNAP"),
    )
    for y, n_positive in zip(y_positions, display_counts):
        values = [
            float(summary.loc[n_positive, column])
            for column, _, _, _ in quantities
        ]
        axis.annotate(
            "",
            xy=(values[1], y),
            xytext=(values[0], y),
            arrowprops={
                "arrowstyle": "->",
                "color": GRAY,
                "linewidth": 1.5,
                "shrinkA": 7,
                "shrinkB": 7,
            },
            zorder=2,
        )
        axis.annotate(
            "",
            xy=(values[2], y),
            xytext=(values[1], y),
            arrowprops={
                "arrowstyle": "->",
                "color": GRAY,
                "linewidth": 1.5,
                "shrinkA": 7,
                "shrinkB": 7,
            },
            zorder=2,
        )
        for quantity_index, ((_, color, marker, label), value) in enumerate(
            zip(quantities, values)
        ):
            axis.scatter(
                value,
                y,
                color=color,
                marker=marker,
                s=78,
                edgecolor="white",
                linewidth=0.8,
                zorder=4,
                label=label if n_positive == display_counts[0] else None,
            )
            if quantity_index == 0:
                text_offset = (7, 13)
                horizontal_alignment = "left"
                vertical_alignment = "bottom"
            elif quantity_index == 1:
                text_offset = (0, -17)
                horizontal_alignment = "center"
                vertical_alignment = "top"
            else:
                text_offset = (-7, 13)
                horizontal_alignment = "right"
                vertical_alignment = "bottom"
            axis.annotate(
                f"{value:.3f}",
                (value, y),
                xytext=text_offset,
                textcoords="offset points",
                ha=horizontal_alignment,
                va=vertical_alignment,
                color=color,
                fontsize=9,
            )
    five_positive_values = [
        float(summary.loc[5, column])
        for column, _, _, _ in quantities
    ]
    axis.annotate(
        "replication\nsupport",
        (
            np.mean(five_positive_values[:2]),
            y_positions[0],
        ),
        xytext=(0, 30),
        textcoords="offset points",
        ha="center",
        color=GRAY,
        fontsize=8,
    )
    axis.annotate(
        "conditional-null\ncalibration",
        (
            np.mean(five_positive_values[1:]),
            y_positions[0],
        ),
        xytext=(0, -42),
        textcoords="offset points",
        ha="center",
        color=GRAY,
        fontsize=8,
    )
    axis.set_yticks(
        y_positions,
        [
            f"5 positives\n(n={int(summary.loc[5, 'n'])})",
            f"100 positives\n(n={int(summary.loc[100, 'n']):,})",
        ],
    )
    axis.set_xlabel("Reported CNAP quantity")
    axis.set_xlim(0.07, 0.20)
    axis.set_ylim(-0.45, 1.45)
    axis.set_title("A. The reporting sequence at two evaluation sizes")
    axis.legend(
        loc="upper center",
        bbox_to_anchor=(0.5, -0.17),
        ncol=3,
        frameon=False,
        fontsize=8,
    )
    axis.spines[["right", "top"]].set_visible(False)

    axis = axes[1]
    adjustment_rows = rows.copy()
    adjustment_rows["total_adjustment"] = (
        adjustment_rows["chance_normalized_ap"]
        - adjustment_rows["calibrated_supported_cnap"]
    )
    median, lower, upper = quantile_summary(
        adjustment_rows,
        "total_adjustment",
    )
    axis.fill_between(x, lower, upper, color=PURPLE, alpha=0.16, linewidth=0)
    axis.plot(
        x,
        median,
        marker="o",
        linewidth=2.4,
        color=PURPLE,
    )
    axis.set_xscale("log")
    axis.set_xticks(x, [str(value) for value in POSITIVE_COUNT_GRID])
    axis.set_xlabel("Positive cases in the evaluation")
    axis.set_ylabel(
        "Observed CNAP − calibrated supported CNAP"
    )
    axis.set_ylim(bottom=0.0)
    axis.set_title(
        "B. In these evaluations, the total adjustment shrinks\n"
        "as the positive-case count increases"
    )
    axis.annotate(
        f"{median[0]:.3f}",
        (x[0], median[0]),
        xytext=(8, 8),
        textcoords="offset points",
        color=PURPLE,
        fontsize=9,
    )
    axis.annotate(
        f"{median[-1]:.3f}",
        (x[-1], median[-1]),
        xytext=(-8, 8),
        textcoords="offset points",
        ha="right",
        color=PURPLE,
        fontsize=9,
    )
    axis.text(
        0.98,
        0.92,
        "Median paired adjustment\nBand: 10th–90th percentile",
        transform=axis.transAxes,
        ha="right",
        va="top",
        color=GRAY,
        fontsize=9,
    )
    axis.spines[["right", "top"]].set_visible(False)
    figure.suptitle(
        "Sparse positive outcomes make the complete adjustment consequential",
        fontsize=13,
        y=1.05,
    )
    figure.text(
        0.5,
        0.985,
        "Bank Marketing model: logistic regression",
        ha="center",
        color=GRAY,
        fontsize=9,
    )
    figure.tight_layout()
    save_figure(figure, "05-real-data-sparse-outcomes")


COMPARISONS = (
    {
        "comparison": "Marginal upgrade",
        "candidate_col": "random_forest",
        "baseline_col": "full_logistic",
        "label": "Random forest − full logistic",
        "color": ORANGE,
    },
    {
        "comparison": "Substantive upgrade",
        "candidate_col": "full_logistic",
        "baseline_col": "demographic_logistic",
        "label": "Full logistic − demographic logistic",
        "color": GREEN,
    },
)


def model_comparison_experiment(
    cli: Path,
    settings: MonteCarloSettings,
    labels: np.ndarray,
    predictions: Mapping[str, np.ndarray],
    reference_prevalence: float,
) -> tuple[pd.DataFrame, pd.DataFrame, pd.DataFrame]:
    metric_rows: list[dict] = []
    paired_rows: list[dict] = []
    distribution_rows: list[dict] = []
    for index, comparison in enumerate(COMPARISONS):
        candidate = comparison["candidate_col"]
        baseline = comparison["baseline_col"]
        context = f"model_comparison_{comparison['comparison'].lower().replace(' ', '_')}"
        score_map = {
            candidate: predictions[candidate],
            baseline: predictions[baseline],
        }
        report = run_cli(
            cli,
            context,
            labels,
            score_map,
            reference_prevalence,
            settings.matched_bootstrap,
            settings.matched_permutations,
            8_000 + index,
            paired_baseline=baseline,
        )
        for score_col, values in score_map.items():
            metric_rows.append(
                {
                    **flatten_metric(
                        context,
                        report,
                        score_col,
                        labels,
                        values,
                        reference_prevalence,
                    ),
                    "model": score_col,
                }
            )
        paired_rows.append(
            {
                **flatten_paired_metric(
                    context,
                    comparison["comparison"],
                    report,
                    candidate,
                    baseline,
                ),
                "label": comparison["label"],
            }
        )
        differences = paired_bootstrap_cnap_differences(
            labels,
            predictions[candidate],
            predictions[baseline],
            reference_prevalence,
            5_000,
            MASTER_SEED + 81_000 + index,
        )
        distribution_rows.extend(
            {
                "comparison": comparison["comparison"],
                "label": comparison["label"],
                "replicate": replicate,
                "cnap_difference": difference,
            }
            for replicate, difference in enumerate(differences)
        )
    metrics = pd.DataFrame(metric_rows).drop_duplicates("model")
    return metrics, pd.DataFrame(paired_rows), pd.DataFrame(distribution_rows)


def plot_model_comparison(
    paired: pd.DataFrame,
    reference_prevalence: float,
) -> None:
    comparison_order = [item["comparison"] for item in COMPARISONS]
    comparison_labels = [
        "Random forest over full logistic",
        "Full logistic over demographic logistic",
    ]
    comparison_colors = [item["color"] for item in COMPARISONS]
    figure, axis = plt.subplots(figsize=(10.8, 5.5))
    y_positions = np.array([1.0, 0.0])
    for y, comparison, comparison_label, color in zip(
        y_positions,
        comparison_order,
        comparison_labels,
        comparison_colors,
    ):
        row = paired.loc[paired["comparison"].eq(comparison)].iloc[0]
        observed = row["observed_cnap_difference"]
        supported = row["estimated_supported_superiority"]
        discounted = observed - supported
        supported_share = np.clip(supported / observed, 0.0, 1.0)
        discounted_share = 1.0 - supported_share
        supported_percent = 100.0 * supported_share
        discounted_percent = 100.0 * discounted_share

        axis.barh(
            y,
            supported_percent,
            height=0.34,
            color=color,
            alpha=0.88,
            zorder=2,
        )
        axis.barh(
            y,
            discounted_percent,
            left=supported_percent,
            height=0.34,
            color=LIGHT_GRAY,
            zorder=2,
        )
        axis.barh(
            y,
            100.0,
            height=0.34,
            facecolor="none",
            edgecolor=GRAY,
            linewidth=0.8,
            zorder=3,
        )
        axis.text(
            0.0,
            y + 0.31,
            f"{comparison_label}  ·  observed advantage {observed:+.4f}",
            ha="left",
            va="bottom",
            fontsize=11,
        )
        axis.text(
            supported_percent / 2.0,
            y,
            f"SUPPORTED\n{supported:+.4f}  ({supported_percent:.0f}%)",
            ha="center",
            va="center",
            color="white",
            fontsize=9,
            fontweight="bold",
        )
        if discounted_percent >= 14.0:
            axis.text(
                supported_percent + discounted_percent / 2.0,
                y,
                f"DISCOUNTED\n{discounted:+.4f}  ({discounted_percent:.0f}%)",
                ha="center",
                va="center",
                color=GRAY,
                fontsize=9,
            )
        else:
            axis.annotate(
                f"discounted {discounted:+.4f} ({discounted_percent:.0f}%)",
                (supported_percent + discounted_percent / 2.0, y),
                xytext=(98.0, y - 0.30),
                ha="right",
                va="top",
                color=GRAY,
                fontsize=9,
                arrowprops={
                    "arrowstyle": "-",
                    "color": GRAY,
                    "linewidth": 0.8,
                },
            )

    axis.set_xlim(0.0, 100.0)
    axis.set_ylim(-0.58, 1.58)
    axis.set_yticks([])
    axis.xaxis.set_major_formatter(PercentFormatter())
    axis.set_xlabel("Share of the observed CNAP advantage")
    axis.spines[["left", "right", "top"]].set_visible(False)
    figure.suptitle(
        "Supported superiority is the part of an observed model advantage\n"
        "that survives the replication-support adjustment",
        fontsize=14,
        y=1.02,
    )
    figure.text(
        0.5,
        0.925,
        f"UCI Bank Marketing temporal holdout · paired rows · reference prevalence "
        f"{reference_prevalence:.1%} · each full bar is the observed paired CNAP "
        "advantage",
        ha="center",
        color=GRAY,
        fontsize=9,
    )
    figure.tight_layout()
    save_figure(figure, "06-supported-model-superiority")


def comparison_information_experiment(
    cli: Path,
    settings: MonteCarloSettings,
    labels: np.ndarray,
    predictions: Mapping[str, np.ndarray],
    reference_prevalence: float,
) -> pd.DataFrame:
    rows: list[dict] = []
    positions = np.arange(len(labels))
    for grid_index, n_positive in enumerate(POSITIVE_COUNT_GRID):
        for repeat in range(settings.comparison_repetitions):
            canonical_labels, _, selected_positions = heldout_design_sample(
                labels,
                predictions["full_logistic"],
                positions,
                n_positive,
                reference_prevalence,
                MASTER_SEED + 90_000 + grid_index * 1_000 + repeat,
            )
            context = f"paired_information_pos_{n_positive}_repeat_{repeat:02d}"
            score_map = {
                "random_forest": predictions["random_forest"][selected_positions],
                "full_logistic": predictions["full_logistic"][selected_positions],
            }
            report = run_cli(
                cli,
                context,
                canonical_labels,
                score_map,
                reference_prevalence,
                settings.comparison_bootstrap,
                2,
                9_000 + grid_index * 100 + repeat,
                paired_baseline="full_logistic",
            )
            rows.append(
                {
                    **flatten_paired_metric(
                        context,
                        "Marginal upgrade",
                        report,
                        "random_forest",
                        "full_logistic",
                    ),
                    "n": len(canonical_labels),
                    "n_positive": n_positive,
                    "repeat": repeat,
                }
            )
    return pd.DataFrame(rows)


def plot_comparison_information(
    rows: pd.DataFrame,
    full_test_pair: pd.Series,
    full_test_positive_count: int,
) -> None:
    summary = rows.groupby("n_positive").agg(
        observed=("observed_cnap_difference", "median"),
        observed_lower=("observed_cnap_difference", lambda values: values.quantile(0.10)),
        observed_upper=("observed_cnap_difference", lambda values: values.quantile(0.90)),
        supported=("estimated_supported_superiority", "median"),
        supported_lower=(
            "estimated_supported_superiority",
            lambda values: values.quantile(0.10),
        ),
        supported_upper=(
            "estimated_supported_superiority",
            lambda values: values.quantile(0.90),
        ),
    )
    evaluation_labels = [
        *(f"{value} positives" for value in POSITIVE_COUNT_GRID),
        f"Full test ({full_test_positive_count:,})",
    ]
    observed = [
        *(summary.loc[value, "observed"] for value in POSITIVE_COUNT_GRID),
        full_test_pair["observed_cnap_difference"],
    ]
    supported = [
        *(summary.loc[value, "supported"] for value in POSITIVE_COUNT_GRID),
        full_test_pair["estimated_supported_superiority"],
    ]
    y_positions = 1.15 * np.arange(len(evaluation_labels))[::-1]
    figure, axis = plt.subplots(figsize=(10.4, 7.0))

    for index, (y, observed_value, supported_value) in enumerate(
        zip(y_positions, observed, supported)
    ):
        axis.annotate(
            "",
            xy=(supported_value, y),
            xytext=(observed_value, y),
            arrowprops={
                "arrowstyle": "->",
                "color": GRAY,
                "linewidth": 1.4,
                "shrinkA": 4,
                "shrinkB": 4,
            },
            zorder=2,
        )
        if index < len(POSITIVE_COUNT_GRID):
            n_positive = POSITIVE_COUNT_GRID[index]
            axis.hlines(
                y + 0.09,
                summary.loc[n_positive, "observed_lower"],
                summary.loc[n_positive, "observed_upper"],
                color=BLUE,
                linewidth=3.0,
                alpha=0.22,
                zorder=1,
            )
            axis.hlines(
                y - 0.09,
                summary.loc[n_positive, "supported_lower"],
                summary.loc[n_positive, "supported_upper"],
                color=VERMILLION,
                linewidth=3.0,
                alpha=0.22,
                zorder=1,
            )
        axis.scatter(
            observed_value,
            y,
            s=70,
            color=BLUE,
            marker="o",
            edgecolor="white",
            linewidth=0.8,
            zorder=4,
        )
        axis.scatter(
            supported_value,
            y,
            s=70,
            color=VERMILLION,
            marker="s",
            edgecolor="white",
            linewidth=0.8,
            zorder=4,
        )

    axis.axvline(0.0, color="black", linewidth=1.2, linestyle="--")
    axis.set_yticks(y_positions, evaluation_labels)
    axis.set_xlabel(
        "Paired CNAP difference: full logistic favored  ←  0  →  random forest favored"
    )
    axis.annotate(
        f"observed {full_test_pair['observed_cnap_difference']:+.4f}\n"
        f"supported {full_test_pair['estimated_supported_superiority']:+.4f}",
        (full_test_pair["observed_cnap_difference"], y_positions[-1]),
        xytext=(18, -4),
        textcoords="offset points",
        ha="left",
        va="center",
        color=GRAY,
        fontsize=9,
    )
    figure.suptitle(
        "Observed model difference and supported superiority are different claims\n"
        "Bank Marketing: random forest − full logistic; the same fitted models are "
        "used in every row",
        fontsize=13,
        y=0.98,
    )
    axis.text(
        0.0,
        -0.14,
        "Thin colored intervals: 10th–90th percentiles over repeated real held-out "
        "subsamples; the full test is one fixed evaluation.",
        transform=axis.transAxes,
        color=GRAY,
        fontsize=9,
    )
    axis.spines[["right", "top"]].set_visible(False)
    figure.subplots_adjust(left=0.18, right=0.97, top=0.76, bottom=0.16)
    figure.text(
        0.28,
        0.81,
        "●  Observed model difference",
        ha="center",
        color=BLUE,
        fontsize=10,
    )
    figure.text(
        0.54,
        0.81,
        "■  Supported superiority",
        ha="center",
        color=VERMILLION,
        fontsize=10,
    )
    figure.text(
        0.80,
        0.81,
        "←  replication-support adjustment",
        ha="center",
        color=GRAY,
        fontsize=9,
    )
    save_figure(figure, "07-evidence-for-model-superiority")


def target_prevalence_experiment(
    cli: Path,
    settings: MonteCarloSettings,
    labels: np.ndarray,
    predictions: Mapping[str, np.ndarray],
    reference_prevalence: float,
) -> pd.DataFrame:
    prevalence_profile = tuple(
        sorted({*TARGET_PREVALENCE_GRID, reference_prevalence})
    )
    context = "target_prevalence_model_comparison"
    report = run_cli(
        cli,
        context,
        labels,
        {
            "random_forest": predictions["random_forest"],
            "full_logistic": predictions["full_logistic"],
        },
        prevalence_profile,
        settings.matched_bootstrap,
        2,
        10_000,
        paired_baseline="full_logistic",
    )
    matches = [
        row
        for row in report["paired"]
        if row["model_a"] == "random_forest"
        and row["model_b"] == "full_logistic"
        and row["reference_prevalence_mode"] != "observed"
    ]
    if len(matches) != len(prevalence_profile) or any(
        row["status"] != "ok" for row in matches
    ):
        raise RuntimeError(
            "expected one successful paired comparison at every target prevalence"
        )
    return pd.DataFrame(
        [
            {
                "context": context,
                "comparison": "Random forest − full logistic",
                "candidate_col": row["model_a"],
                "baseline_col": row["model_b"],
                "reference_prevalence": row["reference_prevalence"],
                "observed_cnap_difference": row["observed_cnap_difference"],
                "replicate_mean_cnap_difference": row[
                    "replicate_mean_cnap_difference"
                ],
                "estimated_supported_superiority": row[
                    "estimated_supported_superiority"
                ],
                "support_penalty": row["support_penalty"],
                "estimated_supported_superiority_mc_se": row[
                    "estimated_supported_superiority_mc_se"
                ],
            }
            for row in matches
        ]
    ).sort_values("reference_prevalence")


def plot_target_prevalence_sensitivity(
    rows: pd.DataFrame,
    study_reference_prevalence: float,
) -> None:
    x = rows["reference_prevalence"].to_numpy()
    observed = rows["observed_cnap_difference"].to_numpy()
    supported = rows["estimated_supported_superiority"].to_numpy()
    reference_row = rows.iloc[
        np.argmin(np.abs(rows["reference_prevalence"] - study_reference_prevalence))
    ]

    figure, axis = plt.subplots(figsize=(10.4, 5.8))
    axis.fill_between(
        x,
        supported,
        observed,
        color=LIGHT_GRAY,
        alpha=0.65,
        linewidth=0,
        label="Discounted by replication support",
    )
    axis.plot(
        x,
        observed,
        color=BLUE,
        marker="o",
        markevery=4,
        markersize=4,
        linewidth=2.2,
    )
    axis.plot(
        x,
        supported,
        color=VERMILLION,
        marker="s",
        markevery=4,
        markersize=4,
        linewidth=2.2,
    )
    axis.axhline(0.0, color="black", linewidth=1.2, linestyle="--")
    axis.axvline(
        study_reference_prevalence,
        color=GRAY,
        linewidth=1.0,
        linestyle=":",
    )
    axis.scatter(
        [study_reference_prevalence],
        [reference_row["observed_cnap_difference"]],
        color=BLUE,
        s=58,
        edgecolor="white",
        linewidth=0.8,
        zorder=5,
    )
    axis.scatter(
        [study_reference_prevalence],
        [reference_row["estimated_supported_superiority"]],
        color=VERMILLION,
        marker="s",
        s=58,
        edgecolor="white",
        linewidth=0.8,
        zorder=5,
    )

    axis.set_xscale("log")
    axis.set_xticks(
        [0.01, 0.02, 0.05, 0.10, 0.20, 0.50],
        ["1%", "2%", "5%", "10%", "20%", "50%"],
    )
    axis.set_xlabel("Reference prevalence in the target population")
    axis.set_ylabel("Random forest − full logistic CNAP")
    axis.set_title(
        "The supported model-replacement claim depends on target-population prevalence\n"
        "Bank Marketing: fitted models, temporal holdout rows, and paired bootstrap "
        "resamples are fixed"
    )
    axis.annotate(
        "Observed target-standardized\ndifference",
        (x[-1], observed[-1]),
        xytext=(-8, 18),
        textcoords="offset points",
        ha="right",
        color=BLUE,
        fontsize=9,
    )
    axis.annotate(
        "Supported superiority",
        (x[-1], supported[-1]),
        xytext=(-8, 14),
        textcoords="offset points",
        ha="right",
        va="bottom",
        color=VERMILLION,
        fontsize=9,
    )
    axis.annotate(
        "Discounted by\nreplication support",
        (0.16, np.interp(0.16, x, (observed + supported) / 2.0)),
        xytext=(16, 24),
        textcoords="offset points",
        color=GRAY,
        fontsize=9,
        arrowprops={"arrowstyle": "-", "color": GRAY, "linewidth": 0.8},
    )
    axis.annotate(
        f"Study reference {study_reference_prevalence:.1%}\n"
        f"observed {reference_row['observed_cnap_difference']:+.4f}\n"
        f"supported {reference_row['estimated_supported_superiority']:+.4f}",
        (
            study_reference_prevalence,
            reference_row["estimated_supported_superiority"],
        ),
        xytext=(-22, -54),
        textcoords="offset points",
        ha="right",
        color=GRAY,
        fontsize=9,
        arrowprops={"arrowstyle": "-", "color": GRAY, "linewidth": 0.8},
    )
    axis.text(
        0.0,
        -0.19,
        "Interpretation assumes the class-conditional score distributions transport "
        "and prevalence is the population change.",
        transform=axis.transAxes,
        color=GRAY,
        fontsize=9,
    )
    axis.spines[["right", "top"]].set_visible(False)
    figure.subplots_adjust(left=0.12, right=0.97, top=0.82, bottom=0.22)
    save_figure(figure, "08-target-prevalence-model-superiority")


def make_report(
    profile: str,
    frame: pd.DataFrame,
    train: pd.DataFrame,
    test: pd.DataFrame,
    reference_prevalence: float,
    full_test: pd.DataFrame,
    prevalence: pd.DataFrame,
    matched: pd.DataFrame,
    null_grid: pd.DataFrame,
    null_reanchoring: pd.DataFrame,
    real_sparse: pd.DataFrame,
    paired_comparisons: pd.DataFrame,
    comparison_information: pd.DataFrame,
    target_prevalence: pd.DataFrame,
) -> None:
    prevalence_summary = prevalence.groupby("target_prevalence").mean(numeric_only=True)
    matched_ordered = matched.set_index("design")
    smallest_null = null_grid.loc[null_grid["n_positive"].eq(5)]
    largest_null = null_grid.loc[null_grid["n_positive"].eq(100)]
    raw_null_means = null_reanchoring.groupby("design")["supported_cnap"].mean()
    calibrated_null_means = null_reanchoring.groupby("design")[
        "calibrated_supported_cnap"
    ].mean()
    real_summary = real_sparse.groupby("n_positive").median(numeric_only=True)
    real_total_adjustment = (
        real_sparse["chance_normalized_ap"]
        - real_sparse["calibrated_supported_cnap"]
    ).groupby(real_sparse["n_positive"]).median()
    paired_summary = paired_comparisons.set_index("comparison")
    marginal_pair = paired_summary.loc["Marginal upgrade"]
    substantive_pair = paired_summary.loc["Substantive upgrade"]
    information_summary = comparison_information.groupby("n_positive").median(
        numeric_only=True
    )
    target_reference_row = target_prevalence.iloc[
        np.argmin(
            np.abs(
                target_prevalence["reference_prevalence"].to_numpy()
                - reference_prevalence
            )
        )
    ]
    target_low_row = target_prevalence.iloc[0]
    target_high_row = target_prevalence.iloc[-1]
    supported_target_rows = target_prevalence.loc[
        target_prevalence["estimated_supported_superiority"].gt(0.0)
    ]
    lines = [
        "# Supported AP story-study results",
        "",
        f"Profile: `{profile}`. Generated by `domain_study/story_study.py`.",
        "",
        "## Design",
        "",
        f"The first 80% of {len(frame):,} chronological Bank Marketing records "
        f"(n={len(train):,}) trained three fixed model specifications: full logistic "
        "regression, random forest, and demographic-only logistic regression. The "
        f"final 20% (n={len(test):,}) supplied untouched class-conditional score "
        "pools. `duration` was excluded. The training prevalence "
        f"({reference_prevalence:.3%}) is the real-data reference prevalence.",
        "",
        "The figures are deliberately ordered as a story: a known AP problem, a "
        "limitation of point estimates, a previously hidden design-specific chance "
        "anchor, the calibration that resolves it, a real rare-outcome example, and "
        "finally the model-replacement decision enabled by supported superiority.",
        "",
        "## 1. Prevalence transport",
        "",
        "Traditional AP changes when the same fitted model is evaluated at a new "
        "outcome prevalence. Prior standardization largely removes that mechanical "
        "movement because every sample is read at the training-period prevalence.",
        "",
        f"- At 2% prevalence, mean traditional AP was "
        f"{prevalence_summary.loc[0.02, 'traditional_ap']:.3f}; at 40% it was "
        f"{prevalence_summary.loc[0.40, 'traditional_ap']:.3f}.",
        f"- The corresponding standardized AP values were "
        f"{prevalence_summary.loc[0.02, 'prior_standardized_ap']:.3f} and "
        f"{prevalence_summary.loc[0.40, 'prior_standardized_ap']:.3f}.",
        "",
        "![Prevalence transport](figures/01-prevalence-transport.png)",
        "",
        "## 2. Same observed performance, different support",
        "",
        "Two stratified samples of held-out Bank Marketing predictions were selected "
        "to have nearly identical observed CNAP and bootstrap means but different "
        "empirical replication distributions. This isolates what supported CNAP "
        "adds: it distinguishes a performance level that repeatedly survives from "
        "the same point estimate carried by a fragile real-data ranking pattern.",
        "",
        f"- More reproducible: observed "
        f"{matched_ordered.loc['stable_pattern', 'chance_normalized_ap']:.3f}, "
        f"bootstrap mean "
        f"{matched_ordered.loc['stable_pattern', 'design_bootstrap_mean']:.3f}, "
        f"supported "
        f"{matched_ordered.loc['stable_pattern', 'design_supported_cnap']:.3f}.",
        f"- Less reproducible: observed "
        f"{matched_ordered.loc['fragile_pattern', 'chance_normalized_ap']:.3f}, "
        f"bootstrap mean "
        f"{matched_ordered.loc['fragile_pattern', 'design_bootstrap_mean']:.3f}, "
        f"supported "
        f"{matched_ordered.loc['fragile_pattern', 'design_supported_cnap']:.3f}.",
        "",
        "![Matched support](figures/02-same-performance-different-support.png)",
        "",
        "## 3. Chance is conditional on the evaluation design",
        "",
        "The heat map uses stratified samples of held-out logistic-regression scores "
        "at the training reference prevalence while varying the positive count and "
        "quantizing those real scores to different resolutions. Under label "
        "permutation, raw supported CNAP is not centered at one universal zero. The "
        "conditional-null anchor changes with finite information and score ties.",
        "",
        f"- With 5 positives, anchors ranged from "
        f"{smallest_null['conditional_null_supported_cnap'].min():+.3f} to "
        f"{smallest_null['conditional_null_supported_cnap'].max():+.3f}.",
        f"- With 100 positives, anchors ranged from "
        f"{largest_null['conditional_null_supported_cnap'].min():+.3f} to "
        f"{largest_null['conditional_null_supported_cnap'].max():+.3f}.",
        "",
        "![Design-specific chance](figures/03-design-specific-chance.png)",
        "",
        "## 4. Calibration restores a common interpretation",
        "",
        "Repeated label permutations within real held-out Bank Marketing score sets "
        "directly verify the point of the calibration. The raw supported score "
        "retains the design-specific offset; rescaling from that conditional chance "
        "anchor to perfect separation re-centers chance at zero for every design "
        "while preserving one as the upper endpoint.",
        "",
        f"- Across the four designs, raw null means ranged from "
        f"{raw_null_means.min():+.3f} to {raw_null_means.max():+.3f}.",
        f"- Calibrated null means ranged from {calibrated_null_means.min():+.3f} "
        f"to {calibrated_null_means.max():+.3f}.",
        "",
        "![Calibration](figures/04-calibration-restores-zero.png)",
        "",
        "## 5. Rare outcomes in the real prediction task",
        "",
        "Repeated samples from the held-out Bank Marketing score pools keep "
        "prevalence near the training reference while changing how many positive "
        "cases inform the evaluation. The reporting sequence is shown directly at "
        "5 and 100 positives, followed by the paired total adjustment across every "
        "evaluation size.",
        "",
        f"- At 5 positives, median observed CNAP was "
        f"{real_summary.loc[5, 'chance_normalized_ap']:.3f}, supported CNAP was "
        f"{real_summary.loc[5, 'supported_cnap']:.3f}, and calibrated supported "
        f"CNAP was {real_summary.loc[5, 'calibrated_supported_cnap']:.3f}.",
        f"- At 100 positives, the corresponding values were "
        f"{real_summary.loc[100, 'chance_normalized_ap']:.3f}, "
        f"{real_summary.loc[100, 'supported_cnap']:.3f}, and "
        f"{real_summary.loc[100, 'calibrated_supported_cnap']:.3f}.",
        f"- The median paired observed-to-calibrated adjustment fell from "
        f"{real_total_adjustment.loc[5]:.3f} to "
        f"{real_total_adjustment.loc[100]:.3f}.",
        "",
        "![Real sparse outcomes](figures/05-real-data-sparse-outcomes.png)",
        "",
        "## 6. Supported model superiority",
        "",
        "Two paired model-replacement questions provide a marginal and a substantive "
        "upgrade. Random forest is compared with the full logistic model; the full "
        "logistic model is separately compared with a demographic-only logistic "
        "baseline. Every paired replication uses the same held-out rows for candidate "
        "and baseline.",
        "",
        f"- Marginal upgrade: observed advantage "
        f"{marginal_pair['observed_cnap_difference']:+.4f} → support adjustment "
        f"{marginal_pair['observed_cnap_difference'] - marginal_pair['estimated_supported_superiority']:+.4f} "
        f"→ supported superiority "
        f"{marginal_pair['estimated_supported_superiority']:+.4f} "
        f"({marginal_pair['estimated_supported_superiority'] / marginal_pair['observed_cnap_difference']:.0%} "
        "of the observed advantage retained).",
        f"- Substantive upgrade: observed advantage "
        f"{substantive_pair['observed_cnap_difference']:+.4f} → support adjustment "
        f"{substantive_pair['observed_cnap_difference'] - substantive_pair['estimated_supported_superiority']:+.4f} "
        f"→ supported superiority "
        f"{substantive_pair['estimated_supported_superiority']:+.4f} "
        f"({substantive_pair['estimated_supported_superiority'] / substantive_pair['observed_cnap_difference']:.0%} "
        "of the observed advantage retained).",
        "",
        "![Supported model superiority](figures/06-supported-model-superiority.png)",
        "",
        "## 7. Evaluation information and the model-comparison claim",
        "",
        "The random forest and full logistic model are held fixed while repeated "
        "paired evaluation samples contain progressively more positive cases. This "
        "changes the information supporting the claim, not either fitted model.",
        "",
        f"- With 5 positives, median observed difference was "
        f"{information_summary.loc[5, 'observed_cnap_difference']:+.4f} and median "
        f"supported superiority was "
        f"{information_summary.loc[5, 'estimated_supported_superiority']:+.4f}.",
        f"- With 100 positives, the corresponding values were "
        f"{information_summary.loc[100, 'observed_cnap_difference']:+.4f} and "
        f"{information_summary.loc[100, 'estimated_supported_superiority']:+.4f}.",
        "",
        "![Evidence for model superiority](figures/07-evidence-for-model-superiority.png)",
        "",
        "## 8. Target-population prevalence and model replacement",
        "",
        "A single paired CLI prevalence sweep reuses the same fitted random forest, "
        "full logistic model, temporal holdout rows, and bootstrap resamples while "
        "changing only the reference prevalence representing the intended target "
        "population. The observed target-standardized difference remains positive "
        "across the plotted range, but supported superiority does not.",
        "",
        f"- At {target_low_row['reference_prevalence']:.0%} target prevalence, the "
        f"observed difference was {target_low_row['observed_cnap_difference']:+.4f} "
        f"and supported superiority was "
        f"{target_low_row['estimated_supported_superiority']:+.4f}.",
        f"- At the study reference prevalence "
        f"({target_reference_row['reference_prevalence']:.1%}), the corresponding "
        f"values were {target_reference_row['observed_cnap_difference']:+.4f} and "
        f"{target_reference_row['estimated_supported_superiority']:+.4f}.",
        f"- At {target_high_row['reference_prevalence']:.0%} target prevalence, the "
        f"corresponding values were "
        f"{target_high_row['observed_cnap_difference']:+.4f} and "
        f"{target_high_row['estimated_supported_superiority']:+.4f}. On the sampled "
        f"grid, supported superiority was positive from approximately "
        f"{supported_target_rows['reference_prevalence'].min():.1%} to "
        f"{supported_target_rows['reference_prevalence'].max():.1%}.",
        "",
        "![Target-prevalence sensitivity](figures/08-target-prevalence-model-superiority.png)",
        "",
        "## Interpretation limits",
        "",
        "- Figure 1 and Figure 5 reuse empirical class-conditional score pools; "
        "they do not create new populations.",
        "- Figure 2 uses selected held-out subsamples to isolate replication support; "
        "selection makes it an illustration rather than a population estimate.",
        "- Figures 3 and 4 use held-out model scores; their label permutations are "
        "controlled conditional-null experiments, not new patient populations.",
        "- Figure 6 uses fixed model specifications; the demographic-only "
        "comparison is a positive control for a clearly substantive upgrade.",
        "- Figure 7 changes evaluation information only. It does not imply that the "
        "underlying models improve or deteriorate with sample size.",
        "- Figure 8 is a prevalence-shift sensitivity analysis. Interpreting the "
        "reference prevalence as a deployment population assumes the "
        "class-conditional score distributions transport; it does not address "
        "arbitrary covariate or concept shift.",
        "- The canonical bootstrap treats evaluation rows as independent units.",
        "- Support is a functional of the declared empirical replication regime, "
        "not a confidence bound.",
        "- Monte Carlo standard errors are numerical uncertainty, not sampling "
        "uncertainty; increase counts for final manuscript estimates.",
        "",
        "Exact CLI inputs, schema-versioned JSON, and analysis-ready tables are "
        "retained under `predictions/`, `reports/`, and `tables/`.",
        "",
        "## Full temporal test context",
        "",
        "| n | positives | test prevalence | reference prevalence | traditional AP | standardized AP |",
        "|---:|---:|---:|---:|---:|---:|",
        (
            f"| {int(full_test.iloc[0]['n']):,} | "
            f"{int(full_test.iloc[0]['positives']):,} | "
            f"{full_test.iloc[0]['test_prevalence']:.4f} | "
            f"{full_test.iloc[0]['training_reference_prevalence']:.4f} | "
            f"{full_test.iloc[0]['traditional_ap']:.4f} | "
            f"{full_test.iloc[0]['standardized_ap']:.4f} |"
        ),
        "",
    ]
    (OUTPUT_DIR / "REPORT.md").write_text("\n".join(lines), encoding="utf-8")


def main() -> None:
    args = parse_args()
    settings = PROFILES[args.profile]
    ensure_dataset()
    prepare_output_directories()
    configure_plots()

    frame = pd.read_csv(CSV_PATH, sep=";")
    split = int(len(frame) * TRAIN_FRACTION)
    train = frame.iloc[:split].copy()
    test = frame.iloc[split:].copy()
    feature_columns = [
        column for column in frame.columns if column not in ("y", "duration")
    ]
    y_train = train["y"].eq("yes").to_numpy()
    y_test = test["y"].eq("yes").to_numpy()
    reference_prevalence = float(y_train.mean())

    models, numeric_columns, categorical_columns = build_models(frame)
    predictions: dict[str, np.ndarray] = {}
    for model_name, model in models.items():
        model.fit(train[feature_columns], y_train)
        predictions[model_name] = model.predict_proba(test[feature_columns])[:, 1]
    test_scores = predictions["full_logistic"]
    full_test = pd.DataFrame(
        [
            {
                "n": len(y_test),
                "positives": int(y_test.sum()),
                "test_prevalence": float(y_test.mean()),
                "training_reference_prevalence": reference_prevalence,
                "traditional_ap": float(average_precision_score(y_test, test_scores)),
                "standardized_ap": standardized_ap(
                    y_test, test_scores, reference_prevalence
                ),
            }
        ]
    )
    save_table(full_test, "full_temporal_test_context.csv")
    save_table(
        pd.DataFrame(
            {
                "row_index": test.index.to_numpy(),
                "label": y_test.astype(np.uint8),
                "score_full_logistic": predictions["full_logistic"],
                "score_random_forest": predictions["random_forest"],
                "score_demographic_logistic": predictions["demographic_logistic"],
            }
        ),
        "temporal_holdout_predictions.csv",
    )

    prevalence = prevalence_transport(y_test, test_scores, reference_prevalence)
    save_table(prevalence, "prevalence_transport.csv")
    plot_prevalence_transport(prevalence, reference_prevalence)

    matched, matched_bootstrap, matched_sources = matched_support_experiment(
        args.cli.resolve(),
        settings,
        y_test,
        test_scores,
        test.index.to_numpy(),
        reference_prevalence,
    )
    save_table(matched, "matched_support_metrics.csv")
    save_table(matched_bootstrap, "matched_bootstrap_distribution.csv")
    save_table(matched_sources, "matched_support_source_rows.csv")
    plot_matched_support(matched, matched_bootstrap)

    null_grid, null_grid_sources = conditional_null_grid(
        args.cli.resolve(),
        settings,
        y_test,
        test_scores,
        test.index.to_numpy(),
        reference_prevalence,
    )
    save_table(null_grid, "conditional_null_grid.csv")
    save_table(null_grid_sources, "conditional_null_grid_source_rows.csv")
    plot_conditional_null_grid(null_grid)

    null_reanchoring, null_reanchoring_sources = null_reanchoring_experiment(
        args.cli.resolve(),
        settings,
        y_test,
        test_scores,
        test.index.to_numpy(),
        reference_prevalence,
    )
    save_table(null_reanchoring, "null_reanchoring.csv")
    save_table(
        null_reanchoring_sources,
        "null_reanchoring_source_rows.csv",
    )
    plot_null_reanchoring(null_reanchoring)

    real_sparse = real_sparse_evaluations(
        args.cli.resolve(),
        settings,
        y_test,
        test_scores,
        reference_prevalence,
    )
    save_table(real_sparse, "real_sparse_evaluations.csv")
    plot_real_sparse_evaluations(real_sparse)

    comparison_metrics, paired_comparisons, comparison_distributions = (
        model_comparison_experiment(
            args.cli.resolve(),
            settings,
            y_test,
            predictions,
            reference_prevalence,
        )
    )
    save_table(comparison_metrics, "model_comparison_metrics.csv")
    save_table(paired_comparisons, "supported_superiority_comparisons.csv")
    save_table(
        comparison_distributions,
        "paired_comparison_bootstrap_distributions.csv",
    )
    plot_model_comparison(
        paired_comparisons,
        reference_prevalence,
    )

    comparison_information = comparison_information_experiment(
        args.cli.resolve(),
        settings,
        y_test,
        predictions,
        reference_prevalence,
    )
    save_table(comparison_information, "model_comparison_information.csv")
    marginal_full_test = paired_comparisons.loc[
        paired_comparisons["comparison"].eq("Marginal upgrade")
    ].iloc[0]
    plot_comparison_information(
        comparison_information,
        marginal_full_test,
        int(y_test.sum()),
    )

    target_prevalence = target_prevalence_experiment(
        args.cli.resolve(),
        settings,
        y_test,
        predictions,
        reference_prevalence,
    )
    save_table(target_prevalence, "target_prevalence_model_superiority.csv")
    plot_target_prevalence_sensitivity(target_prevalence, reference_prevalence)

    manifest = {
        "study": "supported AP eight-figure pedagogical story",
        "profile": args.profile,
        "master_seed": MASTER_SEED,
        "dataset": {
            "name": "UCI Bank Marketing",
            "url": DATA_URL,
            "archive_sha256": ARCHIVE_SHA256,
            "records": len(frame),
            "chronological_train_records": len(train),
            "temporal_test_records": len(test),
        },
        "models": {
            "full_logistic": "one-hot logistic regression using all pre-contact features",
            "random_forest": "one-hot random forest using all pre-contact features",
            "demographic_logistic": (
                "one-hot logistic regression using age, job, marital, and education"
            ),
            "excluded_feature": "duration",
            "numeric_columns": numeric_columns,
            "categorical_columns": categorical_columns,
        },
        "reference_prevalence": reference_prevalence,
        "monte_carlo": asdict(settings),
        "software": {
            "python": platform.python_version(),
            "numpy": np.__version__,
            "pandas": pd.__version__,
            "scikit_learn": sklearn.__version__,
            "matplotlib": matplotlib.__version__,
            "cli": str(args.cli.resolve()),
        },
        "figures": [
            "01-prevalence-transport",
            "02-same-performance-different-support",
            "03-design-specific-chance",
            "04-calibration-restores-zero",
            "05-real-data-sparse-outcomes",
            "06-supported-model-superiority",
            "07-evidence-for-model-superiority",
            "08-target-prevalence-model-superiority",
        ],
    }
    (OUTPUT_DIR / "manifest.json").write_text(
        json.dumps(manifest, indent=2) + "\n", encoding="utf-8"
    )
    make_report(
        args.profile,
        frame,
        train,
        test,
        reference_prevalence,
        full_test,
        prevalence,
        matched,
        null_grid,
        null_reanchoring,
        real_sparse,
        paired_comparisons,
        comparison_information,
        target_prevalence,
    )
    print(f"Wrote story figures and report to {OUTPUT_DIR}")


if __name__ == "__main__":
    main()
