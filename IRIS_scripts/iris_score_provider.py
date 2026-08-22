#!/usr/bin/env python3
"""Strict IRIS score provider for directed supported-evidence tournaments."""

from __future__ import annotations

import argparse
import csv
import datetime as dt
import hashlib
import json
import math
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
from typing import Any, Iterable


SCHEMA_VERSION = 3
BUNDLE_SCHEMA_NAME = "directed_round_robin_input_bundle"
BUNDLE_SCHEMA_VERSION = 2
MODEL_IDS = (
    "full_hla",
    "focal_hla",
    "old_monoallelic",
    "mono_q_full_pn",
    "full_q_mono_pn",
)
METRICS = {"pr": "pr_cnap", "roc": "auroc"}
SELECTORS = {"pr": "nci_best_pr", "roc": "nci_best_roc"}
COUNT_LIKE = {
    "count",
    "log_count",
    "unique_nmer_count",
    "log_unique_nmer_count",
    "unique_nmer_hla_count",
}


class ProviderError(RuntimeError):
    """A fail-closed input or publication error."""


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def canonical_json(value: Any) -> str:
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"))


def load_json(path: Path) -> Any:
    try:
        with path.open(encoding="utf-8") as stream:
            return json.load(stream)
    except (OSError, json.JSONDecodeError) as error:
        raise ProviderError(f"cannot read JSON {path}: {error}") from error


def write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("x", encoding="utf-8", newline="\n") as stream:
        json.dump(value, stream, ensure_ascii=False, indent=2, sort_keys=True)
        stream.write("\n")


def require_file(path: Path, label: str) -> Path:
    if not path.is_file():
        raise ProviderError(f"missing {label}: {path}")
    return path


def require_dir(path: Path, label: str) -> Path:
    if not path.is_dir():
        raise ProviderError(f"missing {label}: {path}")
    return path


def config_path(config: dict[str, Any], raw: str) -> Path:
    path = Path(os.path.expandvars(os.path.expanduser(raw)))
    if not path.is_absolute():
        path = Path(config["_config_dir"]) / path
    return path.resolve()


def read_config(path: Path) -> dict[str, Any]:
    raw = load_json(path.resolve())
    if not isinstance(raw, dict):
        raise ProviderError("configuration root must be a JSON object")
    raw["_config_dir"] = str(path.resolve().parent)
    validate_config(raw)
    return raw


def exact_keys(value: dict[str, Any], allowed: set[str], context: str) -> None:
    unknown = set(value) - allowed
    if unknown:
        raise ProviderError(f"{context} has unknown fields: {sorted(unknown)}")


def stable_id(value: str, context: str) -> None:
    if not value or any(not (c.isascii() and (c.isalnum() or c in "_-.")) for c in value):
        raise ProviderError(f"{context} is not a stable machine identifier: {value!r}")


