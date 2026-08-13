#!/bin/sh
set -eu

mode="${1:-quick}"
study_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
crate_dir=$(CDPATH= cd -- "$study_dir/.." && pwd)

cd "$crate_dir"
cargo build --release --offline
if [ "$mode" = "render" ]; then
  "$study_dir/.venv/bin/python" "$study_dir/banking_story.py" \
    --profile publication \
    --cli "$crate_dir/target/release/supported_ap" \
    --render-only
elif [ "$mode" = "auroc" ]; then
  "$study_dir/.venv/bin/python" "$study_dir/banking_story.py" \
    --profile publication \
    --cli "$crate_dir/target/release/supported_ap" \
    --auroc-only
else
  "$study_dir/.venv/bin/python" "$study_dir/banking_story.py" \
    --profile "$mode" \
    --cli "$crate_dir/target/release/supported_ap"
fi
