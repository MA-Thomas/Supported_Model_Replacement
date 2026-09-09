#!/usr/bin/env python3
"""Small, dependency-free helpers for the IRIS Slurm round-robin wrapper."""

from __future__ import annotations

import argparse
import csv
import json
import math
import os
import resource
import shutil
import statistics
import subprocess
from pathlib import Path
import re
import socket
import sys
import time
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
    if config.get("configuration_kind") == "prebuilt_bundles":
        allowed = {
            "schema_version",
            "configuration_kind",
            "prebuilt_bundle_root",
            "covid_spike_label_specification",
        }
        unknown = set(config) - allowed
        if unknown:
            raise HelperError(f"prebuilt-bundle configuration has unknown fields: {sorted(unknown)}")
        if config.get("schema_version") != 1:
            raise HelperError("prebuilt-bundle configuration schema_version must be 1")
        raw_root = config.get("prebuilt_bundle_root")
        if not isinstance(raw_root, str) or not raw_root.strip():
            raise HelperError("prebuilt_bundle_root must be a nonempty path")
        root = Path(absolute_from(base, raw_root))
        if not root.is_dir():
            raise HelperError(f"prebuilt bundle root does not exist: {root}")
        config["prebuilt_bundle_root"] = str(root)
        config["organizer_binary"] = str(args.organizer.resolve())
        write_json_once_or_equal(args.output.resolve(), config)
        return
    if config.get("configuration_kind") is not None:
        raise HelperError(f"unsupported configuration_kind: {config['configuration_kind']!r}")
    config["organizer_binary"] = str(args.organizer.resolve())
    for key in ("transfer_root", "run_inputs_root"):
        config[key] = absolute_from(base, config[key])
    for model in config["models"].values():
        model["nci_summary"] = absolute_from(base, model["nci_summary"])
    for evaluation in config["evaluations"].values():
        for key in ("evaluation_dir", "mapping_source"):
            evaluation[key] = absolute_from(base, evaluation[key])
    write_json_once_or_equal(args.output.resolve(), config)


def prebuilt_bundle_root(args: argparse.Namespace) -> None:
    config = read_json(args.config.resolve())
    if config.get("configuration_kind") == "prebuilt_bundles":
        root = Path(config.get("prebuilt_bundle_root", ""))
        if not root.is_absolute() or not root.is_dir():
            raise HelperError(f"invalid prepared prebuilt bundle root: {root}")
        print(root)
    elif config.get("configuration_kind") is None:
        print("")
    else:
        raise HelperError(f"unsupported configuration_kind: {config['configuration_kind']!r}")


def install_prebuilt_bundle(args: argparse.Namespace) -> None:
    source = args.source.resolve()
    output = args.output.resolve()
    if not source.is_dir():
        raise HelperError(f"prebuilt bundle does not exist: {source}")
    if output.exists():
        raise HelperError(f"bundle output already exists: {output}")
    output.parent.mkdir(parents=True, exist_ok=True)
    temporary = output.parent / f".{output.name}.copying.{os.getpid()}"
    if temporary.exists():
        raise HelperError(f"temporary bundle path already exists: {temporary}")
    try:
        shutil.copytree(source, temporary)
        os.replace(temporary, output)
    except Exception:
        if temporary.exists():
            shutil.rmtree(temporary)
        raise


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