def validate_config(config: dict[str, Any]) -> None:
    exact_keys(
        config,
        {
            "schema_version", "provider_identity", "organizer_binary", "transfer_root",
            "run_inputs_root", "models", "evaluations", "aggregations", "policies",
            "accepted_measurement_error", "covid_spike_label_specification", "_config_dir",
        },
        "configuration",
    )
    if config.get("schema_version") != SCHEMA_VERSION:
        raise ProviderError(f"configuration schema_version must be {SCHEMA_VERSION}")
    if not str(config.get("provider_identity", "")).strip():
        raise ProviderError("provider_identity must be nonempty")
    models = config.get("models")
    if not isinstance(models, dict) or set(models) != set(MODEL_IDS):
        raise ProviderError(f"models must contain exactly {list(MODEL_IDS)}")
    for model_id, model in models.items():
        exact_keys(model, {"label", "nci_summary"}, f"model {model_id}")
        if not str(model.get("label", "")).strip() or not str(model.get("nci_summary", "")).strip():
            raise ProviderError(f"model {model_id} requires label and nci_summary")
    evaluations = config.get("evaluations")
    if not isinstance(evaluations, dict) or not evaluations:
        raise ProviderError("evaluations must be a nonempty object")
    for evaluation_id, evaluation in evaluations.items():
        stable_id(evaluation_id, "evaluation ID")
        exact_keys(
            evaluation,
            {
                "transfer_name", "evaluation_dir", "run_prefix", "mapping_source",
                "endpoint_fields", "response_field", "label_field", "threshold",
                "comparison_operator", "measurement_error_policy",
            },
            f"evaluation {evaluation_id}",
        )
        fields = evaluation.get("endpoint_fields")
        if not isinstance(fields, list) or not fields or len(fields) != len(set(fields)):
            raise ProviderError(f"evaluation {evaluation_id} endpoint_fields must be unique")
        if evaluation_id == "pdac" and "mutation" in fields:
            raise ProviderError("PDAC endpoint_fields must not contain mutation")
        if evaluation_id == "covid_spike" and fields != ["patient_id", "mutation", "long_peptide"]:
            raise ProviderError("COVID SPIKE endpoint_fields must preserve mutation")
        if bool(evaluation.get("response_field")) == bool(evaluation.get("label_field")):
            raise ProviderError(f"evaluation {evaluation_id} needs exactly one response_field or label_field")
        if evaluation.get("response_field"):
            if evaluation.get("comparison_operator") not in (">", ">="):
                raise ProviderError(f"evaluation {evaluation_id} has invalid comparison_operator")
            threshold = evaluation.get("threshold")
            if not isinstance(threshold, (int, float)) or not math.isfinite(float(threshold)):
                raise ProviderError(f"evaluation {evaluation_id} has invalid threshold")
    spike_label = config.get("covid_spike_label_specification")
    if not isinstance(spike_label, dict):
        raise ProviderError("covid_spike_label_specification must be an object")
    exact_keys(
        spike_label,
        {"id", "description", "threshold", "comparison_operator", "response_field"},
        "covid_spike_label_specification",
    )
    stable_id(str(spike_label.get("id", "")), "COVID SPIKE label specification ID")
    if not str(spike_label.get("description", "")).strip():
        raise ProviderError("covid_spike_label_specification description must be nonempty")
    threshold = spike_label.get("threshold")
    if not isinstance(threshold, (int, float)) or not math.isfinite(float(threshold)):
        raise ProviderError("covid_spike_label_specification threshold must be finite")
    if spike_label.get("comparison_operator") not in (">", ">="):
        raise ProviderError("covid_spike_label_specification has invalid comparison_operator")
    covid_evaluations = {"covid_spike", "covid_nonspike"}
    missing_covid = covid_evaluations - set(evaluations)
    if missing_covid:
        raise ProviderError(f"evaluations are missing COVID cohorts: {sorted(missing_covid)}")
    spike = evaluations["covid_spike"]
    if spike.get("label_field") is not None:
        raise ProviderError("covid_spike must use the selected response threshold")
    if spike.get("response_field") != spike_label.get("response_field"):
        raise ProviderError(
            "covid_spike response_field does not match covid_spike_label_specification"
        )
    if float(spike.get("threshold")) != float(threshold):
        raise ProviderError(
            "covid_spike threshold does not match covid_spike_label_specification"
        )
    if spike.get("comparison_operator") != spike_label["comparison_operator"]:
        raise ProviderError(
            "covid_spike comparison_operator does not match covid_spike_label_specification"
        )
    nonspike = evaluations["covid_nonspike"]
    if (
        nonspike.get("response_field") != "cd8_TNFa_IFNg_dmso_adj"
        or nonspike.get("label_field") is not None
        or float(nonspike.get("threshold")) != 0.0
        or nonspike.get("comparison_operator") != ">"
    ):
        raise ProviderError(
            "covid_nonspike label contract must remain cd8_TNFa_IFNg_dmso_adj > 0"
        )
    aggregations = config.get("aggregations")
    if not isinstance(aggregations, list) or not aggregations:
        raise ProviderError("aggregations must be a nonempty list")
    aggregation_ids: set[str] = set()
    for item in aggregations:
        if not isinstance(item, str):
            raise ProviderError("each aggregation must be a string identifier")
        aggregation_id = item
        stable_id(aggregation_id, "aggregation_id")
        if aggregation_id in COUNT_LIKE or "count" in aggregation_id.lower():
            raise ProviderError(f"count-like aggregation is forbidden: {aggregation_id}")
        if aggregation_id in aggregation_ids:
            raise ProviderError(f"duplicate aggregation: {aggregation_id}")
        aggregation_ids.add(aggregation_id)
    policies = config.get("policies")
    if not isinstance(policies, dict) or set(policies) != {"shared", "pr", "roc"}:
        raise ProviderError("policies must contain exactly shared, pr, and roc")


