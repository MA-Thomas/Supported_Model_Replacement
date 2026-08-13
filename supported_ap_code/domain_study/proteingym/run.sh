#!/bin/sh
set -eu

mode="${1:-quick}"
study_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
crate_dir=$(CDPATH= cd -- "$study_dir/../.." && pwd)
python_bin="$study_dir/../.venv/bin/python"

if [ ! -x "$python_bin" ]; then
  echo "missing study environment: $python_bin" >&2
  echo "create it using the instructions in $study_dir/README.md" >&2
  exit 1
fi

if [ "$mode" = "download" ]; then
  "$python_bin" "$study_dir/proteingym_story.py" --download-only
elif [ "$mode" = "render" ]; then
  "$python_bin" "$study_dir/proteingym_story.py" \
    --profile publication \
    --cli "$crate_dir/target/release/supported_ap" \
    --render-only
elif [ "$mode" = "rust" ]; then
  cd "$crate_dir"
  cargo build --release --offline
  "$python_bin" "$study_dir/proteingym_story.py" \
    --profile publication \
    --cli "$crate_dir/target/release/supported_ap" \
    --rust-only
elif [ "$mode" = "auroc" ]; then
  cd "$crate_dir"
  cargo build --release --offline
  "$python_bin" "$study_dir/proteingym_story.py" \
    --profile publication \
    --cli "$crate_dir/target/release/supported_ap" \
    --auroc-only
else
  cd "$crate_dir"
  cargo build --release --offline
  "$python_bin" "$study_dir/proteingym_story.py" \
    --profile "$mode" \
    --cli "$crate_dir/target/release/supported_ap"
fi
