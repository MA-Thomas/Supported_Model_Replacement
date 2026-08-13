#!/bin/sh
set -eu

mode="${1:-quick}"
study_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
crate_dir=$(CDPATH= cd -- "$study_dir/../.." && pwd)
python_bin="$study_dir/../.venv/bin/python"
example_bin="$crate_dir/target/release/examples/proteingym_nested"

if [ ! -x "$python_bin" ]; then
  echo "missing study environment: $python_bin" >&2
  echo "create it using the parent ProteinGym study instructions" >&2
  exit 1
fi

if [ "$mode" = "render" ]; then
  "$python_bin" "$study_dir/conditional_nested_story.py" \
    --profile publication \
    --example "$example_bin" \
    --render-only
else
  cd "$crate_dir"
  cargo build --release --example proteingym_nested --offline
  "$python_bin" "$study_dir/conditional_nested_story.py" \
    --profile "$mode" \
    --example "$example_bin"
fi