def endpoint_id(evaluation_id: str, fields: list[str], row: dict[str, str]) -> str:
    identity = [[field, row.get(field, "").strip()] for field in fields]
    for field, value in identity:
        if not value:
            raise ProviderError(f"{evaluation_id} endpoint has blank identity field {field}")
    digest = hashlib.sha256(canonical_json(identity).encode()).hexdigest()
    return f"{evaluation_id}.{digest}"


def embedded_transfer_label(row: dict[str, str]) -> bool:
    # Transfer outputs retain the label definition used when they were
    # published. It is audited for internal consistency but is not the
    # tournament label authority; the configured source mapping is.
    raw = row.get("label", "").strip()
    if raw not in ("0", "1"):
        raise ProviderError(f"transfer row has invalid binary label {raw!r}")
    return raw == "1"


def source_label(row: dict[str, str], evaluation: dict[str, Any], path: Path, row_number: int) -> bool:
    if evaluation.get("label_field"):
        raw = row.get(evaluation["label_field"], "").strip()
        if raw not in ("0", "1"):
            raise ProviderError(f"invalid source label {raw!r} in {path}:{row_number}")
        return raw == "1"
    field = evaluation["response_field"]
    raw = row.get(field, "").strip()
    try:
        value = float(raw)
    except ValueError as error:
        raise ProviderError(f"invalid source response {raw!r} in {path}:{row_number}") from error
    if not math.isfinite(value):
        raise ProviderError(f"nonfinite source response in {path}:{row_number}")
    threshold = float(evaluation["threshold"])
    return value > threshold if evaluation["comparison_operator"] == ">" else value >= threshold


def mapping_roster(
    path: Path, evaluation_id: str, evaluation: dict[str, Any]
) -> tuple[dict[str, bool], dict[str, dict[str, str]], dict[str, Any]]:
    labels: dict[str, bool] = {}
    identities: dict[str, dict[str, str]] = {}
    coarse_responses: dict[tuple[str, str], set[float]] = {}
    with path.open(newline="", encoding="utf-8") as stream:
        reader = csv.DictReader(stream)
        label_source = evaluation.get("label_field") or evaluation.get("response_field")
        required = {*evaluation["endpoint_fields"], label_source}
        missing = required - set(reader.fieldnames or [])
        if missing:
            raise ProviderError(f"{path} lacks mapping columns {sorted(missing)}")
        for row_number, row in enumerate(reader, 2):
            eid = endpoint_id(evaluation_id, evaluation["endpoint_fields"], row)
            label = source_label(row, evaluation, path, row_number)
            if eid in labels and labels[eid] != label:
                raise ProviderError(
                    f"conflicting labels at the declared analysis unit in {path}:{row_number}; "
                    "the provider cannot choose or aggregate one"
                )
            labels[eid] = label
            identities[eid] = {field: row[field].strip() for field in evaluation["endpoint_fields"]}
            if evaluation_id == "covid_spike":
                coarse = (row["patient_id"].strip(), row["long_peptide"].strip())
                raw_response = row[evaluation["response_field"]].strip()
                coarse_responses.setdefault(coarse, set()).add(float(raw_response))
    audit: dict[str, Any] = {
        "mapping": str(path.resolve()),
        "mapping_sha256": sha256_file(path),
        "mapping_rows_analysis_units": len(labels),
        "declared_analysis_unit_conflicts": 0,
    }
    if evaluation_id == "covid_spike":
        threshold = float(evaluation["threshold"])
        operator = evaluation["comparison_operator"]
        def classified(value: float) -> bool:
            return value > threshold if operator == ">" else value >= threshold
        audit.update({
            "coarser_patient_long_peptide_units": len(coarse_responses),
            "coarser_units_with_multiple_numeric_responses": sum(len(values) > 1 for values in coarse_responses.values()),
            "coarser_units_crossing_binary_threshold": sum(
                len({classified(value) for value in values}) > 1 for values in coarse_responses.values()
            ),
            "coarser_unit_is_not_analysis_unit": True,
            "interpretation": evaluation.get("measurement_error_policy"),
        })
    return labels, identities, audit


