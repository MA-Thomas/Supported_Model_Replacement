#!/usr/bin/env python3
"""Label-blind complete-F diagnostics for freezing the adaptive L2 grid."""

from __future__ import annotations

import argparse
import csv
import json
import math
from collections import defaultdict
from itertools import combinations
from pathlib import Path
from statistics import median


FLOOR = math.log(1e-12)
DIAGNOSTIC_ALPHAS = (0.0, 0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0, math.inf)
PRODUCTION_ALPHAS = (0.5, 1.0, 2.0, 4.0, 8.0, math.inf)
DIAGNOSTIC_Q = (0.0, 0.5, 1.0, 2.0, 4.0, math.inf)
PRODUCTION_Q = (0.5, 1.0, 2.0, 4.0, math.inf)
KAPPA_MIN = 0.02
KAPPA_MAX = 4.0
KAPPA_POINTS = 8
KAPPA_VALUES = tuple(
    math.exp(math.log(KAPPA_MIN) + index * (math.log(KAPPA_MAX) - math.log(KAPPA_MIN)) / (KAPPA_POINTS - 1))
    for index in range(KAPPA_POINTS)
)
COHORTS = ("pdac", "covid_spike", "covid_nonspike")
MODELS = ("full_hla", "focal_hla", "old_monoallelic", "mono_q_full_pn", "full_q_mono_pn")
BRANCHES = ("pr", "roc")


def token(value: float) -> str:
    return "inf" if math.isinf(value) else f"{value:g}"


def percentile(values: list[float], probability: float) -> float:
    if not values:
        return math.nan
    ordered = sorted(values)
    position = (len(ordered) - 1) * probability
    lower = int(math.floor(position))
    upper = int(math.ceil(position))
    if lower == upper:
        return ordered[lower]
    weight = position - lower
    return ordered[lower] * (1.0 - weight) + ordered[upper] * weight


def summaries(values: list[float]) -> dict[str, float | int]:
    return {
        "n": len(values),
        "min": min(values),
        "p01": percentile(values, 0.01),
        "p05": percentile(values, 0.05),
        "p25": percentile(values, 0.25),
        "median": percentile(values, 0.5),
        "p75": percentile(values, 0.75),
        "p95": percentile(values, 0.95),
        "p99": percentile(values, 0.99),
        "max": max(values),
    }


def average_ranks(values: list[float]) -> list[float]:
    order = sorted(range(len(values)), key=lambda index: (values[index], index))
    ranks = [0.0] * len(values)
    start = 0
    while start < len(order):
        end = start + 1
        while end < len(order) and values[order[end]] == values[order[start]]:
            end += 1
        rank = 0.5 * (start + end - 1)
        for position in range(start, end):
            ranks[order[position]] = rank
        start = end
    return ranks


def pearson(left: list[float], right: list[float]) -> float:
    if len(left) != len(right) or len(left) < 2:
        return math.nan
    left_mean = sum(left) / len(left)
    right_mean = sum(right) / len(right)
    numerator = sum((a - left_mean) * (b - right_mean) for a, b in zip(left, right))
    left_ss = sum((a - left_mean) ** 2 for a in left)
    right_ss = sum((b - right_mean) ** 2 for b in right)
    if left_ss == 0.0 or right_ss == 0.0:
        return math.nan
    return numerator / math.sqrt(left_ss * right_ss)


def spearman(left: list[float], right: list[float]) -> float:
    return pearson(average_ranks(left), average_ranks(right))


def norm_hla(value: str) -> str:
    upper = value.strip().upper()
    if upper.startswith("HLA-"):
        upper = upper[4:]
    return upper.replace("*", "").replace(":", "")


def log_power_mean(values: list[float], alpha: float) -> float:
    if alpha == 0.0:
        return sum(values) / len(values)
    if math.isinf(alpha):
        return max(values)
    maximum = max(values)
    relative = sum(math.exp(alpha * (value - maximum)) for value in values) / len(values)
    return maximum + math.log(relative) / alpha


def breadth(signals: list[float], q_value: float) -> float:
    positive = [value for value in signals if value > 0.0]
    if not positive:
        return 0.0
    total = sum(positive)
    probabilities = [value / total for value in positive]
    if q_value == 0.0:
        inverse_hill = 1.0 / len(probabilities)
    elif q_value == 1.0:
        entropy = -sum(value * math.log(value) for value in probabilities)
        inverse_hill = math.exp(-entropy)
    elif math.isinf(q_value):
        inverse_hill = max(probabilities)
    else:
        inverse_hill = math.exp(-math.log(sum(value**q_value for value in probabilities)) / (1.0 - q_value))
    return 1.0 - inverse_hill


