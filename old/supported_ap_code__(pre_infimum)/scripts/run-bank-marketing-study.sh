#!/bin/sh
set -eu

profile="${1:-publication}"
crate_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
venv="$crate_root/.venv-python-tests"

if [ ! -x "$venv/bin/python" ]; then
    python3 -m venv "$venv"
fi

"$venv/bin/python" -m pip install --quiet \
    -r "$crate_root/python_tests/requirements.txt" \
    -r "$crate_root/domain_study/requirements.txt"
cargo build --quiet --release --features parallel \
    --manifest-path "$crate_root/Cargo.toml" \
    --bin supported_ap_metrics
"$venv/bin/python" "$crate_root/domain_study/story_study.py" \
    --profile "$profile"
