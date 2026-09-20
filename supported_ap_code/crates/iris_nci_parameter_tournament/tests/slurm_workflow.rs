//! Exercise real shell dispatch with lightweight stand-ins for the two Rust
//! binaries. Numerical/bundle behavior is covered by rosters.rs and the organizer.
#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Output};

const STUB: &str = r#"#!/usr/bin/env python3
import hashlib, json, os, sys
from pathlib import Path
args = sys.argv[1:]
with open(os.environ['STUB_LOG'], 'a') as f:
    f.write(json.dumps(args) + '\n')
def arg(name): return Path(args[args.index(name) + 1])
def digest(path): return hashlib.sha256(path.read_bytes()).hexdigest()
def write(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value))
models = ['full_hla', 'focal_hla', 'old_monoallelic', 'mono_q_full_pn', 'full_q_mono_pn']
if args[0] == 'prepare':
    root = arg('--output')
    write(root / 'manifest.json', {'config_sha256': digest(arg('--config'))})
    for model in models:
        for metric in ['pr', 'roc']:
            write(root / model / metric / 'bundle' / 'bundle_manifest.json', {'bundle_content_hash': model + metric})
elif args[0] == 'audit-prepared':
    manifest = json.loads((arg('--package') / 'manifest.json').read_text())
    assert manifest['config_sha256'] == digest(arg('--config')), 'wrong config reused'
elif args[:2] == ['accelerated', 'plan']:
    manifest = json.loads((arg('--bundle') / 'bundle_manifest.json').read_text())
    write(arg('--output') / 'accelerated_plan.json', {
        'bundle_content_hash': manifest['bundle_content_hash'],
        'options': {'batch_size': int(str(arg('--batch-size'))), 'output_scope': 'survivor_set_and_operational_inputs'}})
elif args[0] == 'plan':
    write(arg('--output') / 'plan.json', {'shard_count': 2})
elif args[:2] == ['accelerated', 'run']:
    write(arg('--output') / 'selection_certificate.json', {})
elif args[:2] == ['accelerated', 'audit']:
    assert (arg('--selection') / 'selection_certificate.json').is_file()
elif args[0] in ['run-shard', 'audit']:
    pass
elif args[0] == 'reduce':
    write(arg('--output') / 'reduction_manifest.json', {})
elif args[0] == 'audit-reduction':
    assert (arg('--reduction') / 'reduction_manifest.json').is_file()
elif args[0] in ['finalize-version2', 'finalize-version2-accelerated']:
    if args[0].endswith('accelerated'):
        assert (arg('--selection') / 'selection_certificate.json').is_file()
    else:
        assert (arg('--reduction') / 'reduction_manifest.json').is_file()
    write(arg('--output') / 'selection.json', {})
elif args[0] == 'audit-version2':
    assert (arg('--output') / 'selection.json').is_file()
else:
    raise AssertionError('unexpected command: ' + repr(args))
"#;

fn success(output: Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn dispatch(mode: &str) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let script_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../IRIS_scripts/nci_parameter_tournament");
    let stub = root.join("stub");
    fs::write(&stub, STUB).unwrap();
    fs::set_permissions(&stub, fs::Permissions::from_mode(0o755)).unwrap();
    let config = root.join("config.json");
    fs::write(
        &config,
        if mode == "accelerated" {
            "{\"roster_mode\":\"all_regimes\",\"candidate_count\":4200}"
        } else {
            "{\"candidate_count\":20}"
        },
    )
    .unwrap();
    let run_root = root.join("run with spaces");
    let log = root.join("commands.jsonl");
    let submit = |config: &Path, execution: &str, batch: &str| {
        Command::new("bash")
            .arg(script_root.join("submit_nci_parameter_tournaments_slurm.sh"))
            .arg("--run-root")
            .arg(&run_root)
            .arg("--config")
            .arg(config)
            .arg("--pipeline")
            .arg(&stub)
            .arg("--organizer")
            .arg(&stub)
            .args([
                "--execution-mode",
                execution,
                "--batch-size",
                batch,
                "--prepare-only",
            ])
            .env("STUB_LOG", &log)
            .output()
            .unwrap()
    };
    success(submit(&config, mode, "1"));
    let task_map = fs::read_to_string(run_root.join("task_map.tsv")).unwrap();
    let tasks = if mode == "accelerated" { 10 } else { 20 };
    assert_eq!(task_map.lines().count(), tasks);
    if mode == "accelerated" {
        assert!(task_map.lines().all(|line| line.ends_with("\tcoordinator")));
    }
    success(submit(&config, mode, "1"));
    assert_eq!(
        fs::read_to_string(run_root.join("task_map.tsv")).unwrap(),
        task_map
    );
    for task in 0..tasks {
        success(
            Command::new("bash")
                .arg(script_root.join("run_nci_parameter_tournament_array.slurm"))
                .env("NCI_PARAMETER_RUN_ENV", run_root.join("run.env"))
                .env("SLURM_ARRAY_TASK_ID", task.to_string())
                .env("STUB_LOG", &log)
                .output()
                .unwrap(),
        );
    }
    for _ in 0..2 {
        success(
            Command::new("bash")
                .arg(script_root.join("finalize_nci_parameter_tournaments_slurm.sh"))
                .env("NCI_PARAMETER_RUN_ENV", run_root.join("run.env"))
                .env("STUB_LOG", &log)
                .output()
                .unwrap(),
        );
    }
    let entries: Vec<Vec<String>> = fs::read_to_string(&log)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let expected_finalizer = if mode == "accelerated" {
        "finalize-version2-accelerated"
    } else {
        "finalize-version2"
    };
    assert_eq!(
        entries
            .iter()
            .filter(|a| a[0] == expected_finalizer)
            .count(),
        10
    );
    if mode == "accelerated" {
        assert!(
            !entries
                .iter()
                .any(|a| matches!(a[0].as_str(), "run-shard" | "reduce" | "finalize-version2"))
        );
        assert_eq!(
            entries
                .iter()
                .filter(|a| a.starts_with(&["accelerated".into(), "run".into()]))
                .count(),
            10
        );
        assert!(!submit(&config, mode, "2").status.success());
    } else {
        assert!(!entries.iter().any(|a| a[0] == "accelerated"));
        assert_eq!(entries.iter().filter(|a| a[0] == "run-shard").count(), 20);
    }
    let different = root.join("different.json");
    fs::write(&different, "{}").unwrap();
    assert!(!submit(&different, mode, "1").status.success());
    assert!(
        !submit(
            &config,
            if mode == "accelerated" {
                "exhaustive"
            } else {
                "accelerated"
            },
            "1"
        )
        .status
        .success()
    );
}

#[test]
fn accelerated_scripts_dispatch_ten_coordinators_and_preserve_resume_contracts() {
    dispatch("accelerated");
}

#[test]
fn exhaustive_scripts_keep_the_historical_shard_and_finalization_path() {
    dispatch("exhaustive");
}
