#!/bin/bash
# Submit the portable, distributed, two-stage NCI pipeline from the login node.
set -euo pipefail
SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
exec python3 "${SCRIPT_DIR}/cluster_pipeline.py" submit "$@"