def geometry_string(parameters: dict[str, Any]) -> str:
    keys = ("d_pos", "d_neg", "steepness_pos", "steepness_neg")
    try:
        return ",".join(f"{key}={parameters[key]}" for key in keys)
    except KeyError as error:
        raise ProviderError(f"selected regime lacks geometry field {error.args[0]}") from error


def read_prediction_slice(
    path: Path,
    evaluation_id: str,
    evaluation: dict[str, Any],
    selector: str,
    aggregation_id: str,
) -> tuple[dict[str, tuple[bool, float]], dict[str, dict[str, str]]]:
    selected: dict[str, tuple[bool, float]] = {}
    identities: dict[str, dict[str, str]] = {}
    with path.open(newline="", encoding="utf-8") as stream:
        reader = csv.DictReader(stream)
        required = {"selector", "l3_variant", "is_count_baseline", "label", "score", *evaluation["endpoint_fields"]}
        missing = required - set(reader.fieldnames or [])
        if missing:
            raise ProviderError(f"{path} lacks columns {sorted(missing)}")
        for row_number, row in enumerate(reader, 2):
            if row["selector"] != selector or row["l3_variant"] != aggregation_id:
                continue
            if row["is_count_baseline"].strip().lower() in ("1", "true"):
                raise ProviderError(f"declared non-count aggregation {aggregation_id} is marked count baseline")
            eid = endpoint_id(evaluation_id, evaluation["endpoint_fields"], row)
            if eid in selected:
                raise ProviderError(f"duplicate prediction endpoint in {path}:{row_number}: {eid}")
            label = embedded_transfer_label(row)
            try:
                score = float(row["score"])
            except ValueError as error:
                raise ProviderError(f"nonnumeric score in {path}:{row_number}") from error
            if not math.isfinite(score):
                raise ProviderError(f"nonfinite score in {path}:{row_number}")
            selected[eid] = (label, score)
            identities[eid] = {field: row[field].strip() for field in evaluation["endpoint_fields"]}
    if not selected:
        raise ProviderError(f"no {selector}/{aggregation_id} predictions in {path}")
    return selected, identities


def source_files_from_summary(summary: dict[str, Any]) -> list[Path]:
    keys = (
        "tensor", "observations", "metadata", "mapping", "nci_summary", "e_vac",
        "target_tau_selection_table", "target_presentation_q_source_join",
    )
    paths: list[Path] = []
    for key in keys:
        value = summary.get(key)
        if isinstance(value, str) and value:
            paths.append(Path(value))
    components = summary.get("component_sources", {})
    if isinstance(components, dict):
        for key, value in components.items():
            if key != "q_join" and isinstance(value, str) and value:
                paths.append(Path(value))
    return paths


def validate_transfer_summary_contract(
    summary: dict[str, Any], summary_path: Path, evaluation: dict[str, Any], model_id: str
) -> None:
    if summary.get("response_column") != evaluation.get("response_field"):
        raise ProviderError(f"response-column contract mismatch in {summary_path}")
    if summary.get("label_column") != evaluation.get("label_field"):
        raise ProviderError(f"label-column contract mismatch in {summary_path}")
    declared_threshold = summary.get("label_threshold")
    if evaluation.get("response_field") is not None:
        if not isinstance(declared_threshold, (int, float)) or not math.isfinite(float(declared_threshold)):
            raise ProviderError(f"transfer label threshold is invalid in {summary_path}")
    elif declared_threshold is not None:
        raise ProviderError(f"label-based transfer declares a threshold in {summary_path}")
    expected_component = (
        "external query Q with primary-tensor P/N"
        if model_id in ("mono_q_full_pn", "full_q_mono_pn")
        else "Q and P/N from the same tensor"
    )
    if summary.get("component_model") != expected_component:
        raise ProviderError(f"component-model contract mismatch in {summary_path}")


