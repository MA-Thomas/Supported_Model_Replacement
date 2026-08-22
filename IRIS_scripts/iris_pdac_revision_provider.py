#!/usr/bin/env python3
"""Build a PDAC-only tournament bundle for an audited context revision."""

from __future__ import annotations

import argparse
import copy
import datetime as dt
import json
from pathlib import Path
import shutil
import sys
import tempfile
from typing import Any

import iris_score_provider as provider


SCRIPT_DIR = Path(__file__).resolve().parent
DEFAULT_BASE_CONFIG = SCRIPT_DIR / "config.example.json"
DEFAULT_BASE_BUNDLES = SCRIPT_DIR / "prebuilt_bundles/threshold_zero"
DEFAULT_REVISION_ROOT = Path(
    "/Users/thomm15/Work_Data/IRIS_scripts/Single_Parameter_Set_Evaluation/"
    "downstream_analyses_and_plots/outputs/pdac_no_splen_evac_grid4_revision"
)
EXPECTED_REVISION_ID = "pdac_no_splen_evac_grid4"
EXPECTED_EXCLUDED_PATIENT_IDS = [
    "99122", "99299", "99316", "99366", "99559", "99593", "99646",
]
EXPECTED_COUNTS = (112, 19, 93)


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("validate", "build"))
    parser.add_argument("--metric", choices=sorted(provider.METRICS), required=True)
    parser.add_argument("--base-config", type=Path, default=DEFAULT_BASE_CONFIG)
    parser.add_argument("--base-bundle-root", type=Path, default=DEFAULT_BASE_BUNDLES)
    parser.add_argument("--revision-root", type=Path, default=DEFAULT_REVISION_ROOT)
    parser.add_argument("--organizer", type=Path)
    parser.add_argument("--output", type=Path)
    return parser.parse_args(argv)


def revision_configuration(args: argparse.Namespace) -> tuple[dict[str, Any], dict[str, Any], Path]:
    config = provider.read_config(args.base_config.resolve())
    revision_root = args.revision_root.resolve()
    manifest_path = provider.require_file(revision_root / "revision_manifest.json", "revision manifest")
    manifest = provider.load_json(manifest_path)
    if manifest.get("status") != "complete" or manifest.get("revision_id") != EXPECTED_REVISION_ID:
        raise provider.ProviderError("revision manifest is not the completed declared revision")
    scope = manifest.get("revision_scope", {})
    if scope.get("replaced_evaluation") != "pdac":
        raise provider.ProviderError("revision manifest does not replace PDAC")
    if scope.get("vaccination_grid_id") != 4:
        raise provider.ProviderError("revision manifest does not declare vaccination grid 4")
    if sorted(scope.get("excluded_patient_ids", [])) != EXPECTED_EXCLUDED_PATIENT_IDS:
        raise provider.ProviderError("revision manifest does not exclude exactly the seven declared patients")
    validation = manifest.get("transfer_validation", {})
    counts = (
        validation.get("endpoint_count"),
        validation.get("positive_count"),
        validation.get("negative_count"),
    )
    if counts != EXPECTED_COUNTS:
        raise provider.ProviderError(f"revision transfer counts are {counts}, expected {EXPECTED_COUNTS}")

    revised = copy.deepcopy(config)
    revised["provider_identity"] = "iris-pdac-context-revision-provider-v1"
    revised["transfer_root"] = str(revision_root / "transfers")
    revised["evaluations"] = {"pdac": revised["evaluations"]["pdac"]}
    revised["evaluations"]["pdac"]["mapping_source"] = str(
        revision_root / "inputs/pdac_mono_mapping.csv"
    )
    if args.organizer is not None:
        revised["organizer_binary"] = str(args.organizer.resolve())
    return revised, manifest, manifest_path


def read_json(path: Path) -> dict[str, Any]:
    value = provider.load_json(path)
    if not isinstance(value, dict):
        raise provider.ProviderError(f"expected JSON object in {path}")
    return value


