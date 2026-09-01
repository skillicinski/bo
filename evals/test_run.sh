#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
export UV_CACHE_DIR="$repo_root/tmp/uv-cache"
exec uv run --project "$repo_root/evals" python -m unittest discover -s "$repo_root/evals" -p 'test_*.py'