def validate_upstream_roots(config: dict[str, Any]) -> dict[str, Any]:
    run_inputs = require_dir(config_path(config, config["run_inputs_root"]), "transfer run-input root")
    required_run_inputs = (
        "param_sets_external_validation.csv",
        "mn_tuples_external_validation.csv",
        "mono_inputs_manifest.json",
        "pdac_full_to_mono_observation_crosswalk.csv",
        "covid_spike_full_to_mono_observation_crosswalk.csv",
        "covid_nonspike_full_to_mono_observation_crosswalk.csv",
    )
    files = [require_file(run_inputs / name, "run input") for name in required_run_inputs]
    evaluation_roots: dict[str, str] = {}
    for evaluation_id, evaluation in config["evaluations"].items():
        root = require_dir(config_path(config, evaluation["evaluation_dir"]), f"{evaluation_id} evaluation directory")
        evaluation_roots[evaluation_id] = str(root)
        for suffix in ("full", "focal", "mono"):
            run = root / f"{evaluation['run_prefix']}_{suffix}"
            for name in ("f_tensor.parquet", "f_tensor.observations.parquet", "f_tensor.metadata.json"):
                files.append(require_file(run / name, f"{evaluation_id}/{suffix} {name}"))
        files.append(require_file(config_path(config, evaluation["mapping_source"]), f"{evaluation_id} mapping"))
    return {
        "run_inputs_root": str(run_inputs),
        "evaluation_roots": evaluation_roots,
        "validated_files": [{"path": str(path), "sha256": sha256_file(path)} for path in sorted(set(files))],
    }


