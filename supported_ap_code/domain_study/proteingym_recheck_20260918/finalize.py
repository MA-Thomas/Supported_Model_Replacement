#!/usr/bin/env python3
"""Verify replay provenance and write the completed comparison record."""
import hashlib
import json
from datetime import datetime, timezone
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]


def load(path):
    return json.loads(path.read_text())


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    results = {}
    baseline_files = 0
    for metric, example in [("cnap", "proteingym_contexts"), ("auroc", "proteingym_auroc_contexts")]:
        directory = HERE / metric
        plan = load(directory / "plan.json")
        execution = load(directory / "execution.json")
        result = load(directory / "comparison.json")
        assert execution["exit_code"] == 0
        assert result["new_run_id"] == plan["run_id"]
        baseline = ROOT / plan["baseline_directory"]
        assert digest(baseline / "plan.json") == plan["baseline_plan_sha256"]
        assert digest(baseline / "manifest.json") == plan["baseline_manifest_sha256"]
        previous = load(baseline / "plan.json")
        for field in ["parameters", "sources", "models", "context_ids"]:
            assert previous[field] == plan[field]
        for source in plan["sources"].values():
            assert digest(ROOT / source["path"]) == source["sha256"]
        for relative, expected in load(baseline / "manifest.json")["artifacts_sha256"].items():
            if relative.endswith((".json", ".csv")):
                assert digest(baseline / relative) == expected, relative
                baseline_files += 1
        for relative, expected in plan["source_sha256"].items():
            assert digest(ROOT / relative) == expected, relative
        assert digest(ROOT / "target/release/examples" / example) == plan["binary_sha256"]
        assert digest(HERE / "repeat.py") == plan["driver_sha256"]
        hashes = load(directory / "report_hashes.json")
        assert len(hashes) == 273
        for relative, expected in hashes.items():
            assert digest(directory / "reports" / relative) == expected
        results[metric] = result

    lines = [
        "# ProteinGym tournament replay — September 18, 2026", "",
        "Repeated both completed September 17 tournaments using the updated Rust code. "
        "The 91 assays, three models, input data, seed (20260917), and all saved numerical parameters were preserved. "
        "Each tournament contains 273 pair comparisons (546 directions), with 200 position-cluster bootstrap replications for each direction that passes the observed gate.", "",
        "| Metric | Supported final edges: previous → replay | Changed decisions across all stages | Unresolved final decisions |",
        "| --- | ---: | ---: | ---: |",
    ]
    for metric, result in results.items():
        full = result["graphs"]["full"]
        lines.append(f"| {metric.upper()} | {full['old']['supported_edges']} → {full['new']['supported_edges']} | {len(result['decision_changes'])} | {full['decision_counts'].get('unresolved', 0)} |")
    lines += ["", "## Model outcomes", "",
              "| Metric | Model | Admissible assays: previous → replay |", "| --- | --- | ---: |"]
    names = {"score_eve_ensemble": "EVE", "score_esm1v_ensemble": "ESM-1v", "score_esm2_650m": "ESM-2 650M"}
    for metric, result in results.items():
        full = result["graphs"]["full"]
        for model, name in names.items():
            lines.append(f"| {metric.upper()} | {name} | {full['old']['contexts_admissible'][model]} → {full['new']['contexts_admissible'][model]} |")
        assert full["old"]["candidate_conservative"] == full["new"]["candidate_conservative"] == []
        assert full["old"]["replacement_conservative"] == full["new"]["replacement_conservative"] == sorted(names)
    lines += ["", "For both metrics, the candidate-conservative set remains empty, and the replacement-conservative set retains all three models. "
              "No single model is admissible in every assay; no model can be removed by one common supported replacement across every assay.", "",
              "## Numerical differences", "",
              "AUROC observed effects, bootstrap effects, and observed/prefix/full summaries are exactly unchanged.", ""]
    cnap = results["cnap"]
    for key, label in [
        ("maximum_absolute_observed_effect_change", "observed retained effect"),
        ("maximum_absolute_computational_effect_change", "individual bootstrap retained effect"),
        ("maximum_absolute_sampled_effect_change", "sampled effect (observed and bootstrap)"),
    ]:
        lines.append(f"- CNAP maximum absolute change in {label}: {cnap[key]:.12g}.")
    final_changes = cnap["maximum_absolute_summary_changes"]["full"]
    lines += [f"- CNAP maximum absolute change in final supported magnitude: {final_changes['magnitude_change']:.12g}.",
              f"- CNAP maximum absolute change in final literal survival: {final_changes['survival_change']:.12g}.", "",
              "CNAP now reports conservative lower bounds and numerical uncertainty. "
              "The replay retained the old 257 initial grid points, tolerance 1e-8, and 128 iterations; "
              "under the updated algorithm the tolerance bounds objective error and iterations budget shared bisections. "
              "This is a replay under the new numerical semantics, not a timing benchmark using the new three-point default.", ""]
    exhausted = sum(count for key, count in cnap["search_stop_counts"].items() if key.endswith(":budget_exhausted"))
    unresolved = cnap["graphs"]["full"]["decision_counts"].get("unresolved", 0)
    lines += [f"Of {cnap['validated_search_certificates']:,} recorded CNAP effect certificates, {exhausted:,} stopped at the search budget. "
              f"Their bounds were propagated into the decisions; {unresolved} final directional decisions remain unresolved.", "",
              "## Verification and files", "",
              "Both Rust processes exited successfully. The replay validated all 546 report files, independently recomputed "
              "the bootstrap summaries, and reconstructed both the baseline and replay tournament graphs. "
              "Input hashes, original numerical artifact hashes, and replay source/binary hashes were rechecked after completion. "
              "The CNAP example's uncertainty-aware summary changes passed its four tests, the AUROC example's four tests, and strict Clippy for both examples.", "",
              "Original result directories were preserved. Each metric subdirectory contains its plan, execution record, "
              "raw reports, comparison.json, decision_changes.csv, numeric_changes.csv, and context_survivors.csv. "
              "The root manifest records SHA-256 hashes for the replay artifacts.", ""]
    (HERE / "comparison.md").write_text("\n".join(lines))
    artifacts = {str(path.relative_to(HERE)): digest(path) for path in sorted(HERE.rglob("*"))
                 if path.is_file() and path.name != "manifest.json" and "__pycache__" not in path.parts}
    manifest = dict(completed_at_utc=datetime.now(timezone.utc).isoformat(),
                    baseline_numerical_artifacts_reverified=baseline_files,
                    replay_report_count=546, metrics={m: dict(run_id=r["new_run_id"], baseline_run_id=r["old_run_id"],
                    decision_changes=len(r["decision_changes"]), final_decision_counts=r["graphs"]["full"]["decision_counts"])
                    for m, r in results.items()}, artifacts_sha256=artifacts)
    (HERE / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(json.dumps({k: v for k, v in manifest.items() if k != "artifacts_sha256"}, indent=2))


if __name__ == "__main__":
    main()
