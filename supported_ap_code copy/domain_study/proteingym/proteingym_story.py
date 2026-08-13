#!/usr/bin/env python3
"""Build the ProteinGym observed-evaluation AP model-comparison study."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import shutil
import subprocess
import sys
import urllib.request
import zipfile
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[2]
STUDY = Path(__file__).resolve().parent
RAW = STUDY / "data" / "raw"
OUTPUTS = Path(os.environ.get("SUPPORTED_AP_PROTEINGYM_OUTPUTS", STUDY / "outputs"))
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
from sklearn.metrics import roc_auc_score  # noqa: E402

PROTEINGYM_VERSION = "v1.3"
PROTEINGYM_COMMIT = "144fe22b07dfaeec2b366f2346203a9838a55b4c"


@dataclass(frozen=True)
class Asset:
    filename: str
    url: str
    sha256: str


ASSETS = (
    Asset(
        "DMS_substitutions.csv",
        (
            "https://raw.githubusercontent.com/OATML-Markslab/ProteinGym/"
            f"{PROTEINGYM_COMMIT}/reference_files/DMS_substitutions.csv"
        ),
        "a8f498011532a74aa9fe556a50555a75e928c5837d19c06a87592ae04049b308",
    ),
    Asset(
        "DMS_ProteinGym_substitutions.zip",
        (
            "https://marks.hms.harvard.edu/proteingym/"
            f"ProteinGym_{PROTEINGYM_VERSION}/DMS_ProteinGym_substitutions.zip"
        ),
        "3a83766254ac9ac9984ec25cb73c6e010ea4418f5e35f143933e6b6e6473b921",
    ),
    Asset(
        "zero_shot_substitutions_scores.zip",
        (
            "https://marks.hms.harvard.edu/proteingym/"
            f"ProteinGym_{PROTEINGYM_VERSION}/zero_shot_substitutions_scores.zip"
        ),
        "3fd7cdb5e78f1d43cabfabfeb6578c252b63af23ba2ab44db0094dc3a42de36d",
    ),
)

METADATA_FILENAME = ASSETS[0].filename
DMS_ARCHIVE_FILENAME = ASSETS[1].filename
SCORE_ARCHIVE_FILENAME = ASSETS[2].filename

TARGET_LOWER = 0.01
TARGET_UPPER = 0.90
SUPPORT_ORDER = 2
SENSITIVITY_ORDERS = (1, 2, 5, 10)
MAGNITUDE_THRESHOLD = 0.0
SURVIVAL_FLOOR = 0.0
SURVIVAL_REQUIREMENT = 0.81
AUROC_OPTIMIZATION_ABSOLUTE_GAP = 1e-6
AUROC_OPTIMIZATION_RELATIVE_GAP = 1e-8
AUROC_OPTIMIZATION_TIME_LIMIT_SECONDS = 1.0
AUROC_CONCENTRATION_TOLERANCE = 1e-4
AUROC_CONCENTRATION_MAX_ITERATIONS = 64

TRANSPORT_JUSTIFICATION = (
    "within each retained DMS assay, the class-conditional paired score "
    "distributions are held fixed while the declared deleterious-variant "
    "composition is reweighted"
)
REFERENCE_LIMITATION = (
    "variants are experimentally structured within DMS assays; no "
    "exchangeable-label reference law is asserted"
)

MODEL_COLUMNS = {
    "score_eve_ensemble": "EVE_ensemble",
    "score_esm1v_single": "ESM1v_single",
    "score_esm1v_ensemble": "ESM1v_ensemble",
    "score_esm2_650m": "ESM2_650M",
}
MODEL_LABELS = {
    "score_eve_ensemble": "EVE ensemble",
    "score_esm1v_single": "ESM-1v single",
    "score_esm1v_ensemble": "ESM-1v ensemble",
    "score_esm2_650m": "ESM-2 650M",
}


@dataclass(frozen=True)
class Comparison:
    slug: str
    candidate: str
    incumbent: str

    @property
    def label(self) -> str:
        return f"{MODEL_LABELS[self.candidate]} − {MODEL_LABELS[self.incumbent]}"

    @property
    def short_label(self) -> str:
        short = {
            "score_eve_ensemble": "EVE",
            "score_esm1v_ensemble": "ESM-1v",
            "score_esm2_650m": "ESM-2",
        }
        return f"{short[self.candidate]} − {short[self.incumbent]}"


COMPARISONS = (
    Comparison(
        "esm1v_ensemble_vs_eve_ensemble",
        "score_esm1v_ensemble",
        "score_eve_ensemble",
    ),
    Comparison(
        "esm2_650m_vs_esm1v_ensemble",
        "score_esm2_650m",
        "score_esm1v_ensemble",
    ),
    Comparison(
        "esm2_650m_vs_eve_ensemble",
        "score_esm2_650m",
        "score_eve_ensemble",
    ),
)


@dataclass(frozen=True)
class Profile:
    grid_points: int
    search_tolerance: float
    search_iterations: int


PROFILES = {
    "quick": Profile(65, 1e-6, 64),
    "publication": Profile(257, 1e-8, 128),
}

BLUE = "#0072B2"
ORANGE = "#E69F00"
GREEN = "#009E73"
VERMILLION = "#D55E00"
GRAY = "#5A5A5A"
LIGHT_GRAY = "#D9D9D9"
VERY_LIGHT = "#F2F2F2"
PURPLE = "#CC79A7"
COMPARISON_COLORS = (BLUE, ORANGE, PURPLE)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--profile", choices=PROFILES, default="quick")
    parser.add_argument(
        "--cli",
        type=Path,
        default=ROOT / "target" / "release" / "supported_ap",
    )
    parser.add_argument(
        "--data-dir",
        type=Path,
        default=Path(os.environ.get("PROTEINGYM_DATA_DIR", RAW)),
    )
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument(
        "--download-only",
        action="store_true",
        help="download and verify pinned inputs, then stop",
    )
    mode.add_argument(
        "--render-only",
        action="store_true",
        help="validate and redraw retained publication artifacts",
    )
    mode.add_argument(
        "--rust-only",
        action="store_true",
        help="rerun V17 Rust assessments from the retained aligned score table",
    )
    mode.add_argument(
        "--auroc-only",
        action="store_true",
        help="add observed AUROC reports to retained AP publication artifacts",
    )
    return parser.parse_args()


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(8 * 1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def download_assets(data_dir: Path) -> None:
    data_dir.mkdir(parents=True, exist_ok=True)
    for asset in ASSETS:
        destination = data_dir / asset.filename
        if destination.exists() and sha256(destination) == asset.sha256:
            print(f"verified {asset.filename}")
            continue
        temporary = destination.with_suffix(destination.suffix + ".part")
        if temporary.exists():
            temporary.unlink()
        print(f"downloading {asset.filename} from {asset.url}")
        with urllib.request.urlopen(asset.url) as response, temporary.open("wb") as out:
            shutil.copyfileobj(response, out, length=8 * 1024 * 1024)
        actual = sha256(temporary)
        if actual != asset.sha256:
            temporary.unlink(missing_ok=True)
            raise RuntimeError(
                f"download checksum mismatch for {asset.filename}: "
                f"{actual} != {asset.sha256}"
            )
        temporary.replace(destination)


def verify_assets(data_dir: Path) -> dict[str, str]:
    verified: dict[str, str] = {}
    for asset in ASSETS:
        path = data_dir / asset.filename
        if not path.exists():
            raise FileNotFoundError(
                f"missing {path}; run `sh {STUDY / 'run.sh'} download`"
            )
        actual = sha256(path)
        if actual != asset.sha256:
            raise RuntimeError(
                f"checksum mismatch for {asset.filename}: {actual} != {asset.sha256}"
            )
        verified[asset.filename] = actual
    return verified


def prepare_outputs() -> None:
    if OUTPUTS.exists():
        shutil.rmtree(OUTPUTS)
    for directory in (FIGURES, TABLES, REPORTS, PREDICTIONS):
        directory.mkdir(parents=True, exist_ok=True)


def load_metadata(path: Path) -> pd.DataFrame:
    metadata = pd.read_csv(path)
    required = {
        "DMS_id",
        "DMS_filename",
        "UniProt_ID",
        "taxon",
        "source_organism",
        "DMS_binarization_method",
        "first_author",
        "title",
        "year",
        "coarse_selection_type",
        "selection_assay",
        "DMS_total_number_mutants",
        "DMS_number_single_mutants",
        "DMS_number_multiple_mutants",
    }
    missing = required.difference(metadata.columns)
    if missing:
        raise RuntimeError(f"metadata columns missing: {sorted(missing)}")
    if metadata["DMS_id"].duplicated().any():
        raise RuntimeError("ProteinGym metadata contains duplicate DMS_id values")
    return metadata


def retained_metadata(metadata: pd.DataFrame) -> pd.DataFrame:
    retained = metadata.loc[
        metadata["DMS_binarization_method"].eq("manual")
        & metadata["DMS_number_single_mutants"].gt(0)
    ].copy()
    retained.sort_values("DMS_id", inplace=True)
    retained.reset_index(drop=True, inplace=True)
    if retained.empty:
        raise RuntimeError("manual-cutoff assay regime is empty")
    return retained


def _read_csv_member(
    archive: zipfile.ZipFile,
    member: str,
    columns: list[str],
) -> pd.DataFrame:
    try:
        with archive.open(member) as handle:
            return pd.read_csv(handle, usecols=columns)
    except KeyError as error:
        raise RuntimeError(f"archive member missing: {member}") from error


def build_observed_frame(
    metadata: pd.DataFrame,
    dms_archive_path: Path,
    score_archive_path: Path,
) -> tuple[pd.DataFrame, pd.DataFrame]:
    retained = retained_metadata(metadata)
    paired_frames: list[pd.DataFrame] = []
    audit_rows: list[dict[str, Any]] = []
    source_columns = ["mutant", "DMS_score_bin"]
    score_columns = source_columns + list(MODEL_COLUMNS.values())

    with (
        zipfile.ZipFile(dms_archive_path) as dms_archive,
        zipfile.ZipFile(score_archive_path) as score_archive,
    ):
        for record in retained.to_dict("records"):
            assay_id = str(record["DMS_id"])
            source = _read_csv_member(
                dms_archive,
                f"DMS_ProteinGym_substitutions/{record['DMS_filename']}",
                source_columns,
            )
            scores = _read_csv_member(
                score_archive,
                f"{assay_id}.csv",
                score_columns,
            )
            source = source.loc[~source["mutant"].str.contains(":", regex=False)].copy()
            scores = scores.loc[~scores["mutant"].str.contains(":", regex=False)].copy()
            if source["mutant"].duplicated().any() or scores["mutant"].duplicated().any():
                raise RuntimeError(f"duplicate single-mutant identifier in {assay_id}")
            source.set_index("mutant", inplace=True)
            scores.set_index("mutant", inplace=True)
            if set(source.index) != set(scores.index):
                only_source = len(set(source.index).difference(scores.index))
                only_scores = len(set(scores.index).difference(source.index))
                raise RuntimeError(
                    f"variant-key mismatch in {assay_id}: "
                    f"source-only={only_source}, score-only={only_scores}"
                )
            scores = scores.reindex(source.index)
            source_labels = source["DMS_score_bin"].to_numpy(dtype=int)
            score_labels = scores["DMS_score_bin"].to_numpy(dtype=int)
            if not np.array_equal(source_labels, score_labels):
                raise RuntimeError(f"outcome mismatch between archives in {assay_id}")
            if not np.isin(source_labels, [0, 1]).all():
                raise RuntimeError(f"non-binary DMS_score_bin in {assay_id}")
            if len(source) != int(record["DMS_number_single_mutants"]):
                raise RuntimeError(
                    f"single-mutant count mismatch in {assay_id}: "
                    f"{len(source)} != {record['DMS_number_single_mutants']}"
                )

            paired = pd.DataFrame(
                {
                    "assay_id": assay_id,
                    "variant_id": source.index.to_numpy(),
                    # Positive means experimentally nonfunctional/deleterious.
                    "label": 1 - source_labels,
                }
            )
            unique_scores: dict[str, int] = {}
            for output_column, source_column in MODEL_COLUMNS.items():
                values = scores[source_column].to_numpy(dtype=float)
                if not np.isfinite(values).all():
                    raise RuntimeError(f"missing or nonfinite {source_column} in {assay_id}")
                # ProteinGym orients the released scores toward greater fitness.
                # Negation makes larger scores favor the declared deleterious class.
                paired[output_column] = -values
                unique_scores[output_column] = int(np.unique(values).size)

            positive = int(paired["label"].sum())
            negative = int(len(paired) - positive)
            if positive == 0 or negative == 0:
                raise RuntimeError(f"both outcome classes are required in {assay_id}")
            audit_rows.append(
                {
                    "assay_id": assay_id,
                    "uniprot_id": record["UniProt_ID"],
                    "taxon": record["taxon"],
                    "source_organism": record["source_organism"],
                    "first_author": record["first_author"],
                    "year": int(record["year"]),
                    "title": record["title"],
                    "selection_type": record["coarse_selection_type"],
                    "selection_assay": record["selection_assay"],
                    "binarization_method": record["DMS_binarization_method"],
                    "single_variant_count": len(paired),
                    "deleterious_count": positive,
                    "functional_count": negative,
                    "deleterious_fraction": positive / len(paired),
                    "source_includes_multiple_mutants": bool(
                        record["DMS_number_multiple_mutants"] > 0
                    ),
                    **{
                        f"unique_{column}": count
                        for column, count in unique_scores.items()
                    },
                }
            )
            paired_frames.append(paired)

    observed = pd.concat(paired_frames, ignore_index=True)
    audit = pd.DataFrame(audit_rows).sort_values("assay_id").reset_index(drop=True)
    publication_key = (
        audit["first_author"].astype(str)
        + "|"
        + audit["year"].astype(str)
        + "|"
        + audit["title"].astype(str)
    )
    audit["assays_for_uniprot"] = audit.groupby("uniprot_id")["assay_id"].transform(
        "size"
    )
    audit["assays_from_publication"] = publication_key.map(
        publication_key.value_counts()
    )
    return observed, audit


def observed_ap_command(
    cli: Path,
    input_path: Path,
    output_path: Path,
    profile: Profile,
    comparison: Comparison,
    empirical_order: int,
) -> list[str]:
    return [
        str(cli),
        "ap",
        "observed",
        "--input",
        str(input_path),
        "--output",
        str(output_path),
        "--evaluation-id-col",
        "assay_id",
        "--label-col",
        "label",
        "--model-a-col",
        comparison.candidate,
        "--model-b-col",
        comparison.incumbent,
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
        str(empirical_order),
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


def observed_auroc_command(
    cli: Path,
    input_path: Path,
    output_path: Path,
    comparison: Comparison,
) -> list[str]:
    return [
        str(cli),
        "auroc",
        "observed",
        "--input",
        str(input_path),
        "--output",
        str(output_path),
        "--evaluation-id-col",
        "assay_id",
        "--label-col",
        "label",
        "--model-a-col",
        comparison.candidate,
        "--model-b-col",
        comparison.incumbent,
        "--empirical-order",
        str(SUPPORT_ORDER),
        "--magnitude-threshold",
        str(MAGNITUDE_THRESHOLD),
        "--survival-floor",
        str(SURVIVAL_FLOOR),
        "--survival-requirement",
        str(SURVIVAL_REQUIREMENT),
        "--reference-limitation",
        REFERENCE_LIMITATION,
        "--optimization-absolute-gap",
        str(AUROC_OPTIMIZATION_ABSOLUTE_GAP),
        "--optimization-relative-gap",
        str(AUROC_OPTIMIZATION_RELATIVE_GAP),
        "--optimization-time-limit-seconds",
        str(AUROC_OPTIMIZATION_TIME_LIMIT_SECONDS),
        "--solver-threads",
        "1",
        "--concentration-tolerance",
        str(AUROC_CONCENTRATION_TOLERANCE),
        "--concentration-max-iterations",
        str(AUROC_CONCENTRATION_MAX_ITERATIONS),
        "--execution",
        "parallel",
    ]


def read_json(path: Path) -> dict[str, Any]:
    with path.open() as handle:
        return json.load(handle)


def validate_report(
    report: dict[str, Any],
    expected_ids: list[str],
    empirical_order: int,
) -> None:
    if report.get("schema_version") != 17 or report.get("manuscript_version") != "v17":
        raise RuntimeError("unexpected Rust report schema")
    if report.get("analysis") != "ap_observed_empirical_gate":
        raise RuntimeError("ProteinGym study requires observed AP evidence")
    if report.get("evaluation_ids") != expected_ids:
        raise RuntimeError("Rust report evaluation IDs do not match retained assays")
    result = report["result"]
    if result["evidence"] != "observed":
        raise RuntimeError("Rust report did not return observed evidence")
    if result["evaluation_count"] != len(expected_ids):
        raise RuntimeError("Rust report evaluation count mismatch")
    if result["empirical_order"] != empirical_order:
        raise RuntimeError("Rust report empirical order mismatch")
    if result["forward"]["monte_carlo_standard_error"] is not None:
        raise RuntimeError("observed evidence must not report Monte Carlo error")


def validate_auroc_report(
    report: dict[str, Any],
    expected_ids: list[str],
) -> None:
    if report.get("schema_version") != 17 or report.get("manuscript_version") != "v17":
        raise RuntimeError("unexpected Rust report schema")
    if report.get("analysis") != "auroc_observed_empirical_breakdown":
        raise RuntimeError("ProteinGym study requires observed AUROC evidence")
    if report.get("evaluation_ids") != expected_ids:
        raise RuntimeError("Rust AUROC report evaluation IDs do not match retained assays")
    result = report["result"]
    if result["evidence"] != "observed":
        raise RuntimeError("Rust report did not return observed AUROC evidence")
    if result["empirical_order"] != SUPPORT_ORDER:
        raise RuntimeError("observed AUROC empirical order mismatch")
    if result["computational_order"] is not None:
        raise RuntimeError("observed ProteinGym AUROC must not have a computational order")
    if len(result["evaluation_counts"]) != len(expected_ids):
        raise RuntimeError("observed AUROC evaluation count mismatch")
    gate = result["observed_gate"]
    if gate["concentration_factor"] != 1.0:
        raise RuntimeError("observed AUROC gate must be evaluated at Gamma=1")
    if gate["list_length"] != len(expected_ids):
        raise RuntimeError("observed AUROC gate length mismatch")
    if not (
        gate["supported_magnitude_lower"]
        <= gate["supported_magnitude"]
        <= gate["supported_magnitude_upper"]
    ):
        raise RuntimeError("observed AUROC magnitude is outside its certificate")
    if not (
        gate["literal_survival_lower"]
        <= gate["literal_survival_fraction"]
        <= gate["literal_survival_upper"]
    ):
        raise RuntimeError("observed AUROC survival is outside its certificate")


def run_rust_reports(
    cli: Path,
    input_path: Path,
    profile: Profile,
    assay_ids: list[str],
) -> dict[tuple[str, int], dict[str, Any]]:
    reports: dict[tuple[str, int], dict[str, Any]] = {}
    for comparison in COMPARISONS:
        for order in SENSITIVITY_ORDERS:
            filename = (
                f"{comparison.slug}.json"
                if order == SUPPORT_ORDER
                else f"sensitivity_{comparison.slug}_k{order}.json"
            )
            path = REPORTS / filename
            subprocess.run(
                observed_ap_command(
                    cli,
                    input_path,
                    path,
                    profile,
                    comparison,
                    order,
                ),
                check=True,
            )
            report = read_json(path)
            validate_report(report, assay_ids, order)
            reports[(comparison.slug, order)] = report
    return reports


def run_auroc_reports(
    cli: Path,
    input_path: Path,
    assay_ids: list[str],
) -> dict[str, dict[str, Any]]:
    reports: dict[str, dict[str, Any]] = {}
    for comparison in COMPARISONS:
        path = REPORTS / f"auroc_observed_{comparison.slug}.json"
        subprocess.run(
            observed_auroc_command(cli, input_path, path, comparison),
            check=True,
        )
        report = read_json(path)
        validate_auroc_report(report, assay_ids)
        reports[comparison.slug] = report
    return reports


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


def extract_tables(
    reports: dict[tuple[str, int], dict[str, Any]],
    audit: pd.DataFrame,
) -> tuple[pd.DataFrame, pd.DataFrame, pd.DataFrame]:
    effect_rows: list[dict[str, Any]] = []
    summary_rows: list[dict[str, Any]] = []
    sensitivity_rows: list[dict[str, Any]] = []
    audit_by_id = audit.set_index("assay_id")

    for comparison in COMPARISONS:
        main = reports[(comparison.slug, SUPPORT_ORDER)]
        if len(main["evaluation_ids"]) != len(main["result"]["evaluations"]):
            raise RuntimeError("Rust report ID and evaluation lengths differ")
        for assay_id, evaluation in zip(
            main["evaluation_ids"], main["result"]["evaluations"]
        ):
            effect_rows.append(
                {
                    "comparison": comparison.slug,
                    "comparison_label": comparison.label,
                    "assay_id": assay_id,
                    "uniprot_id": audit_by_id.loc[assay_id, "uniprot_id"],
                    "selection_type": audit_by_id.loc[assay_id, "selection_type"],
                    "retained_cnap_difference": evaluation["forward"]["value"],
                    "limiting_deleterious_prevalence": evaluation["forward"][
                        "limiting_prevalence"
                    ],
                    "reverse_retained_cnap_difference": evaluation["reverse"]["value"],
                    "deleterious_count": evaluation["class_counts"]["positive"],
                    "functional_count": evaluation["class_counts"]["negative"],
                }
            )

        for orientation in ("forward", "reverse"):
            directional = main["result"][orientation]
            survival = directional["literal_survival"]
            summary_rows.append(
                {
                    "comparison": comparison.slug,
                    "comparison_label": comparison.label,
                    "orientation": orientation,
                    "empirical_order": SUPPORT_ORDER,
                    "supported_magnitude": directional["supported_magnitude"],
                    "mean_retained_effect": directional["mean_retained_effect"],
                    "disagreement_cost": directional["replication_disagreement_cost"],
                    "survivor_count": survival["survivor_count"],
                    "evaluation_count": survival["list_length"],
                    "assay_survivor_fraction": survival["survivor_fraction"],
                    "literal_subset_survival": survival["subset_fraction"],
                    "verdict": directional["verdict"],
                }
            )

        for order in SENSITIVITY_ORDERS:
            report = reports[(comparison.slug, order)]
            for orientation in ("forward", "reverse"):
                directional = report["result"][orientation]
                survival = directional["literal_survival"]
                sensitivity_rows.append(
                    {
                        "comparison": comparison.slug,
                        "comparison_label": comparison.label,
                        "orientation": orientation,
                        "empirical_order": order,
                        "supported_magnitude": directional["supported_magnitude"],
                        "literal_subset_survival": survival["subset_fraction"],
                        "survivor_count": survival["survivor_count"],
                        "evaluation_count": survival["list_length"],
                        "verdict": directional["verdict"],
                    }
                )

    return (
        pd.DataFrame(effect_rows),
        pd.DataFrame(summary_rows),
        pd.DataFrame(sensitivity_rows),
    )


def extract_auroc_tables(
    reports: dict[str, dict[str, Any]],
    observed: pd.DataFrame,
) -> tuple[pd.DataFrame, pd.DataFrame]:
    effect_rows: list[dict[str, Any]] = []
    summary_rows: list[dict[str, Any]] = []
    grouped = observed.groupby("assay_id", sort=True)

    for comparison in COMPARISONS:
        effects: list[float] = []
        for assay_id, evaluation in grouped:
            difference = float(
                roc_auc_score(evaluation["label"], evaluation[comparison.candidate])
                - roc_auc_score(evaluation["label"], evaluation[comparison.incumbent])
            )
            effects.append(difference)
            effect_rows.append(
                {
                    "comparison": comparison.slug,
                    "comparison_label": comparison.label,
                    "assay_id": assay_id,
                    "paired_auroc_difference": difference,
                }
            )

        result = reports[comparison.slug]["result"]
        gate = result["observed_gate"]
        survivor_count = sum(effect > SURVIVAL_FLOOR for effect in effects)
        if survivor_count != gate["survivor_count"]:
            raise RuntimeError("Python assay AUROC effects disagree with Rust survivor count")
        evaluation_count = len(effects)
        expected_survival = (
            survivor_count * (survivor_count - 1)
            / (evaluation_count * (evaluation_count - 1))
        )
        if not np.isclose(expected_survival, gate["literal_survival_fraction"]):
            raise RuntimeError("Python assay AUROC effects disagree with Rust survival")
        summary_rows.append(
            {
                "comparison": comparison.slug,
                "comparison_label": comparison.label,
                "empirical_order": SUPPORT_ORDER,
                "mean_paired_auroc_difference": float(np.mean(effects)),
                "supported_magnitude": gate["supported_magnitude"],
                "supported_magnitude_lower": gate["supported_magnitude_lower"],
                "supported_magnitude_upper": gate["supported_magnitude_upper"],
                "survivor_count": survivor_count,
                "evaluation_count": evaluation_count,
                "literal_subset_survival": gate["literal_survival_fraction"],
                "literal_survival_lower": gate["literal_survival_lower"],
                "literal_survival_upper": gate["literal_survival_upper"],
                "verdict": gate["verdict"],
                "breakdown_kind": result["breakdown"]["kind"],
            }
        )

    return pd.DataFrame(effect_rows), pd.DataFrame(summary_rows)


def plot_assay_effects(effects: pd.DataFrame) -> None:
    figure, axes = plt.subplots(
        len(COMPARISONS),
        1,
        figsize=(8.2, 3.0 * len(COMPARISONS) + 0.8),
        sharex=False,
        sharey=True,
    )
    for axis, comparison, color in zip(axes, COMPARISONS, COMPARISON_COLORS):
        values = np.sort(
            effects.loc[
                effects["comparison"].eq(comparison.slug),
                "retained_cnap_difference",
            ].to_numpy()
        )
        positions = np.arange(1, len(values) + 1)
        axis.axhline(0.0, color=GRAY, lw=1.1)
        axis.fill_between(
            positions,
            0.0,
            values,
            where=values >= 0.0,
            color=GREEN,
            alpha=0.12,
        )
        axis.fill_between(
            positions,
            0.0,
            values,
            where=values < 0.0,
            color=VERMILLION,
            alpha=0.10,
        )
        axis.plot(positions, values, color=color, lw=1.8)
        axis.scatter(positions, values, color=color, s=14, zorder=3)
        axis.set_title(comparison.label, loc="left")
        axis.set_ylabel("Retained CNAP difference")
        axis.text(
            0.99,
            0.05,
            f"{int((values > 0).sum())}/{len(values)} above zero",
            transform=axis.transAxes,
            ha="right",
            va="bottom",
            color=GRAY,
        )
    axes[-1].set_xlabel("DMS assays, ordered by retained effect")
    figure.suptitle(
        (
            "Advantage retained across "
            f"{TARGET_LOWER:.0%}–{TARGET_UPPER:.0%} deleterious prevalence"
        ),
        fontsize=13,
        y=1.01,
    )
    figure.tight_layout()
    save_figure(figure, "01-observed-assay-effects")


def plot_support_decision(summary: pd.DataFrame) -> None:
    forward = summary.loc[summary["orientation"].eq("forward")].copy()
    forward.set_index("comparison", inplace=True)
    labels = [comparison.short_label for comparison in COMPARISONS]
    means = [forward.loc[c.slug, "mean_retained_effect"] for c in COMPARISONS]
    supported = [forward.loc[c.slug, "supported_magnitude"] for c in COMPARISONS]
    survival = [forward.loc[c.slug, "literal_subset_survival"] for c in COMPARISONS]
    positions = np.arange(len(COMPARISONS))

    figure, axes = plt.subplots(1, 2, figsize=(11.2, 4.4))
    width = 0.34
    axes[0].bar(
        positions - width / 2,
        means,
        width,
        color=LIGHT_GRAY,
        edgecolor=GRAY,
        label="Mean assay effect",
    )
    axes[0].bar(
        positions + width / 2,
        supported,
        width,
        color=COMPARISON_COLORS,
        label="Order-2 support",
    )
    axes[0].axhline(MAGNITUDE_THRESHOLD, color=GRAY, lw=1.1)
    axes[0].set_ylabel("CNAP difference")
    axes[0].set_title("Magnitude", loc="left")
    axes[0].legend(frameon=False)

    axes[1].bar(positions, survival, width=0.55, color=COMPARISON_COLORS)
    axes[1].axhline(
        SURVIVAL_REQUIREMENT,
        color=VERMILLION,
        ls="--",
        lw=1.4,
        label=f"Required > {SURVIVAL_REQUIREMENT:.2f}",
    )
    axes[1].set_ylim(0.0, 1.02)
    axes[1].set_ylabel("Literal subset survival")
    axes[1].set_title("Persistence above zero", loc="left")
    axes[1].legend(frameon=False)

    for axis in axes:
        axis.set_xticks(positions, labels, rotation=15, ha="right")
    evaluation_counts = summary["evaluation_count"].astype(int).unique()
    if len(evaluation_counts) != 1:
        raise ValueError("comparisons must use the same observed assay list")
    figure.suptitle(
        f"Observed support across {evaluation_counts[0]} DMS assays",
        fontsize=13,
        y=1.02,
    )
    figure.tight_layout()
    save_figure(figure, "02-two-part-support-decision")


def plot_auroc_decision(summary: pd.DataFrame) -> None:
    indexed = summary.set_index("comparison")
    labels = [comparison.short_label.replace(" − ", " - ") for comparison in COMPARISONS]
    means = [indexed.loc[c.slug, "mean_paired_auroc_difference"] for c in COMPARISONS]
    supported = [indexed.loc[c.slug, "supported_magnitude"] for c in COMPARISONS]
    survival = [indexed.loc[c.slug, "literal_subset_survival"] for c in COMPARISONS]
    survivors = [int(indexed.loc[c.slug, "survivor_count"]) for c in COMPARISONS]
    evaluation_counts = [int(indexed.loc[c.slug, "evaluation_count"]) for c in COMPARISONS]
    positions = np.arange(len(COMPARISONS))

    figure, axes = plt.subplots(1, 2, figsize=(11.2, 4.4))
    width = 0.34
    axes[0].bar(
        positions - width / 2,
        means,
        width,
        color=LIGHT_GRAY,
        edgecolor=GRAY,
        label="Mean assay difference",
    )
    axes[0].bar(
        positions + width / 2,
        supported,
        width,
        color=COMPARISON_COLORS,
        label="Order-2 support",
    )
    axes[0].axhline(MAGNITUDE_THRESHOLD, color=GRAY, lw=1.1)
    axes[0].set_ylabel("Paired AUROC difference")
    axes[0].set_title("A. Supported magnitude", loc="left")
    axes[0].legend(frameon=False)

    bars = axes[1].bar(
        positions,
        survival,
        width=0.55,
        color=COMPARISON_COLORS,
    )
    axes[1].axhline(
        SURVIVAL_REQUIREMENT,
        color=VERMILLION,
        ls="--",
        lw=1.4,
        label=f"Required > {SURVIVAL_REQUIREMENT:.2f}",
    )
    axes[1].set_ylim(0.0, 1.02)
    axes[1].set_ylabel("Literal assay-pair survival")
    axes[1].set_title("B. Persistence above zero", loc="left")
    axes[1].legend(frameon=False)
    for bar, count, total in zip(bars, survivors, evaluation_counts):
        axes[1].text(
            bar.get_x() + bar.get_width() / 2,
            bar.get_height() + 0.025,
            f"{count}/{total} assays",
            ha="center",
            va="bottom",
            fontsize=8.5,
            color=GRAY,
        )

    for axis in axes:
        axis.set_xticks(positions, labels, rotation=15, ha="right")
    figure.suptitle(
        "Observed AUROC support across 91 ProteinGym assays",
        fontsize=13,
        y=1.02,
    )
    figure.text(
        0.5,
        0.005,
        r"Observed-only assessment: $M=91$, $K_E=2$, $K_C=0$; no within-assay resampling.",
        ha="center",
        fontsize=8.8,
        color=GRAY,
    )
    figure.tight_layout(rect=(0.0, 0.05, 1.0, 1.0))
    save_figure(figure, "03-observed-auroc-decision")


def _markdown_table(headers: list[str], rows: list[list[str]]) -> str:
    lines = [
        "| " + " | ".join(headers) + " |",
        "|" + "|".join("---" for _ in headers) + "|",
    ]
    lines.extend("| " + " | ".join(row) + " |" for row in rows)
    return "\n".join(lines)


def write_report(
    audit: pd.DataFrame,
    summary: pd.DataFrame,
    auroc_summary: pd.DataFrame,
    profile_name: str,
) -> None:
    forward = summary.loc[summary["orientation"].eq("forward")].set_index("comparison")
    result_rows: list[list[str]] = []
    for comparison in COMPARISONS:
        row = forward.loc[comparison.slug]
        result_rows.append(
            [
                comparison.label,
                f"{row['supported_magnitude']:.4f}",
                f"{int(row['survivor_count'])}/{int(row['evaluation_count'])}",
                f"{row['literal_subset_survival']:.3f}",
                str(row["verdict"]).replace("_", " "),
            ]
        )
    results = _markdown_table(
        [
            "Comparison",
            "Order-2 support",
            "Assays above 0",
            "Subset survival",
            "Verdict",
        ],
        result_rows,
    )
    auroc_by_comparison = auroc_summary.set_index("comparison")
    auroc_rows: list[list[str]] = []
    for comparison in COMPARISONS:
        row = auroc_by_comparison.loc[comparison.slug]
        auroc_rows.append(
            [
                comparison.label,
                f"{row['supported_magnitude']:.4f}",
                f"{int(row['survivor_count'])}/{int(row['evaluation_count'])}",
                f"{row['literal_subset_survival']:.3f}",
                str(row["verdict"]).replace("_", " "),
            ]
        )
    auroc_results = _markdown_table(
        [
            "Comparison",
            "Order-2 support",
            "Assays above 0",
            "Subset survival",
            "Verdict",
        ],
        auroc_rows,
    )
    selection_counts = audit["selection_type"].value_counts().to_dict()
    assay_prevalence = audit["deleterious_fraction"].astype(float)
    prevalence_quartiles = assay_prevalence.quantile([0.25, 0.50, 0.75])
    prevalence_within = int(
        assay_prevalence.between(TARGET_LOWER, TARGET_UPPER, inclusive="both").sum()
    )
    prevalence_below = int((assay_prevalence < TARGET_LOWER).sum())
    prevalence_above = int((assay_prevalence > TARGET_UPPER).sum())
    pooled_prevalence = float(
        audit["deleterious_count"].sum()
        / (audit["deleterious_count"].sum() + audit["functional_count"].sum())
    )
    text = f"""# ProteinGym observed-evaluation study