def assemble(config: dict[str, Any], metric: str) -> tuple[dict[str, Any], dict[str, Any], dict[str, Any]]:
    if metric not in METRICS:
        raise ProviderError(f"unsupported metric {metric}")
    transfer_root = require_dir(config_path(config, config["transfer_root"]), "transfer root")
    upstream = validate_upstream_roots(config)
    selector = SELECTORS[metric]
    aggregations = config["aggregations"]
    systems: list[dict[str, Any]] = []
    model_regimes: dict[str, dict[str, Any]] = {}
    source_summaries: dict[tuple[str, str], tuple[dict[str, Any], Path, Path]] = {}

    for model_id in MODEL_IDS:
        configured_nci = require_file(config_path(config, config["models"][model_id]["nci_summary"]), f"{model_id} NCI summary")
        configured_nci_hash = sha256_file(configured_nci)
        frozen: tuple[str, str, int] | None = None
        for evaluation_id, evaluation in config["evaluations"].items():
            directory = transfer_root / evaluation["transfer_name"] / model_id / metric
            summary_path = require_file(directory / "summary.json", "transfer summary")
            predictions_path = require_file(directory / "long_peptide_predictions.csv", "transfer predictions")
            summary = load_json(summary_path)
            validate_transfer_summary_contract(summary, summary_path, evaluation, model_id)
            selected = summary.get("selected_regimes")
            if not isinstance(selected, list) or len(selected) != 1 or selected[0].get("selector") != selector:
                raise ProviderError(f"{summary_path} does not declare exactly {selector}")
            params = selected[0].get("target_parameters")
            if not isinstance(params, dict):
                raise ProviderError(f"{summary_path} lacks target_parameters")
            regime = (geometry_string(params), f"{params.get('M')}/{params.get('N')}", int(selected[0]["nci_regime_idx"]))
            if frozen is None:
                frozen = regime
            elif frozen != regime:
                raise ProviderError(f"metric branch changes across evaluations for {model_id}/{metric}")
            declared_nci = Path(str(summary.get("nci_summary", "")))
            require_file(declared_nci, "transfer-declared NCI summary")
            if sha256_file(declared_nci) != configured_nci_hash:
                raise ProviderError(f"NCI summary hash mismatch for {model_id}/{evaluation_id}/{metric}")
            expected_key = "(" + ", ".join(evaluation["endpoint_fields"]) + ")"
            if summary.get("endpoint_key") != expected_key:
                raise ProviderError(f"endpoint key mismatch in {summary_path}: expected {expected_key}")
            source_summaries[(evaluation_id, model_id)] = (summary, summary_path, predictions_path)
        assert frozen is not None
        model_regimes[model_id] = {
            "geometry": frozen[0], "m_over_n": frozen[1], "regime_idx": frozen[2],
            "summary_hash": configured_nci_hash,
        }
        for aggregation_id in aggregations:
            system_id = f"{model_id}__{aggregation_id}"
            stable_id(system_id, "system_id")
            systems.append({
                "system_id": system_id,
                "display_label": f"{config['models'][model_id]['label']} / {aggregation_id}",
                "score_column": f"score_{system_id}",
                "annotations": {
                    "provider_domain": "IRIS",
                    "component_model_id": model_id,
                    "metric_branch_id": f"{model_id}_{metric}_branch",
                    "l3_aggregation_id": aggregation_id,
                    "nci_regime": {
                        "geometry": frozen[0],
                        "m_over_n": frozen[1],
                        "summary_hash": configured_nci_hash,
                        "selector": metric,
                    },
                },
            })

    evaluations_output: dict[str, Any] = {}
    for evaluation_id, evaluation in config["evaluations"].items():
        mapping_path = require_file(config_path(config, evaluation["mapping_source"]), f"{evaluation_id} mapping")
        mapping_labels, mapping_identities, mapping_audit = mapping_roster(
            mapping_path, evaluation_id, evaluation
        )
        reference: dict[str, tuple[bool, float]] | None = None
        identities: dict[str, dict[str, str]] | None = None
        score_columns: dict[str, dict[str, float]] = {}
        source_artifacts: list[dict[str, str]] = []
        transfer_records: list[dict[str, Any]] = []
        for model_id in MODEL_IDS:
            summary, summary_path, predictions_path = source_summaries[(evaluation_id, model_id)]
            for source in [summary_path, predictions_path, *source_files_from_summary(summary)]:
                require_file(source, "transfer source artifact")
                source_artifacts.append({"path": str(source.resolve()), "sha256": sha256_file(source)})
            transfer_records.append({
                "model_id": model_id,
                "summary": str(summary_path.resolve()),
                "summary_sha256": sha256_file(summary_path),
                "predictions": str(predictions_path.resolve()),
                "predictions_sha256": sha256_file(predictions_path),
                "selected_regime": model_regimes[model_id],
                "component_model": summary.get("component_model"),
                "component_sources": summary.get("component_sources"),
                "embedded_transfer_label_contract": {
                    "response_column": summary.get("response_column"),
                    "label_column": summary.get("label_column"),
                    "label_threshold": summary.get("label_threshold"),
                    "label_source": summary.get("label_source"),
                },
            })
            for aggregation_id in aggregations:
                rows, row_identities = read_prediction_slice(
                    predictions_path, evaluation_id, evaluation, selector, aggregation_id
                )
                if reference is None:
                    reference = rows
                    identities = row_identities
                else:
                    if set(rows) != set(reference):
                        raise ProviderError(f"endpoint roster mismatch for {evaluation_id}/{model_id}/{aggregation_id}")
                    for eid, (label, _) in rows.items():
                        if reference[eid][0] != label:
                            raise ProviderError(f"label mismatch for {evaluation_id}/{model_id}/{aggregation_id}/{eid}")
                    if row_identities != identities:
                        raise ProviderError(f"endpoint identity mismatch for {evaluation_id}/{model_id}/{aggregation_id}")
                score_columns[f"score_{model_id}__{aggregation_id}"] = {eid: score for eid, (_, score) in rows.items()}
        assert reference is not None and identities is not None
        if set(reference) != set(mapping_labels):
            raise ProviderError(
                f"transfer/mapping endpoint roster mismatch for {evaluation_id}: "
                f"transfer={len(reference)} mapping={len(mapping_labels)}"
            )
        if identities != mapping_identities:
            raise ProviderError(f"transfer/mapping endpoint identities differ for {evaluation_id}")
        transfer_label_disagreements = sum(
            label != mapping_labels[eid] for eid, (label, _) in reference.items()
        )
        if evaluation.get("label_field") and transfer_label_disagreements:
            raise ProviderError(
                f"transfer/mapping label mismatch for committed-label evaluation {evaluation_id}"
            )
        positives = sum(mapping_labels.values())
        if positives == 0 or positives == len(mapping_labels):
            raise ProviderError(f"evaluation {evaluation_id} does not contain both label classes")
        evaluations_output[evaluation_id] = {
            "labels": mapping_labels,
            "identities": identities,
            "scores": score_columns,
            "provenance": {
                "schema_version": 2,
                "evaluation_id": evaluation_id,
                "analysis_unit_fields": evaluation["endpoint_fields"],
                "label_contract": {
                    "response_field": evaluation.get("response_field"),
                    "label_field": evaluation.get("label_field"),
                    "threshold": evaluation.get("threshold"),
                    "comparison_operator": evaluation.get("comparison_operator"),
                },
                "measurement_error_policy": evaluation.get("measurement_error_policy"),
                "accepted_measurement_error": config.get("accepted_measurement_error"),
                "covid_spike_label_specification": (
                    config["covid_spike_label_specification"]
                    if evaluation_id == "covid_spike"
                    else None
                ),
                "tournament_label_authority": "configured_mapping_source",
                "embedded_transfer_label_disagreements": transfer_label_disagreements,
                "embedded_transfer_labels_are_selection_inputs": False,
                "mapping_label_audit": mapping_audit,
                "transfer_records": transfer_records,
                "source_artifacts": sorted({(x["path"], x["sha256"]) for x in source_artifacts}),
                "upstream_validation": upstream,
            },
        }
    return {"schema_version": 2, "systems": systems}, evaluations_output, upstream


