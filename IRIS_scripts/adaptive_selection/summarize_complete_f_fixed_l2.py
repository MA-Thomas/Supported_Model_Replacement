#!/usr/bin/env python3
"""Consolidate complete-F fixed-L2 endpoint performance across primary tasks."""

from __future__ import annotations

import argparse
import csv
from pathlib import Path


COHORTS = ("pdac", "covid_spike", "covid_nonspike")
MODELS = ("full_hla", "focal_hla", "old_monoallelic", "mono_q_full_pn", "full_q_mono_pn")
BRANCHES = ("pr", "roc")
VARIANTS = ("mean", "logmeanexp", "max", "logsumexp")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--transfers", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.output.exists():
        raise SystemExit(f"output already exists: {args.output}")
    args.output.mkdir(parents=True)

    rows: list[dict[str, object]] = []
    for cohort in COHORTS:
        for model in MODELS:
            for branch in BRANCHES:
                path = args.transfers / cohort / model / branch / "transfer_metrics.csv"
                with path.open(newline="") as handle:
                    source = list(csv.DictReader(handle))
                metric = "average_precision" if branch == "pr" else "roc_auc"
                values = {row["l2_variant"]: float(row[metric]) for row in source}
                winner = max(VARIANTS, key=lambda variant: (values[variant], -VARIANTS.index(variant)))
                rows.append({
                    "cohort": cohort,
                    "model": model,
                    "branch": branch,
                    "metric": metric,
                    "mean": values["mean"],
                    "logmeanexp": values["logmeanexp"],
                    "max": values["max"],
                    "logsumexp": values["logsumexp"],
                    "best_of_four": winner,
                    "best_value": values[winner],
                    "best_minus_max": values[winner] - values["max"],
                })

    csv_path = args.output / "fixed_l2_four_way.csv"
    with csv_path.open("w", newline="") as handle:
        writer = csv.DictWriter(handle, fieldnames=list(rows[0]))
        writer.writeheader()
        writer.writerows(rows)

    lines = [
        "# Complete-F fixed-L2 performance summary",
        "",
        "This is an outcome-aware diagnostic of the four biologically central fixed operators. It was not used to choose the label-blind grid bounds or identify redundant grid orders.",
        "",
        "| Cohort | Model | PR winner | ROC winner |",
        "|---|---|---:|---:|",
    ]
    for cohort in COHORTS:
        for model in MODELS:
            selected = {(row["branch"]): row for row in rows if row["cohort"] == cohort and row["model"] == model}
            pr = selected["pr"]
            roc = selected["roc"]
            lines.append(
                f"| {cohort} | {model} | {pr['best_of_four']} ({pr['best_value']:.4f}) | {roc['best_of_four']} ({roc['best_value']:.4f}) |"
            )
    lines.extend([
        "",
        "PDAC PR favors maximum in all five component families. COVID SPIKE retains below-maximum preferences in the full/focal families, while COVID NONSPIKE is mostly maximum with mean-favored ROC exceptions. This supports keeping multiple finite alpha orders plus exact maximum rather than collapsing the production grid to one anchor regime.",
        "",
    ])
    (args.output / "README.md").write_text("\n".join(lines))


if __name__ == "__main__":
    main()
