#!/usr/bin/env python3
"""Small, dependency-free helpers for the IRIS Slurm round-robin wrapper."""

from __future__ import annotations

import argparse
import csv
import json
import math
import os
import statistics
from pathlib import Path
import re
import socket
import sys
from typing import Any


class HelperError(RuntimeError):
    pass


def read_json(path: Path) -> Any:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise HelperError(f"cannot read JSON {path}: {error}") from error


def write_json_once_or_equal(path: Path, value: Any) -> None:
    text = json.dumps(value, indent=2, sort_keys=True) + "\n"
    path.parent.mkdir(parents=True, exist_ok=True)
    if path.exists():
        if path.read_text(encoding="utf-8") != text:
            raise HelperError(f"existing file differs from requested content: {path}")
        return
    path.write_text(text, encoding="utf-8")


def absolute_from(base: Path, value: str) -> str:
    path = Path(value).expanduser()
    return str((path if path.is_absolute() else base / path).resolve())


def prepare_config(args: argparse.Namespace) -> None:
    source = args.source.resolve()
    config = read_json(source)
    label = config.get("covid_spike_label_specification")
    if not isinstance(label, dict) or label.get("id") != args.label_set:
        actual = label.get("id") if isinstance(label, dict) else None
        raise HelperError(
            f"requested COVID SPIKE label set {args.label_set!r}, but config declares {actual!r}"
        )
    base = source.parent
    config["organizer_binary"] = str(args.organizer.resolve())
    for key in ("transfer_root", "run_inputs_root"):
        config[key] = absolute_from(base, config[key])
    for model in config["models"].values():
        model["nci_summary"] = absolute_from(base, model["nci_summary"])
    for evaluation in config["evaluations"].values():
        for key in ("evaluation_dir", "mapping_source"):
            evaluation[key] = absolute_from(base, evaluation[key])
    write_json_once_or_equal(args.output.resolve(), config)


def bundle_label(args: argparse.Namespace) -> None:
    spec = read_json(args.bundle / "tournament_spec.json")
    try:
        actual = spec["annotations"]["scientific_contract"]["covid_spike_label_specification"]["id"]
    except (KeyError, TypeError) as error:
        raise HelperError(f"bundle does not record a COVID SPIKE label specification: {args.bundle}") from error
    if actual != args.label_set:
        raise HelperError(
            f"bundle SPIKE label set {actual!r} does not match requested {args.label_set!r}: {args.bundle}"
        )


def plan_info(args: argparse.Namespace) -> None:
    plan = read_json(args.plan / "plan.json")
    actual_count = plan.get("expected_match_count")
    if actual_count != args.expected_matches:
        raise HelperError(
            f"plan has {actual_count} matches, expected {args.expected_matches}: {args.plan}"
        )
    expected_policy = {
        "target_matches_per_shard": {"matches_per_shard": args.matches_per_shard}
    }
    if plan.get("sharding_policy") != expected_policy:
        raise HelperError(
            f"plan sharding policy does not match --matches-per-shard {args.matches_per_shard}"
        )
    shard_count = plan.get("shard_count")
    if not isinstance(shard_count, int) or shard_count < 1:
        raise HelperError(f"invalid shard_count in {args.plan}")
    print(f"{shard_count}\t{actual_count}")


def check_run_summary(args: argparse.Namespace) -> None:
    summary = read_json(args.path)
    required = {"assigned", "computed", "already_valid", "failed"}
    if not isinstance(summary, dict) or not required <= set(summary):
        raise HelperError(f"invalid organizer run summary: {args.path}")
    if summary["failed"] != 0 or summary["assigned"] != summary["computed"] + summary["already_valid"]:
        raise HelperError(f"organizer run summary is not successful: {args.path}")


def parse_time_file(path: Path) -> tuple[float | None, int | None]:
    if not path.exists():
        return None, None
    text = path.read_text(encoding="utf-8", errors="replace")
    rss_match = re.search(r"Maximum resident set size \(kbytes\):\s*(\d+)", text)
    rss = int(rss_match.group(1)) if rss_match else None
    if rss is None:
        mac_rss_match = re.search(r"^\s*(\d+)\s+maximum resident set size", text, re.MULTILINE)
        if mac_rss_match:
            rss = math.ceil(int(mac_rss_match.group(1)) / 1024)
    elapsed = None
    for line in text.splitlines():
        if "Elapsed (wall clock) time" in line and ": " in line:
            elapsed = parse_elapsed(line.rsplit(": ", 1)[1].strip())
            break
    if elapsed is None:
        mac_elapsed_match = re.search(r"^\s*([0-9.]+)\s+real\b", text, re.MULTILINE)
        if mac_elapsed_match:
            elapsed = float(mac_elapsed_match.group(1))
    return elapsed, rss


def parse_elapsed(value: str) -> float | None:
    parts = value.split(":")
    try:
        if len(parts) == 2:
            minutes, seconds = parts
            return int(minutes) * 60 + float(seconds)
        if len(parts) == 3:
            hours, minutes, seconds = parts
            return int(hours) * 3600 + int(minutes) * 60 + float(seconds)
    except ValueError:
        return None
    return None


