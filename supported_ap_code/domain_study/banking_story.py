#!/usr/bin/env python3
"""Build the Bank Marketing AP and AUROC supported-superiority story from raw data."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import shutil
import subprocess
import sys
import warnings
from dataclasses import asdict, dataclass, replace
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
STUDY = ROOT / "domain_study"
RAW = STUDY / "data" / "raw"
CSV_PATH = RAW / "bank-additional" / "bank-additional-full.csv"
ARCHIVE_PATH = RAW / "bank_marketing.zip"
OUTPUTS = Path(os.environ.get("SUPPORTED_AP_STUDY_OUTPUTS", STUDY / "outputs"))
FIGURES = OUTPUTS / "figures"
TABLES = OUTPUTS / "tables"
REPORTS = OUTPUTS / "reports"
PREDICTIONS = OUTPUTS / "predictions"
MPL_CACHE = STUDY / ".mplconfig"

os.environ.setdefault("MPLBACKEND", "Agg")
os.environ.setdefault("MPLCONFIGDIR", str(MPL_CACHE))

import matplotlib.pyplot as plt  # noqa: E402
import numpy as np  # noqa: E402
import pandas as pd  # noqa: E402
import sklearn  # noqa: E402
from matplotlib.ticker import PercentFormatter  # noqa: E402
from sklearn.compose import ColumnTransformer  # noqa: E402
from sklearn.ensemble import RandomForestClassifier  # noqa: E402
from sklearn.linear_model import LogisticRegression  # noqa: E402
from sklearn.metrics import roc_auc_score  # noqa: E402
from sklearn.pipeline import Pipeline  # noqa: E402
from sklearn.preprocessing import OneHotEncoder, StandardScaler  # noqa: E402

ARCHIVE_SHA256 = "e0bf5f5de5b846e2f18e9d90606637267d46dfa260e0f17bb12e605db5efbeb4"
CSV_SHA256 = "74adfc578bf77a7ff4bb1ba4a9f8709d9e3c6907342959c2c8416847e0afb4d8"
MASTER_SEED = 20260729
TARGET_LOWER = 0.01
TARGET_UPPER = 0.50
SUPPORT_ORDER = 2
MAGNITUDE_THRESHOLD = 0.0
SURVIVAL_FLOOR = 0.0
SURVIVAL_REQUIREMENT = 0.81
AUROC_MAGNITUDE_THRESHOLD = 0.0
AUROC_SURVIVAL_FLOOR = 0.0
AUROC_SURVIVAL_REQUIREMENT = SURVIVAL_REQUIREMENT
AUROC_OPTIMIZATION_ABSOLUTE_GAP = 1e-6
AUROC_OPTIMIZATION_RELATIVE_GAP = 1e-8
AUROC_OPTIMIZATION_TIME_LIMIT_SECONDS = 1.0
TRAIN_FRACTION = 0.80

BLUE = "#0072B2"
ORANGE = "#E69F00"
GREEN = "#009E73"
VERMILLION = "#D55E00"
PURPLE = "#CC79A7"
GRAY = "#5A5A5A"
LIGHT_GRAY = "#D9D9D9"
VERY_LIGHT = "#F2F2F2"

MODEL_LABELS = {
    "score_random_forest": "Random forest",
    "score_full_logistic": "Full logistic",
    "score_demographic_logistic": "Demographic logistic",
}
MODEL_COLORS = {
    "score_random_forest": BLUE,
    "score_full_logistic": ORANGE,
    "score_demographic_logistic": GREEN,
}


@dataclass(frozen=True)
class Profile:
    computational_replications: int
    grid_points: int
    search_tolerance: float
    search_iterations: int
    forest_trees: int
    auroc_computational_replications: int
    auroc_search_tolerance: float


PROFILES = {
    "quick": Profile(80, 33, 1e-6, 64, 300, 24, 0.05),
    "publication": Profile(600, 257, 1e-8, 128, 300, 80, 0.02),
}


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--profile", choices=PROFILES, default="quick")
    parser.add_argument(
        "--cli",
        type=Path,
        default=ROOT / "target" / "release" / "supported_ap",
    )
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument(
        "--render-only",
        action="store_true",
        help="reuse the retained clean publication artifacts and skip fitting/resampling",
    )
    mode.add_argument(
        "--auroc-only",
        action="store_true",
        help="reuse retained predictions and AP reports, run only AUROC, then rerender",
    )
    return parser.parse_args()


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def verify_raw_data() -> None:
    expected = [(ARCHIVE_PATH, ARCHIVE_SHA256), (CSV_PATH, CSV_SHA256)]
    for path, digest in expected:
        if not path.exists():
            raise FileNotFoundError(f"required raw source is missing: {path}")
        actual = sha256(path)
        if actual != digest:
            raise RuntimeError(
                f"raw source checksum mismatch for {path.name}: {actual} != {digest}"
            )


def prepare_outputs() -> None:
    if OUTPUTS.exists():
        shutil.rmtree(OUTPUTS)
    for directory in (FIGURES, TABLES, REPORTS, PREDICTIONS):
        directory.mkdir(parents=True, exist_ok=True)


def load_data() -> pd.DataFrame:
    frame = pd.read_csv(CSV_PATH, sep=";")
    frame["label"] = (frame["y"] == "yes").astype(int)
    return frame


def build_models(frame: pd.DataFrame, profile: Profile) -> dict[str, Pipeline]:
    features = frame.drop(columns=["y", "label", "duration"])
    categorical = features.select_dtypes(include=["object"]).columns.tolist()
    numeric = [column for column in features.columns if column not in categorical]

    def preprocessor(columns: list[str]) -> ColumnTransformer:
        chosen_categorical = [column for column in columns if column in categorical]
        chosen_numeric = [column for column in columns if column in numeric]
        return ColumnTransformer(
            [
                ("numeric", StandardScaler(), chosen_numeric),
                (
                    "categorical",
                    OneHotEncoder(handle_unknown="ignore", sparse_output=False),
                    chosen_categorical,
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

    all_columns = features.columns.tolist()
    demographic = ["age", "job", "marital", "education"]
    return {
        "score_full_logistic": logistic(all_columns),
        "score_random_forest": Pipeline(
            [
                ("preprocess", preprocessor(all_columns)),
                (
                    "model",
                    RandomForestClassifier(
                        n_estimators=profile.forest_trees,
                        min_samples_leaf=20,
                        max_features="sqrt",
                        n_jobs=-1,
                        random_state=MASTER_SEED,
                    ),
                ),
            ]
        ),
        "score_demographic_logistic": logistic(demographic),
    }


def fit_fresh_predictions(frame: pd.DataFrame, profile: Profile) -> tuple[pd.DataFrame, dict]:
    split = int(len(frame) * TRAIN_FRACTION)
    train = frame.iloc[:split].copy()
    test = frame.iloc[split:].copy()
    feature_columns = [
        column for column in frame.columns if column not in {"y", "label", "duration"}
    ]
    models = build_models(frame, profile)
    predictions = pd.DataFrame(
        {
            "row_index": np.arange(split, len(frame)),
            "label": test["label"].to_numpy(),
        }
    )
    for score_column, model in models.items():
        model.fit(train[feature_columns], train["label"])
        with warnings.catch_warnings():
            warnings.filterwarnings(
                "ignore",
                message=".*encountered in matmul",
                category=RuntimeWarning,
                module="sklearn.utils.extmath",
            )
            scores = model.predict_proba(test[feature_columns])[:, 1]
        if not np.isfinite(scores).all() or np.any((scores < 0.0) | (scores > 1.0)):
            raise RuntimeError(f"{score_column} produced invalid probabilities")
        predictions[score_column] = scores

    context = {
        "rows": len(frame),
        "training_rows": len(train),
        "evaluation_rows": len(test),
        "training_positive_count": int(train["label"].sum()),
        "evaluation_positive_count": int(test["label"].sum()),
        "training_prevalence": float(train["label"].mean()),
        "evaluation_prevalence": float(test["label"].mean()),
        "split_index": split,
        "duration_excluded": True,
        "model_training": "fresh_from_raw_data",
    }
    return predictions, context


def configure_plots() -> None:
    plt.rcParams.update(
        {
            "figure.dpi": 140,
            "savefig.dpi": 220,
            "font.size": 10.5,
            "axes.titlesize": 11.5,
            "axes.labelsize": 10.5,
            "legend.fontsize": 9.0,
            "axes.spines.top": False,
            "axes.spines.right": False,
            "axes.grid": True,
            "grid.color": "#E7E7E7",
            "grid.linewidth": 0.7,
            "axes.axisbelow": True,
        }
    )


def save_figure(figure: plt.Figure, stem: str) -> None:
    figure.savefig(FIGURES / f"{stem}.png", bbox_inches="tight")
    figure.savefig(FIGURES / f"{stem}.pdf", bbox_inches="tight")
    plt.close(figure)


def paired_rust_command(
    cli: Path,
    predictions_path: Path,
    output_path: Path,
    profile: Profile,
    candidate: str,
    incumbent: str,
    *,
    resampling_seed: int,
    retain_trace: bool,
) -> list[str]:
    return [
        str(cli),
        "ap",
        "projected",
        "--input",
        str(predictions_path),
        "--output",
        str(output_path),
        "--label-col",
        "label",
        "--model-a-col",
        candidate,
        "--model-b-col",
        incumbent,
        "--interval-lower",
        str(TARGET_LOWER),
        "--interval-upper",
        str(TARGET_UPPER),
        "--search-grid-points",
        str(profile.grid_points),
        "--search-tolerance",
        str(profile.search_tolerance),
        "--search-max-iterations",
        str(profile.search_iterations),
        "--computational-replications",
        str(profile.computational_replications),
        "--computational-order",
        str(SUPPORT_ORDER),
        "--resampling-seed",
        str(resampling_seed),
        "--resampling-unit",
        "independent-observation",
        "--transport-justification",
        (
            "class-conditional score distributions are assumed to transport "
            "over the declared target-prevalence range"
        ),
        "--reference-limitation",
        (
            "the temporal observational holdout may contain heterogeneous or "
            "dependent customer risks; no exchangeable-label law is asserted"
        ),
        *(["--retain-replication-profiles"] if retain_trace else []),
    ]


def run_rust(
    cli: Path,
    predictions_path: Path,
    output_path: Path,
    profile: Profile,
    candidate: str,
    incumbent: str,
    *,
    resampling_seed: int,
    retain_trace: bool,
) -> dict[str, Any]:
    command = paired_rust_command(
        cli,
        predictions_path,
        output_path,
        profile,
        candidate,
        incumbent,
        resampling_seed=resampling_seed,
        retain_trace=retain_trace,
    )
    command.extend(
        [
        "--magnitude-threshold",
        str(MAGNITUDE_THRESHOLD),
        "--survival-floor",
        str(SURVIVAL_FLOOR),
        "--survival-requirement",
        str(SURVIVAL_REQUIREMENT),
        "--execution",
        "parallel",
        ]
    )
    subprocess.run(command, check=True)
    with output_path.open() as handle:
        return json.load(handle)


def auroc_rust_command(
    cli: Path,
    predictions_path: Path,
    output_path: Path,
    profile: Profile,
    candidate: str,
    incumbent: str,
    *,
    resampling_seed: int,
) -> list[str]:
    return [
        str(cli),
        "auroc",
        "projected",
        "--input",
        str(predictions_path),
        "--output",
        str(output_path),
        "--label-col",
        "label",
        "--model-a-col",
        candidate,
        "--model-b-col",
        incumbent,
        "--computational-order",
        str(SUPPORT_ORDER),
        "--magnitude-threshold",
        str(AUROC_MAGNITUDE_THRESHOLD),
        "--survival-floor",
        str(AUROC_SURVIVAL_FLOOR),
        "--survival-requirement",
        str(AUROC_SURVIVAL_REQUIREMENT),
        "--reference-limitation",
        (
            "the temporal observational holdout may contain heterogeneous or "
            "dependent customer risks; no exchangeable-label law is asserted"
        ),
        "--computational-replications",
        str(profile.auroc_computational_replications),
        "--resampling-seed",
        str(resampling_seed),
        "--resampling-unit",
        "independent-observation",
        "--optimization-absolute-gap",
        str(AUROC_OPTIMIZATION_ABSOLUTE_GAP),
        "--optimization-relative-gap",
        str(AUROC_OPTIMIZATION_RELATIVE_GAP),
        "--optimization-time-limit-seconds",
        str(AUROC_OPTIMIZATION_TIME_LIMIT_SECONDS),
        "--solver-threads",
        "1",
        "--concentration-tolerance",
        str(profile.auroc_search_tolerance),
        "--concentration-max-iterations",
        str(profile.search_iterations),
        "--execution",
        "parallel",
    ]


def run_auroc_rust(
    cli: Path,
    predictions_path: Path,
    output_path: Path,
    profile: Profile,
    candidate: str,
    incumbent: str,
    *,
    resampling_seed: int,
) -> dict[str, Any]:
    subprocess.run(
        auroc_rust_command(
            cli,
            predictions_path,
            output_path,
            profile,
            candidate,
            incumbent,
            resampling_seed=resampling_seed,
        ),
        check=True,
    )
    with output_path.open() as handle:
        return json.load(handle)


def staged_evidence(
    report: dict[str, Any], orientation: str = "forward"
) -> tuple[dict[str, Any], dict[str, Any], str]:
    staged = report["result"][orientation]
    if staged["full_assessment"] is not None:
        return staged, staged["full_assessment"], "anchored_full_assessment"
    return staged, staged["observed_gate"], "observed_gate"


def validate_rust_report(report: dict[str, Any], profile: Profile, trace_expected: bool) -> None:
    assessment = report["assessment"]
    result = report["result"]
    target = assessment["target_prevalences"]
    assert report["schema_version"] == 17
    assert report["manuscript_version"] == "v17"
    assert report["analysis"] == "ap_staged_projected_finite_evidence"
    assert target["kind"] == "closed_interval"
    assert target["lower"] == TARGET_LOWER
    assert target["upper"] == TARGET_UPPER
    assert assessment["resampling"]["computational_order"] == SUPPORT_ORDER
    assert assessment["resampling"]["replications"] == profile.computational_replications
    assert result["policy"]["magnitude_threshold"] == MAGNITUDE_THRESHOLD
    assert result["policy"]["survival_floor"] == SURVIVAL_FLOOR
    assert result["policy"]["survival_requirement"] == SURVIVAL_REQUIREMENT
    assert result["empirical_order"] == 1
    assert result["computational_order"] == SUPPORT_ORDER
    assert result["evidence"] == "projected"
    assert result["reference_assessment"]["status"] == "not_asserted"
    any_full_assessment = False
    for orientation in ("forward", "reverse"):
        staged = result[orientation]
        gate = staged["observed_gate"]
        assert gate["literal_survival"]["list_length"] == 1
        full = staged["full_assessment"]
        if gate["verdict"] == "supported_replacement":
            assert full is not None
            any_full_assessment = True
            assert full["literal_survival"]["list_length"] == profile.computational_replications
            anchor = gate["retained_effects"][0]["value"]
            secondary = staged["secondary_unanchored_projection"]
            assert secondary is not None
            unanchored = secondary["retained_effects"]
            for anchored, projected in zip(full["retained_effects"], unanchored):
                assert anchored["value"] <= anchor + 1e-12
                assert anchored["value"] <= projected["value"] + 1e-12
        else:
            assert full is None
            assert staged["secondary_unanchored_projection"] is None
            assert staged["staged_verdict"] == "no_verdict"
    if trace_expected and any_full_assessment:
        assert result["trace"] is not None
        trace = result["trace"]
        assert len(trace["replication_difference_profiles"]) == profile.computational_replications
        assert len(trace["forward_retained_effects"]) == profile.computational_replications
        assert len(trace["prevalences"]) == profile.grid_points
        assert trace["computational_order"] == SUPPORT_ORDER
    else:
        assert result["trace"] is None


def validate_auroc_report(report: dict[str, Any], profile: Profile) -> None:
    assert report["schema_version"] == 17
    assert report["manuscript_version"] == "v17"
    assert report["analysis"] == "auroc_staged_projected_breakdown"
    assessment = report["assessment"]
    estimate = report["result"]
    policy = estimate["policy"]
    counts = assessment["resampling"]["replication_counts"]
    assert assessment["resampling"]["replications"] == profile.auroc_computational_replications
    assert assessment["resampling"]["computational_order"] == SUPPORT_ORDER
    observed = estimate["evaluation_counts"][0]
    assert counts["positive"] == observed["positive"], (
        "projected replications must use the observed positive count (same design)"
    )
    assert counts["negative"] == observed["negative"], (
        "projected replications must use the observed negative count (same design)"
    )
    assert assessment["optimization"]["solver_threads"] == 1
    assert (
        assessment["optimization"]["time_limit_seconds"]
        == AUROC_OPTIMIZATION_TIME_LIMIT_SECONDS
    )
    assert assessment["concentration_search"]["tolerance"] == profile.auroc_search_tolerance
    assert estimate["evidence"] == "projected"
    assert estimate["empirical_order"] == 1
    assert estimate["computational_order"] == SUPPORT_ORDER
    assert policy["magnitude_threshold"] == AUROC_MAGNITUDE_THRESHOLD
    assert policy["survival_floor"] == AUROC_SURVIVAL_FLOOR
    assert policy["survival_requirement"] == AUROC_SURVIVAL_REQUIREMENT
    assert estimate["reference_assessment"]["status"] == "not_asserted"
    gate = estimate["observed_gate"]
    assert gate["list_length"] == 1
    for point in estimate["evaluated_profile"]:
        assert point["supported_magnitude_lower"] <= point["supported_magnitude"]
        assert point["supported_magnitude"] <= point["supported_magnitude_upper"]
        assert point["literal_survival_lower"] <= point["literal_survival_fraction"]
        assert point["literal_survival_fraction"] <= point["literal_survival_upper"]
        expected_length = (
            1
            if estimate["breakdown"]["kind"] == "observed_gate_failure"
            else profile.auroc_computational_replications
        )
        assert point["list_length"] == expected_length

    searched = estimate["evaluated_profile"]
    assert searched[0]["concentration_factor"] == 1.0
    for left, right in zip(searched, searched[1:]):
        assert right["concentration_factor"] > left["concentration_factor"]
        assert right["supported_magnitude"] <= left["supported_magnitude"] + 1e-8
        assert (
            right["literal_survival_fraction"]
            <= left["literal_survival_fraction"] + 1e-12
        )
    breakdown = estimate["breakdown"]
    if breakdown["kind"] == "finite":
        assert breakdown["last_passing"] < breakdown["first_failing"]
        if not breakdown["unresolved_interior"]:
            assert (
                breakdown["first_failing"] - breakdown["last_passing"]
                <= profile.auroc_search_tolerance + 1e-12
            )
    elif breakdown["kind"] in ("at_baseline", "observed_gate_failure"):
        assert estimate["boundary"]["concentration_factor"] == 1.0
    else:
        assert breakdown["kind"] == "no_breakdown_on_empirical_support"


def write_computational_stability(
    cli: Path,
    predictions_path: Path,
    profile: Profile,
    primary: dict[str, Any],
    control: dict[str, Any],
    primary_auroc: dict[str, Any],
    control_auroc: dict[str, Any],
) -> None:
    """Compare the retained run with a second seed and an AP prefix in J."""

    rows: list[dict[str, Any]] = []
    ap_comparisons = [
        (
            "random forest - full logistic",
            "score_random_forest",
            "score_full_logistic",
            primary,
            MASTER_SEED + 101,
            MASTER_SEED + 1101,
        ),
        (
            "full logistic - demographic logistic",
            "score_full_logistic",
            "score_demographic_logistic",
            control,
            MASTER_SEED + 102,
            MASTER_SEED + 1102,
        ),
    ]
    for name, candidate, incumbent, main_report, main_seed, alternate_seed in ap_comparisons:
        alternate = run_rust(
            cli,
            predictions_path,
            REPORTS / f"stability_ap_{candidate}_vs_{incumbent}.json",
            profile,
            candidate,
            incumbent,
            resampling_seed=alternate_seed,
            retain_trace=False,
        )
        validate_rust_report(alternate, profile, False)
        _, main, stage = staged_evidence(main_report)
        _, other, other_stage = staged_evidence(alternate)
        prefix_count = max(SUPPORT_ORDER, profile.computational_replications // 2)
        prefix_profile = replace(profile, computational_replications=prefix_count)
        prefix_report = run_rust(
            cli,
            predictions_path,
            REPORTS / f"stability_ap_{candidate}_vs_{incumbent}_half_j.json",
            prefix_profile,
            candidate,
            incumbent,
            resampling_seed=main_seed,
            retain_trace=False,
        )
        validate_rust_report(prefix_report, prefix_profile, False)
        _, prefix, prefix_stage = staged_evidence(prefix_report)
        if stage != other_stage or stage != prefix_stage:
            raise RuntimeError("AP stability runs reached different V17 stages")
        for metric, main_value, prefix_value, alternate_value in (
            (
                "supported_magnitude",
                main["supported_magnitude"],
                prefix["supported_magnitude"],
                other["supported_magnitude"],
            ),
            (
                "literal_survival_fraction",
                main["literal_survival"]["subset_fraction"],
                prefix["literal_survival"]["subset_fraction"],
                other["literal_survival"]["subset_fraction"],
            ),
        ):
            rows.append(
                {
                    "analysis": "ap_projected",
                    "comparison": name,
                    "v17_stage": stage,
                    "metric": metric,
                    "main_replications": profile.computational_replications,
                    "main_seed": main_seed,
                    "main_value": main_value,
                    "prefix_replications": prefix_count,
                    "prefix_value": prefix_value,
                    "absolute_prefix_difference": abs(main_value - prefix_value),
                    "alternate_seed": alternate_seed,
                    "alternate_value": alternate_value,
                    "absolute_seed_difference": abs(main_value - alternate_value),
                }
            )

    auroc_comparisons = [
        (
            "random forest - full logistic",
            "score_random_forest",
            "score_full_logistic",
            primary_auroc,
            MASTER_SEED + 301,
            MASTER_SEED + 1301,
        ),
        (
            "full logistic - demographic logistic",
            "score_full_logistic",
            "score_demographic_logistic",
            control_auroc,
            MASTER_SEED + 302,
            MASTER_SEED + 1302,
        ),
    ]
    for name, candidate, incumbent, main_report, main_seed, alternate_seed in auroc_comparisons:
        alternate = run_auroc_rust(
            cli,
            predictions_path,
            REPORTS / f"stability_auroc_{candidate}_vs_{incumbent}.json",
            profile,
            candidate,
            incumbent,
            resampling_seed=alternate_seed,
        )
        validate_auroc_report(alternate, profile)
        main = main_report["result"]["baseline"]
        other = alternate["result"]["baseline"]
        for metric in ("supported_magnitude", "literal_survival_fraction"):
            rows.append(
                {
                    "analysis": "auroc_projected_baseline",
                    "comparison": name,
                    "metric": metric,
                    "main_replications": profile.auroc_computational_replications,
                    "main_seed": main_seed,
                    "main_value": main[metric],
                    "prefix_replications": None,
                    "prefix_value": None,
                    "absolute_prefix_difference": None,
                    "alternate_seed": alternate_seed,
                    "alternate_value": other[metric],
                    "absolute_seed_difference": abs(main[metric] - other[metric]),
                }
            )

    pd.DataFrame(rows).to_csv(TABLES / "computational_stability.csv", index=False)


def contact_curves(labels: np.ndarray, scores: np.ndarray) -> tuple[np.ndarray, np.ndarray, np.ndarray]:
    order = np.argsort(-scores, kind="stable")
    ranked = labels[order]
    contacts = np.arange(1, len(labels) + 1)
    cumulative = np.cumsum(ranked)
    fraction_contacted = contacts / len(labels)
    capture = cumulative / labels.sum()
    precision = cumulative / contacts
    return fraction_contacted, capture, precision


def plot_figure_1(predictions: pd.DataFrame, primary: dict[str, Any]) -> None:
    labels = predictions["label"].to_numpy()
    observed = primary["result"]["observed_evaluation"]
    empirical_ap = {
        "score_random_forest": observed["model_a_empirical_ap"],
        "score_full_logistic": observed["model_b_empirical_ap"],
    }
    figure, axes = plt.subplots(1, 2, figsize=(11.5, 4.7))
    for score_column in ("score_random_forest", "score_full_logistic"):
        scores = predictions[score_column].to_numpy()
        x, capture, precision = contact_curves(labels, scores)
        mask = x <= 0.50
        metrics = (
            f"{MODEL_LABELS[score_column]}  "
            f"(AUROC {roc_auc_score(labels, scores):.3f}; "
            f"tie-averaged AP {empirical_ap[score_column]:.3f})"
        )
        axes[0].plot(
            x[mask], capture[mask], lw=2.2, color=MODEL_COLORS[score_column], label=metrics
        )
        axes[1].plot(
            x[mask],
            precision[mask],
            lw=2.2,
            color=MODEL_COLORS[score_column],
            label=MODEL_LABELS[score_column],
        )

    axes[0].plot([0, 0.5], [0, 0.5], ls="--", lw=1.2, color=GRAY, label="Random ordering")
    axes[1].axhline(labels.mean(), ls="--", lw=1.2, color=GRAY, label="Population response rate")
    axes[0].set(
        xlabel="Fraction of customers contacted",
        ylabel="Fraction of all subscribers reached",
        xlim=(0, 0.5),
        ylim=(0, 1.0),
        title="A. How quickly are subscribers found?",
    )
    axes[1].set(
        xlabel="Fraction of customers contacted",
        ylabel="Subscription rate among customers contacted",
        xlim=(0, 0.5),
        ylim=(0, 1.0),
        title="B. How precise is the contacted group?",
    )
    for axis in axes:
        axis.xaxis.set_major_formatter(PercentFormatter(1.0))
        axis.yaxis.set_major_formatter(PercentFormatter(1.0))
        axis.legend(frameon=False, loc="best")
    figure.suptitle(
        "Bank marketing is a retrieval problem: who should be contacted first?",
        fontsize=14,
        fontweight="bold",
    )
    figure.tight_layout()
    save_figure(figure, "01-ranking-for-contact-decisions")


def plot_figure_2(primary: dict[str, Any], reference_prevalence: float) -> pd.DataFrame:
    observed = primary["result"]["observed_evaluation"]
    rows = []
    figure, axes = plt.subplots(1, 2, figsize=(11.5, 4.7), sharex=True)
    for score_column, profile_key in (
        ("score_random_forest", "model_a_profile"),
        ("score_full_logistic", "model_b_profile"),
    ):
        rust_profile = observed[profile_key]
        prevalences = np.asarray([point["prevalence"] for point in rust_profile])
        ap = np.asarray([point["ap"] for point in rust_profile])
        cnap = np.asarray([point["cnap"] for point in rust_profile])
        rows.extend(
            {
                "model": MODEL_LABELS[score_column],
                "target_prevalence": prevalence,
                "standardized_ap": ap_value,
                "cnap": cnap_value,
            }
            for prevalence, ap_value, cnap_value in zip(prevalences, ap, cnap)
        )
        if score_column != "score_full_logistic":
            continue
        axes[0].plot(
            prevalences,
            ap,
            lw=2.2,
            color=MODEL_COLORS[score_column],
            label=MODEL_LABELS[score_column],
        )
        axes[1].plot(
            prevalences,
            cnap,
            lw=2.2,
            color=MODEL_COLORS[score_column],
            label=MODEL_LABELS[score_column],
        )

    axes[0].plot(prevalences, prevalences, ls="--", color=GRAY, label="Chance AP = prevalence")
    axes[1].axhline(0, ls="--", color=GRAY, label="Chance CNAP = 0")
    for axis in axes:
        axis.axvline(
            reference_prevalence,
            color=PURPLE,
            ls=":",
            lw=1.6,
            label=f"Training reference {reference_prevalence:.1%}",
        )
    axes[0].set(
        ylabel="Average precision on the target population",
        title="A. AP moves with the response rate",
    )
    axes[1].set(
        ylabel="Normalized, prevalence-standardized AP (CNAP)",
        title="B. CNAP places chance at zero",
    )
    for axis in axes:
        axis.set_xscale("log")
        axis.set_xlim(TARGET_LOWER, TARGET_UPPER)
        axis.set_xlabel("Target-population subscription prevalence")
        axis.set_xticks([0.01, 0.02, 0.05, 0.10, 0.20, 0.50])
        axis.set_xticklabels(["1%", "2%", "5%", "10%", "20%", "50%"])
        axis.legend(frameon=False, loc="best")
    figure.suptitle(
        "The same ranking has different AP at different target prevalences",
        fontsize=14,
        fontweight="bold",
    )
    figure.tight_layout()
    save_figure(figure, "02-ap-needs-target-prevalence")
    return pd.DataFrame(rows)


def report_parts(report: dict[str, Any]) -> tuple[dict, dict, dict]:
    result = report["result"]
    _, evidence, _ = staged_evidence(report)
    return result, evidence, result["trace"]


def prevalence_axis(axis: plt.Axes) -> None:
    axis.set_xscale("log")
    axis.set_xlim(TARGET_LOWER, TARGET_UPPER)
    axis.set_xticks([0.01, 0.02, 0.05, 0.10, 0.20, 0.50])
    axis.set_xticklabels(["1%", "2%", "5%", "10%", "20%", "50%"])


def plot_comparison_profile(
    axis: plt.Axes,
    report: dict[str, Any],
    reference_prevalence: float,
    title: str,
    color: str,
) -> None:
    estimate, _, _ = report_parts(report)
    profile = estimate["observed_evaluation"]["diagnostic_difference_profile"]
    prevalence = np.asarray([point["prevalence"] for point in profile])
    difference = np.asarray([point["value"] for point in profile])
    limit = estimate["observed_evaluation"]["forward"]["limiting_prevalence"]
    robust = estimate["observed_evaluation"]["forward"]["value"]

    axis.axhline(0, color=GRAY, ls="--", lw=1.2)
    axis.axvline(reference_prevalence, color=PURPLE, ls=":", lw=1.4)
    axis.fill_between(
        prevalence, 0, difference, where=difference >= 0, color=GREEN, alpha=0.10
    )
    axis.fill_between(
        prevalence,
        0,
        difference,
        where=difference < 0,
        color=VERMILLION,
        alpha=0.10,
    )
    axis.plot(prevalence, difference, color=color, lw=2.5)
    axis.scatter([limit], [robust], s=58, color=VERMILLION, zorder=4)
    annotation_offset = (42, 30) if limit <= TARGET_LOWER * 1.2 else (-185, 34)
    axis.annotate(
        f"Range-wide guarantee\n{robust:+.4f}",
        xy=(limit, robust),
        xytext=annotation_offset,
        textcoords="offset points",
        arrowprops={"arrowstyle": "->", "color": VERMILLION},
        color=VERMILLION,
    )

    crossing_indices = np.flatnonzero(np.signbit(difference[1:]) != np.signbit(difference[:-1]))
    if len(crossing_indices):
        index = crossing_indices[0]
        crossing = float(
            prevalence[index]
            + (prevalence[index + 1] - prevalence[index])
            * (-difference[index])
            / (difference[index + 1] - difference[index])
        )
        axis.scatter([crossing], [0], s=45, facecolor="white", edgecolor=color, zorder=5)
        axis.annotate(
            f"Ordering changes\nnear {crossing:.1%}",
            xy=(crossing, 0),
            xytext=(-65, 34),
            textcoords="offset points",
            arrowprops={"arrowstyle": "->", "color": color},
            color=color,
        )
    else:
        axis.text(
            0.04,
            0.92,
            "Candidate favored throughout",
            transform=axis.transAxes,
            color=GREEN,
            va="top",
        )

    axis.text(
        reference_prevalence,
        0.025,
        f"{reference_prevalence:.1%}",
        transform=axis.get_xaxis_transform(),
        color=PURPLE,
        fontsize=8.5,
        ha="center",
        va="bottom",
    )
    axis.set(
        xlabel="Target-population subscription prevalence",
        ylabel="Candidate − incumbent CNAP",
        title=title,
    )
    prevalence_axis(axis)


def plot_figure_3(
    primary: dict[str, Any], control: dict[str, Any], reference_prevalence: float
) -> None:
    figure, axes = plt.subplots(1, 2, figsize=(11.8, 4.9))
    plot_comparison_profile(
        axes[0],
        primary,
        reference_prevalence,
        "A. Close comparison\nRandom forest − full logistic",
        BLUE,
    )
    plot_comparison_profile(
        axes[1],
        control,
        reference_prevalence,
        "B. Positive control\nFull − demographic logistic",
        GREEN,
    )
    figure.suptitle(
        "Can one model choice work throughout the declared prevalence range?",
        fontsize=14,
        fontweight="bold",
    )
    figure.tight_layout()
    save_figure(figure, "03-advantage-across-deployment-range")


def plot_figure_4(control: dict[str, Any]) -> None:
    estimate, forward, trace = report_parts(control)
    prevalence = np.asarray(trace["prevalences"])
    profiles = np.asarray(trace["replication_difference_profiles"])
    robust = np.asarray([effect["value"] for effect in forward["retained_effects"]])
    limiting = np.asarray(
        [effect["limiting_prevalence"] for effect in forward["retained_effects"]]
    )
    anchor = estimate["forward"]["observed_gate"]["supported_magnitude"]
    mean_profile = estimate["mean_diagnostic_difference_profile"]
    ordered = np.argsort(robust)
    selected = ordered[
        np.linspace(0, len(ordered) - 1, min(12, len(ordered)), dtype=int)
    ]
    figure, axes = plt.subplots(1, 2, figsize=(11.8, 4.9))
    for index in selected:
        axes[0].plot(prevalence, profiles[index], color=GREEN, alpha=0.20, lw=1.0)
        axes[0].scatter(
            [limiting[index]],
            [robust[index]],
            s=18,
            color=VERMILLION,
            alpha=0.75,
            zorder=3,
        )
    axes[0].plot(
        [point["prevalence"] for point in mean_profile],
        [point["value"] for point in mean_profile],
        color=GREEN,
        lw=2.6,
        label="Rust-computed mean computational profile",
    )
    axes[0].axhline(0, color=GRAY, ls="--", lw=1.2)
    axes[0].set(
        xlabel="Target-population subscription prevalence",
        ylabel="Candidate − incumbent CNAP",
        title=(
            "A. Each complete replication faces the full range\n"
            "Dots mark its range-wide result"
        ),
    )
    prevalence_axis(axes[0])
    axes[0].legend(frameon=False)
    axes[0].annotate(
        "Selected range-wide effects\n(each is weakest at 1%)",
        xy=(TARGET_LOWER, float(np.median(robust[selected]))),
        xytext=(42, 34),
        textcoords="offset points",
        arrowprops={"arrowstyle": "->", "color": VERMILLION},
        color=VERMILLION,
    )

    axes[1].hist(robust, bins="auto", color=GREEN, alpha=0.72, edgecolor="white")
    average = estimate["forward"]["full_assessment"]["mean_retained_effect"]
    supported = forward["supported_magnitude"]
    axes[1].axvline(average, color=PURPLE, lw=2.0, label=f"Anchored-effect mean {average:+.4f}")
    axes[1].axvline(
        supported,
        color=BLUE,
        lw=2.3,
        label=f"Mean weaker effect over distinct order-2 subsets: {supported:+.4f}",
    )
    axes[1].axvline(0, color=GRAY, ls="--", lw=1.2)
    axes[1].axvline(
        anchor,
        color=VERMILLION,
        ls=":",
        lw=2.0,
        label=f"Observed anchor {anchor:+.4f}",
    )
    axes[1].set(
        xlabel="Range-wide advantage over 1%–50% prevalence",
        ylabel="Computational replications",
        title="B. Finite list of range-wide replication effects",
    )
    axes[1].legend(frameon=False, loc="best")
    figure.suptitle(
        "Observed anchoring constrains the computational challenge",
        fontsize=14,
        fontweight="bold",
    )
    figure.tight_layout()
    save_figure(figure, "04-supported-superiority")



def plot_criterion_summary(
    axis: plt.Axes,
    primary: dict[str, Any],
    control: dict[str, Any],
    criterion: str,
) -> None:
    comparisons = [
        ("Random forest − full logistic", primary, BLUE),
        ("Full − demographic logistic", control, GREEN),
    ]
    first_result = comparisons[0][1]["result"]
    if criterion == "magnitude":
        threshold = first_result["policy"]["magnitude_threshold"]
        xlabel = "Supported CNAP magnitude"
        title = "A. Magnitude criterion"
        estimate_of = lambda evidence: evidence["supported_magnitude"]
        formatter = lambda value: f"{value:+.4f}"
        point_label = "criterion value"
    else:
        threshold = first_result["policy"]["survival_requirement"]
        xlabel = "Literal distinct-subset survival fraction"
        title = "B. Survival criterion"
        estimate_of = lambda evidence: evidence["literal_survival"]["subset_fraction"]
        formatter = lambda value: f"{value:.3f}"
        point_label = "criterion value"
    axis.axvline(
        threshold,
        color=GRAY,
        ls="--",
        lw=1.2,
        label=f"Required threshold {formatter(threshold)}",
    )
    plotted_values = [threshold]
    for position, (label, report, color) in enumerate(comparisons[::-1]):
        staged = report["result"]["forward"]
        gate_value = estimate_of(staged["observed_gate"])
        plotted_values.append(gate_value)
        axis.scatter(
            [gate_value], [position], color=color, marker="o", s=55, zorder=3
        )
        axis.annotate(
            f"Stage 1 {formatter(gate_value)}",
            xy=(gate_value, position),
            xytext=(7, 11),
            textcoords="offset points",
            color=color,
            fontsize=8.8,
        )
        full = staged["full_assessment"]
        if full is None:
            axis.annotate(
                "stops after observed gate",
                xy=(gate_value, position),
                xytext=(7, -16),
                textcoords="offset points",
                color=VERMILLION,
                fontsize=8.8,
            )
            continue
        full_value = estimate_of(full)
        plotted_values.append(full_value)
        axis.plot([gate_value, full_value], [position, position], color=color, lw=1.5)
        axis.scatter(
            [full_value], [position], color=color, marker="D", s=48, zorder=4
        )
        axis.annotate(
            f"Stage 2 {formatter(full_value)}",
            xy=(full_value, position),
            xytext=(7, -16),
            textcoords="offset points",
            color=color,
            fontsize=8.8,
        )
    axis.scatter([], [], color=GRAY, marker="o", s=48, label="Stage 1: observed gate")
    axis.scatter([], [], color=GRAY, marker="D", s=48, label="Stage 2: anchored computational")
    axis.set_yticks([0, 1], [comparisons[1][0], comparisons[0][0]])
    span = max(plotted_values) - min(plotted_values)
    padding = 0.12 * span if span > 0 else 0.05
    if criterion == "magnitude":
        axis.set_xlim(
            min(plotted_values) - padding,
            max(plotted_values) + 0.55 * max(span, 0.05),
        )
    else:
        axis.set_xlim(-0.05, 1.05)
    axis.set_ylim(-0.25, 1.25)
    axis.set_xlabel(xlabel)
    axis.set_title(title)
    axis.legend(
        title="Legend",
        frameon=True,
        fancybox=False,
        framealpha=1.0,
        edgecolor=GRAY,
        loc="center",
        bbox_to_anchor=(0.75, 0.5),
        borderpad=0.4,
        fontsize=9.5,
        title_fontsize=10,
    )


def plot_figure_5(primary: dict[str, Any], control: dict[str, Any]) -> None:
    figure, axes = plt.subplots(1, 2, figsize=(11.8, 4.9))
    plot_criterion_summary(axes[0], primary, control, "magnitude")
    plot_criterion_summary(axes[1], primary, control, "literal_survival")
    figure.suptitle(
        "The observed gate comes before the anchored computational challenge",
        fontsize=14,
        fontweight="bold",
    )
    figure.tight_layout()
    save_figure(figure, "05-two-part-replacement-decision")


def projected_design(report: dict[str, Any]) -> tuple[int, int]:
    """Positive and negative counts of the projected replications.

    Under same-design resampling these equal the observed holdout counts, so
    they are read back from the report rather than declared as constants. That
    keeps the figure caption, the report text, and the manifest from drifting
    away from what was actually computed.
    """
    counts = report["result"]["evaluation_counts"][0]
    return counts["positive"], counts["negative"]


def auroc_parts(report: dict[str, Any]) -> tuple[dict, dict]:
    result = report["result"]
    return result, result


def breakdown_label(breakdown: dict[str, Any]) -> str:
    if breakdown["kind"] == "observed_gate_failure":
        return "Observed gate failed; no computational search"
    if breakdown["kind"] == "at_baseline":
        return "Γ† = 1"
    if breakdown["kind"] == "finite":
        return (
            f"{breakdown['last_passing']:.2f} < Γ† ≤ "
            f"{breakdown['first_failing']:.2f}"
        )
    return "No breakdown on empirical support"


def plot_figure_6(
    _predictions: pd.DataFrame,
    primary_auroc: dict[str, Any],
    control_auroc: dict[str, Any],
) -> None:
    comparisons = [
        (
            "Random forest -\nfull logistic",
            "score_random_forest",
            "score_full_logistic",
            primary_auroc,
            BLUE,
        ),
        (
            "Full - demographic\nlogistic",
            "score_full_logistic",
            "score_demographic_logistic",
            control_auroc,
            GREEN,
        ),
    ]
    figure, axes = plt.subplots(1, 3, figsize=(15.2, 4.9))

    positions = np.arange(len(comparisons))
    baseline_magnitudes = [
        item[3]["result"]["baseline"]["supported_magnitude"] for item in comparisons
    ]
    axes[0].axvline(0, color=GRAY, ls="--", lw=1.2)
    for position, (comparison, _, _, report, color), difference in zip(
        positions, comparisons, baseline_magnitudes
    ):
        axes[0].barh(position, difference, color=color, alpha=0.78, height=0.52)
        _, estimate = auroc_parts(report)
        breakdown = estimate["breakdown"]
        if breakdown["kind"] == "observed_gate_failure":
            takeaway = "Observed gate fails"
        elif breakdown["kind"] == "at_baseline":
            takeaway = "No support"
        elif breakdown["kind"] == "finite":
            takeaway = f"Breaks by Γ={breakdown['first_failing']:.2f}"
        else:
            takeaway = "No breakdown"
        axes[0].annotate(
            f"{difference:+.3f}\n{takeaway}",
            xy=(difference, position),
            xytext=(6, 0),
            textcoords="offset points",
            ha="left",
            va="center",
            color=color,
        )
    axes[0].set_yticks(positions, [item[0] for item in comparisons])
    axes[0].set(
        xlabel="Supported AUROC advantage",
        title="A. Staged result at the observed mix (Γ=1)",
        xlim=(-0.02, 0.20),
    )
    axes[0].invert_yaxis()

    _, control_estimate = auroc_parts(control_auroc)
    fixed = control_estimate["evaluated_profile"]
    factors = np.asarray([point["concentration_factor"] for point in fixed])
    magnitude = np.asarray([point["supported_magnitude"] for point in fixed])
    survival = np.asarray(
        [point["literal_survival_fraction"] for point in fixed]
    )
    magnitude_lower = np.asarray([point["supported_magnitude_lower"] for point in fixed])
    magnitude_upper = np.asarray([point["supported_magnitude_upper"] for point in fixed])
    survival_lower = np.asarray(
        [point["literal_survival_lower"] for point in fixed]
    )
    survival_upper = np.asarray(
        [point["literal_survival_upper"] for point in fixed]
    )
    verdicts = np.asarray([point["verdict"] for point in fixed])

    profile_specs = [
        (
            axes[1],
            magnitude,
            magnitude_lower,
            magnitude_upper,
            AUROC_MAGNITUDE_THRESHOLD,
            "Supported AUROC advantage",
            "B. Supported advantage shrinks",
        ),
        (
            axes[2],
            survival,
            survival_lower,
            survival_upper,
            AUROC_SURVIVAL_REQUIREMENT,
            "Literal distinct-subset survival fraction",
            "C. Survival fraction eventually fails",
        ),
    ]
    verdict_styles = (
        ("verified_pass", "Verified pass", GREEN, "o"),
        ("unresolved", "Unresolved", ORANGE, "s"),
        ("verified_failure", "Verified failure", VERMILLION, "X"),
    )
    for axis, point, lower, upper, threshold, ylabel, title in profile_specs:
        axis.axhline(threshold, color=GRAY, ls="--", lw=1.2, label="Required level")
        axis.fill_between(
            factors,
            lower,
            upper,
            color=PURPLE,
            alpha=0.18,
            label="Numerical certificate interval",
        )
        axis.plot(
            factors,
            point,
            color=GREEN,
            lw=2.3,
            label="Finite-evidence midpoint",
        )
        for verdict, label, color, marker in verdict_styles:
            selected = verdicts == verdict
            if selected.any():
                axis.scatter(
                    factors[selected],
                    point[selected],
                    color=color,
                    edgecolor="white",
                    linewidth=0.6,
                    marker=marker,
                    s=52,
                    zorder=4,
                    label=label,
                )
        axis.set(
            xlabel="Case-mix concentration Γ",
            ylabel=ylabel,
            title=title,
            xlim=(1.0, float(factors.max())),
        )

    breakdown = control_estimate["breakdown"]
    if breakdown["kind"] == "finite":
        last_passing = breakdown["last_passing"]
        first_failing = breakdown["first_failing"]
        if last_passing <= float(factors.max()):
            for axis in axes[1:]:
                axis.axvspan(
                    last_passing,
                    min(first_failing, float(factors.max())),
                    color=VERMILLION,
                    alpha=0.12,
                    label="Reported breakdown bracket",
                )
        else:
            axes[1].text(
                0.98,
                0.05,
                f"Estimated breakdown beyond plotted grid\n{breakdown_label(breakdown)}",
                transform=axes[1].transAxes,
                ha="right",
                va="bottom",
                color=VERMILLION,
            )

    axes[1].legend(frameon=False, fontsize=7.5, loc="best")
    axes[2].set_ylim(-0.05, 1.05)
    axes[2].legend(frameon=False, fontsize=7.5, loc="lower left")
    figure.suptitle(
        "How much case-mix change overturns the AUROC decision?",
        fontsize=14,
        fontweight="bold",
    )
    figure.text(
        0.5,
        0.005,
        (
            "Γ=1 is the observed case mix; larger Γ permits greater "
            "concentration on fewer cases."
        ),
        ha="center",
        fontsize=8.8,
        color=GRAY,
    )
    figure.tight_layout(rect=(0.07, 0.04, 0.99, 0.95), w_pad=3.0)
    save_figure(figure, "06-auroc-case-mix-breakdown")


def write_summary_tables(
    predictions: pd.DataFrame,
    context: dict,
    ap_rows: pd.DataFrame,
    primary: dict[str, Any],
    control: dict[str, Any],
    primary_auroc: dict[str, Any],
    control_auroc: dict[str, Any],
) -> None:
    labels = predictions["label"].to_numpy()
    primary_observed = primary["result"]["observed_evaluation"]
    control_observed = control["result"]["observed_evaluation"]
    rust_ap = {
        "score_random_forest": primary_observed["model_a_empirical_ap"],
        "score_full_logistic": primary_observed["model_b_empirical_ap"],
        "score_demographic_logistic": control_observed["model_b_empirical_ap"],
    }
    metrics = []
    for score_column in MODEL_LABELS:
        scores = predictions[score_column].to_numpy()
        metrics.append(
            {
                "model": MODEL_LABELS[score_column],
                "auroc": roc_auc_score(labels, scores),
                "tie_averaged_ap_at_evaluation_prevalence": rust_ap[score_column],
            }
        )
    pd.DataFrame(metrics).to_csv(TABLES / "model_metrics.csv", index=False)
    ap_rows.to_csv(TABLES / "prevalence_standardization.csv", index=False)
    predictions.to_csv(PREDICTIONS / "temporal_holdout_predictions.csv", index=False)

    comparison_rows = []
    for name, report in [
        ("random forest - full logistic", primary),
        ("full logistic - demographic logistic", control),
    ]:
        estimate = report["result"]
        staged = estimate["forward"]
        gate = staged["observed_gate"]
        full = staged["full_assessment"]
        secondary = staged["secondary_unanchored_projection"]
        _, reverse, reverse_stage = staged_evidence(report, "reverse")
        selected = full if full is not None else gate
        survival = selected["literal_survival"]
        comparison_rows.append(
            {
                "comparison": name,
                "target_prevalence_lower": TARGET_LOWER,
                "target_prevalence_upper": TARGET_UPPER,
                "empirical_order": 1,
                "computational_order": SUPPORT_ORDER,
                "magnitude_threshold": MAGNITUDE_THRESHOLD,
                "survival_floor": SURVIVAL_FLOOR,
                "survival_requirement": SURVIVAL_REQUIREMENT,
                "observed_gate_magnitude": gate["supported_magnitude"],
                "observed_gate_survival": gate["literal_survival"]["subset_fraction"],
                "full_assessment_performed": full is not None,
                "anchored_full_magnitude": None if full is None else full["supported_magnitude"],
                "anchored_full_survival": None if full is None else full["literal_survival"]["subset_fraction"],
                "secondary_unanchored_magnitude": (
                    None if secondary is None else secondary["supported_magnitude"]
                ),
                "reported_stage": "anchored_full_assessment" if full is not None else "observed_gate",
                "reported_supported_superiority": selected["supported_magnitude"],
                "reverse_reported_stage": reverse_stage,
                "reverse_supported_superiority": reverse["supported_magnitude"],
                "literal_survival_fraction": survival["subset_fraction"],
                "survivor_count": survival["survivor_count"],
                "reported_list_length": survival["list_length"],
                "verdict": staged["staged_verdict"],
                "reference_status": estimate["reference_assessment"]["status"],
            }
        )
    pd.DataFrame(comparison_rows).to_csv(
        TABLES / "supported_superiority_summary.csv", index=False
    )

    auroc_rows = []
    profile_rows = []
    for name, candidate, incumbent, report in [
        (
            "random forest - full logistic",
            "score_random_forest",
            "score_full_logistic",
            primary_auroc,
        ),
        (
            "full logistic - demographic logistic",
            "score_full_logistic",
            "score_demographic_logistic",
            control_auroc,
        ),
    ]:
        _, estimate = auroc_parts(report)
        breakdown = estimate["breakdown"]
        if breakdown["kind"] == "at_baseline":
            last_passing = None
            first_failing = 1.0
            first_failure = breakdown["first_failure"]
        elif breakdown["kind"] == "finite":
            last_passing = breakdown["last_passing"]
            first_failing = breakdown["first_failing"]
            first_failure = breakdown["first_failure"]
        else:
            last_passing = None
            first_failing = None
            first_failure = None
        ordinary_difference = roc_auc_score(
            labels, predictions[candidate]
        ) - roc_auc_score(labels, predictions[incumbent])
        auroc_rows.append(
            {
                "comparison": name,
                "ordinary_paired_auroc_difference": ordinary_difference,
                "evidence": estimate["evidence"],
                "projected_positive_count": estimate["evaluation_counts"][0]["positive"],
                "projected_negative_count": estimate["evaluation_counts"][0]["negative"],
                "empirical_order": estimate["empirical_order"],
                "computational_order": estimate["computational_order"],
                "observed_gate_magnitude": estimate["observed_gate"]["supported_magnitude"],
                "observed_gate_survival": estimate["observed_gate"]["literal_survival_fraction"],
                "magnitude_threshold": AUROC_MAGNITUDE_THRESHOLD,
                "survival_floor": AUROC_SURVIVAL_FLOOR,
                "survival_requirement": AUROC_SURVIVAL_REQUIREMENT,
                "baseline_supported_magnitude": estimate["baseline"]["supported_magnitude"],
                "baseline_literal_survival": estimate["baseline"][
                    "literal_survival_fraction"
                ],
                "breakdown_kind": breakdown["kind"],
                "last_passing_factor": last_passing,
                "first_failing_factor": first_failing,
                "first_failure": first_failure,
                "reference_status": estimate["reference_assessment"]["status"],
            }
        )
        for point in estimate["evaluated_profile"]:
            factor = point["concentration_factor"]
            profile_rows.append(
                {
                    "comparison": name,
                    "concentration_factor": factor,
                    "minimum_ess_fraction": 1.0 / factor,
                    "supported_magnitude": point["supported_magnitude"],
                    "supported_magnitude_numerical_lower": point["supported_magnitude_lower"],
                    "supported_magnitude_numerical_upper": point["supported_magnitude_upper"],
                    "literal_survival_fraction": point["literal_survival_fraction"],
                    "literal_survival_numerical_lower": point["literal_survival_lower"],
                    "literal_survival_numerical_upper": point["literal_survival_upper"],
                    "survivor_count": point["survivor_count"],
                    "list_length": point["list_length"],
                    "verdict": point["verdict"],
                }
            )
    pd.DataFrame(auroc_rows).to_csv(
        TABLES / "auroc_case_mix_summary.csv", index=False
    )
    pd.DataFrame(profile_rows).to_csv(
        TABLES / "auroc_finite_evidence_profile.csv", index=False
    )
    pd.DataFrame([context]).to_csv(TABLES / "study_context.csv", index=False)


def write_report(
    profile_name: str,
    profile: Profile,
    predictions: pd.DataFrame,
    context: dict,
    primary: dict[str, Any],
    control: dict[str, Any],
    primary_auroc: dict[str, Any],
    control_auroc: dict[str, Any],
    *,
    reused_retained_ap: bool,
) -> None:
    primary_estimate, primary_forward, _ = report_parts(primary)
    control_estimate, control_forward, _ = report_parts(control)
    primary_staged = primary_estimate["forward"]
    control_staged = control_estimate["forward"]
    primary_gate = primary_staged["observed_gate"]
    control_gate = control_staged["observed_gate"]
    primary_full = primary_staged["full_assessment"]
    control_full = control_staged["full_assessment"]
    _, primary_reverse, primary_reverse_stage = staged_evidence(primary, "reverse")
    _, control_reverse, control_reverse_stage = staged_evidence(control, "reverse")
    primary_survival = primary_forward["literal_survival"]
    control_survival = control_forward["literal_survival"]
    _, primary_auroc_estimate = auroc_parts(primary_auroc)
    _, control_auroc_estimate = auroc_parts(control_auroc)
    labels = predictions["label"].to_numpy()
    primary_ordinary_auroc = roc_auc_score(
        labels, predictions["score_random_forest"]
    ) - roc_auc_score(labels, predictions["score_full_logistic"])
    control_ordinary_auroc = roc_auc_score(
        labels, predictions["score_full_logistic"]
    ) - roc_auc_score(labels, predictions["score_demographic_logistic"])

    provenance = (
        "The fitted models, temporal predictions, and AP reports were reused from "
        "the retained checksum-verified clean run; this refresh did not refit models "
        "or rerun AP."
        if reused_retained_ap
        else "Every fitted model and prediction was regenerated from the copied raw "
        "UCI Bank Marketing CSV during this clean run."
    )
    text = f"""# Bank Marketing supported-superiority study

