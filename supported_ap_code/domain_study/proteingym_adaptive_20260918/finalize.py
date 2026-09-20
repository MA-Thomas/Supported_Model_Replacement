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
        for field in ["sources", "models", "context_ids"]:
            assert previous[field] == plan[field]
        expected_parameters = dict(previous["parameters"])
        if metric == "cnap":
            expected_parameters["grid_points"] = 3
        assert plan["parameters"] == expected_parameters
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

    timing = {}
    for metric in results:
        plan = load(HERE / metric / "plan.json")
        execution = load(HERE / metric / "execution.json")
        baseline = ROOT / plan["baseline_directory"]
        declared = datetime.fromisoformat(load(baseline / "plan.json")["declared_at_utc"]).timestamp()
        old_end = max(p.stat().st_mtime for p in (baseline / "reports").glob("*.json"))
        old_seconds = old_end - declared
        timing[metric] = dict(
            yesterday_estimated_compute_seconds=old_seconds,
            yesterday_estimation_method="plan declaration timestamp to last Rust report modification timestamp; excludes subsequent plots",
            adaptive_wall_seconds=execution["wall_seconds"],
            adaptive_timing_method="time.perf_counter around subprocess launch and wait, without reused reports",
            old_over_new_ratio=old_seconds / execution["wall_seconds"],
            adaptive_rayon_threads=execution["rayon_threads"],
            baseline_thread_count="not recorded",
        )
    (HERE / "timing.json").write_text(json.dumps(timing, indent=2) + "\n")
    lines = [
        "# ProteinGym replay with adaptive defaults — September 18, 2026", "",
        "Repeated both completed September 17 tournaments using the intended defaults of the updated Rust search. "
        "CNAP now starts with three prevalence points and adaptively subdivides, with an objective-gap tolerance of 1e-8 and a budget of 128 shared bisections. "
        "The search CLI options were omitted so the runner used the library defaults. "
        "The input data, 91 assays, three models, seed (20260917), 200 position-bootstrap draws, prevalence interval, support order, and decision thresholds were preserved.", "",
        "## Runtime", "",
        "| Metric | Yesterday, estimated compute seconds | Adaptive run, measured seconds | Yesterday / adaptive |",
        "| --- | ---: | ---: | ---: |",
    ]
    for metric, t in timing.items():
        lines.append(f"| {metric.upper()} | {t['yesterday_estimated_compute_seconds']:.3f} | {t['adaptive_wall_seconds']:.3f} | {t['old_over_new_ratio']:.3f} |")
    lines += ["", "The new runs executed sequentially, with no forced thread limit; both used the automatic Rayon pool. "
              "Elapsed time was measured separately for each process and excludes compilation, post-run validation, and plot rendering. "
              "Yesterday's estimates use plan creation and last report timestamps; its thread count and background machine load were not recorded. "
              "These are observed run-time comparisons, not a controlled estimate of algorithm-only speedup.", "",
              "## Decisions", "",
              "| Metric | Supported final edges: yesterday → adaptive | Changed/unresolved assessments across stages | Unresolved final decisions |",
              "| --- | ---: | ---: | ---: |"]
    names = {"score_eve_ensemble": "EVE", "score_esm1v_ensemble": "ESM-1v", "score_esm2_650m": "ESM-2 650M"}
    for metric, result in results.items():
        full = result["graphs"]["full"]
        lines.append(f"| {metric.upper()} | {full['old']['supported_edges']} → {full['new']['supported_edges']} | {len(result['decision_changes'])} | {full['decision_counts'].get('unresolved', 0)} |")
    lines += ["", "| Metric | Model | Admissible assays: yesterday → adaptive |", "| --- | --- | ---: |"]
    for metric, result in results.items():
        full = result["graphs"]["full"]
        for model, name in names.items():
            lines.append(f"| {metric.upper()} | {name} | {full['old']['contexts_admissible'][model]} → {full['new']['contexts_admissible'][model]} |")
    for metric, result in results.items():
        full = result["graphs"]["full"]
        lines += ["", f"{metric.upper()} final assay survivor-set changes: {len(full['context_survivor_changes'])}. "
                  f"Candidate-conservative survivors: {[names[m] for m in full['new']['candidate_conservative']]}; "
                  f"replacement-conservative survivors: {[names[m] for m in full['new']['replacement_conservative']]}", ""]
    lines += ["## Numerical differences", "",
              "| Metric | Maximum observed effect change | Maximum bootstrap effect change | Maximum final magnitude change | Maximum final survival change |",
              "| --- | ---: | ---: | ---: | ---: |"]
    for metric, result in results.items():
        changes = result["maximum_absolute_summary_changes"]["full"]
        lines.append(f"| {metric.upper()} | {result['maximum_absolute_observed_effect_change']:.12g} | {result['maximum_absolute_computational_effect_change']:.12g} | {changes['magnitude_change']:.12g} | {changes['survival_change']:.12g} |")
    cnap = results["cnap"]
    exhausted = sum(n for key,n in cnap["search_stop_counts"].items() if key.endswith(":budget_exhausted"))
    lines += ["", f"Of {cnap['validated_search_certificates']:,} retained CNAP search certificates, {exhausted:,} exhausted their numerical search budget. "
              "Budget-limited effects carry bounds through to the final policy, which can still resolve a decision if both bounds agree. "
              f"Maximum difference between the newly sampled effects and yesterday's values: {cnap['maximum_absolute_sampled_effect_change']:.12g}.", "",
              "## Verification", "",
              "Both processes exited successfully. All 546 report files were validated; bootstrap summaries were independently recomputed "
              "and tournament graphs reconstructed. Inputs, original numerical outputs, and the executed source/binary hashes "
              "were verified again after completion. Original result directories and the earlier 257-point replay are preserved. "
              "Machine-readable results, timings, individual decision/value comparisons, commands, logs, and raw reports accompany this report.", ""]
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
