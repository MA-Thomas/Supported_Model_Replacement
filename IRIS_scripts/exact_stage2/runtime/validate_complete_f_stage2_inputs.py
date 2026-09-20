#!/usr/bin/env python3
"""Validate a generated complete-F TOML/query pair against its bundle manifest."""

from __future__ import annotations

import argparse
import csv
import hashlib
import json
import os
import re
import shutil
import sys
import tempfile
from pathlib import Path


class ContractError(RuntimeError):
    pass


DATASET_DESTINATIONS = {
    "COVID_NONSPIKE": "Cansu_Covid_Nonspike",
    "COVID_SPIKE": "Cansu_Covid_Spike",
    "PDAC": "Jayon",
}


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def normalize_hla(value: str) -> str:
    value = value.strip().upper()
    if value.startswith("HLA-"):
        value = value[4:]
    return value.replace("*", "").replace(":", "")


def toml_string(path: Path, key: str) -> str:
    pattern = re.compile(rf'^\s*{re.escape(key)}\s*=\s*"([^"]+)"')
    for line in path.read_text(encoding="utf-8").splitlines():
        match = pattern.match(line)
        if match:
            return match.group(1)
    raise ContractError(f"{path} lacks TOML string {key}")


def load_manifest(bundle: Path) -> tuple[Path, dict[str, object]]:
    manifest_path = bundle / "manifest.json"
    try:
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise ContractError(f"cannot read bundle manifest {manifest_path}: {exc}") from exc
    if not isinstance(manifest, dict):
        raise ContractError(f"bundle manifest is not an object: {manifest_path}")
    return manifest_path, manifest


def dataset_audit(manifest: dict[str, object], dataset: str) -> dict[str, object]:
    audits = manifest.get("datasets")
    if not isinstance(audits, dict) or dataset not in audits:
        raise ContractError(f"bundle manifest lacks dataset {dataset}")
    audit = audits[dataset]
    if not isinstance(audit, dict):
        raise ContractError(f"bundle manifest dataset {dataset} is not an object")
    if audit.get("floor_candidate_rows") != 0 or audit.get("input_audit_status") != "pass":
        raise ContractError(f"dataset {dataset} is not a validated complete-F roster")
    return audit


def verify_manifest_output(
    bundle: Path, manifest: dict[str, object], name: str
) -> Path:
    path = bundle / name
    registry = manifest.get("output_files", {})
    record = registry.get(name) if isinstance(registry, dict) else None
    if not path.is_file() or not isinstance(record, dict):
        raise ContractError(f"manifest output is missing: {path}")
    if path.stat().st_size != record.get("bytes") or sha256_file(path) != record.get(
        "sha256"
    ):
        raise ContractError(f"manifest hash/size mismatch: {path}")
    return path


def validate(bundle: Path, dataset: str, representation: str) -> dict[str, object]:
    bundle = bundle.expanduser().resolve()
    _, manifest = load_manifest(bundle)
    audit = dataset_audit(manifest, dataset)
    if representation == "full":
        query_key = "full_deduplicated_query"
        config_key = "full_deduplicated_config"
        expected_rows = audit.get("full_query_rows")
    else:
        query_key = "mono_query"
        config_key = "mono_config"
        expected_rows = audit.get("mono_query_rows")
    outputs = audit.get("output_files", {})
    query_name = outputs.get(query_key)
    config_name = outputs.get(config_key)
    if not isinstance(query_name, str) or not isinstance(config_name, str):
        raise ContractError(f"dataset {dataset} lacks {representation} query/config outputs")
    query = bundle / query_name
    config = bundle / config_name
    for name in (query_name, config_name):
        verify_manifest_output(bundle, manifest, name)

    configured_query = toml_string(config, "query_peptide_input_tuples_file")
    expanded = os.path.expandvars(
        configured_query.replace("${EXTERNAL_VALIDATION_INPUT_ROOT}", str(bundle))
    )
    resolved_query = Path(expanded)
    if not resolved_query.is_absolute():
        resolved_query = bundle / resolved_query
    if resolved_query.resolve() != query:
        raise ContractError(
            f"generated TOML resolves query to {resolved_query}, expected manifest query {query}"
        )

    keys: set[tuple[str, str, int, str]] = set()
    lengths: dict[int, int] = {}
    active_envs: set[int] = set()
    hlas: set[str] = set()
    with query.open(newline="", encoding="utf-8-sig") as handle:
        reader = csv.DictReader(handle)
        required = {"peptide", "HLA-RE", "env_id", "TCGA_EXPR_TYPE"}
        if reader.fieldnames is None or not required.issubset(reader.fieldnames):
            raise ContractError(f"query lacks required Stage-2 columns: {query}")
        for row_number, row in enumerate(reader, start=2):
            peptide = row["peptide"].strip().upper()
            hla = normalize_hla(row["HLA-RE"])
            expression = row["TCGA_EXPR_TYPE"].strip()
            try:
                env_id = int(row["env_id"])
            except ValueError as exc:
                raise ContractError(f"invalid env_id in {query} row {row_number}") from exc
            if not peptide or not hla or not expression or not 9 <= len(peptide) <= 12:
                raise ContractError(f"invalid complete-F compute key in {query} row {row_number}")
            key = (peptide, hla, env_id, expression)
            if key in keys:
                raise ContractError(f"duplicate Stage-2 compute key in {query} row {row_number}: {key}")
            keys.add(key)
            lengths[len(peptide)] = lengths.get(len(peptide), 0) + 1
            active_envs.add(env_id)
            hlas.add(hla)
    if len(keys) != expected_rows:
        raise ContractError(
            f"query row count {len(keys)} differs from manifest count {expected_rows}"
        )
    return {
        "status": "pass",
        "dataset": dataset,
        "representation": representation,
        "query": str(query),
        "query_sha256": sha256_file(query),
        "query_rows": len(keys),
        "unique_compute_keys": len(keys),
        "active_environment_ids": sorted(active_envs),
        "distinct_hla": len(hlas),
        "peptide_length_counts": {str(key): lengths[key] for key in sorted(lengths)},
        "manifest_exact_match": True,
        "config": str(config),
        "config_sha256": sha256_file(config),
    }


