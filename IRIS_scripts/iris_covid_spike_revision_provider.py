#!/usr/bin/env python3
"""Build a COVID-SPIKE-only bundle for the audited 0.53 label revision."""

from __future__ import annotations

import argparse
import copy
import csv
import datetime as dt
import json
from pathlib import Path
import shutil
import sys
import tempfile
from typing import Any

import iris_score_provider as provider


SCRIPT_DIR = Path(__file__).resolve().parent
DEFAULT_BASE_BUNDLES = SCRIPT_DIR / "prebuilt_bundles/threshold_zero"
DEFAULT_HIGHER_THRESHOLD_BUNDLES = (
    SCRIPT_DIR / "prebuilt_bundles/higher_threshold_0p53"
)
DEFAULT_ORGANIZER = (
    SCRIPT_DIR.parent
    / "supported_ap_code/target/debug/directed_round_robin_organizer"
)

REVISION_ID = "covid_spike_higher_threshold_0p53"
REPLACED_EVALUATION = "covid_spike"
BASE_LABEL_SET_ID = "threshold_zero"
REVISION_LABEL_SET_ID = "higher_threshold_0p53"
EXPECTED_THRESHOLD = 0.53
EXPECTED_ENDPOINTS = 1765
EXPECTED_POSITIVES = 667
EXPECTED_NEGATIVES = 1098
EXPECTED_BASE_NEGATIVES = 606
EXPECTED_POSITIVE_TO_NEGATIVE = 492
EXPECTED_MAPPING_SHA256 = (
    "53df8ce1cf1c0598400f46cf2165bc0c5ac32f3a6d89836131b883ab843ddbf2"
)


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("validate", "build"))
    parser.add_argument("--metric", choices=sorted(provider.METRICS), required=True)
    parser.add_argument("--base-bundle-root", type=Path, default=DEFAULT_BASE_BUNDLES)
    parser.add_argument(
        "--higher-threshold-bundle-root",
        type=Path,
        default=DEFAULT_HIGHER_THRESHOLD_BUNDLES,
    )
    parser.add_argument("--organizer", type=Path, default=DEFAULT_ORGANIZER)
    parser.add_argument("--output", type=Path)
    return parser.parse_args(argv)


def read_json(path: Path) -> dict[str, Any]:
    value = provider.load_json(path)
    if not isinstance(value, dict):
        raise provider.ProviderError(f"expected JSON object in {path}")
    return value


def read_labels(path: Path) -> dict[str, int]:
    with path.open(newline="", encoding="utf-8") as stream:
        reader = csv.DictReader(stream)
        if set(reader.fieldnames or []) != {"endpoint_id", "label"}:
            raise provider.ProviderError(f"unexpected endpoint columns in {path}")
        labels: dict[str, int] = {}
        for row_number, row in enumerate(reader, 2):
            endpoint_id = row["endpoint_id"]
            if endpoint_id in labels:
                raise provider.ProviderError(
                    f"duplicate endpoint {endpoint_id} in {path}:{row_number}"
                )
            if row["label"] not in ("0", "1"):
                raise provider.ProviderError(f"invalid label in {path}:{row_number}")
            labels[endpoint_id] = int(row["label"])
    return labels


def label_specification(spec: dict[str, Any]) -> dict[str, Any]:
    try:
        value = spec["annotations"]["scientific_contract"][
            "covid_spike_label_specification"
        ]
    except (KeyError, TypeError) as error:
        raise provider.ProviderError(
            "bundle lacks the declared COVID SPIKE label specification"
        ) from error
    if not isinstance(value, dict):
        raise provider.ProviderError("COVID SPIKE label specification is not an object")
    return value


def validate_label_specification(
    value: dict[str, Any], expected_id: str, expected_threshold: float
) -> None:
    expected = {
        "id": expected_id,
        "description": (
            "COVID SPIKE response is positive when cd8_IFNg_dmso_adj is "
            f"strictly greater than {expected_threshold}."
        ),
        "threshold": expected_threshold,
        "comparison_operator": ">",
        "response_field": "cd8_IFNg_dmso_adj",
    }
    if value != expected:
        raise provider.ProviderError(
            f"COVID SPIKE label specification is {value!r}, expected {expected!r}"
        )


def validate_policy_compatibility(
    base_spec: dict[str, Any], revision_spec: dict[str, Any]
) -> None:
    base_policy = copy.deepcopy(base_spec)
    revision_policy = copy.deepcopy(revision_spec)
    base_policy["evaluations"] = []
    revision_policy["evaluations"] = []
    base_policy["annotations"] = {}
    revision_policy["annotations"] = {}
    if base_policy != revision_policy:
        raise provider.ProviderError(
            "higher-threshold bundle changes tournament policy outside annotations"
        )