def check_frozen_hybrid_bundle(args: argparse.Namespace) -> None:
    registry = read_json(args.bundle / "systems.json")
    spec = read_json(args.bundle / "tournament_spec.json")
    manifest = read_json(args.bundle / "bundle_manifest.json")
    systems = registry.get("systems")
    if not isinstance(systems, list) or len(systems) != 65:
        raise HelperError(f"frozen-hybrid bundle must contain exactly 65 systems: {args.bundle}")
    expected_models = {
        "full_hla",
        "focal_hla",
        "old_monoallelic",
        "mono_q_full_pn",
        "full_q_mono_pn",
    }
    fixed_suffixes = {
        "max",
        "mean",
        "median",
        "logsumexp",
        "logmeanexp",
        "top_frac_mean_frac0p01",
        "top_frac_mean_frac0p02",
        "top_frac_mean_frac0p05",
        "top_k_mean_k2",
        "top_k_mean_k3",
        "top_k_logsumexp_k2",
        "top_k_logsumexp_k10",
    }
    hybrid_suffix = "__endpoint_local_epitope_second_hla_hybrid_v1"
    expected_system_ids = {
        f"{model}__{suffix}"
        for model in expected_models
        for suffix in fixed_suffixes
    }
    expected_hybrid_ids = {f"{model}{hybrid_suffix}" for model in expected_models}
    expected_system_ids.update(expected_hybrid_ids)
    observed_system_ids = {
        system.get("system_id") for system in systems if isinstance(system, dict)
    }
    if observed_system_ids != expected_system_ids or any(
        system.get("score_column") != f"score_{system.get('system_id')}"
        for system in systems
        if isinstance(system, dict)
    ):
        raise HelperError(f"bundle does not contain the exact frozen 65-system roster: {args.bundle}")
    contract = spec.get("annotations", {}).get("scientific_contract", {})
    adaptive = contract.get("frozen_adaptive_l2")
    expected_adaptive = {
        "method": "endpoint_local_epitope_second_hla_hybrid",
        "epitope_gate_center": -2.2,
        "epitope_gate_width": 0.13,
        "second_hla_threshold": -6.45,
        "second_hla_gate_width": 0.02,
        "hla_bonus": 1.0,
        "hla_weight": 0.12,
        "solver_absolute_tolerance": 1e-10,
        "solver_max_iterations": 64,
    }
    if (
        adaptive != expected_adaptive
        or contract.get("systems_per_component_model") != 13
        or set(contract.get("component_models", [])) != expected_models
        or spec.get("evaluations") != ["pdac", "covid_spike", "covid_nonspike"]
        or manifest.get("score_provider_identity")
        != "iris-rust-fullroster-frozen-hybrid-provider-v1"
    ):
        raise HelperError(f"bundle does not declare the frozen adaptive-L2 contract: {args.bundle}")


def check_revision_bundle(args: argparse.Namespace) -> None:
    spec = read_json(args.bundle / "tournament_spec.json")
    if spec.get("evaluations") != [args.evaluation]:
        raise HelperError(
            f"revision bundle must contain exactly {args.evaluation!r}: {args.bundle}"
        )
    revision = spec.get("annotations", {}).get("context_revision", {})
    if revision.get("revision_id") != args.revision_id:
        raise HelperError(
            f"bundle revision ID {revision.get('revision_id')!r} does not match "
            f"{args.revision_id!r}: {args.bundle}"
        )
    if revision.get("replaced_evaluation") != args.evaluation:
        raise HelperError(f"bundle does not declare replacement of {args.evaluation}: {args.bundle}")
    registry = read_json(args.bundle / "systems.json")
    if len(registry.get("systems", [])) != args.expected_systems:
        raise HelperError(f"unexpected system count in revision bundle: {args.bundle}")
    endpoint_path = args.bundle / "evaluations" / args.evaluation / "endpoints.csv"
    with endpoint_path.open(newline="", encoding="utf-8") as stream:
        rows = list(csv.DictReader(stream))
    labels = [row.get("label") for row in rows]
    if len(rows) != args.expected_endpoints or labels.count("1") != args.expected_positive:
        raise HelperError(
            f"revision endpoint roster differs from the declared contract: {args.bundle}"
        )


def bundle_dimensions(args: argparse.Namespace) -> None:
    registry = read_json(args.bundle / "systems.json")
    spec = read_json(args.bundle / "tournament_spec.json")
    systems = registry.get("systems")
    evaluations = spec.get("evaluations")
    if not isinstance(systems, list) or len(systems) < 2:
        raise HelperError(f"bundle must contain at least two systems: {args.bundle}")
    if not isinstance(evaluations, list) or not evaluations:
        raise HelperError(f"bundle must contain at least one evaluation: {args.bundle}")
    system_ids = [system.get("system_id") for system in systems if isinstance(system, dict)]
    score_columns = [system.get("score_column") for system in systems if isinstance(system, dict)]
    if (
        len(system_ids) != len(systems)
        or len(set(system_ids)) != len(systems)
        or None in system_ids
        or len(score_columns) != len(systems)
        or len(set(score_columns)) != len(systems)
        or None in score_columns
        or len(set(evaluations)) != len(evaluations)
        or any(not isinstance(value, str) or not value for value in evaluations)
    ):
        raise HelperError(f"bundle has invalid system or evaluation identities: {args.bundle}")
    expected_matches = len(evaluations) * len(systems) * (len(systems) - 1) // 2
    print(f"{expected_matches}\t{len(evaluations)}\t{len(systems)}")


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