def tournament_spec(config: dict[str, Any], metric: str) -> dict[str, Any]:
    shared = config["policies"]["shared"]
    metric_policy = config["policies"][metric]
    evaluations = list(config["evaluations"])
    spec: dict[str, Any] = {
        "schema_version": 2,
        "metric": METRICS[metric],
        "verdict_field": "forward.staged_verdict" if metric == "pr" else "baseline.verdict",
        "evidence_policy": shared["evidence_policy"],
        "computational_design": shared["computational_design"],
        "reference_assessment": shared["reference_assessment"],
        "master_seed": shared["master_seed"],
        "seed_derivation_version": 1,
        "evaluations": evaluations,
        "conjunction_rule": "all_evaluations",
        "graph_maximality_rule": "source_strongly_connected_components",
        "selection_rule": "source_scc_maximal_vertices",
        "pr_cnap": metric_policy if metric == "pr" else None,
        "auroc": metric_policy if metric == "roc" else None,
        "annotations": {
            "provider_domain": "IRIS",
            "scientific_contract": {
                "l3_aggregation_ids": config["aggregations"],
                "covid_spike_label_specification": config["covid_spike_label_specification"],
                "evaluation_label_contracts": {
                    evaluation_id: {
                        "endpoint_fields": evaluation["endpoint_fields"],
                        "response_field": evaluation.get("response_field"),
                        "label_field": evaluation.get("label_field"),
                        "threshold": evaluation.get("threshold"),
                        "comparison_operator": evaluation.get("comparison_operator"),
                        "measurement_error_policy": evaluation.get("measurement_error_policy"),
                    }
                    for evaluation_id, evaluation in config["evaluations"].items()
                },
            },
        },
        "operational_tie_break": None,
    }
    return spec