This study compares already-released protein variant-effect scores. It trains no
model and performs no computational resampling. Each retained deep-mutational-
scanning assay is one observed full evaluation, so the evidence list has
`M = {len(audit)}` assay-level effects.

## Declared regime

- ProteinGym `{PROTEINGYM_VERSION}` substitution assays and metadata pinned at
  commit `{PROTEINGYM_COMMIT}`.
- Manual biological binarization cutoffs only.
- Single amino-acid substitutions only, including the single-substitution rows
  of assays that also measured multiple substitutions.
- Experimentally nonfunctional/deleterious variants are the positive class.
- Released ProteinGym fitness-oriented scores are negated so larger values
  favor the positive class.
- Every retained variant has finite EVE ensemble, ESM-1v single, ESM-1v
  ensemble, and ESM-2 650M scores on the same row.
- Target deleterious prevalence: {TARGET_LOWER:.0%}–{TARGET_UPPER:.0%}.
- Empirical order `K_E = {SUPPORT_ORDER}`; magnitude threshold `delta = {MAGNITUDE_THRESHOLD:g}`;
  survival floor `d = {SURVIVAL_FLOOR:g}`; literal-survival requirement
  `gamma = {SURVIVAL_REQUIREMENT:g}`.
- Profile: `{profile_name}`. All AP, CNAP, prevalence minimization, support, and
  literal-survival calculations are performed by the V17 Rust implementation.