def summarize_shard(args: argparse.Namespace) -> None:
    plan = read_json(args.plan / "plan.json")
    match_ids = [
        match["match_id"]
        for match in plan.get("matches", [])
        if match.get("shard_id") == args.shard_id
    ]
    artifact_count = 0
    artifact_bytes = 0
    for match_id in match_ids:
        for suffix in (".json", ".json.sha256"):
            path = args.results / "matches" / f"{match_id}{suffix}"
            if path.is_file():
                artifact_count += suffix == ".json"
                artifact_bytes += path.stat().st_size
    elapsed_seconds, max_rss_kb = parse_time_file(args.time_file)
    run_summary = None
    if args.run_summary.is_file():
        try:
            run_summary = read_json(args.run_summary)
        except HelperError:
            run_summary = None
    summary = {
        "schema_version": 1,
        "mode": args.mode,
        "metric": args.metric,
        "shard_id": args.shard_id,
        "hostname": socket.gethostname(),
        "slurm_job_id": os.environ.get("SLURM_JOB_ID"),
        "slurm_array_task_id": os.environ.get("SLURM_ARRAY_TASK_ID"),
        "slurm_cpus_per_task": os.environ.get("SLURM_CPUS_PER_TASK"),
        "exit_code": args.exit_code,
        "assigned_matches": len(match_ids),
        "completed_match_artifacts": artifact_count,
        "artifact_bytes": artifact_bytes,
        "elapsed_seconds": elapsed_seconds,
        "maximum_resident_set_kb": max_rss_kb,
        "run_summary": run_summary,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    temporary = args.output.with_suffix(args.output.suffix + ".tmp")
    temporary.write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    temporary.replace(args.output)


def aggregate_usage(args: argparse.Namespace) -> None:
    paths = [
        path
        for path in sorted(args.usage_root.glob("*/*.json"))
        if re.fullmatch(r"shard_\d+\.json", path.name)
    ]
    records = [read_json(path) for path in paths]
    if not records:
        raise HelperError(f"no resource summaries found below {args.usage_root}")
    args.output.mkdir(parents=True, exist_ok=True)
    csv_path = args.output / "shard_resource_usage.csv"
    fields = [
        "mode", "metric", "shard_id", "hostname", "slurm_job_id",
        "slurm_array_task_id", "slurm_cpus_per_task", "exit_code", "assigned_matches",
        "completed_match_artifacts", "artifact_bytes", "elapsed_seconds",
        "maximum_resident_set_kb",
    ]
    with csv_path.open("w", newline="", encoding="utf-8") as stream:
        writer = csv.DictWriter(stream, fieldnames=fields, lineterminator="\n")
        writer.writeheader()
        writer.writerows({key: record.get(key) for key in fields} for record in records)
    elapsed = [float(record["elapsed_seconds"]) for record in records if record.get("elapsed_seconds") is not None]
    rss = [int(record["maximum_resident_set_kb"]) for record in records if record.get("maximum_resident_set_kb") is not None]
    aggregate = {
        "schema_version": 1,
        "array_tasks": len(records),
        "failed_tasks": sum(record.get("exit_code") != 0 for record in records),
        "assigned_matches": sum(int(record.get("assigned_matches", 0)) for record in records),
        "completed_match_artifacts": sum(
            int(record.get("completed_match_artifacts", 0)) for record in records
        ),
        "artifact_bytes": sum(int(record.get("artifact_bytes", 0)) for record in records),
        "elapsed_seconds": {
            "minimum": min(elapsed) if elapsed else None,
            "median": statistics.median(elapsed) if elapsed else None,
            "maximum": max(elapsed) if elapsed else None,
        },
        "maximum_resident_set_kb": max(rss) if rss else None,
    }
    (args.output / "resource_usage_summary.json").write_text(
        json.dumps(aggregate, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )


def positive_int(value: str) -> int:
    parsed = int(value)
    if parsed < 1:
        raise argparse.ArgumentTypeError("must be positive")
    return parsed


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser(description=__doc__)
    commands = root.add_subparsers(dest="command", required=True)

    config = commands.add_parser("prepare-config")
    config.add_argument("--source", type=Path, required=True)
    config.add_argument("--output", type=Path, required=True)
    config.add_argument("--organizer", type=Path, required=True)
    config.add_argument("--label-set", required=True)
    config.set_defaults(function=prepare_config)

    bundle = commands.add_parser("check-bundle-label")
    bundle.add_argument("--bundle", type=Path, required=True)
    bundle.add_argument("--label-set", required=True)
    bundle.set_defaults(function=bundle_label)

    plan = commands.add_parser("plan-info")
    plan.add_argument("--plan", type=Path, required=True)
    plan.add_argument("--expected-matches", type=positive_int, required=True)
    plan.add_argument("--matches-per-shard", type=positive_int, required=True)
    plan.set_defaults(function=plan_info)

    run_summary = commands.add_parser("check-run-summary")
    run_summary.add_argument("--path", type=Path, required=True)
    run_summary.set_defaults(function=check_run_summary)

    summary = commands.add_parser("summarize-shard")
    summary.add_argument("--mode", choices=("pilot", "full"), required=True)
    summary.add_argument("--metric", choices=("pr", "roc"), required=True)
    summary.add_argument("--shard-id", type=int, required=True)
    summary.add_argument("--plan", type=Path, required=True)
    summary.add_argument("--results", type=Path, required=True)
    summary.add_argument("--time-file", type=Path, required=True)
    summary.add_argument("--run-summary", type=Path, required=True)
    summary.add_argument("--exit-code", type=int, required=True)
    summary.add_argument("--output", type=Path, required=True)
    summary.set_defaults(function=summarize_shard)

    aggregate = commands.add_parser("aggregate-usage")
    aggregate.add_argument("--usage-root", type=Path, required=True)
    aggregate.add_argument("--output", type=Path, required=True)
    aggregate.set_defaults(function=aggregate_usage)
    return root


def main() -> int:
    try:
        args = parser().parse_args()
        args.function(args)
        return 0
    except (HelperError, KeyError, OSError, ValueError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
