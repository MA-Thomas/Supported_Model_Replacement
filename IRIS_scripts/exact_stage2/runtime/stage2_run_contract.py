#!/usr/bin/env python3
"""Stage 2 run provenance, task validation, and atomic publication.

This helper is intentionally standard-library-only so it can run on cluster
nodes without an additional Python environment.  It provides one contract to
both submit_stage2_new.sh and run_stage_2_new.sh:

* resolve the authoritative query/HLA inputs from the Q-model TOML;
* fingerprint all numerically relevant inputs into immutable run manifests;
* exclude environment chunks with no query rows before Slurm submission;
* validate Q/Pi or PN task output against the exact filtered query roster;
* publish validated task files without clobbering conflicting existing files;
* write machine-readable completion manifests only after validation succeeds.
"""

from __future__ import annotations

import argparse
import csv
import hashlib
import json
import math
import os
import re
import sys
import tempfile
from collections import Counter, defaultdict
from dataclasses import dataclass
from decimal import Decimal, InvalidOperation
from pathlib import Path
from typing import Dict, Iterable, List, Mapping, MutableMapping, Optional, Sequence, Set, Tuple


SCHEMA_VERSION = 1
Q_FILENAME_RE = re.compile(r"^query_peptides_results_HLA_(.+)_q_values\.csv$")
PI_FILENAME_RE = re.compile(r"^query_peptides_results_HLA_(.+)_pi_values\.csv$")
PN_FILENAME_RE = re.compile(
    r"^query_peptides_results_dpos_(.+?)_dneg_(.+?)_steepness_pos_(.+?)"
    r"_steepness_neg_(.+?)_fft_size_(.+?)_tau_thymus_(.+?)"
    r"_HLA_(.+?)_expr_(.+)\.csv$"
)


class ContractError(RuntimeError):
    """A Stage 2 provenance or output contract violation."""


def normalize_hla(value: str) -> str:
    normalized = value.strip()
    if normalized.startswith("HLA-"):
        normalized = normalized[4:]
    return normalized.replace("*", "").replace(":", "").upper()


def canonical_decimal(value: str) -> str:
    try:
        number = Decimal(value.strip())
    except InvalidOperation as exc:
        raise ContractError(f"Invalid numeric value {value!r}") from exc
    if not number.is_finite():
        raise ContractError(f"Non-finite numeric value {value!r}")
    normalized = number.normalize()
    if normalized == 0:
        normalized = Decimal(0)
    return format(normalized, "f")


def parse_finite(value: str, field: str, path: Path, row_number: int) -> float:
    try:
        parsed = float(value)
    except ValueError as exc:
        raise ContractError(
            f"Invalid {field}={value!r} in {path} row {row_number}"
        ) from exc
    if not math.isfinite(parsed):
        raise ContractError(
            f"Non-finite {field}={value!r} in {path} row {row_number}"
        )
    return parsed