## Data retained

- {len(audit)} assays, {audit['uniprot_id'].nunique()} UniProt proteins, and
  {audit[['first_author', 'year', 'title']].drop_duplicates().shape[0]} publications.
- {int(audit['single_variant_count'].sum()):,} single substitutions:
  {int(audit['deleterious_count'].sum()):,} deleterious and
  {int(audit['functional_count'].sum()):,} functional.
- Assay types: {', '.join(f'{name} {count}' for name, count in selection_counts.items())}.
- {int((audit['assays_for_uniprot'] > 1).sum())} assay rows share a protein with
  another retained assay; {int((audit['assays_from_publication'] > 1).sum())}
  come from publications contributing more than one assay.
- Assay-specific deleterious fractions range from {assay_prevalence.min():.3f}
  to {assay_prevalence.max():.3f}, with median
  {prevalence_quartiles.loc[0.50]:.3f} and interquartile range
  [{prevalence_quartiles.loc[0.25]:.3f}, {prevalence_quartiles.loc[0.75]:.3f}].
- {prevalence_within} assays fall within the declared
  {TARGET_LOWER:.0%}–{TARGET_UPPER:.0%} target interval,
  {prevalence_below} fall below it, and {prevalence_above} exceed it. The pooled
  deleterious fraction is {pooled_prevalence:.3f}; it weights assays by their
  retained substitution counts and is not the prevalence of a typical assay.