def write_csv_new(path: Path, fieldnames: list[str], rows: Iterable[dict[str, Any]]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("x", newline="", encoding="utf-8") as stream:
        writer = csv.DictWriter(stream, fieldnames=fieldnames, lineterminator="\n")
        writer.writeheader()
        writer.writerows(rows)


def hashed(relative: Path, root: Path) -> dict[str, str]:
    return {"path": relative.as_posix(), "sha256": sha256_file(root / relative)}


def organizer_command(config: dict[str, Any], *args: str) -> str:
    binary = require_file(config_path(config, config["organizer_binary"]), "organizer binary")
    completed = subprocess.run([str(binary), *args], text=True, capture_output=True, check=False)
    if completed.returncode != 0:
        raise ProviderError(f"organizer command failed: {completed.stderr.strip()}")
    return completed.stdout.strip()


def build_bundle(config: dict[str, Any], metric: str, output: Path) -> dict[str, Any]:
    output = output.resolve()
    if output.exists():
        raise ProviderError(f"output already exists: {output}")
    output.parent.mkdir(parents=True, exist_ok=True)
    staging = Path(tempfile.mkdtemp(prefix=f".{output.name}.tmp-", dir=output.parent))
    try:
        registry, evaluations, _ = assemble(config, metric)
        spec = tournament_spec(config, metric)
        write_json(staging / "systems.json", registry)
        write_json(staging / "tournament_spec.json", spec)
        manifest_evaluations = []
        score_columns = [system["score_column"] for system in sorted(registry["systems"], key=lambda x: x["system_id"])]
        for evaluation_id, value in evaluations.items():
            directory = staging / "evaluations" / evaluation_id
            ids = sorted(value["labels"])
            write_csv_new(
                directory / "endpoints.csv", ["endpoint_id", "label"],
                ({"endpoint_id": eid, "label": int(value["labels"][eid])} for eid in ids),
            )
            write_csv_new(
                directory / "scores.csv", ["endpoint_id", *score_columns],
                ({"endpoint_id": eid, **{column: repr(value["scores"][column][eid]) for column in score_columns}} for eid in ids),
            )
            identity_relative = Path("evaluations") / evaluation_id / "endpoint_identities.csv"
            identity_fields = config["evaluations"][evaluation_id]["endpoint_fields"]
            write_csv_new(
                staging / identity_relative, ["endpoint_id", *identity_fields],
                ({"endpoint_id": eid, **value["identities"][eid]} for eid in ids),
            )
            provenance = value["provenance"]
            provenance["endpoint_identity_table"] = {
                "path": "endpoint_identities.csv", "sha256": sha256_file(staging / identity_relative)
            }
            provenance["source_artifacts"] = [
                {"path": path, "sha256": digest} for path, digest in provenance["source_artifacts"]
            ]
            write_json(directory / "source_provenance.json", provenance)
            base = Path("evaluations") / evaluation_id
            manifest_evaluations.append({
                "evaluation_id": evaluation_id,
                "endpoints": hashed(base / "endpoints.csv", staging),
                "scores": hashed(base / "scores.csv", staging),
                "source_provenance": hashed(base / "source_provenance.json", staging),
            })
        manifest = {
            "schema_name": BUNDLE_SCHEMA_NAME,
            "schema_version": BUNDLE_SCHEMA_VERSION,
            "metric": METRICS[metric],
            "bundle_creation_time": dt.datetime.now(dt.timezone.utc).isoformat().replace("+00:00", "Z"),
            "systems": hashed(Path("systems.json"), staging),
            "tournament_spec": hashed(Path("tournament_spec.json"), staging),
            "evaluations": manifest_evaluations,
            "score_provider_identity": config["provider_identity"],
            "minimum_organizer_schema_version": BUNDLE_SCHEMA_VERSION,
            "bundle_content_hash": "",
        }
        write_json(staging / "bundle_manifest.json", manifest)
        content_hash = organizer_command(config, "bundle-content-hash", "--bundle", str(staging))
        if len(content_hash) != 64 or any(c not in "0123456789abcdef" for c in content_hash):
            raise ProviderError(f"organizer returned invalid content hash: {content_hash!r}")
        (staging / "bundle_manifest.json").unlink()
        manifest["bundle_content_hash"] = content_hash
        write_json(staging / "bundle_manifest.json", manifest)
        organizer_command(config, "validate-bundle", "--bundle", str(staging))
        staging.rename(output)
        return {"bundle": str(output), "metric": METRICS[metric], "bundle_content_hash": content_hash,
                "evaluations": len(evaluations), "systems": len(registry["systems"])}
    except Exception:
        shutil.rmtree(staging, ignore_errors=True)
        raise


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)
    for command in ("validate", "build"):
        sub = subparsers.add_parser(command)
        sub.add_argument("--config", type=Path, required=True)
        sub.add_argument("--metric", choices=sorted(METRICS), required=True)
        if command == "build":
            sub.add_argument("--output", type=Path, required=True)
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    try:
        args = parse_args(argv)
        config = read_config(args.config)
        if args.command == "validate":
            registry, evaluations, upstream = assemble(config, args.metric)
            result = {"status": "valid", "metric": METRICS[args.metric],
                      "systems": len(registry["systems"]), "evaluations": len(evaluations),
                      "upstream_files": len(upstream["validated_files"])}
        else:
            result = build_bundle(config, args.metric, args.output)
        json.dump(result, sys.stdout, indent=2, sort_keys=True)
        sys.stdout.write("\n")
        return 0
    except ProviderError as error:
        print(f"error: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
