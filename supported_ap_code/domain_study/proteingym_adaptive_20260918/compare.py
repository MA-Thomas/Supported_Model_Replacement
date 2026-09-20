#!/usr/bin/env python3
"""Validate replayed traces and compare decisions, numerical values and graphs."""
import argparse
import csv
import hashlib
import json
import math
from pathlib import Path
from collections import Counter

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
STAGES = ("observed", "prefix", "full")


def load(path):
    return json.loads(path.read_text())


def save(path, obj):
    path.write_text(json.dumps(obj, indent=2, allow_nan=False) + "\n")


def table(path, rows, fields):
    with path.open("w", newline="") as f:
        writer = csv.DictWriter(f, fields)
        writer.writeheader()
        writer.writerows(rows)


def retained_upper(effect):
    return effect.get("search", {}).get("upper_bound", effect["value"])


def independent_summary(anchor, effects, parameters):
    k = parameters["computational_order"]
    n = len(effects)
    denominator = math.comb(n, k)
    ordered = sorted(effects)
    magnitude = math.fsum(min(anchor, value) * math.comb(n-i-1, k-1) / denominator
                          for i, value in enumerate(ordered[:n-k+1]))
    successes = sum(value > parameters["floor"] for value in effects)
    survival = math.comb(successes, k) / denominator if successes >= k and anchor > parameters["floor"] else 0.0
    return magnitude, survival


def summary(direction, stage):
    return direction["observed_gate"] if stage == "observed" else direction[stage]


def status(direction, stage):
    current = summary(direction, stage) or direction["observed_gate"]
    bounds = current.get("search")
    return bounds["verdict"] if bounds else ("verified_pass" if current["supported"] else "verified_failure")