## Results

{results}

`Order-2 support` is the supported magnitude of the finite list of assay-level
effects after each assay is first challenged throughout the complete declared
prevalence interval. `Subset survival` is the literal fraction of distinct
order-2 assay subsets whose two retained effects both strictly exceed zero.

## Observed-only AUROC companion

{auroc_results}

AUROC is unchanged by the target prevalence. Each assay therefore contributes
its ordinary paired AUROC difference at `Gamma = 1`, after which the same
order-2 empirical support and literal assay-pair survival rule is applied. All
three declared comparisons fail this observed gate, so no within-assay
computational resampling or case-mix breakdown search is performed. This is the
`M = {len(audit)}, K_E = {SUPPORT_ORDER}, K_C = 0` boundary of the AUROC
construction.

## Interpretation

The results report observed support across the retained ProteinGym assay
contexts. They do not describe sampling from a population of hypothetical
assays, and repeated proteins or publications are not relabeled as independent
research programs. Publication and protein multiplicities are retained in
`tables/assay_audit.csv`.

The prevalence calculation holds each assay's observed class-conditional paired
score distributions fixed while changing finite library composition. It does
not transport performance to mutation mechanisms, proteins, or assays absent
from the retained collection. Variants within an assay are experimentally
structured, so no exchangeable-label reference law is asserted.