Profile: `{profile_name}`. {provenance}

## Design

- Training/evaluation split: first {context['training_rows']:,} rows / final
  {context['evaluation_rows']:,} rows in source order.
- `duration` excluded before fitting.
- Training prevalence: {context['training_prevalence']:.2%}.
- Evaluation prevalence: {context['evaluation_prevalence']:.2%}.
- Declared target-prevalence interval: 1%–50%.
- Empirical order: K_E=1 for the single temporal holdout.
- Computational order: K_C=2 within an empirical evaluation that clears Stage 1.
- Magnitude threshold: δ={MAGNITUDE_THRESHOLD:g}.
- Literal-survival floor: d={SURVIVAL_FLOOR:g}.
- Required literal distinct-subset survival fraction: γ={SURVIVAL_REQUIREMENT:.2f}.
- Projected AP computational replications: {profile.computational_replications:,}.
- AP prevalence search: {profile.grid_points} diagnostic grid points with
  tolerance {profile.search_tolerance:g} and at most {profile.search_iterations} iterations.
- The temporal holdout is the one observed empirical evaluation. Its range-wide
  effect is assessed first. Only a direction that clears this observed gate
  receives the additional, separately identified computational challenge.
- Every computational effect is anchored by the observed effect from which it
  was derived. Computational replications are not additional empirical rows and
  carry no confidence interpretation.
