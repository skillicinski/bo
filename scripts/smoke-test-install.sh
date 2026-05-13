#!/usr/bin/env bash
# Smoke test: verify bo works as an installed binary outside the repo.
#
# Simulates a fresh user by:
#   1. Installing from the local repo into a temp prefix
#   2. Running commands from a non-repo directory with a temp HOME
#   3. Verifying core commands succeed without repo-relative files
#
# Usage:
#   ./scripts/smoke-test-install.sh
#
# For CI (approximates --git --tag install):
#   ./scripts/smoke-test-install.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

# Create isolated directories
INSTALL_ROOT="$(mktemp -d)"
FAKE_HOME="$(mktemp -d)"
WORK_DIR="$(mktemp -d)"

cleanup() {
    rm -rf "$INSTALL_ROOT" "$FAKE_HOME" "$WORK_DIR"
}
trap cleanup EXIT

echo "=== Installing bo from repo into $INSTALL_ROOT ==="
cargo install --path "$REPO_DIR" --locked --root "$INSTALL_ROOT" --quiet

BO="$INSTALL_ROOT/bin/bo"

if [[ ! -x "$BO" ]]; then
    echo "FAIL: bo binary not found at $BO"
    exit 1
fi

echo "=== Running smoke tests from $WORK_DIR with HOME=$FAKE_HOME ==="
cd "$WORK_DIR"

# 1. --help works
echo -n "  bo --help ... "
HOME="$FAKE_HOME" "$BO" --help > /dev/null 2>&1
echo "OK"

# 2. config get model (no seed required)
echo -n "  bo config get model ... "
MODEL=$(HOME="$FAKE_HOME" "$BO" config get model 2>/dev/null)
if [[ "$MODEL" != "gpt-4o" ]]; then
    echo "FAIL: expected 'gpt-4o', got '$MODEL'"
    exit 1
fi
echo "OK (default: $MODEL)"

# 3. seed into a tree
TREE_DIR="$FAKE_HOME/test-tree"
echo -n "  bo seed $TREE_DIR ... "
HOME="$FAKE_HOME" "$BO" seed "$TREE_DIR" > /dev/null 2>&1
echo "OK"

# 4. list (empty tree)
echo -n "  bo list (empty) ... "
HOME="$FAKE_HOME" "$BO" list > /dev/null 2>&1
echo "OK"

# 5. config set model
echo -n "  bo config set model gpt-4.1-mini ... "
HOME="$FAKE_HOME" "$BO" config set model gpt-4.1-mini > /dev/null 2>&1
echo "OK"

# 6. config get model (after set)
echo -n "  bo config get model (after set) ... "
MODEL=$(HOME="$FAKE_HOME" "$BO" config get model 2>/dev/null)
if [[ "$MODEL" != "gpt-4.1-mini" ]]; then
    echo "FAIL: expected 'gpt-4.1-mini', got '$MODEL'"
    exit 1
fi
echo "OK ($MODEL)"

# 7. JSON output works
echo -n "  bo --json list ... "
JSON=$(HOME="$FAKE_HOME" "$BO" --json list 2>/dev/null)
if ! echo "$JSON" | python3 -c "import sys,json; json.load(sys.stdin)" 2>/dev/null; then
    echo "FAIL: invalid JSON output"
    exit 1
fi
echo "OK"

# 8. search (no results, but shouldn't crash)
echo -n "  bo search nonexistent (exits 1, no crash) ... "
if HOME="$FAKE_HOME" "$BO" search nonexistent > /dev/null 2>&1; then
    echo "FAIL: expected exit 1"
    exit 1
fi
echo "OK (exit 1 as expected)"

# 9. Verify no .env or repo files needed
echo -n "  no repo-relative files required ... "
if [[ -f "$WORK_DIR/.env" ]] || [[ -f "$FAKE_HOME/.env" ]]; then
    echo "FAIL: .env file found"
    exit 1
fi
echo "OK"

echo ""
echo "=== All smoke tests passed ==="
echo "  Install root: $INSTALL_ROOT"
echo "  Binary size: $(du -h "$BO" | cut -f1)"
