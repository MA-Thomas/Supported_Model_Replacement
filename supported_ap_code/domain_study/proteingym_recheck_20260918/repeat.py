#!/usr/bin/env python3
"""Replay the two saved September 17 plans into independent output directories."""
import hashlib
import json
import os
from pathlib import Path
import platform
import subprocess
from datetime import datetime, timezone

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
STUDIES = {"cnap": "proteingym_contexts", "auroc": "proteingym_auroc_contexts"}


def digest(path):
    h = hashlib.sha256()
    with path.open("rb") as f:
        for block in iter(lambda: f.read(1048576), b""):
            h.update(block)
    return h.hexdigest()


def save(path, obj):
    path.parent.mkdir(parents=True, exist_ok=True)
    temp = path.with_suffix(path.suffix + ".tmp")
    temp.write_text(json.dumps(obj, indent=2, allow_nan=False) + "\n")
    temp.replace(path)


def prepare(metric, study):
    previous = ROOT / "domain_study" / study / "outputs"
    baseline = json.loads((previous / "plan.json").read_text())
    manifest = json.loads((previous / "manifest.json").read_text())
    for source in baseline["sources"].values():
        assert digest(ROOT / source["path"]) == source["sha256"], source
    # Validate numerical baselines without opening documentation or figure files.
    for relative, expected in manifest["artifacts_sha256"].items():
        if relative.endswith((".json", ".csv")):
            assert digest(previous / relative) == expected, relative
    sources = list((ROOT / "src").rglob("*.rs"))
    sources += [ROOT / "Cargo.toml", ROOT / "Cargo.lock", ROOT / "examples" / (study + ".rs")]
    binary = ROOT / "target/release/examples" / study
    declaration = {
        "metric": metric,
        "baseline_directory": str(previous.relative_to(ROOT)),
        "baseline_run_id": baseline["run_id"],
        "baseline_plan_sha256": digest(previous / "plan.json"),
        "baseline_manifest_sha256": digest(previous / "manifest.json"),
        "parameters": baseline["parameters"],
        "context_ids": baseline["context_ids"],
        "models": baseline["models"],
        "sources": baseline["sources"],
        "source_sha256": {str(p.relative_to(ROOT)): digest(p) for p in sorted(sources)},
        "binary_sha256": digest(binary),
        "driver_sha256": digest(Path(__file__)),
        "rustc": subprocess.check_output(["rustc", "--version"], text=True).strip(),
        "platform": platform.platform(),
        "rayon_threads": 6 if metric == "cnap" else 2,
        "numerical_change": "supported-ap 0.3.0: bounded adaptive interval search; tolerance is objective gap, iterations are shared bisections" if metric == "cnap" else "recompiled current code; same AUROC and resampling implementation",
    }
    run_id = hashlib.sha256(json.dumps(declaration, sort_keys=True).encode()).hexdigest()
    output = HERE / metric
    path = output / "plan.json"
    if path.exists():
        assert json.loads(path.read_text())["run_id"] == run_id, "incompatible replay directory"
    else:
        save(path, dict(declaration, run_id=run_id, declared_at_utc=datetime.now(timezone.utc).isoformat()))
    command = [str(binary), "--input", str(ROOT / baseline["sources"]["scores"]["path"]),
               "--output-dir", str(output / "reports"), "--run-id", run_id]
    for key, value in baseline["parameters"].items():
        command += ["--" + key.replace("_", "-"), str(value)]
    return output, command, declaration["rayon_threads"]


def main():
    prepared = [(metric, *prepare(metric, study)) for metric, study in STUDIES.items()]
    running = []
    for metric, output, command, threads in prepared:
        env = dict(os.environ, RAYON_NUM_THREADS=str(threads))
        log = (output / "execution.log").open("a")
        start = datetime.now(timezone.utc).isoformat()
        process = subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT, env=env)
        record = dict(metric=metric, pid=process.pid, command=command, started_at_utc=start)
        save(output / "execution.json", record)
        print(f"Started {metric}: PID {process.pid}, {len(command)} command arguments", flush=True)
        running.append((output, process, log, record))
    failures = []
    for output, process, log, record in running:
        code = process.wait()
        log.close()
        save(output / "execution.json", dict(record, exit_code=code, completed_at_utc=datetime.now(timezone.utc).isoformat()))
        print(f"Finished {record['metric']}: exit {code}", flush=True)
        if code:
            failures.append(record["metric"])
    if failures:
        raise SystemExit("Replay failed: " + ", ".join(failures))


if __name__ == "__main__":
    main()
