#!/usr/bin/env python3
"""Build the conditional ProteinGym full-nesting pedagogical illustration."""

from __future__ import annotations

import argparse
import json
import math
import os
import platform
import shutil
import subprocess
import sys
from dataclasses import asdict, dataclass
from itertools import combinations
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))
from domain_study.search_evidence import effect_columns, evidence_columns
STUDY = Path(__file__).resolve().parent
PARENT_STUDY = ROOT / "domain_study" / "proteingym"
SOURCE_SCORES = PARENT_STUDY / "outputs" / "predictions" / "proteingym_observed_scores.csv"
SOURCE_AUDIT = PARENT_STUDY / "outputs" / "tables" / "assay_audit.csv"
SOURCE_MANIFEST = PARENT_STUDY / "outputs" / "manifest.json"
OUTPUTS = Path(os.environ.get("SUPPORTED_AP_PROTEINGYM_NESTED_OUTPUTS", STUDY / "outputs"))
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
from matplotlib.colors import TwoSlopeNorm  # noqa: E402

TARGET_LOWER = 0.40
TARGET_UPPER = 0.50
EMPIRICAL_ORDER = 2
COMPUTATIONAL_ORDER = 2
MAGNITUDE_THRESHOLD = 0.0
SURVIVAL_FLOOR = 0.0
SURVIVAL_REQUIREMENT = 0.60
MASTER_SEED = 20260809
MAXIMUM_REDRAWS = 10_000
CANDIDATE = "score_esm1v_ensemble"
INCUMBENT = "score_esm1v_single"

TRANSPORT_JUSTIFICATION = (
    "conditional illustration: within each retained DMS assay, the observed "
    "class-conditional paired score distributions are held fixed while the "
    "hypothetical 40%-50% deleterious-variant composition is reweighted"
)
REFERENCE_LIMITATION = (
    "conditional illustration only; variants are experimentally structured "
    "within DMS assays and no exchangeable-label reference law is asserted"
)

BLUE = "#0072B2"
ORANGE = "#E69F00"
GREEN = "#009E73"
VERMILLION = "#D55E00"
PURPLE = "#CC79A7"
GRAY = "#5A5A5A"
LIGHT_GRAY = "#D9D9D9"
VERY_LIGHT = "#F2F2F2"


@dataclass(frozen=True)
class Profile:
    computational_replications: int
    grid_points: int
    search_tolerance: float
    search_iterations: int


PROFILES = {
    "quick": Profile(16, 3, 1e-6, 64),
    "publication": Profile(200, 3, 1e-8, 128),
}


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--profile", choices=PROFILES, default="quick")
    parser.add_argument(
        "--example",
        type=Path,
        default=ROOT / "target" / "release" / "examples" / "proteingym_nested",
    )
    parser.add_argument(
        "--render-only",
        action="store_true",
        help="validate and redraw the isolated retained nested-study artifacts",
    )
    return parser.parse_args()


def prepare_outputs() -> None:
    if OUTPUTS.exists():
        shutil.rmtree(OUTPUTS)
    for directory in (FIGURES, TABLES, REPORTS, PREDICTIONS):
        directory.mkdir(parents=True, exist_ok=True)


def position_from_variant(variant: pd.Series) -> pd.Series:
    extracted = variant.astype(str).str.extract(r"^[A-Za-z*](\d+)[A-Za-z*]$", expand=False)
    if extracted.isna().any():
        examples = variant.loc[extracted.isna()].astype(str).head(5).tolist()
        raise RuntimeError(f"single-substitution position could not be parsed: {examples}")
    return extracted.astype(int)