def verify_revision_transfers(
    manifest: dict[str, Any], manifest_path: Path, metric: str
) -> None:
    records = manifest["transfer_validation"].get("records", [])
    indexed = {(row.get("model_id"), row.get("selector")): row for row in records}
    for model_id in provider.MODEL_IDS:
        record = indexed.get((model_id, metric))
        if record is None:
            raise provider.ProviderError(f"revision manifest lacks {model_id}/{metric}")
        summary_path = Path(record["summary"])
        predictions_path = Path(record["predictions"])
        for path, digest in (
            (summary_path, record["summary_sha256"]),
            (predictions_path, record["predictions_sha256"]),
        ):
            provider.require_file(path, "revision transfer artifact")
            if provider.sha256_file(path) != digest:
                raise provider.ProviderError(f"revision transfer hash mismatch: {path}")
        summary = read_json(summary_path)
        if summary.get("e_vac_grid_id") != 4:
            raise provider.ProviderError(f"revision transfer does not use E_vac grid 4: {summary_path}")
        e_vac_path = Path(str(summary.get("e_vac", "")))
        provider.require_file(e_vac_path, "transfer E_vac input")
        if summary.get("e_vac_sha256") != provider.sha256_file(e_vac_path):
            raise provider.ProviderError(f"transfer E_vac hash mismatch: {summary_path}")
        if "ln(1 + E_vac)" not in str(summary.get("score_transform", "")):
            raise provider.ProviderError(f"transfer does not declare the vaccine score transform: {summary_path}")
    if provider.sha256_file(manifest_path) == "":
        raise provider.ProviderError("unreachable empty revision manifest hash")


def compatible_registry_and_spec(
    config: dict[str, Any], metric: str, registry: dict[str, Any], base_bundle: Path
) -> tuple[dict[str, Any], dict[str, Any]]:
    base_registry = read_json(base_bundle / "systems.json")
    if registry != base_registry:
        raise provider.ProviderError("revised and base system registries differ")
    base_spec = read_json(base_bundle / "tournament_spec.json")
    if base_spec.get("metric") != provider.METRICS[metric]:
        raise provider.ProviderError("base bundle metric differs from requested metric")
    generated = provider.tournament_spec(config, metric)
    for key in (
        "schema_version", "metric", "verdict_field", "evidence_policy",
        "computational_design", "reference_assessment", "master_seed",
        "seed_derivation_version", "conjunction_rule", "graph_maximality_rule",
        "selection_rule", "pr_cnap", "auroc", "operational_tie_break",
    ):
        if generated.get(key) != base_spec.get(key):
            raise provider.ProviderError(f"revised policy differs from base policy at {key}")
    spec = copy.deepcopy(base_spec)
    spec["evaluations"] = ["pdac"]
    annotations = spec.setdefault("annotations", {})
    annotations["context_revision"] = {
        "revision_id": EXPECTED_REVISION_ID,
        "replaced_evaluation": "pdac",
        "excluded_patient_ids": EXPECTED_EXCLUDED_PATIENT_IDS,
        "vaccination_grid_id": 4,
    }
    return base_registry, spec


