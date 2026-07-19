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

# 2. config --model (no seed required, default model accepted)
echo -n "  bo config --model gpt-4.1-mini ... "
HOME="$FAKE_HOME" "$BO" config --model gpt-4.1-mini > /dev/null 2>&1
echo "OK"

# 3. seed into a tree
TREE_DIR="$FAKE_HOME/test-tree"
STATE="$TREE_DIR/.bo/state.json"
echo -n "  bo seed --path $TREE_DIR --name test-tree --provider openai --model gpt-4.1-mini ... "
HOME="$FAKE_HOME" "$BO" seed \
    --path "$TREE_DIR" \
    --name test-tree \
    --provider openai \
    --model gpt-4.1-mini > /dev/null 2>&1
echo "OK"

# 4. state absent immediately after seed
echo -n "  state absent after seed ... "
if [[ -f "$STATE" ]]; then
    echo "FAIL: state exists immediately after seed"
    exit 1
fi
echo "OK"

# 5. list (empty tree)
echo -n "  bo list (empty) ... "
HOME="$FAKE_HOME" "$BO" list > /dev/null 2>&1
echo "OK"

# 6. status shows model after config
echo -n "  bo status shows model ... "
STATUS=$(HOME="$FAKE_HOME" "$BO" status 2>/dev/null)
if ! echo "$STATUS" | grep -q "gpt-4.1-mini"; then
    echo "FAIL: expected gpt-4.1-mini in status output"
    exit 1
fi
echo "OK"

# 7. JSON output works
echo -n "  bo --json list ... "
JSON=$(HOME="$FAKE_HOME" "$BO" --json list 2>/dev/null)
if ! echo "$JSON" | python3 -c "import sys,json; json.load(sys.stdin)" 2>/dev/null; then
    echo "FAIL: invalid JSON output"
    exit 1
fi
echo "OK"

# 8. show nonexistent (exits 1, no crash)
echo -n "  bo show nonexistent (exits 1, no crash) ... "
if HOME="$FAKE_HOME" "$BO" show nonexistent > /dev/null 2>&1; then
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

# 10. collect a local markdown note
echo -n "  bo collect local note ... "
cat > "$WORK_DIR/note.md" << 'EOF'
# Smoke Test Note
This is a test note to verify local collect works.
EOF
if ! HOME="$FAKE_HOME" "$BO" collect "$WORK_DIR/note.md" > /dev/null 2>&1; then
    echo "FAIL: collect local note failed"
    exit 1
fi
echo "OK"

# 11. state has one bo://note/ leaf
echo -n "  state has one bo://note/ leaf ... "
if [[ ! -f "$STATE" ]]; then
    echo "FAIL: state not created after collect"
    exit 1
fi
if ! python3 -c '
import sys, json
with open(sys.argv[1]) as f:
    data = json.load(f)
assert len(data["leaves"]) == 1
assert data["leaves"][0]["url"].startswith("bo://note/")
' "$STATE" 2>/dev/null; then
    echo "FAIL: state did not validate"
    exit 1
fi
echo "OK"

echo ""
echo "=== All smoke tests passed ==="
echo "  Install root: $INSTALL_ROOT"
echo "  Binary size: $(du -h "$BO" | cut -f1)"