def build_nested_input() -> tuple[pd.DataFrame, pd.DataFrame, Path]:
    for path in (SOURCE_SCORES, SOURCE_AUDIT, SOURCE_MANIFEST):
        if not path.exists():
            raise FileNotFoundError(
                f"required retained ProteinGym artifact is missing: {path}; "
                "run the parent publication study first"
            )
    manifest = json.loads(SOURCE_MANIFEST.read_text())
    if manifest.get("manuscript_version") != "v17" or manifest.get("profile") != "publication":
        raise RuntimeError("the conditional study requires retained V17 publication artifacts")
    scores = pd.read_csv(SOURCE_SCORES)
    required = {"assay_id", "variant_id", "label", CANDIDATE, INCUMBENT}
    missing = required.difference(scores.columns)
    if missing:
        raise RuntimeError(f"retained score columns are missing: {sorted(missing)}")
    scores = scores.loc[:, ["assay_id", "variant_id", "label", CANDIDATE, INCUMBENT]].copy()
    scores["position"] = position_from_variant(scores["variant_id"])
    scores["position_id"] = scores["assay_id"].astype(str) + ":" + scores["position"].astype(str)
    if scores.duplicated(["assay_id", "variant_id"]).any():
        raise RuntimeError("retained ProteinGym input has duplicate assay-variant rows")
    audit = pd.read_csv(SOURCE_AUDIT).sort_values("assay_id").reset_index(drop=True)
    if sorted(scores["assay_id"].unique()) != audit["assay_id"].tolist():
        raise RuntimeError("retained scores and assay audit identify different empirical rows")
    path = PREDICTIONS / "conditional_nested_input.csv"
    scores.to_csv(path, index=False)
    return scores, audit, path


def rust_command(example: Path, input_path: Path, output_path: Path, profile: Profile) -> list[str]:
    return [
        str(example),
        "--input",
        str(input_path),
        "--output",
        str(output_path),
        "--evaluation-id-col",
        "assay_id",
        "--cluster-id-col",
        "position_id",
        "--label-col",
        "label",
        "--model-a-col",
        CANDIDATE,
        "--model-b-col",
        INCUMBENT,
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
        "--empirical-order",
        str(EMPIRICAL_ORDER),
        "--computational-order",
        str(COMPUTATIONAL_ORDER),
        "--computational-replications",
        str(profile.computational_replications),
        "--resampling-seed",
        str(MASTER_SEED),
        "--maximum-redraws",
        str(MAXIMUM_REDRAWS),
        "--magnitude-threshold",
        str(MAGNITUDE_THRESHOLD),
        "--survival-floor",
        str(SURVIVAL_FLOOR),
        "--survival-requirement",
        str(SURVIVAL_REQUIREMENT),
        "--transport-justification",
        TRANSPORT_JUSTIFICATION,
        "--reference-limitation",
        REFERENCE_LIMITATION,
        "--execution",
        "parallel",
    ]


def validate_report(report: dict[str, Any], profile: Profile, audit: pd.DataFrame) -> None:
    if report.get("schema_version") != 17 or report.get("manuscript_version") != "v17":
        raise RuntimeError("unexpected nested report schema")
    if report.get("analysis") != "ap_conditional_clustered_nested_illustration":
        raise RuntimeError("unexpected nested analysis identifier")
    if report.get("conditional_illustration") is not True:
        raise RuntimeError("nested report must identify itself as conditional")
    expected_ids = audit["assay_id"].tolist()
    if report.get("evaluation_ids") != expected_ids:
        raise RuntimeError("nested report evaluation IDs do not match the retained assay audit")
    assessment = report["assessment"]
    target = assessment["target_prevalences"]
    if target["lower"] != TARGET_LOWER or target["upper"] != TARGET_UPPER:
        raise RuntimeError("nested report used the wrong target-prevalence interval")
    if assessment["empirical_order"] != EMPIRICAL_ORDER:
        raise RuntimeError("nested report used the wrong empirical order")
    if assessment["computational_order"] != COMPUTATIONAL_ORDER:
        raise RuntimeError("nested report used the wrong computational order")
    if assessment["computational_replications_per_row"] != profile.computational_replications:
        raise RuntimeError("nested report used the wrong replication count")
    if assessment["resampling"]["unit"] != "residue_position_within_assay":
        raise RuntimeError("nested report did not use position clusters")
    result = report["result"]
    if result["evaluation_count"] != len(expected_ids):
        raise RuntimeError("nested report evaluation count mismatch")
    gate = result["forward"]["observed_gate"]
    if gate["verdict"] != "supported_replacement":
        raise RuntimeError("the conditional forward assessment did not clear Stage 1")
    full = result["forward"]["full_assessment"]
    if full is None:
        raise RuntimeError("the passing conditional gate did not enter Stage 2")
    if len(full["retained_effect_rows"]) != len(expected_ids):
        raise RuntimeError("nested report lost empirical row identity")
    for row in full["retained_effect_rows"]:
        if len(row["computational"]) != profile.computational_replications:
            raise RuntimeError("nested report has an incomplete computational row")
    if full["supported_magnitude"] > gate["supported_magnitude"] + 1e-12:
        raise RuntimeError("anchored magnitude exceeds its observed gate")
    if (
        full["literal_survival"]["subset_fraction"]
        > gate["literal_survival"]["subset_fraction"] + 1e-12
    ):
        raise RuntimeError("anchored survival exceeds its observed gate")