def build_bundle(args: argparse.Namespace) -> dict[str, Any]:
    config, revision_manifest, revision_manifest_path = revision_configuration(args)
    verify_revision_transfers(revision_manifest, revision_manifest_path, args.metric)
    registry, evaluations, _ = provider.assemble(config, args.metric)
    base_bundle = (args.base_bundle_root.resolve() / args.metric)
    provider.require_dir(base_bundle, "base bundle")
    provider.organizer_command(config, "validate-bundle", "--bundle", str(base_bundle))
    registry, spec = compatible_registry_and_spec(config, args.metric, registry, base_bundle)
    if args.command == "validate":
        return {
            "status": "valid",
            "revision_id": EXPECTED_REVISION_ID,
            "metric": provider.METRICS[args.metric],
            "systems": len(registry["systems"]),
            "evaluations": 1,
            "endpoints": len(evaluations["pdac"]["labels"]),
        }
    if args.output is None:
        raise provider.ProviderError("build requires --output")
    output = args.output.resolve()
    if output.exists():
        raise provider.ProviderError(f"output already exists: {output}")
    output.parent.mkdir(parents=True, exist_ok=True)
    staging = Path(tempfile.mkdtemp(prefix=f".{output.name}.tmp-", dir=output.parent))
    try:
        provider.write_json(staging / "systems.json", registry)
        provider.write_json(staging / "tournament_spec.json", spec)
        value = evaluations["pdac"]
        directory = staging / "evaluations/pdac"
        endpoint_ids = sorted(value["labels"])
        score_columns = [
            system["score_column"]
            for system in sorted(registry["systems"], key=lambda row: row["system_id"])
        ]
        provider.write_csv_new(
            directory / "endpoints.csv",
            ["endpoint_id", "label"],
            ({"endpoint_id": endpoint_id, "label": int(value["labels"][endpoint_id])}
             for endpoint_id in endpoint_ids),
        )
        provider.write_csv_new(
            directory / "scores.csv",
            ["endpoint_id", *score_columns],
            ({
                "endpoint_id": endpoint_id,
                **{column: repr(value["scores"][column][endpoint_id]) for column in score_columns},
            } for endpoint_id in endpoint_ids),
        )
        identity_fields = config["evaluations"]["pdac"]["endpoint_fields"]
        identity_path = directory / "endpoint_identities.csv"
        provider.write_csv_new(
            identity_path,
            ["endpoint_id", *identity_fields],
            ({"endpoint_id": endpoint_id, **value["identities"][endpoint_id]}
             for endpoint_id in endpoint_ids),
        )
        provenance = value["provenance"]
        provenance["revision"] = {
            "revision_id": EXPECTED_REVISION_ID,
            "revision_manifest": str(revision_manifest_path.resolve()),
            "revision_manifest_sha256": provider.sha256_file(revision_manifest_path),
            "base_bundle": str(base_bundle),
            "base_bundle_content_hash": read_json(base_bundle / "bundle_manifest.json")["bundle_content_hash"],
            "excluded_patient_ids": EXPECTED_EXCLUDED_PATIENT_IDS,
            "vaccination_grid_id": 4,
            "endpoint_count": 112,
            "positive_count": 19,
            "negative_count": 93,
        }
        provenance["endpoint_identity_table"] = {
            "path": "endpoint_identities.csv",
            "sha256": provider.sha256_file(identity_path),
        }
        provenance["source_artifacts"] = [
            {"path": path, "sha256": digest}
            for path, digest in provenance["source_artifacts"]
        ]
        provider.write_json(directory / "source_provenance.json", provenance)
        base = Path("evaluations/pdac")
        manifest = {
            "schema_name": provider.BUNDLE_SCHEMA_NAME,
            "schema_version": provider.BUNDLE_SCHEMA_VERSION,
            "metric": provider.METRICS[args.metric],
            "bundle_creation_time": dt.datetime.now(dt.timezone.utc).isoformat().replace("+00:00", "Z"),
            "systems": provider.hashed(Path("systems.json"), staging),
            "tournament_spec": provider.hashed(Path("tournament_spec.json"), staging),
            "evaluations": [{
                "evaluation_id": "pdac",
                "endpoints": provider.hashed(base / "endpoints.csv", staging),
                "scores": provider.hashed(base / "scores.csv", staging),
                "source_provenance": provider.hashed(base / "source_provenance.json", staging),
            }],
            "score_provider_identity": config["provider_identity"],
            "minimum_organizer_schema_version": provider.BUNDLE_SCHEMA_VERSION,
            "bundle_content_hash": "",
        }
        provider.write_json(staging / "bundle_manifest.json", manifest)
        content_hash = provider.organizer_command(
            config, "bundle-content-hash", "--bundle", str(staging)
        )
        (staging / "bundle_manifest.json").unlink()
        manifest["bundle_content_hash"] = content_hash
        provider.write_json(staging / "bundle_manifest.json", manifest)
        provider.organizer_command(config, "validate-bundle", "--bundle", str(staging))
        staging.rename(output)
        return {
            "status": "built",
            "revision_id": EXPECTED_REVISION_ID,
            "metric": provider.METRICS[args.metric],
            "bundle": str(output),
            "bundle_content_hash": content_hash,
            "systems": len(registry["systems"]),
            "evaluations": 1,
            "endpoints": len(endpoint_ids),
        }
    except Exception:
        shutil.rmtree(staging, ignore_errors=True)
        raise


def main(argv: list[str] | None = None) -> int:
    try:
        args = parse_args(argv)
        if args.command == "validate" and args.output is not None:
            raise provider.ProviderError("validate does not accept --output")
        result = build_bundle(args)
        json.dump(result, sys.stdout, indent=2, sort_keys=True)
        sys.stdout.write("\n")
        return 0
    except (provider.ProviderError, FileNotFoundError, KeyError, OSError, ValueError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