def collect_dataset_outputs(audit: dict[str, object]) -> list[str]:
    names: set[str] = set()

    def collect(value: object) -> None:
        if not isinstance(value, dict):
            return
        outputs = value.get("output_files")
        if isinstance(outputs, dict):
            for name in outputs.values():
                if isinstance(name, str):
                    names.add(name)
        views = value.get("views")
        if isinstance(views, dict):
            for view in views.values():
                collect(view)

    collect(audit)
    if not names:
        raise ContractError("dataset audit declares no output files")
    return sorted(names)


def install(
    bundle: Path,
    dataset: str,
    *,
    work_data_root: Path,
    destination: Path | None = None,
    archive_existing: bool = False,
) -> dict[str, object]:
    bundle = bundle.expanduser().resolve()
    manifest_path, manifest = load_manifest(bundle)
    audit = dataset_audit(manifest, dataset)
    validation = {
        representation: validate(bundle, dataset, representation)
        for representation in ("full", "mono")
    }

    if destination is None:
        relative = DATASET_DESTINATIONS.get(dataset)
        if relative is None:
            raise ContractError(
                f"no canonical cohort destination for {dataset}; pass --destination"
            )
        destination = work_data_root.expanduser().resolve() / relative
    else:
        destination = destination.expanduser().resolve()
    destination.mkdir(parents=True, exist_ok=True)

    names = collect_dataset_outputs(audit)
    sources = [verify_manifest_output(bundle, manifest, name) for name in names]
    install_manifest_name = f"{dataset.lower()}_complete_f_inputs_manifest.json"
    targets = [destination / source.name for source in sources]
    targets.append(destination / install_manifest_name)
    existing = [path for path in targets if path.exists()]
    if existing and not archive_existing:
        preview = ", ".join(str(path) for path in existing[:5])
        raise ContractError(
            "refusing to overwrite installed complete-F inputs; first existing "
            f"targets: {preview}"
        )
    archives = {
        path: path.with_name(f"{path.stem}_deprecated{path.suffix}")
        for path in existing
    }
    archive_conflicts = [path for path in archives.values() if path.exists()]
    if archive_conflicts:
        preview = ", ".join(str(path) for path in archive_conflicts[:5])
        raise ContractError(
            "refusing to replace existing deprecation archive; first conflicts: "
            f"{preview}"
        )

    install_manifest = {
        "schema_version": 1,
        "dataset": dataset,
        "destination": str(destination),
        "source_bundle": str(bundle),
        "source_bundle_manifest": str(manifest_path),
        "source_bundle_manifest_sha256": sha256_file(manifest_path),
        "validation": validation,
        "files": {
            source.name: {
                "bytes": source.stat().st_size,
                "sha256": sha256_file(source),
            }
            for source in sources
        },
    }

    staging = Path(tempfile.mkdtemp(prefix=".complete_f_install.", dir=destination))
    installed: list[Path] = []
    archived: list[tuple[Path, Path]] = []
    try:
        for source in sources:
            staged = staging / source.name
            shutil.copyfile(source, staged)
            if sha256_file(staged) != sha256_file(source):
                raise ContractError(f"copy verification failed for {source}")
        (staging / install_manifest_name).write_text(
            json.dumps(install_manifest, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
        for current, archive in archives.items():
            os.replace(current, archive)
            archived.append((current, archive))
        for staged in sorted(staging.iterdir()):
            target = destination / staged.name
            os.replace(staged, target)
            installed.append(target)
    except Exception:
        for path in installed:
            path.unlink(missing_ok=True)
        for current, archive in reversed(archived):
            if archive.exists():
                os.replace(archive, current)
        raise
    finally:
        shutil.rmtree(staging, ignore_errors=True)

    return {
        "status": "installed",
        "dataset": dataset,
        "destination": str(destination),
        "installed_files": [str(path) for path in installed],
        "archived_files": [str(archive) for _, archive in archived],
        "install_manifest": str(destination / install_manifest_name),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)
    validate_parser = subparsers.add_parser("validate")
    validate_parser.add_argument("--bundle", type=Path, required=True)
    validate_parser.add_argument("--dataset", required=True)
    validate_parser.add_argument(
        "--representation", choices=("full", "mono"), required=True
    )
    install_parser = subparsers.add_parser("install")
    install_parser.add_argument("--bundle", type=Path, required=True)
    install_parser.add_argument("--dataset", required=True)
    install_parser.add_argument(
        "--work-data-root", type=Path, default=Path("/Users/thomm15/Work_Data")
    )
    install_parser.add_argument("--destination", type=Path)
    install_parser.add_argument(
        "--archive-existing",
        action="store_true",
        help="rename one existing installation with _deprecated before installing",
    )
    args = parser.parse_args()
    try:
        if args.command == "validate":
            report = validate(args.bundle, args.dataset, args.representation)
        else:
            report = install(
                args.bundle,
                args.dataset,
                work_data_root=args.work_data_root,
                destination=args.destination,
                archive_existing=args.archive_existing,
            )
    except ContractError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 2
    print(json.dumps(report, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