def validate_bundle_pair(
    base_bundle: Path, revision_bundle: Path, organizer: Path
) -> tuple[dict[str, Any], dict[str, Any], dict[str, Any]]:
    for bundle, description in (
        (base_bundle, "threshold-zero bundle"),
        (revision_bundle, "higher-threshold bundle"),
    ):
        provider.require_dir(bundle, description)
        provider.organizer_command(
            {"organizer_binary": str(organizer)},
            "validate-bundle",
            "--bundle",
            str(bundle),
        )

    base_registry = read_json(base_bundle / "systems.json")
    revision_registry = read_json(revision_bundle / "systems.json")
    if base_registry != revision_registry:
        raise provider.ProviderError("base and higher-threshold registries differ")
    if len(revision_registry.get("systems", [])) != 60:
        raise provider.ProviderError("higher-threshold registry does not contain 60 systems")

    base_spec = read_json(base_bundle / "tournament_spec.json")
    revision_spec = read_json(revision_bundle / "tournament_spec.json")
    expected_evaluations = {"covid_spike", "covid_nonspike", "pdac"}
    if set(base_spec.get("evaluations", [])) != expected_evaluations:
        raise provider.ProviderError("base bundle has an unexpected evaluation set")
    if set(revision_spec.get("evaluations", [])) != expected_evaluations:
        raise provider.ProviderError("higher-threshold bundle has an unexpected evaluation set")
    validate_policy_compatibility(base_spec, revision_spec)
    validate_label_specification(label_specification(base_spec), BASE_LABEL_SET_ID, 0.0)
    validate_label_specification(
        label_specification(revision_spec), REVISION_LABEL_SET_ID, EXPECTED_THRESHOLD
    )

    base_evaluation = base_bundle / "evaluations" / REPLACED_EVALUATION
    revision_evaluation = revision_bundle / "evaluations" / REPLACED_EVALUATION
    base_labels = read_labels(base_evaluation / "endpoints.csv")
    revision_labels = read_labels(revision_evaluation / "endpoints.csv")
    if set(base_labels) != set(revision_labels):
        raise provider.ProviderError("SPIKE endpoint identities changed with the threshold")
    positives = sum(revision_labels.values())
    negatives = len(revision_labels) - positives
    if (len(revision_labels), positives, negatives) != (
        EXPECTED_ENDPOINTS,
        EXPECTED_POSITIVES,
        EXPECTED_NEGATIVES,
    ):
        raise provider.ProviderError(
            "higher-threshold SPIKE counts are "
            f"{(len(revision_labels), positives, negatives)}, expected "
            f"{(EXPECTED_ENDPOINTS, EXPECTED_POSITIVES, EXPECTED_NEGATIVES)}"
        )
    transitions: dict[tuple[int, int], int] = {}
    for endpoint_id, base_label in base_labels.items():
        transition = (base_label, revision_labels[endpoint_id])
        transitions[transition] = transitions.get(transition, 0) + 1
    expected_transitions = {
        (0, 0): EXPECTED_BASE_NEGATIVES,
        (1, 0): EXPECTED_POSITIVE_TO_NEGATIVE,
        (1, 1): EXPECTED_POSITIVES,
    }
    if transitions != expected_transitions:
        raise provider.ProviderError(
            f"unexpected SPIKE label transitions: {transitions}"
        )
    if provider.sha256_file(base_evaluation / "scores.csv") != provider.sha256_file(
        revision_evaluation / "scores.csv"
    ):
        raise provider.ProviderError("SPIKE score vectors changed with the label threshold")
    if provider.sha256_file(
        base_evaluation / "endpoint_identities.csv"
    ) != provider.sha256_file(revision_evaluation / "endpoint_identities.csv"):
        raise provider.ProviderError("SPIKE endpoint identity table changed with the threshold")

    provenance = read_json(revision_evaluation / "source_provenance.json")
    label_contract = provenance.get("label_contract", {})
    if label_contract != {
        "comparison_operator": ">",
        "label_field": None,
        "response_field": "cd8_IFNg_dmso_adj",
        "threshold": EXPECTED_THRESHOLD,
    }:
        raise provider.ProviderError("higher-threshold source label contract is invalid")
    mapping_audit = provenance.get("mapping_label_audit", {})
    if (
        mapping_audit.get("mapping_rows_analysis_units") != EXPECTED_ENDPOINTS
        or mapping_audit.get("declared_analysis_unit_conflicts") != 0
        or mapping_audit.get("mapping_sha256") != EXPECTED_MAPPING_SHA256
    ):
        raise provider.ProviderError("higher-threshold mapping audit is invalid")
    return revision_registry, revision_spec, provenance


