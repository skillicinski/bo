#!/bin/sh
set -eu

repo=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
bo=${BO_BIN:-$repo/target/debug/bo}
if [ ! -x "$bo" ]; then
    mkdir -p "$(dirname "$bo")"
    go build -o "$bo" "$repo/cmd/bo"
fi
: "${DEEPSEEK_API_KEY:?DEEPSEEK_API_KEY is required}"

run_id="agent-$(date +%s)-$$"
work="$repo/evals/work/$run_id"
report="$repo/evals/results/$run_id"
home="$work/home"
target="$home/.bo/$run_id"
raw="$report/raw"
mkdir -p "$home" "$report" "$raw"

HOME="$home" "$bo" seed --name "$run_id" >"$report/seed.log"
urls=$(awk 'NF && $1 !~ /^#/ { print $1 }' "$repo/evals/manifest.txt")
set +e
HOME="$home" "$bo" snap "$run_id" $urls >"$report/snap.log" 2>&1
snap_status=$?
set -e

for file in "$target"/*.md; do
    [ -f "$file" ] || continue
    cp "$file" "$raw/$(basename "$file")"
done

hashes() {
    for file in "$target"/*.md; do
        [ -f "$file" ] || continue
        shasum -a 256 "$file"
    done | sort
}

hashes >"$report/raw-before.sha256"
failed=$(awk '/^failed:/{print}' "$report/snap.log" || true)
if [ -n "$failed" ]; then
    printf '%s\n' "$failed" >"$report/expected-failures.log"
else
    : >"$report/expected-failures.log"
fi

set +e
HOME="$home" "$bo" agent "$run_id" >"$report/agent.log" 2>&1
agent_status=$?
set -e

cp "$target/state.json" "$report/state.json"
if [ -d "$target/summaries" ]; then
    mkdir -p "$report/summaries"
    for file in "$target"/summaries/*.md; do
        [ -f "$file" ] || continue
        cp "$file" "$report/summaries/$(basename "$file")"
    done
fi

: >"$report/missing-summaries.log"
python3 - "$target/state.json" "$target" >"$report/missing-summaries.log" <<'PY'
import json
import os
import sys

state_path, target = sys.argv[1:]
with open(state_path, encoding="utf-8") as file:
    state = json.load(file)

raw_records = {record["filename"]: record["url"] for record in state["raw"]}
source_keys = set()
for filename in os.listdir(target):
    if filename.lower().endswith(".md") and os.path.isfile(os.path.join(target, filename)):
        source_keys.add(raw_records.get(filename, f"raw:{filename}"))

summaries = {record["source_key"]: record for record in state["summaries"]}
for source_key in sorted(source_keys):
    record = summaries.get(source_key)
    path = os.path.join(target, "summaries", record["filename"]) if record else ""
    if not record or not os.path.isfile(path) or os.path.getsize(path) == 0:
        print(f"missing summary: {source_key}")
PY
missing=$(wc -l <"$report/missing-summaries.log" | tr -d ' ')

hashes >"$report/raw-after.sha256"
if ! diff -u "$report/raw-before.sha256" "$report/raw-after.sha256" >"$report/raw-hash-diff.log"; then
    printf '%s\n' "raw hashes changed" >>"$report/missing-summaries.log"
    missing=$((missing + 1))
fi

printf 'snap status: %s\nagent status: %s\nmissing summaries or hash changes: %s\nexpected failures: %s\nreport: %s\n' \
    "$snap_status" "$agent_status" "$missing" "$(wc -l <"$report/expected-failures.log" | tr -d ' ')" "$report"
[ "$agent_status" -eq 0 ] && [ "$missing" -eq 0 ]
