#!/bin/sh
set -eu
mode="${1:-publication}"
study_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
crate_dir=$(CDPATH= cd -- "$study_dir/../.." && pwd)
python_bin="$crate_dir/domain_study/.venv/bin/python"
if [ ! -x "$python_bin" ]; then
  echo "Create the parent domain_study/.venv environment first." >&2
  exit 1
fi
if [ "$#" -gt 0 ]; then shift; fi
case "$mode" in
  render) exec "$python_bin" -B "$study_dir/auroc_story.py" --profile publication --render-only "$@" ;;
  quick|publication)
    cargo build --manifest-path "$crate_dir/Cargo.toml" --release --example proteingym_auroc_contexts --offline
    exec "$python_bin" -B "$study_dir/auroc_story.py" --profile "$mode" "$@" ;;
  *) echo "Usage: sh run.sh [publication|quick|render] [driver arguments]" >&2; exit 2 ;;
esac
