#!/bin/sh
set -eu

profile="${1:-extensive}"
crate_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
venv="$crate_root/.venv-python-tests"

if [ ! -x "$venv/bin/python" ]; then
    python3 -m venv "$venv"
fi

"$venv/bin/python" -m pip install --quiet -r "$crate_root/python_tests/requirements.txt"
cargo build --quiet --manifest-path "$crate_root/Cargo.toml" --bin supported_ap_metrics
"$venv/bin/python" -m pytest "$crate_root/python_tests" --profile "$profile"