def load_rosters(task: Path) -> list[list[float]]:
    summary = json.loads((task / "summary.json").read_text())
    mapping_path = Path(summary["mapping"])
    if not mapping_path.is_file():
        mapping_path = task.parents[3] / "full_roster_inputs" / mapping_path.name
    tau: dict[tuple[str, int, str, str], float] = {}
    with (task / "target_tau_selection_by_observation.csv").open(newline="") as handle:
        for row in csv.DictReader(handle):
            key = (row["patient_id"], int(row["env_id"]), row["peptide"], norm_hla(row["hla"]))
            if key in tau:
                raise ValueError(f"duplicate tau identity in {task}: {key}")
            tau[key] = float(row["score"])

    grouped: dict[tuple[str, str, str], list[float]] = defaultdict(list)
    seen: set[tuple[str, str, str, str, str]] = set()
    with mapping_path.open(newline="") as handle:
        for row in csv.DictReader(handle):
            nmer = row["nmer"]
            if not 9 <= len(nmer) <= 12:
                continue
            if row["mapping_status"].strip().lower() != "scoreable":
                raise ValueError(f"non-scoreable mapping row in {mapping_path}")
            mutation = row.get("mutation", "")
            endpoint = (row["patient_id"], mutation, row["long_peptide"])
            hla = norm_hla(row["HLA-RE"])
            candidate = (*endpoint, nmer, hla)
            if candidate in seen:
                continue
            seen.add(candidate)
            join = (row["patient_id"], int(row["env_id"]), nmer, hla)
            if join not in tau:
                raise ValueError(f"mapping row lacks tau score in {task}: {join}")
            grouped[endpoint].append(tau[join])

    with (task / "long_peptide_predictions.csv").open(newline="") as handle:
        expected = {
            (row["patient_id"], row.get("mutation", ""), row["long_peptide"])
            for row in csv.DictReader(handle)
            if row["l2_variant"] == "max"
        }
    if set(grouped) != expected:
        raise ValueError(f"endpoint roster mismatch in {task}")
    return [grouped[key] for key in sorted(grouped)]