def validate(report, plan, old):
    assert report["run_id"] == plan["run_id"]
    for field in ["context_id", "pair_index", "observation_count", "positive_count", "position_count", "prefix_replications"]:
        assert report[field] == old[field], (field, report["context_id"])
    p = plan["parameters"]
    for name in ["forward", "reverse"]:
        direction = report[name]
        for key in ["candidate", "incumbent"]:
            assert direction[key] == old[name][key]
        for effect in [direction["observed"]] + direction["computational"]:
            assert math.isfinite(effect["value"])
            if effect.get("search"):
                certificate = effect["search"]
                assert certificate["lower_bound"] == effect["value"]
                assert effect["value"] <= certificate["sampled_value"] <= certificate["upper_bound"]
                assert certificate["evaluations"] <= p["grid_points"] + p["max_iterations"]
        gate = direction["observed_gate"]
        assert gate["magnitude"] == direction["observed"]["value"]
        assert gate["supported"] == (gate["magnitude"] > p["delta"] and gate["survival"] > p["gamma"])
        if gate["supported"]:
            assert len(direction["computational"]) == p["replications"]
            for stage, count in [("full", p["replications"]), ("prefix", p["replications"] // 2)]:
                effects = direction["computational"][:count]
                low = independent_summary(direction["observed"]["value"], [e["value"] for e in effects], p)
                high = independent_summary(retained_upper(direction["observed"]), [retained_upper(e) for e in effects], p)
                result = direction[stage]
                for actual, expected in zip([result["magnitude"], result["survival"]], low):
                    assert abs(actual - expected) < 1e-10, (report["context_id"], stage, actual, expected)
                if result.get("search"):
                    bounds = result["search"]
                    assert bounds["supported_magnitude_lower"] <= low[0] + 1e-12
                    assert bounds["supported_magnitude_upper"] >= high[0] - 1e-12
                    assert bounds["literal_survival_lower"] <= low[1] + 1e-12
                    assert bounds["literal_survival_upper"] >= high[1] - 1e-12
                    passing = bounds["supported_magnitude_lower"] > p["delta"] and bounds["literal_survival_lower"] > p["gamma"]
                    failing = bounds["supported_magnitude_upper"] <= p["delta"] or bounds["literal_survival_upper"] <= p["gamma"]
                    expected_status = "verified_pass" if passing else "verified_failure" if failing else "unresolved"
                    assert bounds["verdict"] == expected_status
                    assert result["supported"] == passing
                else:
                    assert result["supported"] == (low[0] > p["delta"] and low[1] > p["gamma"])
            assert direction["final_supported"] == direction["full"]["supported"]
        else:
            assert not direction["computational"] and direction["full"] is None and direction["prefix"] is None
            assert not direction["final_supported"]


def survivors(models, edges):
    return set(models) - {b for a, b in edges}


def reductions(models, graphs):
    context_sets = {c: survivors(models, edges) for c, edges in graphs.items()}
    common = set.intersection(*graphs.values())
    return dict(
        supported_edges=sum(map(len, graphs.values())),
        candidate_conservative=sorted(set.intersection(*context_sets.values())),
        replacement_conservative=sorted(survivors(models, common)),
        common_edges=[list(edge) for edge in sorted(common)],
        contexts_admissible={m: sum(m in values for values in context_sets.values()) for m in models},
    ), context_sets


def compare(metric):
    output = HERE / metric
    plan = load(output / "plan.json")
    baseline_dir = ROOT / plan["baseline_directory"]
    old_summary = load(baseline_dir / "summary.json")
    expected = {f"context-{i:03}-pair-{j}.json" for i in range(len(plan["context_ids"])) for j in range(3)}
    assert {p.name for p in (output / "reports").glob("*.json")} == expected, "incomplete report set"
    models = plan["models"]
    old_graphs = {s: {c: set() for c in plan["context_ids"]} for s in STAGES}
    new_graphs = {s: {c: set() for c in plan["context_ids"]} for s in STAGES}
    possible_graphs = {s: {c: set() for c in plan["context_ids"]} for s in STAGES}
    decisions = {s: Counter() for s in STAGES}
    changes, numeric, contexts = [], [], []
    stops = Counter()
    maximum_gaps = Counter()
    checked_effects = 0
    same_draw_counts = 0
    observed_abs, computational_abs, sampled_abs = [], [], []
    report_hashes = {}
    for filename in sorted(expected):
        path = output / "reports" / filename
        new = load(path)
        old = load(baseline_dir / "reports" / filename)
        validate(new, plan, old)
        report_hashes[filename] = hashlib.sha256(path.read_bytes()).hexdigest()
        for orientation in ["forward", "reverse"]:
            a, b = old[orientation], new[orientation]
            edge = (b["candidate"], b["incumbent"])
            context = new["context_id"]
            for stage in STAGES:
                old_value = summary(a, stage)
                new_value = summary(b, stage)
                was = bool(old_value and old_value["supported"])
                now = bool(new_value and new_value["supported"])
                new_status = status(b, stage)
                decisions[stage][new_status] += 1
                if was: old_graphs[stage][context].add(edge)
                if now: new_graphs[stage][context].add(edge)
                if now or new_status == "unresolved": possible_graphs[stage][context].add(edge)
                if was != now or new_status == "unresolved":
                    changes.append(dict(context=context, candidate=edge[0], incumbent=edge[1], stage=stage,
                                        old_supported=was, new_supported=now, new_status=new_status))
                if old_value and new_value:
                    numeric.append(dict(context=context, candidate=edge[0], incumbent=edge[1], stage=stage,
                                        old_magnitude=old_value["magnitude"], new_magnitude_lower=new_value["magnitude"],
                                        magnitude_change=new_value["magnitude"]-old_value["magnitude"],
                                        old_survival=old_value["survival"], new_survival_lower=new_value["survival"],
                                        survival_change=new_value["survival"]-old_value["survival"]))
            observed_abs.append(abs(b["observed"]["value"]-a["observed"]["value"]))
            if len(a["computational"]) == len(b["computational"]): same_draw_counts += 1
            for role, pairs in [("observed", [(a["observed"], b["observed"])]),
                                ("computational", list(zip(a["computational"],b["computational"])) )]:
                for before, after in pairs:
                    if role == "computational": computational_abs.append(abs(after["value"]-before["value"]))
                    if after.get("search"):
                        certificate = after["search"]
                        stops[(role, certificate["stop_reason"])] += 1
                        maximum_gaps[role] = max(maximum_gaps[role], certificate["upper_bound"]-certificate["lower_bound"])
                        sampled_abs.append(abs(certificate["sampled_value"]-before["value"]))
                        checked_effects += 1
    results = {}
    for stage in STAGES:
        previous, old_sets = reductions(models,old_graphs[stage])
        current, new_sets = reductions(models,new_graphs[stage])
        possible, definite_sets = reductions(models,possible_graphs[stage])
        assert previous == old_summary["reductions"][stage], (metric,stage,"baseline graph mismatch")
        changed = [c for c in plan["context_ids"] if old_sets[c] != new_sets[c]]
        results[stage] = dict(old=previous,new=current,decision_counts=dict(decisions[stage]),
                              context_survivor_changes=changed,
                              definite_survivors_if_unresolved_edges_pass=possible,
                              graph_result_is_resolved=decisions[stage]["unresolved"] == 0)
        for c in plan["context_ids"]:
            contexts.append(dict(context=c,stage=stage,old_survivors=";".join(sorted(old_sets[c])),
                                 new_possible_survivors=";".join(sorted(new_sets[c])),
                                 new_definite_survivors=";".join(sorted(definite_sets[c]))))
    maxima = {stage: {field: max((abs(r[field]) for r in numeric if r["stage"] == stage),default=0.0)
                      for field in ["magnitude_change","survival_change"]} for stage in STAGES}
    result = dict(metric=metric,old_run_id=plan["baseline_run_id"],new_run_id=plan["run_id"],
                  context_count=len(plan["context_ids"]),directed_assessments=len(expected)*2,
                  inputs_and_scientific_parameters_unchanged=True,
                  parameter_changes={k: dict(old=plan["baseline_parameters"][k], new=v)
                      for k,v in plan["parameters"].items() if plan["baseline_parameters"][k] != v},
                  graphs=results,
                  decision_changes=changes,comparison_rows=len(numeric),
                  maximum_absolute_observed_effect_change=max(observed_abs,default=0.0),
                  maximum_absolute_computational_effect_change=max(computational_abs,default=0.0),
                  maximum_absolute_sampled_effect_change=max(sampled_abs,default=0.0),
                  maximum_absolute_summary_changes=maxima,
                  directions_with_same_retained_draw_count=same_draw_counts,
                  search_stop_counts={role+":"+stop:n for (role,stop),n in stops.items()},
                  maximum_search_gap=dict(maximum_gaps),validated_search_certificates=checked_effects)
    save(output / "comparison.json",result)
    save(output / "report_hashes.json",report_hashes)
    table(output / "decision_changes.csv",changes,["context","candidate","incumbent","stage","old_supported","new_supported","new_status"])
    table(output / "numeric_changes.csv",numeric,list(numeric[0]))
    table(output / "context_survivors.csv",contexts,list(contexts[0]))
    print(json.dumps({"metric":metric,"decision_changes":len(changes),"full":results["full"],"maximum_absolute_summary_changes":maxima},indent=2))
    return result


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("metrics",nargs="*",default=["cnap","auroc"])
    args = parser.parse_args()
    for metric in args.metrics:
        compare(metric)
