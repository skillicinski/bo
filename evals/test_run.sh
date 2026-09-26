#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
exec python3 -m unittest discover -s "$repo_root/evals" -p 'test_*.py'
