#!/usr/bin/env python3
"""Operational validation and resource summaries for adaptive-Hill-q Slurm jobs."""

from __future__ import annotations

import argparse
import json
import math
import os
import re
from pathlib import Path


def read_json(path: Path) -> dict:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise ValueError(f"expected a JSON object: {path}")
    return value


def atomic_json(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.tmp.{os.getpid()}")
    temporary.write_text(
        json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    os.replace(temporary, path)


def plan_info(args: argparse.Namespace) -> None:
    plan = read_json(args.plan / "plan.json")
    fields = (
        "shard_count",
        "unique_match_count",
        "selection_replications",
        "matches_per_shard",
    )
    values = [plan.get(field) for field in fields]
    if any(not isinstance(value, int) or value < 1 for value in values):
        raise ValueError(f"invalid plan dimensions: {args.plan}")
    print("\t".join(str(value) for value in values))


def normalized_order(value: object) -> float:
    token = str(value).strip().lower()
    if token in {"inf", "+inf", "infinity", "+infinity"}:
        return math.inf
    parsed = float(token)
    if not math.isfinite(parsed):
        raise ValueError(f"invalid finite aggregation order: {value}")
    return parsed


def check_plan_grid(args: argparse.Namespace) -> None:
    plan = read_json(args.plan / "plan.json")
    grid = plan.get("grid")
    if not isinstance(grid, dict):
        raise ValueError(f"plan has no grid contract: {args.plan}")

    expected_orders = {
        "alpha_values": [normalized_order(value) for value in args.alpha_values],
        "q_values": [normalized_order(value) for value in args.q_values],
    }
    for field, expected in expected_orders.items():
        actual = grid.get(field)
        if not isinstance(actual, list):
            raise ValueError(f"plan grid has no {field} list: {args.plan}")
        try:
            normalized = [normalized_order(value) for value in actual]
        except (TypeError, ValueError) as error:
            raise ValueError(f"invalid plan grid {field}: {error}") from error
        if normalized != expected:
            raise ValueError(
                f"existing plan {field} differs: expected {args.__dict__[field]}, got {actual}"
            )

    expected_scalars = {
        "c_min": args.c_min,
        "c_max": args.c_max,
        "c_step": args.c_step,
        "kappa_min": args.kappa_min,
        "kappa_max": args.kappa_max,
    }
    for field, expected in expected_scalars.items():
        actual = grid.get(field)
        if not isinstance(actual, (int, float)) or not math.isclose(
            float(actual), expected, rel_tol=1e-12, abs_tol=1e-12
        ):
            raise ValueError(
                f"existing plan {field} differs: expected {expected}, got {actual}"
            )
    if grid.get("kappa_points") != args.kappa_points:
        raise ValueError(
            "existing plan kappa_points differs: "
            f"expected {args.kappa_points}, got {grid.get('kappa_points')}"
        )


def check_summary(args: argparse.Namespace) -> None:
    summary = read_json(args.path)
    if summary.get("shard_id") != args.shard_id:
        raise ValueError(f"wrong shard id in {args.path}")
    completed = summary.get("completed_matches")
    if not isinstance(completed, int) or completed < 1:
        raise ValueError(f"invalid completed match count in {args.path}")


def summarize(args: argparse.Namespace) -> None:
    measured = read_json(args.time_file)
    summary = read_json(args.run_summary) if args.run_summary.is_file() else {}
    result = args.results / f"shard_{args.shard_id}.json"
    value = {
        "schema_version": 1,
        "mode": args.mode,
        "shard_id": args.shard_id,
        "completed_matches": summary.get("completed_matches", 0),
        "elapsed_seconds": measured.get("elapsed_seconds"),
        "maximum_resident_set_kb": measured.get("maximum_resident_set_kb"),
        "exit_code": args.exit_code,
        "result_bytes": result.stat().st_size if result.is_file() else 0,
        "hostname": os.uname().nodename,
    }
    atomic_json(args.output, value)


def aggregate(args: argparse.Namespace) -> None:
    resource_paths = [
        path
        for path in sorted(args.usage_root.glob("shard_*.json"))
        if re.fullmatch(r"shard_[0-9]+\.json", path.name)
    ]
    rows = [read_json(path) for path in resource_paths]
    if not rows:
        raise ValueError(f"no resource summaries under {args.usage_root}")
    elapsed = [row["elapsed_seconds"] for row in rows if row.get("elapsed_seconds")]
    value = {
        "schema_version": 1,
        "shards": len(rows),
        "completed_matches": sum(int(row.get("completed_matches", 0)) for row in rows),
        "failed_shards": sum(int(row.get("exit_code", 1) != 0) for row in rows),
        "summed_shard_hours": sum(elapsed) / 3600.0,
        "mean_shard_hours": (sum(elapsed) / len(elapsed) / 3600.0) if elapsed else None,
        "maximum_resident_set_kb": max(
            (int(row.get("maximum_resident_set_kb", 0)) for row in rows), default=0
        ),
        "result_bytes": sum(int(row.get("result_bytes", 0)) for row in rows),
    }
    args.output.mkdir(parents=True, exist_ok=True)
    atomic_json(args.output / "summary.json", value)
    print(json.dumps(value, indent=2, sort_keys=True))


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser()
    commands = root.add_subparsers(dest="command", required=True)

    command = commands.add_parser("plan-info")
    command.add_argument("--plan", type=Path, required=True)
    command.set_defaults(function=plan_info)

    command = commands.add_parser("check-plan-grid")
    command.add_argument("--plan", type=Path, required=True)
    command.add_argument("--alpha-values", nargs="+", required=True)
    command.add_argument("--q-values", nargs="+", required=True)
    command.add_argument("--c-min", type=float, required=True)
    command.add_argument("--c-max", type=float, required=True)
    command.add_argument("--c-step", type=float, required=True)
    command.add_argument("--kappa-min", type=float, required=True)
    command.add_argument("--kappa-max", type=float, required=True)
    command.add_argument("--kappa-points", type=int, required=True)
    command.set_defaults(function=check_plan_grid)

    command = commands.add_parser("check-summary")
    command.add_argument("--path", type=Path, required=True)
    command.add_argument("--shard-id", type=int, required=True)
    command.set_defaults(function=check_summary)

    command = commands.add_parser("summarize")
    command.add_argument("--mode", choices=("pilot", "full"), required=True)
    command.add_argument("--shard-id", type=int, required=True)
    command.add_argument("--time-file", type=Path, required=True)
    command.add_argument("--run-summary", type=Path, required=True)
    command.add_argument("--results", type=Path, required=True)
    command.add_argument("--exit-code", type=int, required=True)
    command.add_argument("--output", type=Path, required=True)
    command.set_defaults(function=summarize)

    command = commands.add_parser("aggregate")
    command.add_argument("--usage-root", type=Path, required=True)
    command.add_argument("--output", type=Path, required=True)
    command.set_defaults(function=aggregate)
    return root


def main() -> int:
    args = parser().parse_args()
    try:
        args.function(args)
    except (OSError, ValueError, json.JSONDecodeError) as error:
        print(f"ERROR: {error}", file=__import__("sys").stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