def build_bundle(args: argparse.Namespace) -> dict[str, Any]:
    organizer = args.organizer.resolve()
    provider.require_file(organizer, "organizer")
    base_bundle = args.base_bundle_root.resolve() / args.metric
    source_bundle = args.higher_threshold_bundle_root.resolve() / args.metric
    registry, source_spec, source_provenance = validate_bundle_pair(
        base_bundle, source_bundle, organizer
    )
    base_manifest = read_json(base_bundle / "bundle_manifest.json")
    source_manifest = read_json(source_bundle / "bundle_manifest.json")
    result = {
        "status": "valid",
        "revision_id": REVISION_ID,
        "metric": provider.METRICS[args.metric],
        "systems": len(registry["systems"]),
        "evaluations": 1,
        "endpoints": EXPECTED_ENDPOINTS,
        "positive": EXPECTED_POSITIVES,
        "negative": EXPECTED_NEGATIVES,
    }
    if args.command == "validate":
        return result
    if args.output is None:
        raise provider.ProviderError("build requires --output")
    output = args.output.resolve()
    if output.exists():
        raise provider.ProviderError(f"output already exists: {output}")
    output.parent.mkdir(parents=True, exist_ok=True)
    staging = Path(tempfile.mkdtemp(prefix=f".{output.name}.tmp-", dir=output.parent))
    try:
        provider.write_json(staging / "systems.json", registry)
        revision_spec = copy.deepcopy(source_spec)
        revision_spec["evaluations"] = [REPLACED_EVALUATION]
        revision_spec.setdefault("annotations", {})["context_revision"] = {
            "revision_id": REVISION_ID,
            "replaced_evaluation": REPLACED_EVALUATION,
            "base_label_set_id": BASE_LABEL_SET_ID,
            "replacement_label_set_id": REVISION_LABEL_SET_ID,
            "response_field": "cd8_IFNg_dmso_adj",
            "comparison_operator": ">",
            "threshold": EXPECTED_THRESHOLD,
            "endpoint_count": EXPECTED_ENDPOINTS,
            "positive_count": EXPECTED_POSITIVES,
            "negative_count": EXPECTED_NEGATIVES,
            "positive_to_negative_count": EXPECTED_POSITIVE_TO_NEGATIVE,
        }
        provider.write_json(staging / "tournament_spec.json", revision_spec)

        source_evaluation = source_bundle / "evaluations" / REPLACED_EVALUATION
        output_evaluation = staging / "evaluations" / REPLACED_EVALUATION
        output_evaluation.mkdir(parents=True)
        for name in ("endpoints.csv", "scores.csv", "endpoint_identities.csv"):
            shutil.copyfile(source_evaluation / name, output_evaluation / name)
        provenance = copy.deepcopy(source_provenance)
        provenance["revision"] = {
            "revision_id": REVISION_ID,
            "replaced_evaluation": REPLACED_EVALUATION,
            "base_label_set_id": BASE_LABEL_SET_ID,
            "replacement_label_set_id": REVISION_LABEL_SET_ID,
            "base_bundle": str(base_bundle),
            "base_bundle_content_hash": base_manifest["bundle_content_hash"],
            "source_full_bundle": str(source_bundle),
            "source_full_bundle_content_hash": source_manifest["bundle_content_hash"],
            "response_field": "cd8_IFNg_dmso_adj",
            "comparison_operator": ">",
            "threshold": EXPECTED_THRESHOLD,
            "endpoint_count": EXPECTED_ENDPOINTS,
            "positive_count": EXPECTED_POSITIVES,
            "negative_count": EXPECTED_NEGATIVES,
            "positive_to_negative_count": EXPECTED_POSITIVE_TO_NEGATIVE,
        }
        provider.write_json(output_evaluation / "source_provenance.json", provenance)

        relative = Path("evaluations") / REPLACED_EVALUATION
        manifest = {
            "schema_name": provider.BUNDLE_SCHEMA_NAME,
            "schema_version": provider.BUNDLE_SCHEMA_VERSION,
            "metric": provider.METRICS[args.metric],
            "bundle_creation_time": dt.datetime.now(dt.timezone.utc)
            .isoformat()
            .replace("+00:00", "Z"),
            "systems": provider.hashed(Path("systems.json"), staging),
            "tournament_spec": provider.hashed(Path("tournament_spec.json"), staging),
            "evaluations": [
                {
                    "evaluation_id": REPLACED_EVALUATION,
                    "endpoints": provider.hashed(relative / "endpoints.csv", staging),
                    "scores": provider.hashed(relative / "scores.csv", staging),
                    "source_provenance": provider.hashed(
                        relative / "source_provenance.json", staging
                    ),
                }
            ],
            "score_provider_identity": "iris-covid-spike-label-revision-provider-v1",
            "minimum_organizer_schema_version": provider.BUNDLE_SCHEMA_VERSION,
            "bundle_content_hash": "",
        }
        provider.write_json(staging / "bundle_manifest.json", manifest)
        content_hash = provider.organizer_command(
            {"organizer_binary": str(organizer)},
            "bundle-content-hash",
            "--bundle",
            str(staging),
        )
        (staging / "bundle_manifest.json").unlink()
        manifest["bundle_content_hash"] = content_hash
        provider.write_json(staging / "bundle_manifest.json", manifest)
        provider.organizer_command(
            {"organizer_binary": str(organizer)},
            "validate-bundle",
            "--bundle",
            str(staging),
        )
        staging.rename(output)
        return {**result, "status": "built", "bundle": str(output), "bundle_content_hash": content_hash}
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