def run_measured(args: argparse.Namespace) -> int:
    command = list(args.command)
    if command and command[0] == "--":
        command = command[1:]
    if not command:
        raise HelperError("run-measured requires a command after --")
    args.stdout_file.parent.mkdir(parents=True, exist_ok=True)
    args.time_file.parent.mkdir(parents=True, exist_ok=True)
    started = time.monotonic()
    with args.stdout_file.open("xb") as stdout:
        completed = subprocess.run(command, stdout=stdout, check=False)
    elapsed = time.monotonic() - started
    usage = resource.getrusage(resource.RUSAGE_CHILDREN)
    maximum_rss_kb = int(usage.ru_maxrss)
    if sys.platform == "darwin":
        maximum_rss_kb //= 1024
    measurement = {
        "schema_version": 1,
        "elapsed_seconds": elapsed,
        "maximum_resident_set_kb": maximum_rss_kb,
        "exit_code": completed.returncode,
    }
    temporary = args.time_file.with_name(f".{args.time_file.name}.tmp.{os.getpid()}")
    temporary.write_text(
        json.dumps(measurement, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    os.replace(temporary, args.time_file)
    return completed.returncode


def parse_time_file(path: Path) -> tuple[float | None, int | None]:
    if not path.exists():
        return None, None
    text = path.read_text(encoding="utf-8", errors="replace")
    try:
        measurement = json.loads(text)
    except json.JSONDecodeError:
        measurement = None
    if isinstance(measurement, dict):
        elapsed = measurement.get("elapsed_seconds")
        maximum_rss = measurement.get("maximum_resident_set_kb")
        return (
            float(elapsed) if isinstance(elapsed, (int, float)) else None,
            int(maximum_rss) if isinstance(maximum_rss, int) else None,
        )
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

    prebuilt = commands.add_parser("prebuilt-bundle-root")
    prebuilt.add_argument("--config", type=Path, required=True)
    prebuilt.set_defaults(function=prebuilt_bundle_root)

    install = commands.add_parser("install-prebuilt-bundle")
    install.add_argument("--source", type=Path, required=True)
    install.add_argument("--output", type=Path, required=True)
    install.set_defaults(function=install_prebuilt_bundle)

    bundle = commands.add_parser("check-bundle-label")
    bundle.add_argument("--bundle", type=Path, required=True)
    bundle.add_argument("--label-set", required=True)
    bundle.set_defaults(function=bundle_label)

    frozen = commands.add_parser("check-frozen-hybrid-bundle")
    frozen.add_argument("--bundle", type=Path, required=True)
    frozen.set_defaults(function=check_frozen_hybrid_bundle)

    revision_bundle = commands.add_parser("check-revision-bundle")
    revision_bundle.add_argument("--bundle", type=Path, required=True)
    revision_bundle.add_argument("--revision-id", required=True)
    revision_bundle.add_argument("--evaluation", required=True)
    revision_bundle.add_argument("--expected-systems", type=positive_int, required=True)
    revision_bundle.add_argument("--expected-endpoints", type=positive_int, required=True)
    revision_bundle.add_argument("--expected-positive", type=positive_int, required=True)
    revision_bundle.set_defaults(function=check_revision_bundle)

    dimensions = commands.add_parser("bundle-dimensions")
    dimensions.add_argument("--bundle", type=Path, required=True)
    dimensions.set_defaults(function=bundle_dimensions)

    plan = commands.add_parser("plan-info")
    plan.add_argument("--plan", type=Path, required=True)
    plan.add_argument("--expected-matches", type=positive_int, required=True)
    plan.add_argument("--matches-per-shard", type=positive_int, required=True)
    plan.set_defaults(function=plan_info)

    run_summary = commands.add_parser("check-run-summary")
    run_summary.add_argument("--path", type=Path, required=True)
    run_summary.set_defaults(function=check_run_summary)

    measured = commands.add_parser("run-measured")
    measured.add_argument("--time-file", type=Path, required=True)
    measured.add_argument("--stdout-file", type=Path, required=True)
    measured.add_argument("command", nargs=argparse.REMAINDER)
    measured.set_defaults(function=run_measured)

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
        result = args.function(args)
        return result if isinstance(result, int) else 0
    except (HelperError, KeyError, OSError, ValueError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