- AUROC projected evaluation size: same design as the temporal holdout
  ({projected_design(primary_auroc)[0]:,} subscribers and
  {projected_design(primary_auroc)[1]:,} non-subscribers).
- AUROC projected computational replications: {profile.auroc_computational_replications:,};
  concentration-search tolerance: {profile.auroc_search_tolerance:g}.
- AUROC global-optimization gaps: absolute {AUROC_OPTIMIZATION_ABSOLUTE_GAP:g},
  relative {AUROC_OPTIMIZATION_RELATIVE_GAP:g}; {AUROC_OPTIMIZATION_TIME_LIMIT_SECONDS:g}-second
  wall-clock limit and one solver thread per job. Unresolved intervals propagate
  rather than being assigned a direction.

## Primary comparison

Random forest − full logistic:

- Stage 1 observed magnitude: {primary_gate['supported_magnitude']:+.4f};
- Stage 1 literal survival above d: {primary_gate['literal_survival']['subset_fraction']:.3f};
- Stage 1 verdict: `{primary_gate['verdict']}`; the computational challenge was
  {'entered' if primary_full is not None else 'not entered'}.
- reported staged magnitude: {primary_forward['supported_magnitude']:+.4f};
- literal distinct-subset survival at the reported stage: {primary_survival['subset_fraction']:.3f};
- single-effect survivor fraction: {primary_survival['survivor_fraction']:.3f}
  ({primary_survival['survivor_count']} of {primary_survival['list_length']} effects);