def write_csv(path: Path, rows: list[dict[str, object]]) -> None:
    if not rows:
        return
    with path.open("w", newline="") as handle:
        writer = csv.DictWriter(handle, fieldnames=list(rows[0]))
        writer.writeheader()
        writer.writerows(rows)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--transfers", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.output.exists():
        raise SystemExit(f"output already exists: {args.output}")
    args.output.mkdir(parents=True)

    task_rows: list[dict[str, object]] = []
    alpha_rows: list[dict[str, object]] = []
    q_rows: list[dict[str, object]] = []
    alpha_similarity_rows: list[dict[str, object]] = []
    q_similarity_rows: list[dict[str, object]] = []
    task_gate_rows: list[dict[str, object]] = []
    all_completed = 0
    all_zero = 0

    for cohort in COHORTS:
        for model in MODELS:
            for branch in BRANCHES:
                task = args.transfers / cohort / model / branch
                rosters = load_rosters(task)
                counts = [len(roster) for roster in rosters]
                scores = [score for roster in rosters for score in roster]
                signals = [[max(math.exp(score) - 1e-12, 0.0) for score in roster] for roster in rosters]
                zero_count = sum(value == 0.0 for roster in signals for value in roster)
                all_completed += len(scores)
                all_zero += zero_count
                leader_shares = [max(row) / sum(row) if sum(row) > 0.0 else 0.0 for row in signals]
                count_summary = summaries([float(value) for value in counts])
                score_summary = summaries(scores)
                leader_summary = summaries(leader_shares)
                task_rows.append({
                    "cohort": cohort,
                    "model": model,
                    "branch": branch,
                    "endpoints": len(rosters),
                    "candidates": len(scores),
                    "completed_zero_candidates": zero_count,
                    "candidate_count_min": count_summary["min"],
                    "candidate_count_median": count_summary["median"],
                    "candidate_count_p95": count_summary["p95"],
                    "candidate_count_max": count_summary["max"],
                    "score_p01": score_summary["p01"],
                    "score_median": score_summary["median"],
                    "score_p99": score_summary["p99"],
                    "leader_share_p05": leader_summary["p05"],
                    "leader_share_median": leader_summary["median"],
                    "leader_share_p95": leader_summary["p95"],
                })

                anchors_by_alpha: dict[float, list[float]] = {}
                for alpha in DIAGNOSTIC_ALPHAS:
                    anchors = [log_power_mean(roster, alpha) for roster in rosters]
                    anchors_by_alpha[alpha] = anchors
                    gaps = [max(roster) - anchor for roster, anchor in zip(rosters, anchors)]
                    anchor_summary = summaries(anchors)
                    gap_summary = summaries(gaps)
                    alpha_rows.append({
                        "cohort": cohort,
                        "model": model,
                        "branch": branch,
                        "alpha": token(alpha),
                        "anchor_p01": anchor_summary["p01"],
                        "anchor_median": anchor_summary["median"],
                        "anchor_p99": anchor_summary["p99"],
                        "max_minus_anchor_median": gap_summary["median"],
                        "max_minus_anchor_p95": gap_summary["p95"],
                        "candidate_count_vs_gap_spearman": spearman([float(value) for value in counts], gaps),
                    })
                alpha_pairs = list(dict.fromkeys([
                    *zip(DIAGNOSTIC_ALPHAS, DIAGNOSTIC_ALPHAS[1:]),
                    *combinations(PRODUCTION_ALPHAS, 2),
                ]))
                for left, right in alpha_pairs:
                    left_values = anchors_by_alpha[left]
                    right_values = anchors_by_alpha[right]
                    alpha_similarity_rows.append({
                        "cohort": cohort,
                        "model": model,
                        "branch": branch,
                        "alpha_left": token(left),
                        "alpha_right": token(right),
                        "anchor_spearman": spearman(left_values, right_values),
                        "identical_descending_order": sorted(range(len(rosters)), key=lambda i: (-left_values[i], i)) == sorted(range(len(rosters)), key=lambda i: (-right_values[i], i)),
                    })
                breadth_by_q: dict[float, list[float]] = {}
                for q_value in DIAGNOSTIC_Q:
                    values = [breadth(row, q_value) for row in signals]
                    breadth_by_q[q_value] = values
                    value_summary = summaries(values)
                    q_rows.append({
                        "cohort": cohort,
                        "model": model,
                        "branch": branch,
                        "q": token(q_value),
                        "breadth_p01": value_summary["p01"],
                        "breadth_median": value_summary["median"],
                        "breadth_p99": value_summary["p99"],
                        "candidate_count_vs_breadth_spearman": spearman([float(value) for value in counts], values),
                    })
                q_pairs = list(dict.fromkeys([
                    *zip(DIAGNOSTIC_Q, DIAGNOSTIC_Q[1:]),
                    *combinations(PRODUCTION_Q, 2),
                ]))
                for left, right in q_pairs:
                    q_similarity_rows.append({
                        "cohort": cohort,
                        "model": model,
                        "branch": branch,
                        "q_left": token(left),
                        "q_right": token(right),
                        "breadth_spearman": spearman(breadth_by_q[left], breadth_by_q[right]),
                    })
                meaningful_anchors: list[float] = []
                meaningful_upper: list[float] = []
                total_signal = [sum(row) for row in signals]
                ceiling = [math.log(1e-12 + value) for value in total_signal]
                for alpha in PRODUCTION_ALPHAS:
                    anchors = anchors_by_alpha[alpha]
                    for q_value in PRODUCTION_Q:
                        breadth_values = breadth_by_q[q_value]
                        for anchor, upper, d_q in zip(anchors, ceiling, breadth_values):
                            offer = d_q * max(upper - anchor, 0.0)
                            if offer > 1e-12:
                                meaningful_anchors.append(anchor)
                                meaningful_upper.append(anchor + offer)
                task_gate_rows.append({
                    "cohort": cohort,
                    "model": model,
                    "branch": branch,
                    "anchor_p01": percentile(meaningful_anchors, 0.01),
                    "anchor_p05": percentile(meaningful_anchors, 0.05),
                    "offered_upper_p95": percentile(meaningful_upper, 0.95),
                    "offered_upper_p99": percentile(meaningful_upper, 0.99),
                })

    conservative_c_min = math.floor((min(float(row["anchor_p01"]) for row in task_gate_rows) - 0.5) * 10.0) / 10.0
    conservative_c_max = math.ceil((max(float(row["offered_upper_p99"]) for row in task_gate_rows) + 0.5) * 10.0) / 10.0
    lean_c_min = math.floor((min(float(row["anchor_p05"]) for row in task_gate_rows) - 0.3) * 10.0) / 10.0
    lean_c_max = math.ceil((max(float(row["offered_upper_p95"]) for row in task_gate_rows) + 0.3) * 10.0) / 10.0
    c_step = 0.1
    c_points = int(round((conservative_c_max - conservative_c_min) / c_step)) + 1
    proposed_points = len(PRODUCTION_ALPHAS) * len(PRODUCTION_Q) * len(KAPPA_VALUES) * c_points
    lean_alpha = (0.5, 1.0, 2.0, 4.0, math.inf)
    lean_q = (0.5, 1.0, 2.0, math.inf)
    lean_c_points = int(round((lean_c_max - lean_c_min) / c_step)) + 1
    lean_points = len(lean_alpha) * len(lean_q) * len(KAPPA_VALUES) * lean_c_points
    old_points = 8 * 281 * 113
    old_hours = 4611.962890845388
    recommendation = {
        "status": "provisional_label_blind_grid",
        "alpha_values": [token(value) for value in PRODUCTION_ALPHAS],
        "q_values": [token(value) for value in PRODUCTION_Q],
        "c_min": conservative_c_min,
        "c_max": conservative_c_max,
        "c_step": c_step,
        "c_points": c_points,
        "kappa_values": list(KAPPA_VALUES),
        "kappa_min": KAPPA_MIN,
        "kappa_max": KAPPA_MAX,
        "kappa_points": KAPPA_POINTS,
        "joint_points_per_component_metric": proposed_points,
        "old_joint_points_per_component_metric": old_points,
        "raw_grid_ratio_vs_old": proposed_points / old_points,
        "naive_old_runtime_scaled_hours": old_hours * proposed_points / old_points,
        "lean_alternative": {
            "alpha_values": [token(value) for value in lean_alpha],
            "q_values": [token(value) for value in lean_q],
            "c_min": lean_c_min,
            "c_max": lean_c_max,
            "c_step": c_step,
            "c_points": lean_c_points,
            "kappa_values": list(KAPPA_VALUES),
            "kappa_min": KAPPA_MIN,
            "kappa_max": KAPPA_MAX,
            "kappa_points": KAPPA_POINTS,
            "joint_points_per_component_metric": lean_points,
            "raw_grid_ratio_vs_old": lean_points / old_points,
            "naive_old_runtime_scaled_hours": old_hours * lean_points / old_points,
        },
        "runtime_caveat": "The cluster plan's unique_match_count is authoritative; complete-F can reduce tie-based deduplication.",
        "completed_candidates": all_completed,
        "completed_zero_candidates": all_zero,
        "sensitivity_only": {"alpha_values": ["0", "0.25"], "q_values": ["0"]},
    }

    write_csv(args.output / "task_geometry.csv", task_rows)
    write_csv(args.output / "alpha_geometry.csv", alpha_rows)
    write_csv(args.output / "q_geometry.csv", q_rows)
    write_csv(args.output / "alpha_rank_similarity.csv", alpha_similarity_rows)
    write_csv(args.output / "q_breadth_similarity.csv", q_similarity_rows)
    write_csv(args.output / "gate_interval_geometry.csv", task_gate_rows)
    (args.output / "grid_recommendation.json").write_text(json.dumps(recommendation, indent=2) + "\n")

    cohort_counts: dict[str, list[int]] = defaultdict(list)
    for row in task_rows:
        if row["model"] == "full_hla" and row["branch"] == "pr":
            cohort_counts[str(row["cohort"])].append(int(row["candidate_count_median"]))
    report = f"""# Complete-F label-blind grid diagnostics

All calculations ignore endpoint labels. They describe roster multiplicity, candidate-score geometry,
power-anchor behavior, Hill breadth, and the self-gate's meaningful score interval.

## Result

- Completed candidate entries examined across the 30 primary tasks: {all_completed:,}
- Exact completed-zero entries: {all_zero:,}
- Proposed alpha values: {' '.join(token(value) for value in PRODUCTION_ALPHAS)}
- Proposed Hill-q values: {' '.join(token(value) for value in PRODUCTION_Q)}
- Provisional c lattice: {conservative_c_min:g} to {conservative_c_max:g} by {c_step:g} ({c_points} values)
- Proposed kappa values: {' '.join(token(value) for value in KAPPA_VALUES)}
- Joint tuples per component/metric: {proposed_points:,}, versus {old_points:,} previously
- Raw grid ratio versus old: {proposed_points / old_points:.3f}
- Naive runtime scaling from the prior run: {old_hours * proposed_points / old_points:.1f} aggregate shard-hours
- Lean alternative: alpha {' '.join(token(value) for value in lean_alpha)}; q {' '.join(token(value) for value in lean_q)};
  c {lean_c_min:g} to {lean_c_max:g}; {lean_points:,} joint tuples; {old_hours * lean_points / old_points:.1f} naively scaled hours

The runtime projection is not a commitment. The Linux cluster planner's deduplicated
`unique_match_count` is the authoritative work estimate, and complete-F scores may retain fewer exact ties.

Alpha 0, alpha 0.25, and q 0 remain sensitivity-only because they are especially responsive to roster
opportunity. The production c interval is derived from the union of per-task 1st-percentile anchors and
99th-percentile offered upper endpoints, with a 0.5-log-unit margin; it should be reviewed rather than
silently frozen.
"""
    (args.output / "README.md").write_text(report)


if __name__ == "__main__":
    main()