## Figures

1. `01-observed-assay-effects` shows the complete finite list of retained
   assay-level advantages for all three comparisons.
2. `02-two-part-support-decision` separates supported magnitude from literal
   persistence within the observed empirical gate.
3. `03-observed-auroc-decision` gives the corresponding observed-only AUROC
   magnitude and assay-pair survival results.
"""
    (OUTPUTS / "REPORT.md").write_text(text)


def write_manifest(
    profile_name: str,
    profile: Profile,
    hashes: dict[str, str],
    audit: pd.DataFrame,
    cli: Path,
) -> None:
    assay_prevalence = audit["deleterious_fraction"].astype(float)
    prevalence_quartiles = assay_prevalence.quantile([0.25, 0.50, 0.75])
    manifest = {
        "study": "proteingym_observed_model_comparison",
        "manuscript_version": "v17",
        "profile": profile_name,
        "parameters": asdict(profile),
        "proteingym_version": PROTEINGYM_VERSION,
        "proteingym_commit": PROTEINGYM_COMMIT,
        "assets": [
            {
                "filename": asset.filename,
                "url": asset.url,
                "sha256": hashes[asset.filename],
            }
            for asset in ASSETS
        ],
        "regime": {
            "binarization_method": "manual",
            "mutation_depth": 1,
            "positive_class": "experimentally_nonfunctional_or_deleterious",
            "released_score_orientation": "negated_from_fitness_to_deleteriousness",
            "target_prevalence_interval": [TARGET_LOWER, TARGET_UPPER],
            "empirical_order": SUPPORT_ORDER,
            "sensitivity_orders": list(SENSITIVITY_ORDERS),
            "magnitude_threshold": MAGNITUDE_THRESHOLD,
            "survival_floor": SURVIVAL_FLOOR,
            "survival_requirement": SURVIVAL_REQUIREMENT,
            "transport_justification": TRANSPORT_JUSTIFICATION,
            "reference_limitation": REFERENCE_LIMITATION,
            "auroc_observed": {
                "empirical_order": SUPPORT_ORDER,
                "optimization_absolute_gap": AUROC_OPTIMIZATION_ABSOLUTE_GAP,
                "optimization_relative_gap": AUROC_OPTIMIZATION_RELATIVE_GAP,
                "optimization_time_limit_seconds": AUROC_OPTIMIZATION_TIME_LIMIT_SECONDS,
                "concentration_tolerance": AUROC_CONCENTRATION_TOLERANCE,
                "concentration_max_iterations": AUROC_CONCENTRATION_MAX_ITERATIONS,
                "computational_resampling": False,
            },
        },
        "comparisons": [asdict(comparison) for comparison in COMPARISONS],
        "retained": {
            "assays": len(audit),
            "single_variants": int(audit["single_variant_count"].sum()),
            "uniprot_proteins": int(audit["uniprot_id"].nunique()),
            "publications": int(
                audit[["first_author", "year", "title"]].drop_duplicates().shape[0]
            ),
            "assay_deleterious_prevalence": {
                "minimum": float(assay_prevalence.min()),
                "first_quartile": float(prevalence_quartiles.loc[0.25]),
                "median": float(prevalence_quartiles.loc[0.50]),
                "third_quartile": float(prevalence_quartiles.loc[0.75]),
                "maximum": float(assay_prevalence.max()),
                "within_target_interval": int(
                    assay_prevalence.between(
                        TARGET_LOWER, TARGET_UPPER, inclusive="both"
                    ).sum()
                ),
                "below_target_interval": int(
                    (assay_prevalence < TARGET_LOWER).sum()
                ),
                "above_target_interval": int(
                    (assay_prevalence > TARGET_UPPER).sum()
                ),
                "pooled": float(
                    audit["deleterious_count"].sum()
                    / (
                        audit["deleterious_count"].sum()
                        + audit["functional_count"].sum()
                    )
                ),
            },
        },
        "software": {
            "python": sys.version,
            "platform": platform.platform(),
            "numpy": np.__version__,
            "pandas": pd.__version__,
            "rust_cli": str(cli.resolve()),
        },
        "artifacts": {
            "paired_input": "outputs/predictions/proteingym_observed_scores.csv",
            "assay_audit": "outputs/tables/assay_audit.csv",
            "effects": "outputs/tables/observed_assay_effects.csv",
            "support_summary": "outputs/tables/support_summary.csv",
            "order_sensitivity": "outputs/tables/empirical_order_sensitivity.csv",
            "auroc_effects": "outputs/tables/observed_auroc_assay_effects.csv",
            "auroc_support_summary": "outputs/tables/observed_auroc_support_summary.csv",
        },
    }
    (OUTPUTS / "manifest.json").write_text(json.dumps(manifest, indent=2))


def render_retained(profile_name: str) -> None:
    required = [
        OUTPUTS / "manifest.json",
        TABLES / "assay_audit.csv",
        TABLES / "observed_assay_effects.csv",
        TABLES / "support_summary.csv",
        TABLES / "observed_auroc_assay_effects.csv",
        TABLES / "observed_auroc_support_summary.csv",
    ]
    for path in required:
        if not path.exists():
            raise FileNotFoundError(f"retained artifact missing: {path}")
    manifest = read_json(OUTPUTS / "manifest.json")
    if manifest["profile"] != profile_name:
        raise RuntimeError(
            f"retained profile is {manifest['profile']}, not requested {profile_name}"
        )
    audit = pd.read_csv(TABLES / "assay_audit.csv")
    effects = pd.read_csv(TABLES / "observed_assay_effects.csv")
    summary = pd.read_csv(TABLES / "support_summary.csv")
    auroc_summary = pd.read_csv(TABLES / "observed_auroc_support_summary.csv")
    configure_plots()
    plot_assay_effects(effects)
    plot_support_decision(summary)
    plot_auroc_decision(auroc_summary)
    write_report(audit, summary, auroc_summary, profile_name)


def run_clean(args: argparse.Namespace, profile: Profile) -> None:
    hashes = verify_assets(args.data_dir)
    prepare_outputs()
    metadata = load_metadata(args.data_dir / METADATA_FILENAME)
    observed, audit = build_observed_frame(
        metadata,
        args.data_dir / DMS_ARCHIVE_FILENAME,
        args.data_dir / SCORE_ARCHIVE_FILENAME,
    )
    input_path = PREDICTIONS / "proteingym_observed_scores.csv"
    observed.to_csv(input_path, index=False)
    audit.to_csv(TABLES / "assay_audit.csv", index=False)
    assay_ids = sorted(audit["assay_id"].tolist())
    reports = run_rust_reports(args.cli, input_path, profile, assay_ids)
    auroc_reports = run_auroc_reports(args.cli, input_path, assay_ids)
    effects, summary, sensitivity = extract_tables(reports, audit)
    auroc_effects, auroc_summary = extract_auroc_tables(auroc_reports, observed)
    effects.to_csv(TABLES / "observed_assay_effects.csv", index=False)
    summary.to_csv(TABLES / "support_summary.csv", index=False)
    sensitivity.to_csv(TABLES / "empirical_order_sensitivity.csv", index=False)
    auroc_effects.to_csv(TABLES / "observed_auroc_assay_effects.csv", index=False)
    auroc_summary.to_csv(TABLES / "observed_auroc_support_summary.csv", index=False)
    configure_plots()
    plot_assay_effects(effects)
    plot_support_decision(summary)
    plot_auroc_decision(auroc_summary)
    write_report(audit, summary, auroc_summary, args.profile)
    write_manifest(args.profile, profile, hashes, audit, args.cli)


def rerun_rust_from_retained(args: argparse.Namespace, profile: Profile) -> None:
    input_path = PREDICTIONS / "proteingym_observed_scores.csv"
    audit_path = TABLES / "assay_audit.csv"
    manifest_path = OUTPUTS / "manifest.json"
    for path in (input_path, audit_path, manifest_path):
        if not path.exists():
            raise FileNotFoundError(f"retained V17 migration input missing: {path}")
    audit = pd.read_csv(audit_path)
    observed = pd.read_csv(input_path)
    assay_ids = sorted(audit["assay_id"].tolist())
    reports = run_rust_reports(args.cli, input_path, profile, assay_ids)
    auroc_reports = run_auroc_reports(args.cli, input_path, assay_ids)
    effects, summary, sensitivity = extract_tables(reports, audit)
    auroc_effects, auroc_summary = extract_auroc_tables(auroc_reports, observed)
    effects.to_csv(TABLES / "observed_assay_effects.csv", index=False)
    summary.to_csv(TABLES / "support_summary.csv", index=False)
    sensitivity.to_csv(TABLES / "empirical_order_sensitivity.csv", index=False)
    auroc_effects.to_csv(TABLES / "observed_auroc_assay_effects.csv", index=False)
    auroc_summary.to_csv(TABLES / "observed_auroc_support_summary.csv", index=False)
    configure_plots()
    plot_assay_effects(effects)
    plot_support_decision(summary)
    plot_auroc_decision(auroc_summary)
    write_report(audit, summary, auroc_summary, args.profile)
    retained_manifest = read_json(manifest_path)
    hashes = {
        asset["filename"]: asset["sha256"]
        for asset in retained_manifest["assets"]
    }
    write_manifest(args.profile, profile, hashes, audit, args.cli)


def run_auroc_from_retained(args: argparse.Namespace, profile: Profile) -> None:
    input_path = PREDICTIONS / "proteingym_observed_scores.csv"
    audit_path = TABLES / "assay_audit.csv"
    effects_path = TABLES / "observed_assay_effects.csv"
    summary_path = TABLES / "support_summary.csv"
    manifest_path = OUTPUTS / "manifest.json"
    for path in (input_path, audit_path, effects_path, summary_path, manifest_path):
        if not path.exists():
            raise FileNotFoundError(f"retained AUROC input missing: {path}")
    retained_manifest = read_json(manifest_path)
    if retained_manifest["profile"] != args.profile:
        raise RuntimeError(
            f"retained profile is {retained_manifest['profile']}, not requested {args.profile}"
        )

    observed = pd.read_csv(input_path)
    audit = pd.read_csv(audit_path)
    effects = pd.read_csv(effects_path)
    summary = pd.read_csv(summary_path)
    assay_ids = sorted(audit["assay_id"].tolist())
    auroc_reports = run_auroc_reports(args.cli, input_path, assay_ids)
    auroc_effects, auroc_summary = extract_auroc_tables(auroc_reports, observed)
    auroc_effects.to_csv(TABLES / "observed_auroc_assay_effects.csv", index=False)
    auroc_summary.to_csv(TABLES / "observed_auroc_support_summary.csv", index=False)

    configure_plots()
    plot_assay_effects(effects)
    plot_support_decision(summary)
    plot_auroc_decision(auroc_summary)
    write_report(audit, summary, auroc_summary, args.profile)
    hashes = {
        asset["filename"]: asset["sha256"]
        for asset in retained_manifest["assets"]
    }
    write_manifest(args.profile, profile, hashes, audit, args.cli)


def main() -> None:
    args = parse_args()
    profile = PROFILES[args.profile]
    if args.download_only:
        download_assets(args.data_dir)
        verify_assets(args.data_dir)
        return
    if args.render_only:
        render_retained(args.profile)
        return
    if not args.cli.exists():
        raise FileNotFoundError(f"Rust CLI missing: {args.cli}")
    if args.rust_only:
        rerun_rust_from_retained(args, profile)
        return
    if args.auroc_only:
        run_auroc_from_retained(args, profile)
        return
    run_clean(args, profile)


if __name__ == "__main__":
    main()