def configure_plots() -> None:
    plt.rcParams.update(
        {
            "figure.dpi": 140,
            "savefig.dpi": 220,
            "font.size": 10.5,
            "axes.titlesize": 11.5,
            "axes.labelsize": 10.5,
            "legend.fontsize": 8.7,
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


def report_parts(report: dict[str, Any]) -> tuple[dict[str, Any], dict[str, Any]]:
    forward = report["result"]["forward"]
    full = forward["full_assessment"]
    if full is None:
        raise RuntimeError("conditional illustration requires the full forward assessment")
    return forward["observed_gate"], full


def extract_tables(
    report: dict[str, Any], audit: pd.DataFrame
) -> tuple[pd.DataFrame, pd.DataFrame, pd.DataFrame]:
    gate, full = report_parts(report)
    ids = report["evaluation_ids"]
    evaluations = report["result"]["evaluations"]
    rows = full["retained_effect_rows"]
    row_survival = full["literal_survival"]["rows"]
    designs = {row["evaluation_id"]: row for row in report["assessment"]["row_designs"]}
    diagnostics = {
        row["evaluation_id"]: row for row in report["computational_diagnostics"]
    }
    records: list[dict[str, Any]] = []
    for assay_id, evaluation, retained, survival in zip(ids, evaluations, rows, row_survival):
        effects = np.array([effect["value"] for effect in retained["computational"]])
        record = {
            "assay_id": assay_id,
            "observed_anchor": retained["observed"]["value"],
            **effect_columns(retained["observed"], "observed_"),
            "limiting_prevalence": retained["observed"]["limiting_prevalence"],
            "computational_minimum": float(np.min(effects)),
            "computational_q05": float(np.quantile(effects, 0.05)),
            "computational_median": float(np.median(effects)),
            "computational_q95": float(np.quantile(effects, 0.95)),
            "computational_maximum": float(np.max(effects)),
            "computational_survivors": survival["computational_survivor_count"],
            "row_subset_survival": survival["subset_fraction"],
            "position_cluster_count": designs[assay_id]["position_cluster_count"],
            "rejected_missing_class_draws": diagnostics[assay_id][
                "rejected_missing_class_draws"
            ],
            "diagnostic_profile": evaluation["diagnostic_difference_profile"],
        }
        records.append(record)
    anchors = pd.DataFrame(records)
    anchors = anchors.merge(
        audit.loc[:, ["assay_id", "uniprot_id", "selection_type"]],
        on="assay_id",
        how="left",
        validate="one_to_one",
    )

    pair_rows = []
    for left, right in combinations(anchors.itertuples(index=False), 2):
        pair_rows.append(
            {
                "assay_left": left.assay_id,
                "assay_right": right.assay_id,
                "pair_minimum": min(left.observed_anchor, right.observed_anchor),
                "clears_zero": left.observed_anchor > 0 and right.observed_anchor > 0,
            }
        )
    pair_minima = pd.DataFrame(pair_rows)

    summary = pd.DataFrame(
        [
            {
                "stage": "observed_gate",
                "supported_magnitude": gate["supported_magnitude"],
                "literal_survival": gate["literal_survival"]["subset_fraction"],
                "verdict": gate["verdict"],
                **evidence_columns(gate),
                "mean_observed_anchor": gate["mean_retained_effect"],
                "survivor_count": gate["literal_survival"]["survivor_count"],
                "effect_count": gate["literal_survival"]["list_length"],
            },
            {
                "stage": "anchored_full_assessment",
                "supported_magnitude": full["supported_magnitude"],
                "literal_survival": full["literal_survival"]["subset_fraction"],
                "verdict": full["verdict"],
                **evidence_columns(full),
                "mean_observed_anchor": gate["mean_retained_effect"],
                "survivor_count": np.nan,
                "effect_count": len(rows),
            },
        ]
    )
    return anchors, pair_minima, summary


def build_survival_curve(report: dict[str, Any], levels: np.ndarray) -> pd.DataFrame:
    gate, full = report_parts(report)
    del gate
    retained_rows = full["retained_effect_rows"]
    m = len(retained_rows)
    denominator_empirical = math.comb(m, EMPIRICAL_ORDER)
    records = []
    for level in levels:
        observed_survivors = sum(row["observed"]["value"] > level for row in retained_rows)
        observed_fraction = (
            math.comb(observed_survivors, EMPIRICAL_ORDER) / denominator_empirical
            if observed_survivors >= EMPIRICAL_ORDER
            else 0.0
        )
        q = []
        for row in retained_rows:
            effects = [effect["value"] for effect in row["computational"]]
            survivors = sum(value > level for value in effects)
            denominator = math.comb(len(effects), COMPUTATIONAL_ORDER)
            subset = (
                math.comb(survivors, COMPUTATIONAL_ORDER) / denominator
                if survivors >= COMPUTATIONAL_ORDER
                else 0.0
            )
            q.append(subset if row["observed"]["value"] > level else 0.0)
        sum_q = float(np.sum(q))
        sum_q2 = float(np.sum(np.square(q)))
        nested_fraction = (sum_q * sum_q - sum_q2) / (m * (m - 1))
        records.append(
            {
                "effect_level": level,
                "observed_pair_survival": observed_fraction,
                "anchored_nested_survival": nested_fraction,
            }
        )
    return pd.DataFrame(records)


def plot_condition_challenge(report: dict[str, Any], anchors: pd.DataFrame) -> None:
    prevalences = np.array(
        [
            point["prevalence"]
            for point in report["result"]["evaluations"][0]["diagnostic_difference_profile"]
        ]
    )
    profiles = np.array(
        [
            [point["value"] for point in evaluation["diagnostic_difference_profile"]]
            for evaluation in report["result"]["evaluations"]
        ]
    )
    order = np.argsort(anchors["observed_anchor"].to_numpy())
    ordered_anchors = anchors.iloc[order].reset_index(drop=True)
    profiles = profiles[order]
    bound = float(np.quantile(np.abs(profiles), 0.98))
    bound = max(bound, 1e-6)
    figure, (heat, dots) = plt.subplots(
        1,
        2,
        figsize=(11.4, 7.2),
        gridspec_kw={"width_ratios": [3.2, 1.25], "wspace": 0.12},
        sharey=True,
    )
    image = heat.imshow(
        profiles,
        aspect="auto",
        origin="lower",
        extent=[prevalences[0], prevalences[-1], -0.5, len(order) - 0.5],
        cmap="RdBu_r",
        norm=TwoSlopeNorm(vmin=-bound, vcenter=0.0, vmax=bound),
        interpolation="nearest",
    )
    heat.set_title("A. Paired CNAP advantage throughout the conditional range", loc="left")
    heat.set_xlabel("Target deleterious prevalence")
    heat.set_ylabel("Assays, ordered by retained effect")
    heat.grid(False)
    colorbar = figure.colorbar(image, ax=heat, fraction=0.035, pad=0.02)
    colorbar.set_label("ESM-1v ensemble − single")
    y = np.arange(len(ordered_anchors))
    values = ordered_anchors["observed_anchor"].to_numpy()
    colors = np.where(values > 0, BLUE, VERMILLION)
    dots.scatter(values, y, c=colors, s=18, edgecolor="white", linewidth=0.25)
    dots.axvline(0, color="black", linewidth=1)
    dots.set_title("B. Infimum retained by each assay", loc="left")
    dots.set_xlabel("Observed anchor")
    dots.grid(axis="x")
    dots.grid(axis="y", visible=False)
    dots.text(
        0.03,
        0.98,
        f"{int((values > 0).sum())}/{len(values)} anchors > 0",
        transform=dots.transAxes,
        ha="left",
        va="top",
        fontweight="bold",
    )
    figure.suptitle(
        "Conditional illustration: every assay first faces the complete 40%–50% prevalence challenge",
        fontsize=13,
        fontweight="bold",
        y=1.01,
    )
    save_figure(figure, "01-condition-challenge-to-assay-anchors")


def plot_observed_gate(
    report: dict[str, Any], anchors: pd.DataFrame, pair_minima: pd.DataFrame
) -> None:
    gate, _ = report_parts(report)
    figure = plt.figure(figsize=(11.4, 7.4))
    grid = figure.add_gridspec(2, 2, width_ratios=[1.65, 1], hspace=0.42, wspace=0.28)
    distribution = figure.add_subplot(grid[:, 0])
    magnitude = figure.add_subplot(grid[0, 1])
    survival = figure.add_subplot(grid[1, 1])

    values = pair_minima["pair_minimum"].to_numpy()
    distribution.hist(values, bins=45, color=BLUE, alpha=0.82, edgecolor="white")
    distribution.axvline(0, color="black", linewidth=1.1, label="Floor d = 0")
    distribution.axvline(
        gate["supported_magnitude"],
        color=ORANGE,
        linewidth=2,
        label="Mean of pair minima (support)",
    )
    distribution.set_title("A. All 4,095 observed order-2 challenge minima", loc="left")
    distribution.set_xlabel("Minimum retained effect in an assay pair")
    distribution.set_ylabel("Number of distinct assay pairs")
    distribution.legend(frameon=False)
    distribution.text(
        0.03,
        0.97,
        f"{int(pair_minima['clears_zero'].sum()):,}/{len(pair_minima):,} pairs clear zero",
        transform=distribution.transAxes,
        va="top",
        fontweight="bold",
    )

    magnitude_values = [gate["mean_retained_effect"], gate["supported_magnitude"]]
    magnitude.barh([0, 1], magnitude_values, color=[LIGHT_GRAY, ORANGE])
    magnitude.axvline(MAGNITUDE_THRESHOLD, color="black", linewidth=1)
    magnitude.set_yticks([0, 1], ["Mean assay anchor", "Order-2 support"])
    magnitude.set_title("B. Noncompensation changes magnitude", loc="left")
    magnitude.set_xlabel("Paired CNAP effect")
    for index, value in enumerate(magnitude_values):
        magnitude.text(value, index, f"  {value:.5f}", va="center", fontweight="bold")

    observed_survival = gate["literal_survival"]["subset_fraction"]
    survival.barh([0], [observed_survival], color=GREEN, height=0.46)
    survival.axvline(SURVIVAL_REQUIREMENT, color=VERMILLION, linewidth=2)
    survival.set_xlim(0, 1)
    survival.set_yticks([0], ["Observed pair survival"])
    survival.set_xlabel("Fraction of order-2 assay challenges")
    survival.set_title("C. The illustrative empirical gate", loc="left")
    survival.text(
        observed_survival,
        0,
        f"  {observed_survival:.3f}",
        va="center",
        fontweight="bold",
    )
    survival.text(
        SURVIVAL_REQUIREMENT,
        0.94,
        f"γ = {SURVIVAL_REQUIREMENT:.2f}",
        transform=survival.get_xaxis_transform(),
        ha="center",
        va="top",
        color=VERMILLION,
        fontweight="bold",
    )
    figure.suptitle(
        "Stage 1: pair complete empirical evaluations before averaging",
        fontsize=13,
        fontweight="bold",
        y=0.99,
    )
    save_figure(figure, "02-observed-order-2-gate")


def plot_full_nested(
    report: dict[str, Any], anchors: pd.DataFrame, survival_curve: pd.DataFrame
) -> None:
    gate, full = report_parts(report)
    ordered = anchors.sort_values("observed_anchor").reset_index(drop=True)
    x = np.arange(len(ordered))
    figure = plt.figure(figsize=(12.0, 10.0))
    grid = figure.add_gridspec(2, 2, hspace=0.38, wspace=0.28)
    rows_axis = figure.add_subplot(grid[0, :])
    curve_axis = figure.add_subplot(grid[1, 0])
    decision_grid = grid[1, 1].subgridspec(2, 1, hspace=0.58)
    magnitude_axis = figure.add_subplot(decision_grid[0, 0])
    survival_axis = figure.add_subplot(decision_grid[1, 0])

    rows_axis.fill_between(
        x,
        ordered["computational_q05"],
        ordered["computational_q95"],
        color=BLUE,
        alpha=0.20,
        label="Computational 5%–95% interval",
    )
    rows_axis.plot(
        x,
        ordered["computational_median"],
        color=BLUE,
        linewidth=1.2,
        label="Computational median",
    )
    rows_axis.scatter(
        x,
        ordered["observed_anchor"],
        color=ORANGE,
        s=20,
        zorder=3,
        label="Mandatory observed anchor",
    )
    rows_axis.axhline(0, color="black", linewidth=1)
    rows_axis.set_title(
        "A. Each empirical row keeps its anchor and adds position-bootstrap challenges",
        loc="left",
    )
    rows_axis.set_xlabel("Assays, ordered by observed anchor")
    rows_axis.set_ylabel("Retained paired effect")
    rows_axis.legend(frameon=False, ncol=3, loc="upper left")

    curve_axis.plot(
        survival_curve["effect_level"],
        survival_curve["observed_pair_survival"],
        color=GRAY,
        linewidth=2,
        label="Observed-only, K_C = 0",
    )
    curve_axis.plot(
        survival_curve["effect_level"],
        survival_curve["anchored_nested_survival"],
        color=PURPLE,
        linewidth=2.2,
        label="Full anchored, K_C = 2",
    )
    curve_axis.axvline(SURVIVAL_FLOOR, color="black", linewidth=1)
    curve_axis.axhline(SURVIVAL_REQUIREMENT, color=VERMILLION, linewidth=1.5)
    curve_axis.set_xlim(
        float(survival_curve["effect_level"].quantile(0.12)),
        float(survival_curve["effect_level"].quantile(0.88)),
    )
    curve_axis.set_ylim(0, 1)
    curve_axis.set_title("B. Anchoring can only lower challenge survival", loc="left")
    curve_axis.set_xlabel("Effect floor s")
    curve_axis.set_ylabel("Complete-challenge survival")
    curve_axis.legend(frameon=False)

    stage_names = ["Observed gate", "Full nested"]
    magnitude = [gate["supported_magnitude"], full["supported_magnitude"]]
    survival = [
        gate["literal_survival"]["subset_fraction"],
        full["literal_survival"]["subset_fraction"],
    ]
    positions = np.arange(2)
    magnitude_bars = magnitude_axis.bar(positions, magnitude, color=ORANGE, width=0.58)
    magnitude_axis.axhline(MAGNITUDE_THRESHOLD, color="black", linewidth=1)
    magnitude_axis.set_xticks(positions, stage_names)
    magnitude_axis.set_ylabel("Supported magnitude")
    magnitude_axis.set_ylim(
        min(-0.02, min(magnitude) * 1.25),
        max(0.003, max(magnitude) * 4),
    )
    magnitude_axis.set_title("C. Magnitude criterion", loc="left", pad=8)
    for bar, value in zip(magnitude_bars, magnitude):
        if value >= 0:
            label_y = value + 0.00035
            vertical = "bottom"
            color = ORANGE
        else:
            label_y = value + 0.0013
            vertical = "bottom"
            color = "white"
        magnitude_axis.text(
            bar.get_x() + bar.get_width() / 2,
            label_y,
            f"{value:.4f}",
            ha="center",
            va=vertical,
            fontweight="bold",
            color=color,
        )

    survival_bars = survival_axis.bar(positions, survival, color=GREEN, width=0.58)
    survival_axis.axhline(
        SURVIVAL_REQUIREMENT,
        color=VERMILLION,
        linewidth=1.3,
        linestyle="--",
        label=f"Illustrative γ = {SURVIVAL_REQUIREMENT:.2f}",
    )
    survival_axis.set_xticks(positions, stage_names)
    survival_axis.set_ylim(0, 1)
    survival_axis.set_ylabel("Literal survival")
    survival_axis.set_title("D. Breadth criterion", loc="left")
    survival_axis.legend(frameon=False, loc="upper right")
    for bar, value in zip(survival_bars, survival):
        survival_axis.text(
            bar.get_x() + bar.get_width() / 2,
            value + 0.025,
            f"{value:.3f}",
            ha="center",
            fontweight="bold",
        )
    figure.suptitle(
        "Stage 2: the observed-anchored full nested calculation",
        fontsize=13,
        fontweight="bold",
        y=0.99,
    )
    save_figure(figure, "03-observed-anchored-full-nested-result")


def write_report(
    profile_name: str,
    profile: Profile,
    report: dict[str, Any],
    anchors: pd.DataFrame,
    pair_minima: pd.DataFrame,
) -> None:
    gate, full = report_parts(report)
    nested_verdict = full["verdict"].replace("_", " ")
    redraws = int(anchors["rejected_missing_class_draws"].sum())
    text = f"""# Conditional ProteinGym full-nesting illustration

This is a pedagogical conditional calculation, not a substantive ProteinGym
replacement conclusion. It asks what the V17 full nested calculation would
return **if** 40%–50% deleterious prevalence were scientifically applicable,
residue position were the actionable within-assay resampling unit, and the
illustrative policy required more than 60% literal survival.

## Conditional specification

- Comparison: ESM-1v ensemble − ESM-1v single.
- Empirical evaluations: 91 retained ProteinGym assays.
- Target prevalence: 40%–50%.
- Empirical order: K_E=2; computational order: K_C=2.
- Magnitude threshold and survival floor: δ=d=0.
- Illustrative survival requirement: γ=0.60.
- Computational replications: J={profile.computational_replications} per assay.
- Resampling: draw the observed number of residue-position clusters with
  replacement within each assay; carry all substitutions at a selected
  position; redraw a complete bootstrap only when it lacks an outcome class.
- Profile: `{profile_name}`.

## Stage 1: observed empirical gate

- Mean assay anchor: {gate['mean_retained_effect']:+.6f}.
- Order-2 supported magnitude: {gate['supported_magnitude']:+.6f}.
- Positive anchors: {gate['literal_survival']['survivor_count']}/{gate['literal_survival']['list_length']}.
- Surviving assay pairs: {int(pair_minima['clears_zero'].sum()):,}/{len(pair_minima):,}
  = {gate['literal_survival']['subset_fraction']:.6f}.
- Gate verdict: `{gate['verdict']}`.

## Stage 2: observed-anchored computational challenge

- Full supported magnitude: {full['supported_magnitude']:+.6f}.
- Full literal survival: {full['literal_survival']['subset_fraction']:.6f}.
- Full conditional verdict: `{nested_verdict}`.
- Missing-class complete bootstrap redraws: {redraws:,} across
  {len(anchors) * profile.computational_replications:,} retained computational
  evaluations.

Every complete challenge selects two assay rows, retains both observed anchors,
selects two computational effects within each row, and takes the minimum over
all six effects before averaging. Computational multiplicity never creates new
empirical rows.

## Interpretation

The interval, comparison, breadth requirement, and position-bootstrap law are
introduced to display the complete nested arithmetic on real paired assay data.
They were not established prospectively as the scientifically appropriate
ProteinGym deployment regime. Passage or failure therefore describes this
conditional calculation only and must not be reported as evidence for replacing
a ProteinGym model in practice.
"""
    (OUTPUTS / "REPORT.md").write_text(text)


def write_manifest(profile_name: str, profile: Profile, report: dict[str, Any]) -> None:
    manifest = {
        "study": "proteingym_conditional_full_nesting_illustration",
        "manuscript_version": "v17",
        "conditional_illustration": True,
        "profile": profile_name,
        "parameters": asdict(profile),
        "comparison": {"candidate": CANDIDATE, "incumbent": INCUMBENT},
        "target_prevalence_interval": [TARGET_LOWER, TARGET_UPPER],
        "empirical_order": EMPIRICAL_ORDER,
        "computational_order": COMPUTATIONAL_ORDER,
        "magnitude_threshold": MAGNITUDE_THRESHOLD,
        "survival_floor": SURVIVAL_FLOOR,
        "survival_requirement": SURVIVAL_REQUIREMENT,
        "master_seed": MASTER_SEED,
        "resampling_unit": report["assessment"]["resampling"],
        "source_artifacts": {
            "scores": str(SOURCE_SCORES.relative_to(ROOT)),
            "assay_audit": str(SOURCE_AUDIT.relative_to(ROOT)),
            "parent_manifest": str(SOURCE_MANIFEST.relative_to(ROOT)),
        },
        "software": {
            "python": sys.version,
            "platform": platform.platform(),
            "numpy": np.__version__,
            "pandas": pd.__version__,
        },
    }
    (OUTPUTS / "manifest.json").write_text(json.dumps(manifest, indent=2))


def render(
    profile_name: str,
    profile: Profile,
    report: dict[str, Any],
    audit: pd.DataFrame,
) -> None:
    anchors, pair_minima, summary = extract_tables(report, audit)
    retained_rows = report_parts(report)[1]["retained_effect_rows"]
    all_values = [
        row["observed"]["value"] for row in retained_rows
    ] + [
        effect["value"]
        for row in retained_rows
        for effect in row["computational"]
    ]
    levels = np.unique(
        np.concatenate(
            [
                np.linspace(min(all_values), max(all_values), 401),
                np.array([SURVIVAL_FLOOR]),
            ]
        )
    )
    survival_curve = build_survival_curve(report, levels)
    anchors.drop(columns=["diagnostic_profile"]).to_csv(
        TABLES / "assay_anchor_and_computational_summary.csv", index=False
    )
    pair_minima.to_csv(TABLES / "observed_assay_pair_minima.csv", index=False)
    survival_curve.to_csv(TABLES / "nested_survival_curve.csv", index=False)
    summary.to_csv(TABLES / "staged_summary.csv", index=False)
    configure_plots()
    plot_condition_challenge(report, anchors)
    plot_observed_gate(report, anchors, pair_minima)
    plot_full_nested(report, anchors, survival_curve)
    write_report(profile_name, profile, report, anchors, pair_minima)


def run_clean(args: argparse.Namespace, profile: Profile) -> None:
    if not args.example.exists():
        raise FileNotFoundError(
            f"study-specific Rust example is missing: {args.example}; "
            "build it with `cargo build --release --example proteingym_nested --offline`"
        )
    prepare_outputs()
    _, audit, input_path = build_nested_input()
    report_path = REPORTS / "conditional_nested_esm1v_ensemble_vs_single.json"
    subprocess.run(rust_command(args.example, input_path, report_path, profile), check=True)
    report = json.loads(report_path.read_text())
    validate_report(report, profile, audit)
    render(args.profile, profile, report, audit)
    write_manifest(args.profile, profile, report)
    print(f"Conditional nested illustration complete: {OUTPUTS}")


def render_only(profile_name: str, profile: Profile) -> None:
    report_path = REPORTS / "conditional_nested_esm1v_ensemble_vs_single.json"
    for path in (report_path, SOURCE_AUDIT, OUTPUTS / "manifest.json"):
        if not path.exists():
            raise FileNotFoundError(f"render-only input is missing: {path}")
    manifest = json.loads((OUTPUTS / "manifest.json").read_text())
    if manifest.get("profile") != profile_name:
        raise RuntimeError(
            f"retained profile is {manifest.get('profile')}, not requested {profile_name}"
        )
    audit = pd.read_csv(SOURCE_AUDIT).sort_values("assay_id").reset_index(drop=True)
    report = json.loads(report_path.read_text())
    validate_report(report, profile, audit)
    render(profile_name, profile, report, audit)
    print(f"Conditional nested figures refreshed: {FIGURES}")


def main() -> None:
    args = parse_args()
    profile = PROFILES[args.profile]
    if args.render_only:
        render_only(args.profile, profile)
    else:
        run_clean(args, profile)


if __name__ == "__main__":
    main()