- reversed orientation ({primary_reverse_stage}): magnitude {primary_reverse['supported_magnitude']:+.4f},
  literal survival {primary_reverse['literal_survival']['subset_fraction']:.3f};
- staged decision: `{primary_staged['staged_verdict']}`.

## Positive control

Full logistic − demographic logistic:

- Stage 1 observed magnitude: {control_gate['supported_magnitude']:+.4f};
- Stage 1 literal survival above d: {control_gate['literal_survival']['subset_fraction']:.3f};
- Stage 1 verdict: `{control_gate['verdict']}`; the anchored computational challenge was
  {'entered' if control_full is not None else 'not entered'}.
- Stage 2 anchored magnitude: {control_forward['supported_magnitude']:+.4f};
- Stage 2 anchored literal survival above d: {control_survival['subset_fraction']:.3f};
- secondary unanchored computational magnitude (diagnostic only):
  {control_staged['secondary_unanchored_projection']['supported_magnitude']:+.4f};
- single-effect survivor fraction: {control_survival['survivor_fraction']:.3f}
  ({control_survival['survivor_count']} of {control_survival['list_length']} effects);
- reversed orientation ({control_reverse_stage}): magnitude {control_reverse['supported_magnitude']:+.4f},
  literal survival {control_reverse['literal_survival']['subset_fraction']:.3f};