def atomic_json(path: Path, payload: Mapping[str, object]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, tmp_name = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as handle:
            json.dump(payload, handle, indent=2, sort_keys=True)
            handle.write("\n")
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(tmp_name, path)
    except Exception:
        try:
            os.unlink(tmp_name)
        except FileNotFoundError:
            pass
        raise


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def digest_path(path: Path) -> Mapping[str, object]:
    path = path.expanduser().resolve()
    if not path.exists():
        raise ContractError(f"Fingerprint input does not exist: {path}")
    if path.is_file():
        return {
            "path": str(path),
            "kind": "file",
            "size": path.stat().st_size,
            "sha256": sha256_file(path),
        }
    if not path.is_dir():
        raise ContractError(f"Fingerprint input is not a file or directory: {path}")

    digest = hashlib.sha256()
    n_files = 0
    total_bytes = 0
    for child in sorted(p for p in path.rglob("*") if p.is_file()):
        relative = child.relative_to(path).as_posix()
        child_hash = sha256_file(child)
        size = child.stat().st_size
        digest.update(relative.encode("utf-8"))
        digest.update(b"\0")
        digest.update(str(size).encode("ascii"))
        digest.update(b"\0")
        digest.update(child_hash.encode("ascii"))
        digest.update(b"\n")
        n_files += 1
        total_bytes += size
    return {
        "path": str(path),
        "kind": "directory",
        "files": n_files,
        "size": total_bytes,
        "sha256": digest.hexdigest(),
    }


def canonical_hash(payload: Mapping[str, object]) -> str:
    encoded = json.dumps(payload, sort_keys=True, separators=(",", ":")).encode("utf-8")
    return hashlib.sha256(encoded).hexdigest()


def parse_simple_toml_strings(path: Path) -> Dict[str, str]:
    """Read the top-level string assignments used by the Runner configs.

    The active model files use simple ``key = "value"`` assignments.  Avoiding
    tomllib keeps this helper compatible with cluster Python versions < 3.11.
    """

    assignments: Dict[str, str] = {}
    pattern = re.compile(r'^\s*([A-Za-z0-9_]+)\s*=\s*"((?:[^"\\]|\\.)*)"')
    with path.open(encoding="utf-8") as handle:
        for line in handle:
            match = pattern.match(line)
            if match:
                assignments[match.group(1)] = bytes(
                    match.group(2), "utf-8"
                ).decode("unicode_escape")
    return assignments


def resolve_config_value(value: str, runner_root: Path) -> Path:
    expanded = Path(os.path.expandvars(os.path.expanduser(value)))
    return expanded if expanded.is_absolute() else runner_root / expanded


def config_inputs(config: Path, runner_root: Path) -> Tuple[Path, Path, List[Path]]:
    values = parse_simple_toml_strings(config)
    try:
        query = resolve_config_value(values["query_peptide_input_tuples_file"], runner_root)
        env_dict = resolve_config_value(values["hla_env_dict"], runner_root)
    except KeyError as exc:
        raise ContractError(f"Missing required TOML key {exc.args[0]!r} in {config}") from exc

    referenced: List[Path] = []
    for key, value in values.items():
        lower = key.lower()
        if lower == "output_dir":
            continue
        if lower.endswith(("_file", "_csv", "_dir")):
            referenced.append(resolve_config_value(value, runner_root))
    return query.resolve(), env_dict.resolve(), referenced


def find_header(headers: Sequence[str], choices: Sequence[str], path: Path) -> str:
    stripped = {header.strip(): header for header in headers}
    for choice in choices:
        if choice in stripped:
            return stripped[choice]
    raise ContractError(f"{path} is missing required column; expected one of {choices}")


@dataclass(frozen=True)
class QueryRow:
    peptide: str
    hla: str
    env_id: int
    expression: str


@dataclass(frozen=True)
class PnShardTask:
    task_id: int
    parent_task_id: int
    parameter_chunk: int
    parameter_start: int
    parameter_stop: int
    env_chunk: int
    env_start: int
    env_stop: int
    shard_index: int
    shard_count: int
    expected_query_rows: int


@dataclass(frozen=True)
class EnvironmentSummary:
    representation: str
    environments: int
    min_alleles: int
    max_alleles: int


def load_query(path: Path) -> List[QueryRow]:
    rows: List[QueryRow] = []
    with path.open(newline="", encoding="utf-8-sig") as handle:
        reader = csv.DictReader(handle)
        if reader.fieldnames is None:
            raise ContractError(f"Query CSV has no header: {path}")
        peptide_col = find_header(reader.fieldnames, ("peptide", "nmer"), path)
        hla_col = find_header(reader.fieldnames, ("HLA-RE", "hla", "HLA"), path)
        env_col = find_header(reader.fieldnames, ("env_id",), path)
        expression_col = find_header(
            reader.fieldnames, ("TCGA_EXPR_TYPE", "expression_dataset", "cancer_type"), path
        )
        for row_number, record in enumerate(reader, start=2):
            peptide = (record.get(peptide_col) or "").strip().upper()
            hla = normalize_hla(record.get(hla_col) or "")
            expression = (record.get(expression_col) or "").strip()
            env_raw = (record.get(env_col) or "").strip()
            if not peptide or not hla or not expression or not env_raw:
                raise ContractError(f"Missing query key field in {path} row {row_number}")
            try:
                env_id = int(env_raw)
            except ValueError as exc:
                raise ContractError(
                    f"Invalid env_id={env_raw!r} in {path} row {row_number}"
                ) from exc
            rows.append(QueryRow(peptide, hla, env_id, expression))
    return rows


def assert_unique_compute_keys(query_rows: Sequence[QueryRow], path: Path) -> None:
    """Fail if the query roster contains duplicate computational identities.

    The Stage-2 compute identity is (peptide, HLA-RE, env_id, expression) — the
    same key `load_query` already normalizes on.  Duplicate rows on this key mean
    the identical P·N·Q·pi calculation would be enumerated more than once (e.g.
    homozygous allele copies or repeated parent peptides leaking from the
    environment/provenance into the query enumeration).  The roster must be a set
    over this key; provenance belongs in a separate sidecar.  Enforced here so a
    bad roster fails before any Slurm job is submitted.
    """
    counts: Counter = Counter(
        (r.peptide, r.hla, r.env_id, r.expression) for r in query_rows
    )
    dups = [(key, c) for key, c in counts.items() if c > 1]
    if dups:
        dups.sort(key=lambda kc: kc[1], reverse=True)
        excess = sum(c - 1 for _, c in dups)
        preview = ", ".join(
            f"{pep}/{hla}/env{env}/{expr}×{c}"
            for (pep, hla, env, expr), c in dups[:10]
        )
        raise ContractError(
            f"Query roster {path} has {len(dups)} duplicate compute keys "
            f"({excess} excess rows). The roster must be unique on "
            f"(peptide, HLA-RE, env_id, expression). First offenders: {preview}"
        )


def load_environment_definitions(path: Path) -> Dict[int, Tuple[str, ...]]:
    definitions: Dict[int, Tuple[str, ...]] = {}
    with path.open(newline="", encoding="utf-8-sig") as handle:
        reader = csv.DictReader(handle)
        if reader.fieldnames is None:
            raise ContractError(f"HLA environment dictionary has no header: {path}")
        env_col = find_header(reader.fieldnames, ("env_id",), path)
        alleles_col = find_header(
            reader.fieldnames,
            ("allele_environment", "alleles", "hla_environment"),
            path,
        )
        for row_number, record in enumerate(reader, start=2):
            try:
                env_id = int((record.get(env_col) or "").strip())
            except ValueError as exc:
                raise ContractError(
                    f"Invalid env_id in {path} row {row_number}"
                ) from exc
            if env_id in definitions:
                raise ContractError(f"Duplicate env_id {env_id} in {path}")
            raw_alleles = (record.get(alleles_col) or "").strip()
            alleles = tuple(
                normalize_hla(value)
                for value in raw_alleles.split(",")
                if value.strip()
            )
            if not alleles or any(not allele for allele in alleles):
                raise ContractError(
                    f"Empty HLA environment in {path} row {row_number}"
                )
            definitions[env_id] = alleles
    if not definitions:
        raise ContractError(f"HLA environment dictionary is empty: {path}")
    return definitions


def validate_environment_representation(
    query_rows: Sequence[QueryRow],
    env_dict_path: Path,
    representation: str,
    expected_env_count: int,
) -> EnvironmentSummary:
    if representation not in {"full", "mono"}:
        raise ContractError(
            f"Unsupported HLA environment representation {representation!r}"
        )
    definitions = load_environment_definitions(env_dict_path)
    expected_ids = set(range(expected_env_count))
    actual_ids = set(definitions)
    if actual_ids != expected_ids:
        missing = sorted(expected_ids - actual_ids)
        extra = sorted(actual_ids - expected_ids)
        raise ContractError(
            f"Environment IDs in {env_dict_path} must be contiguous over "
            f"[0, {expected_env_count}); missing={missing[:20]} extra={extra[:20]}"
        )

    allele_counts = [len(alleles) for alleles in definitions.values()]
    if representation == "mono":
        non_mono = sorted(
            (env_id, alleles)
            for env_id, alleles in definitions.items()
            if len(alleles) != 1
        )
        if non_mono:
            raise ContractError(
                "Mono representation requires exactly one HLA allele per environment; "
                f"first offenders={non_mono[:10]}"
            )

    mismatches = []
    for row in query_rows:
        alleles = definitions.get(row.env_id)
        if alleles is None:
            mismatches.append((row.env_id, row.hla, "missing environment"))
        elif row.hla not in alleles:
            mismatches.append((row.env_id, row.hla, alleles))
    if mismatches:
        raise ContractError(
            "Query HLA must be present in its representation-specific environment; "
            f"first mismatches={mismatches[:10]}"
        )

    return EnvironmentSummary(
        representation=representation,
        environments=len(definitions),
        min_alleles=min(allele_counts),
        max_alleles=max(allele_counts),
    )


def env_range_for_chunk(
    chunk_id: int, total_env_ids: int, env_chunks: int
) -> Tuple[int, int]:
    per_chunk = total_env_ids // env_chunks
    start = chunk_id * per_chunk
    stop = start + per_chunk
    if chunk_id == env_chunks - 1:
        stop = total_env_ids
    return start, stop


def active_chunks(query: Sequence[QueryRow], total_env_ids: int, env_chunks: int) -> List[int]:
    env_ids = {row.env_id for row in query}
    invalid = sorted(env_id for env_id in env_ids if env_id < 0 or env_id >= total_env_ids)
    if invalid:
        raise ContractError(
            f"Query contains env_ids outside [0, {total_env_ids}): {invalid[:20]}"
        )
    active: List[int] = []
    for chunk_id in range(env_chunks):
        start, stop = env_range_for_chunk(chunk_id, total_env_ids, env_chunks)
        if any(start <= env_id < stop for env_id in env_ids):
            active.append(chunk_id)
    return active


def unique_digest_entries(entries: Iterable[Tuple[str, Path]]) -> List[Mapping[str, object]]:
    seen: Set[Path] = set()
    result: List[Mapping[str, object]] = []
    for role, path in entries:
        resolved = path.expanduser().resolve()
        if resolved in seen:
            continue
        seen.add(resolved)
        item = dict(digest_path(resolved))
        item["role"] = role
        result.append(item)
    return sorted(result, key=lambda item: (str(item["role"]), str(item["path"])))


def common_contract_without_operational_tools(
    payload: Mapping[str, object],
) -> Mapping[str, object]:
    """Compare numerical provenance while permitting retry-tool updates."""
    comparable = {
        key: value
        for key, value in payload.items()
        if key not in {"run_id", "common_fingerprint"}
    }
    inputs = comparable.get("inputs")
    if isinstance(inputs, list):
        comparable["inputs"] = [
            item
            for item in inputs
            if not isinstance(item, dict)
            or item.get("role") not in {"submit_script", "contract_helper"}
        ]
    return comparable


def count_csv_rows(path: Path) -> int:
    with path.open(newline="", encoding="utf-8-sig") as handle:
        reader = csv.reader(handle)
        next(reader, None)
        return sum(1 for _ in reader)


def command_prepare_run(args: argparse.Namespace) -> int:
    if not 0 <= args.max_num_ps_values_log2 < 64:
        raise ContractError(
            "max-num-ps-values-log2 must be in [0, 64); "
            f"got {args.max_num_ps_values_log2}"
        )

    run_root = Path(args.run_root).resolve()
    runner_root = Path(args.runner_root).resolve()
    config = Path(args.q_model_config).resolve()
    query, env_dict, referenced = config_inputs(config, runner_root)
    query_rows = load_query(query)
    assert_unique_compute_keys(query_rows, query)
    environment_summary = validate_environment_representation(
        query_rows,
        env_dict,
        args.hla_environment_representation,
        args.total_env_ids,
    )
    active = active_chunks(query_rows, args.total_env_ids, args.env_chunks)

    common_entries: List[Tuple[str, Path]] = [
        ("runner_binary", Path(args.runner)),
        ("q_model_config", config),
        ("query_peptides", query),
        ("hla_env_dict", env_dict),
        ("submit_script", Path(args.submit_script)),
        ("run_script", Path(args.run_script)),
        ("contract_helper", Path(args.contract_helper)),
    ]
    regime_manifest = getattr(args, "regime_manifest", "")
    exact_grid = None
    if regime_manifest:
        import survivor_grid
        grid_path = Path(regime_manifest).resolve()
        exact_grid, grid_params, grid_union = survivor_grid.load(
            grid_path, args.hla_environment_representation, args.pn_hla_scope)
        if (grid_params != Path(args.param_file).resolve() or grid_union != Path(args.mn_tuples).resolve()
                or args.total_params != len(exact_grid["geometries"]) * 6
                or args.param_chunks != len(exact_grid["geometries"])):
            raise ContractError("Submission geometry/files disagree with the exact survivor grid")
        common_entries.extend([("survivor_grid", grid_path),
            ("survivor_grid_helper", Path(survivor_grid.__file__).resolve())])
        common_entries.extend(("survivor_grid_file", survivor_grid.checked_file(grid_path.parent, rec))
            for rec in [exact_grid["parameter_file"], exact_grid["mn_union_file"]]
            + [g["mn_file"] for g in exact_grid["geometries"]])
    common_entries.extend(("q_config_reference", path) for path in referenced)
    representation_crosswalk = getattr(args, "representation_crosswalk", "")
    if args.hla_environment_representation == "mono":
        if representation_crosswalk:
            common_entries.append(
                ("representation_crosswalk", Path(representation_crosswalk))
            )
    elif representation_crosswalk:
        raise ContractError(
            "--representation-crosswalk is valid only for mono representation"
        )
    common_payload: MutableMapping[str, object] = {
        "schema_version": SCHEMA_VERSION,
        "dataset": args.dataset,
        "runner_root": str(runner_root),
        "total_env_ids": args.total_env_ids,
        "env_chunks": args.env_chunks,
        "hla_environment_representation": args.hla_environment_representation,
        "environment_allele_count_min": environment_summary.min_alleles,
        "environment_allele_count_max": environment_summary.max_alleles,
        "query_input_file": str(query),
        "query_rows": len(query_rows),
        "active_env_ids": sorted({row.env_id for row in query_rows}),
        "active_env_chunks": active,
        "zero_input_env_chunks": [i for i in range(args.env_chunks) if i not in active],
        "inputs": unique_digest_entries(common_entries),
    }
    common_fingerprint = canonical_hash(common_payload)
    common_payload["run_id"] = args.run_id
    common_payload["common_fingerprint"] = common_fingerprint

    run_root.mkdir(parents=True, exist_ok=True)
    common_path = run_root / "run_manifest.json"
    if common_path.exists():
        existing = json.loads(common_path.read_text(encoding="utf-8"))
        if existing.get("common_fingerprint") != common_fingerprint:
            operational_only = (
                args.allow_operational_provenance_change
                and common_contract_without_operational_tools(existing)
                == common_contract_without_operational_tools(common_payload)
            )
            if not operational_only:
                raise ContractError(
                    f"Run ID {args.run_id!r} already exists with different common inputs: "
                    f"{common_path}"
                )
            common_fingerprint = str(existing["common_fingerprint"])
            common_payload["common_fingerprint"] = common_fingerprint
            print(
                "Reusing existing common fingerprint; only the submission launcher "
                "and/or contract helper changed.",
                file=sys.stderr,
            )
    else:
        atomic_json(common_path, common_payload)

    mode_inputs: List[Tuple[str, Path]] = []
    if exact_grid is not None:
        mode_inputs.append(("survivor_grid", grid_path))
        mode_inputs.extend(("survivor_geometry_mn", survivor_grid.checked_file(grid_path.parent, g["mn_file"]))
                           for g in exact_grid["geometries"])
    if args.mode == "pn":
        param_file = Path(args.param_file).resolve()
        mn_tuples = Path(args.mn_tuples).resolve()
        matrix_dir = runner_root / "data" / "matrices"
        actual_params = count_csv_rows(param_file)
        if actual_params != args.total_params:
            raise ContractError(
                f"TOTAL_PARAMS={args.total_params}, but {param_file} contains "
                f"{actual_params} data rows"
            )
        mode_inputs.extend((
            ("parameter_sets", param_file),
            ("mn_tuples", mn_tuples),
            ("distance_matrices", matrix_dir),
        ))
        shard_target = int(getattr(args, "pn_peptides_per_shard", 0))
        shard_plan = str(getattr(args, "pn_shard_plan", ""))
        if shard_target:
            if args.dataset not in {"PDAC", "COVID_SPIKE", "COVID_NONSPIKE"}:
                raise ContractError(
                    "P/N peptide sharding is restricted to PDAC and the two COVID datasets"
                )
            if not shard_plan:
                raise ContractError("Sharded P/N requires --pn-shard-plan")
            plan_payload, _ = load_pn_shard_plan(Path(shard_plan).resolve())
            if int(plan_payload["target_rows"]) != shard_target:
                raise ContractError("Shard-plan target does not match submission target")
            mode_inputs.append(("pn_shard_plan", Path(shard_plan).resolve()))
            finalize_script = str(getattr(args, "finalize_script", ""))
            if not finalize_script:
                raise ContractError("Sharded P/N requires --finalize-script")
            mode_inputs.append(("pn_shard_finalizer", Path(finalize_script).resolve()))
    elif args.mode == "qpi":
        mode_inputs.extend((
            ("pmhc_config", Path(args.pmhc_config).resolve()),
            ("pmhc_parameters", Path(args.pmhc_parameters_file).resolve()),
        ))

    mode_payload: MutableMapping[str, object] = {
        "schema_version": SCHEMA_VERSION,
        "dataset": args.dataset,
        "mode": args.mode,
        "common_fingerprint": common_fingerprint,
        "query_input_file": str(query),
        "active_env_chunks": active,
        "total_params": args.total_params,
        "param_chunks": args.param_chunks,
        "compute_in_vitro": bool(args.in_vitro),
        "in_vitro_peptide_conc": args.peptide_conc,
        "inputs": unique_digest_entries(mode_inputs),
    }
    if args.mode == "pn":
        mode_payload["pn_hla_scope"] = args.pn_hla_scope
        mode_payload["max_num_ps_values_log2"] = args.max_num_ps_values_log2
        mode_payload["max_num_ps_values"] = 1 << args.max_num_ps_values_log2
        mode_payload["pn_peptides_per_shard"] = int(
            getattr(args, "pn_peptides_per_shard", 0)
        )
    mode_fingerprint = canonical_hash(mode_payload)
    mode_payload["run_id"] = args.run_id
    mode_payload["mode_fingerprint"] = mode_fingerprint
    mode_path = run_root / f"{args.mode}_manifest.json"
    if mode_path.exists():
        existing = json.loads(mode_path.read_text(encoding="utf-8"))
        if existing.get("mode_fingerprint") != mode_fingerprint:
            raise ContractError(
                f"Run ID {args.run_id!r} already has a different {args.mode} contract: "
                f"{mode_path}"
            )
    else:
        atomic_json(mode_path, mode_payload)

    print(f"{mode_fingerprint}\t{query}\t{mode_path}")
    return 0


def command_resolve_config(args: argparse.Namespace) -> int:
    query, env_dict, _ = config_inputs(
        Path(args.q_model_config).resolve(), Path(args.runner_root).resolve()
    )
    print(query if args.key == "query" else env_dict)
    return 0


def command_active_chunks(args: argparse.Namespace) -> int:
    active = active_chunks(
        load_query(Path(args.query)), args.total_env_ids, args.env_chunks
    )
    print(",".join(str(chunk) for chunk in active))
    return 0


def command_validate_representation(args: argparse.Namespace) -> int:
    query_path = Path(args.query).resolve()
    env_dict_path = Path(args.env_dict).resolve()
    query_rows = load_query(query_path)
    assert_unique_compute_keys(query_rows, query_path)
    summary = validate_environment_representation(
        query_rows,
        env_dict_path,
        args.hla_environment_representation,
        args.total_env_ids,
    )
    print(json.dumps({
        "representation": summary.representation,
        "environments": summary.environments,
        "min_alleles": summary.min_alleles,
        "max_alleles": summary.max_alleles,
        "query_rows": len(query_rows),
    }, sort_keys=True))
    return 0


def load_param_rows(path: Path, start: int, stop: int) -> List[Tuple[str, str, str, str, str]]:
    required = ("d_pos", "d_neg", "steepness_pos", "steepness_neg", "tau_thymus")
    rows: List[Tuple[str, str, str, str, str]] = []
    with path.open(newline="", encoding="utf-8-sig") as handle:
        reader = csv.DictReader(handle)
        if reader.fieldnames is None or any(name not in reader.fieldnames for name in required):
            raise ContractError(f"Parameter file lacks required columns {required}: {path}")
        all_rows = list(reader)
    if start < 0 or stop > len(all_rows) or start >= stop:
        raise ContractError(
            f"Invalid parameter range [{start}, {stop}) for {len(all_rows)} rows in {path}"
        )
    for record in all_rows[start:stop]:
        rows.append(tuple(canonical_decimal(record[name]) for name in required))
    if len(set(rows)) != len(rows):
        raise ContractError(f"Duplicate parameter identities in {path} range [{start}, {stop})")
    return rows


def load_mn(path: Path) -> Set[Tuple[int, int]]:
    result: Set[Tuple[int, int]] = set()
    with path.open(newline="", encoding="utf-8-sig") as handle:
        reader = csv.DictReader(handle)
        if reader.fieldnames is None:
            raise ContractError(f"M/N file has no header: {path}")
        m_col = find_header(reader.fieldnames, ("at_least_M", "M"), path)
        n_col = find_header(reader.fieldnames, ("at_most_N", "N"), path)
        for row_number, record in enumerate(reader, start=2):
            try:
                pair = (int(record[m_col]), int(record[n_col]))
            except (TypeError, ValueError) as exc:
                raise ContractError(f"Invalid M/N value in {path} row {row_number}") from exc
            if pair in result:
                raise ContractError(f"Duplicate M/N pair {pair} in {path}")
            result.add(pair)
    if not result:
        raise ContractError(f"M/N file is empty: {path}")
    return result


def load_mn_order(path: Path) -> List[Tuple[int, int]]:
    pairs = load_mn(path)
    ordered: List[Tuple[int, int]] = []
    with path.open(newline="", encoding="utf-8-sig") as handle:
        reader = csv.DictReader(handle)
        assert reader.fieldnames is not None
        m_col = find_header(reader.fieldnames, ("at_least_M", "M"), path)
        n_col = find_header(reader.fieldnames, ("at_most_N", "N"), path)
        for record in reader:
            ordered.append((int(record[m_col]), int(record[n_col])))
    if set(ordered) != pairs:
        raise ContractError(f"Inconsistent M/N ordering in {path}")
    return ordered


def selected_query(
    query_path: Path, env_start: int, env_stop: int
) -> List[QueryRow]:
    return [
        row for row in load_query(query_path) if env_start <= row.env_id < env_stop
    ]


def query_compute_key(row: QueryRow) -> Tuple[str, str, int, str]:
    return (row.peptide, row.hla, row.env_id, row.expression)


def select_query_shard(
    query_rows: Sequence[QueryRow], shard_index: int, shard_count: int
) -> List[QueryRow]:
    """Select a deterministic, balanced subset of unique compute identities."""
    if shard_count < 1:
        raise ContractError(f"shard_count must be positive; got {shard_count}")
    if shard_index < 0 or shard_index >= shard_count:
        raise ContractError(
            f"shard_index must be in [0, {shard_count}); got {shard_index}"
        )
    ordered = sorted(query_rows, key=query_compute_key)
    return [row for index, row in enumerate(ordered) if index % shard_count == shard_index]


def build_pn_shard_tasks(
    query_rows: Sequence[QueryRow],
    total_env_ids: int,
    env_chunks: int,
    total_params: int,
    param_chunks: int,
    target_rows: int,
) -> List[PnShardTask]:
    if target_rows < 1:
        raise ContractError(f"target_rows must be positive; got {target_rows}")
    if total_params < param_chunks or total_params < 1 or param_chunks < 1:
        raise ContractError(
            f"invalid parameter geometry: total={total_params}, chunks={param_chunks}"
        )
    params_per_chunk = total_params // param_chunks
    tasks: List[PnShardTask] = []
    next_task_id = 0
    for parameter_chunk in range(param_chunks):
        parameter_start = parameter_chunk * params_per_chunk
        parameter_stop = parameter_start + params_per_chunk
        if parameter_chunk == param_chunks - 1:
            parameter_stop = total_params
        for env_chunk in active_chunks(query_rows, total_env_ids, env_chunks):
            env_start, env_stop = env_range_for_chunk(
                env_chunk, total_env_ids, env_chunks
            )
            env_rows = [
                row for row in query_rows if env_start <= row.env_id < env_stop
            ]
            shard_count = max(1, math.ceil(len(env_rows) / target_rows))
            parent_task_id = parameter_chunk * env_chunks + env_chunk
            for shard_index in range(shard_count):
                expected = len(select_query_shard(env_rows, shard_index, shard_count))
                tasks.append(
                    PnShardTask(
                        task_id=next_task_id,
                        parent_task_id=parent_task_id,
                        parameter_chunk=parameter_chunk,
                        parameter_start=parameter_start,
                        parameter_stop=parameter_stop,
                        env_chunk=env_chunk,
                        env_start=env_start,
                        env_stop=env_stop,
                        shard_index=shard_index,
                        shard_count=shard_count,
                        expected_query_rows=expected,
                    )
                )
                next_task_id += 1
    return tasks


def shard_task_payload(task: PnShardTask) -> Mapping[str, int]:
    return {
        field: int(getattr(task, field))
        for field in PnShardTask.__dataclass_fields__
    }


def write_pn_shard_plan(
    path: Path,
    query_path: Path,
    total_env_ids: int,
    env_chunks: int,
    total_params: int,
    param_chunks: int,
    target_rows: int,
) -> Mapping[str, object]:
    query_rows = load_query(query_path)
    assert_unique_compute_keys(query_rows, query_path)
    tasks = build_pn_shard_tasks(
        query_rows,
        total_env_ids,
        env_chunks,
        total_params,
        param_chunks,
        target_rows,
    )
    payload: MutableMapping[str, object] = {
        "schema_version": SCHEMA_VERSION,
        "kind": "pn_peptide_shard_plan",
        "query": str(query_path.resolve()),
        "query_sha256": sha256_file(query_path),
        "total_env_ids": total_env_ids,
        "env_chunks": env_chunks,
        "total_params": total_params,
        "param_chunks": param_chunks,
        "target_rows": target_rows,
        "tasks": [shard_task_payload(task) for task in tasks],
    }
    payload["plan_fingerprint"] = canonical_hash(payload)
    if path.exists():
        existing = json.loads(path.read_text(encoding="utf-8"))
        if existing != payload:
            raise ContractError(f"Refusing to replace different shard plan: {path}")
    else:
        atomic_json(path, payload)
    return payload


def load_pn_shard_plan(path: Path) -> Tuple[Mapping[str, object], List[PnShardTask]]:
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise ContractError(f"Invalid shard plan: {path}") from exc
    if payload.get("schema_version") != SCHEMA_VERSION or payload.get("kind") != "pn_peptide_shard_plan":
        raise ContractError(f"Unsupported shard plan: {path}")
    raw_tasks = payload.get("tasks")
    if not isinstance(raw_tasks, list):
        raise ContractError(f"Shard plan has no task list: {path}")
    try:
        tasks = [PnShardTask(**{key: int(value) for key, value in row.items()}) for row in raw_tasks]
    except (TypeError, ValueError) as exc:
        raise ContractError(f"Malformed shard task in {path}") from exc
    if [task.task_id for task in tasks] != list(range(len(tasks))):
        raise ContractError(f"Shard task IDs must be contiguous from zero: {path}")
    fingerprint = payload.get("plan_fingerprint")
    comparable = dict(payload)
    comparable.pop("plan_fingerprint", None)
    if fingerprint != canonical_hash(comparable):
        raise ContractError(f"Shard plan fingerprint mismatch: {path}")
    return payload, tasks


def shard_storage_dir(run_root: Path, task: PnShardTask) -> Path:
    return (
        run_root
        / ".pn_shards"
        / f"TASK_{task.parent_task_id}"
        / f"SHARD_{task.shard_index}_OF_{task.shard_count}"
    )


def file_inventory(path: Path, run_root: Path) -> Mapping[str, object]:
    return {
        "path": str(path.resolve().relative_to(run_root.resolve())),
        "size": path.stat().st_size,
        "sha256": sha256_file(path),
    }


def validate_qpi(directory: Path, query_rows: Sequence[QueryRow]) -> List[Path]:
    q_expected: Dict[str, Set[Tuple[str, int]]] = defaultdict(set)
    pi_expected: Dict[str, Set[str]] = defaultdict(set)
    for row in query_rows:
        q_expected[row.hla].add((row.peptide, row.env_id))
        pi_expected[row.hla].add(row.peptide)

    expected_names = {
        *(f"query_peptides_results_HLA_{hla}_q_values.csv" for hla in q_expected),
        *(f"query_peptides_results_HLA_{hla}_pi_values.csv" for hla in pi_expected),
    }
    actual_files = sorted(directory.glob("*.csv")) if directory.exists() else []
    actual_names = {path.name for path in actual_files}
    missing = sorted(expected_names - actual_names)
    extra = sorted(actual_names - expected_names)
    if missing or extra:
        raise ContractError(
            f"Q/Pi file inventory mismatch in {directory}: "
            f"missing={missing[:20]} extra={extra[:20]}"
        )

    for path in actual_files:
        q_match = Q_FILENAME_RE.match(path.name)
        pi_match = PI_FILENAME_RE.match(path.name)
        if q_match:
            hla = normalize_hla(q_match.group(1))
            seen: Set[Tuple[str, int]] = set()
            with path.open(newline="", encoding="utf-8-sig") as handle:
                reader = csv.DictReader(handle)
                if reader.fieldnames is None:
                    raise ContractError(f"Q file has no header: {path}")
                for name in ("peptide", "env_id", "q_value"):
                    if name not in reader.fieldnames:
                        raise ContractError(f"Q file lacks {name!r}: {path}")
                row_count = 0
                for row_number, record in enumerate(reader, start=2):
                    peptide = (record["peptide"] or "").strip().upper()
                    try:
                        env_id = int(record["env_id"])
                    except ValueError as exc:
                        raise ContractError(f"Invalid env_id in {path} row {row_number}") from exc
                    parse_finite(record["q_value"], "q_value", path, row_number)
                    key = (peptide, env_id)
                    if key in seen:
                        raise ContractError(f"Duplicate Q key {key} in {path}")
                    seen.add(key)
                    row_count += 1
            if seen != q_expected[hla]:
                raise ContractError(
                    f"Q key coverage mismatch in {path}: expected={len(q_expected[hla])} "
                    f"actual={len(seen)}"
                )
        elif pi_match:
            hla = normalize_hla(pi_match.group(1))
            seen_peptides: Set[str] = set()
            with path.open(newline="", encoding="utf-8-sig") as handle:
                reader = csv.DictReader(handle)
                if reader.fieldnames is None:
                    raise ContractError(f"Pi file has no header: {path}")
                for name in ("peptide", "pi_value"):
                    if name not in reader.fieldnames:
                        raise ContractError(f"Pi file lacks {name!r}: {path}")
                for row_number, record in enumerate(reader, start=2):
                    peptide = (record["peptide"] or "").strip().upper()
                    parse_finite(record["pi_value"], "pi_value", path, row_number)
                    if peptide in seen_peptides:
                        raise ContractError(f"Duplicate Pi peptide {peptide!r} in {path}")
                    seen_peptides.add(peptide)
            if seen_peptides != pi_expected[hla]:
                raise ContractError(
                    f"Pi key coverage mismatch in {path}: expected={len(pi_expected[hla])} "
                    f"actual={len(seen_peptides)}"
                )
        else:
            raise ContractError(f"Unexpected Q/Pi filename: {path}")
    return actual_files


def parse_pn_filename(path: Path) -> Tuple[Tuple[str, str, str, str, str], str, str]:
    match = PN_FILENAME_RE.match(path.name)
    if not match:
        raise ContractError(f"Unexpected PN filename: {path}")
    parameter = tuple(canonical_decimal(match.group(index)) for index in (1, 2, 3, 4, 6))
    hla = normalize_hla(match.group(7))
    expression = match.group(8)
    return parameter, hla, expression


def validate_pn(
    directory: Path,
    query_rows: Sequence[QueryRow],
    parameter_rows: Sequence[Tuple[str, str, str, str, str]],
    mn_pairs: Set[Tuple[int, int]],
) -> List[Path]:
    contexts: Dict[Tuple[str, str], Set[Tuple[str, int]]] = defaultdict(set)
    for row in query_rows:
        contexts[(row.hla, row.expression)].add((row.peptide, row.env_id))

    expected_file_keys = {
        (parameter, hla, expression)
        for parameter in parameter_rows
        for hla, expression in contexts
    }
    actual_files = sorted(directory.glob("*.csv")) if directory.exists() else []
    actual_by_key: Dict[
        Tuple[Tuple[str, str, str, str, str], str, str], Path
    ] = {}
    for path in actual_files:
        key = parse_pn_filename(path)
        if key in actual_by_key:
            raise ContractError(f"Duplicate PN file identity {key}: {path}")
        actual_by_key[key] = path
    actual_keys = set(actual_by_key)
    if actual_keys != expected_file_keys:
        missing = sorted(expected_file_keys - actual_keys, key=str)
        extra = sorted(actual_keys - expected_file_keys, key=str)
        raise ContractError(
            f"PN file inventory mismatch in {directory}: "
            f"expected={len(expected_file_keys)} actual={len(actual_keys)} "
            f"missing={missing[:5]} extra={extra[:5]}"
        )

    for (_, hla, expression), path in actual_by_key.items():
        expected_peptides = contexts[(hla, expression)]
        expected_row_count = len(expected_peptides) * len(mn_pairs)
        seen: Set[Tuple[str, int, int, int]] = set()
        with path.open(newline="", encoding="utf-8-sig") as handle:
            reader = csv.DictReader(handle)
            if reader.fieldnames is None:
                raise ContractError(f"PN file has no header: {path}")
            for name in ("peptide", "env_id", "M", "N", "p_pos", "p_neg"):
                if name not in reader.fieldnames:
                    raise ContractError(f"PN file lacks {name!r}: {path}")
            for row_number, record in enumerate(reader, start=2):
                peptide = (record["peptide"] or "").strip().upper()
                try:
                    key = (
                        peptide,
                        int(record["env_id"]),
                        int(record["M"]),
                        int(record["N"]),
                    )
                except ValueError as exc:
                    raise ContractError(f"Invalid PN key in {path} row {row_number}") from exc
                parse_finite(record["p_pos"], "p_pos", path, row_number)
                parse_finite(record["p_neg"], "p_neg", path, row_number)
                if (key[0], key[1]) not in expected_peptides or (key[2], key[3]) not in mn_pairs:
                    raise ContractError(f"Unexpected PN row key {key} in {path}")
                if key in seen:
                    raise ContractError(f"Duplicate PN row key {key} in {path}")
                seen.add(key)
        if len(seen) != expected_row_count:
            raise ContractError(
                f"PN row coverage mismatch in {path}: expected={expected_row_count} "
                f"actual={len(seen)}"
            )
    return actual_files


def publish_files(files: Sequence[Path], source_dir: Path, final_dir: Path) -> List[Path]:
    final_dir.mkdir(parents=True, exist_ok=True)
    published: List[Path] = []
    for source in files:
        relative = source.relative_to(source_dir)
        target = final_dir / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        if target.exists():
            if target.stat().st_size != source.stat().st_size or sha256_file(target) != sha256_file(source):
                raise ContractError(f"Refusing to clobber conflicting output file: {target}")
            source.unlink()
        else:
            os.replace(source, target)
        published.append(target)
    return published


def task_mn_file(args, start, stop):
    # Resolve from the signed mode inputs, never trust a task-supplied union CSV.
    root = Path(getattr(args, "run_root", None) or getattr(args, "final_root", "."))
    mode_path = root / "pn_manifest.json"
    if mode_path.is_file():
        mode = json.loads(mode_path.read_text())
        if mode.get("mode_fingerprint") != args.fingerprint:
            raise ContractError("Task fingerprint differs from P/N mode manifest")
        if any(x["role"] == "survivor_grid" for x in mode["inputs"]):
            import survivor_grid
            return survivor_grid.mode_grid(mode_path, start, stop)
    if os.environ.get("GRID_PROFILE") == "survivor-exact":
        raise ContractError("Exact task lacks its frozen survivor-grid mode contract")
    return Path(args.mn_tuples)


def command_validate_publish(args: argparse.Namespace) -> int:
    staging_root = Path(args.staging_root).resolve()
    final_root = Path(args.final_root).resolve()
    query_rows = selected_query(Path(args.query), args.env_start, args.env_stop)
    dirname = (
        f"results_pn_env_id_{args.env_start}_{args.env_stop}"
        if args.mode == "pn"
        else f"results_qpi_env_id_{args.env_start}_{args.env_stop}"
    )
    source_dir = staging_root / dirname
    final_dir = final_root / dirname

    if not query_rows:
        files = sorted(source_dir.glob("*.csv")) if source_dir.exists() else []
        if files:
            raise ContractError(
                f"Zero-input task unexpectedly produced {len(files)} CSVs under {source_dir}"
            )
        published: List[Path] = []
        status = "zero_input"
    elif args.mode == "qpi":
        staged = validate_qpi(source_dir, query_rows)
        published = publish_files(staged, source_dir, final_dir)
        validate_qpi(final_dir, query_rows)
        status = "success"
    else:
        parameters = load_param_rows(
            Path(args.param_file), args.parameter_start, args.parameter_stop
        )
        mn_pairs = load_mn(task_mn_file(args, args.parameter_start, args.parameter_stop))
        staged = validate_pn(source_dir, query_rows, parameters, mn_pairs)
        published = publish_files(staged, source_dir, final_dir)
        status = "success"

    manifest_path = final_dir / f"TASK_{args.task_id}.done.json"
    payload: MutableMapping[str, object] = {
        "schema_version": SCHEMA_VERSION,
        "status": status,
        "run_id": args.run_id,
        "fingerprint": args.fingerprint,
        "task_id": args.task_id,
        "mode": args.mode,
        "env_start": args.env_start,
        "env_stop": args.env_stop,
        "parameter_start": args.parameter_start,
        "parameter_stop": args.parameter_stop,
        "query_rows": len(query_rows),
        "expected_files": len(published),
        "actual_files": len(published),
        "output_files": [file_inventory(path, final_root) for path in published],
    }
    atomic_json(manifest_path, payload)
    print(manifest_path)
    return 0


def command_build_pn_shard_plan(args: argparse.Namespace) -> int:
    payload = write_pn_shard_plan(
        Path(args.output).resolve(),
        Path(args.query).resolve(),
        args.total_env_ids,
        args.env_chunks,
        args.total_params,
        args.param_chunks,
        args.target_rows,
    )
    print(
        json.dumps(
            {
                "output": str(Path(args.output).resolve()),
                "tasks": len(payload["tasks"]),
                "plan_fingerprint": payload["plan_fingerprint"],
            },
            sort_keys=True,
        )
    )
    return 0


def command_pn_shard_task(args: argparse.Namespace) -> int:
    _, tasks = load_pn_shard_plan(Path(args.plan).resolve())
    if args.task_id < 0 or args.task_id >= len(tasks):
        raise ContractError(
            f"Shard task {args.task_id} is outside [0, {len(tasks)})"
        )
    task = tasks[args.task_id]
    values = [
        task.task_id,
        task.parent_task_id,
        task.parameter_start,
        task.parameter_stop,
        task.env_chunk,
        task.env_start,
        task.env_stop,
        task.shard_index,
        task.shard_count,
        task.expected_query_rows,
    ]
    print("\t".join(str(value) for value in values))
    return 0


def command_pn_shard_task_ids(args: argparse.Namespace) -> int:
    _, tasks = load_pn_shard_plan(Path(args.plan).resolve())
    selected_env_chunks: Optional[Set[int]] = None
    if args.env_chunks:
        selected_env_chunks = {int(value) for value in args.env_chunks.split(",")}
    run_root = Path(args.run_root).resolve()
    selected: List[int] = []
    for task in tasks:
        if selected_env_chunks is not None and task.env_chunk not in selected_env_chunks:
            continue
        if args.missing_only:
            parent_dir = run_root / f"results_pn_env_id_{task.env_start}_{task.env_stop}"
            parent_manifest = parent_dir / f"TASK_{task.parent_task_id}.done.json"
            try:
                verify_task_manifest(parent_manifest, args.fingerprint, run_root, False)
                continue
            except ContractError:
                pass
            manifest = shard_storage_dir(run_root, task) / "SHARD.done.json"
            try:
                verify_task_manifest(manifest, args.fingerprint, run_root, False)
                continue
            except ContractError:
                pass
        selected.append(task.task_id)
    print(",".join(str(value) for value in selected))
    return 0


def command_pn_shard_parent_ids(args: argparse.Namespace) -> int:
    _, tasks = load_pn_shard_plan(Path(args.plan).resolve())
    selected_env_chunks: Optional[Set[int]] = None
    if args.env_chunks:
        selected_env_chunks = {int(value) for value in args.env_chunks.split(",")}
    parents = sorted(
        {
            task.parent_task_id
            for task in tasks
            if selected_env_chunks is None or task.env_chunk in selected_env_chunks
        }
    )
    if args.missing_only:
        run_root = Path(args.run_root).resolve()
        remaining: List[int] = []
        by_parent = {task.parent_task_id: task for task in tasks}
        for parent in parents:
            task = by_parent[parent]
            final_dir = run_root / f"results_pn_env_id_{task.env_start}_{task.env_stop}"
            manifest = final_dir / f"TASK_{parent}.done.json"
            try:
                verify_task_manifest(manifest, args.fingerprint, run_root, False)
            except ContractError:
                remaining.append(parent)
        parents = remaining
    print(",".join(str(value) for value in parents))
    return 0


def command_pn_shard_parent(args: argparse.Namespace) -> int:
    _, tasks = load_pn_shard_plan(Path(args.plan).resolve())
    matching = [task for task in tasks if task.parent_task_id == args.parent_task_id]
    if not matching:
        raise ContractError(f"Unknown parent task ID {args.parent_task_id}")
    task = matching[0]
    print(
        "\t".join(
            str(value)
            for value in (
                task.parent_task_id,
                task.parameter_start,
                task.parameter_stop,
                task.env_chunk,
                task.env_start,
                task.env_stop,
                task.shard_count,
            )
        )
    )
    return 0


def command_validate_pn_shard(args: argparse.Namespace) -> int:
    plan_path = Path(args.plan).resolve()
    _, tasks = load_pn_shard_plan(plan_path)
    if args.task_id < 0 or args.task_id >= len(tasks):
        raise ContractError(f"Unknown shard task ID {args.task_id}")
    task = tasks[args.task_id]
    staging_root = Path(args.staging_root).resolve()
    run_root = Path(args.run_root).resolve()
    source_dir = staging_root / f"results_pn_env_id_{task.env_start}_{task.env_stop}"
    env_rows = selected_query(Path(args.query), task.env_start, task.env_stop)
    shard_rows = select_query_shard(env_rows, task.shard_index, task.shard_count)
    if len(shard_rows) != task.expected_query_rows:
        raise ContractError(
            f"Shard plan expected {task.expected_query_rows} rows but selected {len(shard_rows)}"
        )
    parameters = load_param_rows(
        Path(args.param_file), task.parameter_start, task.parameter_stop
    )
    mn_pairs = load_mn(task_mn_file(args, task.parameter_start, task.parameter_stop))
    staged = validate_pn(source_dir, shard_rows, parameters, mn_pairs)
    destination = shard_storage_dir(run_root, task)
    published = publish_files(staged, source_dir, destination)
    manifest = destination / "SHARD.done.json"
    payload: MutableMapping[str, object] = {
        "schema_version": SCHEMA_VERSION,
        "status": "success",
        "run_id": args.run_id,
        "fingerprint": args.fingerprint,
        "task_id": task.task_id,
        "parent_task_id": task.parent_task_id,
        "shard_index": task.shard_index,
        "shard_count": task.shard_count,
        "env_start": task.env_start,
        "env_stop": task.env_stop,
        "parameter_start": task.parameter_start,
        "parameter_stop": task.parameter_stop,
        "query_rows": len(shard_rows),
        "expected_files": len(published),
        "actual_files": len(published),
        "output_files": [file_inventory(path, run_root) for path in published],
    }
    atomic_json(manifest, payload)
    print(manifest)
    return 0


def command_merge_pn_shards(args: argparse.Namespace) -> int:
    _, tasks = load_pn_shard_plan(Path(args.plan).resolve())
    parent_tasks = [task for task in tasks if task.parent_task_id == args.parent_task_id]
    if not parent_tasks:
        raise ContractError(f"Unknown parent task ID {args.parent_task_id}")
    parent_tasks.sort(key=lambda task: task.shard_index)
    reference = parent_tasks[0]
    if [task.shard_index for task in parent_tasks] != list(range(reference.shard_count)):
        raise ContractError(f"Parent task {args.parent_task_id} has an incomplete shard plan")

    run_root = Path(args.run_root).resolve()
    for task in parent_tasks:
        manifest = shard_storage_dir(run_root, task) / "SHARD.done.json"
        verify_task_manifest(manifest, args.fingerprint, run_root, True)

    env_rows = selected_query(Path(args.query), reference.env_start, reference.env_stop)
    parameter_rows = load_param_rows(
        Path(args.param_file), reference.parameter_start, reference.parameter_stop
    )
    mn_order = load_mn_order(task_mn_file(args, reference.parameter_start, reference.parameter_stop))
    mn_rank = {pair: index for index, pair in enumerate(mn_order)}
    context_rank: Dict[Tuple[str, str], Dict[Tuple[str, int], int]] = defaultdict(dict)
    for row in env_rows:
        context = (row.hla, row.expression)
        context_rank[context].setdefault(
            (row.peptide, row.env_id), len(context_rank[context])
        )

    staging_parent = run_root / ".staging" / "pn_merge"
    staging_parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(
        prefix=f"TASK_{args.parent_task_id}.", dir=staging_parent
    ) as tmp_name:
        merged_dir = Path(tmp_name) / f"results_pn_env_id_{reference.env_start}_{reference.env_stop}"
        merged_dir.mkdir(parents=True)
        files_by_name: Dict[str, List[Path]] = defaultdict(list)
        for task in parent_tasks:
            for path in shard_storage_dir(run_root, task).glob("*.csv"):
                files_by_name[path.name].append(path)

        for filename, shard_files in sorted(files_by_name.items()):
            parameter, hla, expression = parse_pn_filename(Path(filename))
            if parameter not in parameter_rows:
                raise ContractError(f"Unexpected parameter identity while merging {filename}")
            headers: Optional[List[str]] = None
            records: List[Mapping[str, str]] = []
            seen: Set[Tuple[str, int, int, int]] = set()
            for shard_file in sorted(shard_files):
                with shard_file.open(newline="", encoding="utf-8-sig") as handle:
                    reader = csv.DictReader(handle)
                    if reader.fieldnames is None:
                        raise ContractError(f"PN shard file has no header: {shard_file}")
                    if headers is None:
                        headers = list(reader.fieldnames)
                    elif headers != list(reader.fieldnames):
                        raise ContractError(f"PN shard headers differ for {filename}")
                    for row_number, record in enumerate(reader, start=2):
                        key = (
                            (record.get("peptide") or "").strip().upper(),
                            int(record["env_id"]),
                            int(record["M"]),
                            int(record["N"]),
                        )
                        if key in seen:
                            raise ContractError(f"Duplicate PN row across shards: {key}")
                        seen.add(key)
                        records.append(dict(record))
            if headers is None:
                raise ContractError(f"No shard headers found for {filename}")
            ranks = context_rank[(hla, expression)]
            try:
                records.sort(
                    key=lambda row: (
                        ranks[((row["peptide"] or "").strip().upper(), int(row["env_id"]))],
                        mn_rank[(int(row["M"]), int(row["N"]))],
                    )
                )
            except KeyError as exc:
                raise ContractError(f"Unexpected PN merge key in {filename}: {exc}") from exc
            with (merged_dir / filename).open("w", newline="", encoding="utf-8") as handle:
                writer = csv.DictWriter(handle, fieldnames=headers, lineterminator="\n")
                writer.writeheader()
                writer.writerows(records)

        staged = validate_pn(merged_dir, env_rows, parameter_rows, set(mn_order))
        final_dir = run_root / f"results_pn_env_id_{reference.env_start}_{reference.env_stop}"
        published = publish_files(staged, merged_dir, final_dir)

    manifest = final_dir / f"TASK_{args.parent_task_id}.done.json"
    payload: MutableMapping[str, object] = {
        "schema_version": SCHEMA_VERSION,
        "status": "success",
        "run_id": args.run_id,
        "fingerprint": args.fingerprint,
        "task_id": args.parent_task_id,
        "mode": "pn",
        "sharded": True,
        "shard_count": reference.shard_count,
        "env_start": reference.env_start,
        "env_stop": reference.env_stop,
        "parameter_start": reference.parameter_start,
        "parameter_stop": reference.parameter_stop,
        "query_rows": len(env_rows),
        "expected_files": len(published),
        "actual_files": len(published),
        "output_files": [file_inventory(path, run_root) for path in published],
    }
    atomic_json(manifest, payload)
    print(manifest)
    return 0


def verify_task_manifest(path: Path, fingerprint: str, run_root: Path, verify_hashes: bool) -> None:
    if not path.is_file():
        raise ContractError(f"Completion manifest is absent: {path}")
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise ContractError(f"Invalid completion manifest: {path}") from exc
    if payload.get("schema_version") != SCHEMA_VERSION:
        raise ContractError(f"Unsupported completion schema in {path}")
    if payload.get("status") not in {"success", "zero_input"}:
        raise ContractError(f"Task is not complete according to {path}")
    if payload.get("fingerprint") != fingerprint:
        raise ContractError(f"Fingerprint mismatch in {path}")
    output_files = payload.get("output_files")
    if not isinstance(output_files, list):
        raise ContractError(f"Missing output inventory in {path}")
    if payload.get("actual_files") != len(output_files):
        raise ContractError(f"Output count mismatch in {path}")
    for item in output_files:
        if not isinstance(item, dict) or "path" not in item:
            raise ContractError(f"Malformed output inventory in {path}")
        output = run_root / str(item["path"])
        if not output.is_file() or output.stat().st_size != item.get("size"):
            raise ContractError(f"Missing or size-mismatched task output: {output}")
        if verify_hashes and sha256_file(output) != item.get("sha256"):
            raise ContractError(f"Checksum mismatch for task output: {output}")


def command_task_complete(args: argparse.Namespace) -> int:
    try:
        verify_task_manifest(
            Path(args.manifest),
            args.fingerprint,
            Path(args.run_root),
            args.verify_hashes,
        )
    except ContractError as exc:
        print(str(exc), file=sys.stderr)
        return 1
    return 0


def command_audit_run(args: argparse.Namespace) -> int:
    run_root = Path(args.run_root).resolve()
    mode_manifest_path = Path(args.mode_manifest).resolve()
    mode_payload = json.loads(mode_manifest_path.read_text(encoding="utf-8"))
    common_payload = json.loads((run_root / "run_manifest.json").read_text(encoding="utf-8"))
    mode = str(mode_payload["mode"])
    fingerprint = str(mode_payload["mode_fingerprint"])
    active = [int(value) for value in mode_payload["active_env_chunks"]]
    env_chunks = int(common_payload["env_chunks"])
    total_env_ids = int(common_payload["total_env_ids"])
    param_chunks = int(mode_payload["param_chunks"])
    if mode == "qpi":
        expected_tasks = active
    elif mode == "pn":
        expected_tasks = [
            param_chunk * env_chunks + env_chunk
            for param_chunk in range(param_chunks)
            for env_chunk in active
        ]
    else:
        raise ContractError(f"Unsupported audit mode {mode!r}")

    failures: List[str] = []
    total_files = 0
    valid_tasks = 0
    expected_manifest_paths: Set[Path] = set()
    for task_id in expected_tasks:
        env_chunk = task_id % env_chunks if mode == "pn" else task_id
        env_start, env_stop = env_range_for_chunk(env_chunk, total_env_ids, env_chunks)
        directory = run_root / f"results_{'pn' if mode == 'pn' else 'qpi'}_env_id_{env_start}_{env_stop}"
        manifest = directory / f"TASK_{task_id}.done.json"
        expected_manifest_paths.add(manifest.resolve())
        try:
            verify_task_manifest(manifest, fingerprint, run_root, args.verify_hashes)
            payload = json.loads(manifest.read_text(encoding="utf-8"))
            total_files += int(payload["actual_files"])
            valid_tasks += 1
        except (ContractError, OSError, ValueError, KeyError, json.JSONDecodeError) as exc:
            failures.append(f"task {task_id}: {exc}")

    pattern = "results_pn_env_id_*/TASK_*.done.json" if mode == "pn" else "results_qpi_env_id_*/TASK_*.done.json"
    actual_manifest_paths = {path.resolve() for path in run_root.glob(pattern)}
    unexpected = sorted(str(path) for path in actual_manifest_paths - expected_manifest_paths)
    summary = {
        "schema_version": SCHEMA_VERSION,
        "run_id": mode_payload["run_id"],
        "mode": mode,
        "fingerprint": fingerprint,
        "expected_tasks": len(expected_tasks),
        "valid_tasks": valid_tasks,
        "invalid_or_missing_tasks": len(failures),
        "unexpected_manifests": len(unexpected),
        "recorded_output_files": total_files,
        "verified_output_hashes": bool(args.verify_hashes),
        "failure_examples": failures[:20],
        "unexpected_manifest_examples": unexpected[:20],
    }
    print(json.dumps(summary, indent=2, sort_keys=True))
    return 0 if not failures and not unexpected else 1


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)

    prepare = subparsers.add_parser("prepare-run")
    prepare.add_argument("--run-root", required=True)
    prepare.add_argument("--run-id", required=True)
    prepare.add_argument("--dataset", required=True)
    prepare.add_argument("--mode", choices=("pn", "qpi"), required=True)
    prepare.add_argument("--runner-root", required=True)
    prepare.add_argument("--runner", required=True)
    prepare.add_argument("--q-model-config", required=True)
    prepare.add_argument("--submit-script", required=True)
    prepare.add_argument("--run-script", required=True)
    prepare.add_argument("--contract-helper", required=True)
    prepare.add_argument("--total-env-ids", type=int, required=True)
    prepare.add_argument("--env-chunks", type=int, required=True)
    prepare.add_argument("--total-params", type=int, required=True)
    prepare.add_argument("--param-chunks", type=int, required=True)
    prepare.add_argument("--param-file", required=True)
    prepare.add_argument("--mn-tuples", required=True)
    prepare.add_argument("--pmhc-config", required=True)
    prepare.add_argument("--pmhc-parameters-file", required=True)
    prepare.add_argument("--in-vitro", type=int, choices=(0, 1), required=True)
    prepare.add_argument("--peptide-conc", type=float, required=True)
    prepare.add_argument("--pn-hla-scope", choices=("all", "focal"), required=True)
    prepare.add_argument("--max-num-ps-values-log2", type=int, required=True)
    prepare.add_argument(
        "--hla-environment-representation",
        choices=("full", "mono"),
        required=True,
    )
    prepare.add_argument("--regime-manifest", default="")
    prepare.add_argument("--representation-crosswalk", default="")
    prepare.add_argument("--pn-peptides-per-shard", type=int, default=0)
    prepare.add_argument("--pn-shard-plan", default="")
    prepare.add_argument("--finalize-script", default="")
    prepare.add_argument("--allow-operational-provenance-change", action="store_true")
    prepare.set_defaults(func=command_prepare_run)

    resolve = subparsers.add_parser("resolve-config")
    resolve.add_argument("--q-model-config", required=True)
    resolve.add_argument("--runner-root", required=True)
    resolve.add_argument("--key", choices=("query", "env_dict"), required=True)
    resolve.set_defaults(func=command_resolve_config)

    active = subparsers.add_parser("active-chunks")
    active.add_argument("--query", required=True)
    active.add_argument("--total-env-ids", type=int, required=True)
    active.add_argument("--env-chunks", type=int, required=True)
    active.set_defaults(func=command_active_chunks)

    representation = subparsers.add_parser("validate-representation")
    representation.add_argument("--query", required=True)
    representation.add_argument("--env-dict", required=True)
    representation.add_argument(
        "--hla-environment-representation",
        choices=("full", "mono"),
        required=True,
    )
    representation.add_argument("--total-env-ids", type=int, required=True)
    representation.set_defaults(func=command_validate_representation)

    publish = subparsers.add_parser("validate-publish")
    publish.add_argument("--mode", choices=("pn", "qpi"), required=True)
    publish.add_argument("--run-id", required=True)
    publish.add_argument("--fingerprint", required=True)
    publish.add_argument("--task-id", type=int, required=True)
    publish.add_argument("--query", required=True)
    publish.add_argument("--staging-root", required=True)
    publish.add_argument("--final-root", required=True)
    publish.add_argument("--env-start", type=int, required=True)
    publish.add_argument("--env-stop", type=int, required=True)
    publish.add_argument("--parameter-start", type=int, required=True)
    publish.add_argument("--parameter-stop", type=int, required=True)
    publish.add_argument("--param-file", required=True)
    publish.add_argument("--mn-tuples", required=True)
    publish.set_defaults(func=command_validate_publish)

    shard_plan = subparsers.add_parser("build-pn-shard-plan")
    shard_plan.add_argument("--query", required=True)
    shard_plan.add_argument("--output", required=True)
    shard_plan.add_argument("--total-env-ids", type=int, required=True)
    shard_plan.add_argument("--env-chunks", type=int, required=True)
    shard_plan.add_argument("--total-params", type=int, required=True)
    shard_plan.add_argument("--param-chunks", type=int, required=True)
    shard_plan.add_argument("--target-rows", type=int, required=True)
    shard_plan.set_defaults(func=command_build_pn_shard_plan)

    shard_task = subparsers.add_parser("pn-shard-task")
    shard_task.add_argument("--plan", required=True)
    shard_task.add_argument("--task-id", type=int, required=True)
    shard_task.set_defaults(func=command_pn_shard_task)

    shard_ids = subparsers.add_parser("pn-shard-task-ids")
    shard_ids.add_argument("--plan", required=True)
    shard_ids.add_argument("--run-root", required=True)
    shard_ids.add_argument("--fingerprint", required=True)
    shard_ids.add_argument("--env-chunks", default="")
    shard_ids.add_argument("--missing-only", action="store_true")
    shard_ids.set_defaults(func=command_pn_shard_task_ids)

    parent_ids = subparsers.add_parser("pn-shard-parent-ids")
    parent_ids.add_argument("--plan", required=True)
    parent_ids.add_argument("--run-root", required=True)
    parent_ids.add_argument("--fingerprint", required=True)
    parent_ids.add_argument("--env-chunks", default="")
    parent_ids.add_argument("--missing-only", action="store_true")
    parent_ids.set_defaults(func=command_pn_shard_parent_ids)

    parent_spec = subparsers.add_parser("pn-shard-parent")
    parent_spec.add_argument("--plan", required=True)
    parent_spec.add_argument("--parent-task-id", type=int, required=True)
    parent_spec.set_defaults(func=command_pn_shard_parent)

    validate_shard = subparsers.add_parser("validate-pn-shard")
    validate_shard.add_argument("--plan", required=True)
    validate_shard.add_argument("--task-id", type=int, required=True)
    validate_shard.add_argument("--run-id", required=True)
    validate_shard.add_argument("--fingerprint", required=True)
    validate_shard.add_argument("--query", required=True)
    validate_shard.add_argument("--staging-root", required=True)
    validate_shard.add_argument("--run-root", required=True)
    validate_shard.add_argument("--param-file", required=True)
    validate_shard.add_argument("--mn-tuples", required=True)
    validate_shard.set_defaults(func=command_validate_pn_shard)

    merge_shards = subparsers.add_parser("merge-pn-shards")
    merge_shards.add_argument("--plan", required=True)
    merge_shards.add_argument("--parent-task-id", type=int, required=True)
    merge_shards.add_argument("--run-id", required=True)
    merge_shards.add_argument("--fingerprint", required=True)
    merge_shards.add_argument("--query", required=True)
    merge_shards.add_argument("--run-root", required=True)
    merge_shards.add_argument("--param-file", required=True)
    merge_shards.add_argument("--mn-tuples", required=True)
    merge_shards.set_defaults(func=command_merge_pn_shards)

    complete = subparsers.add_parser("task-complete")
    complete.add_argument("--manifest", required=True)
    complete.add_argument("--fingerprint", required=True)
    complete.add_argument("--run-root", required=True)
    complete.add_argument("--verify-hashes", action="store_true")
    complete.set_defaults(func=command_task_complete)

    audit = subparsers.add_parser("audit-run")
    audit.add_argument("--run-root", required=True)
    audit.add_argument("--mode-manifest", required=True)
    audit.add_argument("--verify-hashes", action="store_true")
    audit.set_defaults(func=command_audit_run)
    return parser


def main() -> int:
    parser = build_parser()
    args = parser.parse_args()
    try:
        return int(args.func(args))
    except ContractError as exc:
        print(f"ERROR: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