- staged decision: `{control_staged['staged_verdict']}`.

## AUROC empirical case-mix companion

The AUROC analysis reuses the same fitted models and temporal-holdout score
pools. Unlike AP, AUROC is unchanged by prevalence alone. The challenge instead
allows increasingly concentrated redistributions of the represented
subscribers and non-subscribers. Its projected replications use the fixed
same design as the observed temporal holdout, so the only difference between
replications is which observed cases each one happens to contain.

Random forest − full logistic:

- ordinary paired AUROC difference: {primary_ordinary_auroc:+.4f};
- Stage 1 observed magnitude at Γ=1: {primary_auroc_estimate['observed_gate']['supported_magnitude']:+.4f};
- Stage 1 observed survival at Γ=1: {primary_auroc_estimate['observed_gate']['literal_survival_fraction']:.3f};
- estimated breakdown: {breakdown_label(primary_auroc_estimate['breakdown'])};
- first failure: `{primary_auroc_estimate['breakdown'].get('first_failure', 'none')}`.

Full logistic − demographic logistic:

- ordinary paired AUROC difference: {control_ordinary_auroc:+.4f};
- Stage 1 observed magnitude at Γ=1: {control_auroc_estimate['observed_gate']['supported_magnitude']:+.4f};
- Stage 2 anchored computational magnitude at Γ=1:
  {control_auroc_estimate['baseline']['supported_magnitude']:+.4f};
- Stage 2 anchored survival at Γ=1: {control_auroc_estimate['baseline']['literal_survival_fraction']:.3f};
- estimated breakdown: {breakdown_label(control_auroc_estimate['breakdown'])};
- first failure: `{control_auroc_estimate['breakdown'].get('first_failure', 'none')}`.

The AUROC optimization intervals are numerical solver certificates, not
statistical confidence intervals. The case-mix challenge only redistributes
customer types represented in the temporal holdout and does not establish
transport to absent types.

## Interpretation

The observed empirical challenge is logically prior to the computational
challenge. When Stage 2 is reached, supported magnitude and literal survival are
computed from the same finite list of observed-anchored, prevalence-robust
computational effects. A supported replacement verdict requires both strict
inequalities at every required stage; no confidence-bound layer is applied.

The prevalence-standardization interpretation assumes transport of the
class-conditional score distributions over the declared target-prevalence
range. Rows are treated as independent computational-resampling units, and the
projected procedure retains the observed evaluation class counts.

Paired CNAP is tie-averaged before the prevalence and replication challenges.
The diagnostic exchangeable-label reference law is not asserted for this
heterogeneous observational holdout. Seed and replication-count diagnostics are
reported separately in `tables/computational_stability.csv`.
"""
    (OUTPUTS / "REPORT.md").write_text(text)


def auroc_manifest(profile: Profile) -> dict[str, Any]:
    return {
        "evidence": "projected_fixed_model",
        "replication_design": "same_as_observed",
        "computational_replications": profile.auroc_computational_replications,
        "empirical_order": 1,
        "computational_order": SUPPORT_ORDER,
        "magnitude_threshold": AUROC_MAGNITUDE_THRESHOLD,
        "survival_floor": AUROC_SURVIVAL_FLOOR,
        "survival_requirement": AUROC_SURVIVAL_REQUIREMENT,
        "optimization_absolute_gap": AUROC_OPTIMIZATION_ABSOLUTE_GAP,
        "optimization_relative_gap": AUROC_OPTIMIZATION_RELATIVE_GAP,
        "optimization_time_limit_seconds": AUROC_OPTIMIZATION_TIME_LIMIT_SECONDS,
        "concentration_search_tolerance": profile.auroc_search_tolerance,
        "interpretation": "observed_gate_then_observed_anchored_computational_challenge",
    }


def write_manifest(profile_name: str, profile: Profile, context: dict, cli: Path) -> None:
    manifest = {
        "manuscript_version": "v17",
        "profile": profile_name,
        "parameters": asdict(profile),
        "empirical_order": 1,
        "computational_order": SUPPORT_ORDER,
        "magnitude_threshold": MAGNITUDE_THRESHOLD,
        "survival_floor": SURVIVAL_FLOOR,
        "survival_requirement": SURVIVAL_REQUIREMENT,
        "target_prevalence_interval": [TARGET_LOWER, TARGET_UPPER],
        "auroc": auroc_manifest(profile),
        "master_seed": MASTER_SEED,
        "computational_stability": {
            "ap_alternate_seed_offsets": [1101, 1102],
            "auroc_alternate_seed_offsets": [1301, 1302],
            "ap_replication_count_check": "separate Rust run at half the replication count",
            "table": "domain_study/outputs/tables/computational_stability.csv",
        },
        "raw_sources": {
            str(ARCHIVE_PATH.relative_to(ROOT)): ARCHIVE_SHA256,
            str(CSV_PATH.relative_to(ROOT)): CSV_SHA256,
        },
        "study_context": context,
        "software": {
            "python": sys.version,
            "platform": platform.platform(),
            "numpy": np.__version__,
            "pandas": pd.__version__,
            "matplotlib": plt.matplotlib.__version__,
            "scikit_learn": sklearn.__version__,
            "rust_cli": str(cli),
        },
        "clean_build": {
            "models_refit_from_raw": True,
            "prior_predictions_used": False,
            "prior_reports_used": False,
            "prior_figures_used": False,
        },
    }
    (OUTPUTS / "manifest.json").write_text(json.dumps(manifest, indent=2))


def read_json(path: Path) -> dict[str, Any]:
    with path.open() as handle:
        return json.load(handle)


def render_figures_and_tables(
    profile_name: str,
    profile: Profile,
    predictions: pd.DataFrame,
    context: dict,
    primary: dict[str, Any],
    control: dict[str, Any],
    primary_auroc: dict[str, Any],
    control_auroc: dict[str, Any],
    *,
    reused_retained_ap: bool = False,
) -> None:
    plot_figure_1(predictions, primary)
    ap_rows = plot_figure_2(primary, context["training_prevalence"])
    plot_figure_3(primary, control, context["training_prevalence"])
    plot_figure_4(control)
    plot_figure_5(primary, control)
    plot_figure_6(predictions, primary_auroc, control_auroc)
    write_summary_tables(
        predictions,
        context,
        ap_rows,
        primary,
        control,
        primary_auroc,
        control_auroc,
    )
    write_report(
        profile_name,
        profile,
        predictions,
        context,
        primary,
        control,
        primary_auroc,
        control_auroc,
        reused_retained_ap=reused_retained_ap,
    )


def render_only(args: argparse.Namespace, profile: Profile) -> None:
    manifest_path = OUTPUTS / "manifest.json"
    predictions_path = PREDICTIONS / "temporal_holdout_predictions.csv"
    primary_path = REPORTS / "random_forest_vs_full_logistic.json"
    control_path = REPORTS / "full_vs_demographic_logistic.json"
    primary_auroc_path = REPORTS / "auroc_random_forest_vs_full_logistic.json"
    control_auroc_path = REPORTS / "auroc_full_vs_demographic_logistic.json"
    for path in (
        manifest_path,
        predictions_path,
        primary_path,
        control_path,
        primary_auroc_path,
        control_auroc_path,
    ):
        if not path.exists():
            raise FileNotFoundError(f"render-only input is missing: {path}")

    manifest = read_json(manifest_path)
    if manifest["profile"] != args.profile:
        raise RuntimeError(
            f"retained profile is {manifest['profile']}, not requested {args.profile}"
        )
    context = manifest["study_context"]
    predictions = pd.read_csv(predictions_path)
    primary = read_json(primary_path)
    control = read_json(control_path)
    primary_auroc = read_json(primary_auroc_path)
    control_auroc = read_json(control_auroc_path)
    validate_rust_report(primary, profile, True)
    validate_rust_report(control, profile, control["result"]["trace"] is not None)
    validate_auroc_report(primary_auroc, profile)
    validate_auroc_report(control_auroc, profile)

    if control["result"]["trace"] is None:
        raise RuntimeError("render-only requires retained AP replication profiles")

    configure_plots()
    render_figures_and_tables(
        args.profile,
        profile,
        predictions,
        context,
        primary,
        control,
        primary_auroc,
        control_auroc,
        reused_retained_ap=True,
    )
    print(f"Figures refreshed from retained clean-build artifacts: {FIGURES}")


def run_auroc_only(args: argparse.Namespace, profile: Profile) -> None:
    manifest_path = OUTPUTS / "manifest.json"
    predictions_path = PREDICTIONS / "temporal_holdout_predictions.csv"
    primary_path = REPORTS / "random_forest_vs_full_logistic.json"
    control_path = REPORTS / "full_vs_demographic_logistic.json"
    for path in (manifest_path, predictions_path, primary_path, control_path):
        if not path.exists():
            raise FileNotFoundError(f"AUROC-only retained input is missing: {path}")

    manifest = read_json(manifest_path)
    if manifest["profile"] != args.profile:
        raise RuntimeError(
            f"retained profile is {manifest['profile']}, not requested {args.profile}"
        )
    context = manifest["study_context"]
    predictions = pd.read_csv(predictions_path)
    primary = read_json(primary_path)
    control = read_json(control_path)
    validate_rust_report(primary, profile, primary["result"]["trace"] is not None)
    validate_rust_report(control, profile, control["result"]["trace"] is not None)
    if control["result"]["trace"] is None:
        raise RuntimeError(
            "AUROC-only rendering requires the positive-control AP replication profiles; "
            "the AP analysis will not be rerun implicitly"
        )

    primary_auroc = run_auroc_rust(
        args.cli,
        predictions_path,
        REPORTS / "auroc_random_forest_vs_full_logistic.json",
        profile,
        "score_random_forest",
        "score_full_logistic",
        resampling_seed=MASTER_SEED + 301,
    )
    control_auroc = run_auroc_rust(
        args.cli,
        predictions_path,
        REPORTS / "auroc_full_vs_demographic_logistic.json",
        profile,
        "score_full_logistic",
        "score_demographic_logistic",
        resampling_seed=MASTER_SEED + 302,
    )
    validate_auroc_report(primary_auroc, profile)
    validate_auroc_report(control_auroc, profile)
    configure_plots()
    render_figures_and_tables(
        args.profile,
        profile,
        predictions,
        context,
        primary,
        control,
        primary_auroc,
        control_auroc,
        reused_retained_ap=True,
    )
    manifest["parameters"] = asdict(profile)
    manifest["auroc"] = auroc_manifest(profile)
    manifest["auroc_refresh"] = {
        "reused_retained_predictions": True,
        "reused_retained_ap_reports": True,
        "ap_analysis_rerun": False,
    }
    manifest_path.write_text(json.dumps(manifest, indent=2))
    print(f"AUROC extension complete from retained AP artifacts: {OUTPUTS}")


def main() -> None:
    args = parse_args()
    profile = PROFILES[args.profile]
    verify_raw_data()
    if not args.cli.exists():
        raise FileNotFoundError(
            f"Rust CLI not found at {args.cli}; run `cargo build --release` first"
        )
    if args.render_only:
        render_only(args, profile)
        return
    if args.auroc_only:
        run_auroc_only(args, profile)
        return
    prepare_outputs()
    configure_plots()

    frame = load_data()
    predictions, context = fit_fresh_predictions(frame, profile)
    prediction_input = PREDICTIONS / "rust_input.csv"
    predictions.to_csv(prediction_input, index=False)

    primary_path = REPORTS / "random_forest_vs_full_logistic.json"
    control_path = REPORTS / "full_vs_demographic_logistic.json"
    primary = run_rust(
        args.cli,
        prediction_input,
        primary_path,
        profile,
        "score_random_forest",
        "score_full_logistic",
        resampling_seed=MASTER_SEED + 101,
        retain_trace=True,
    )
    control = run_rust(
        args.cli,
        prediction_input,
        control_path,
        profile,
        "score_full_logistic",
        "score_demographic_logistic",
        resampling_seed=MASTER_SEED + 102,
        retain_trace=True,
    )
    validate_rust_report(primary, profile, True)
    validate_rust_report(control, profile, True)

    primary_auroc_path = REPORTS / "auroc_random_forest_vs_full_logistic.json"
    control_auroc_path = REPORTS / "auroc_full_vs_demographic_logistic.json"
    primary_auroc = run_auroc_rust(
        args.cli,
        prediction_input,
        primary_auroc_path,
        profile,
        "score_random_forest",
        "score_full_logistic",
        resampling_seed=MASTER_SEED + 301,
    )
    control_auroc = run_auroc_rust(
        args.cli,
        prediction_input,
        control_auroc_path,
        profile,
        "score_full_logistic",
        "score_demographic_logistic",
        resampling_seed=MASTER_SEED + 302,
    )
    validate_auroc_report(primary_auroc, profile)
    validate_auroc_report(control_auroc, profile)

    write_computational_stability(
        args.cli,
        prediction_input,
        profile,
        primary,
        control,
        primary_auroc,
        control_auroc,
    )

    render_figures_and_tables(
        args.profile,
        profile,
        predictions,
        context,
        primary,
        control,
        primary_auroc,
        control_auroc,
    )
    write_manifest(args.profile, profile, context, args.cli)
    prediction_input.unlink()
    print(f"Study complete: {OUTPUTS}")


if __name__ == "__main__":
    main()
