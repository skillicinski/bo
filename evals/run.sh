#!/bin/sh
set -eu

repo=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
bo=${BO_BIN:-$repo/target/debug/bo}
if [ ! -x "$bo" ]; then
    cargo build --manifest-path "$repo/Cargo.toml" --quiet
fi

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
cp "$target/state.json" "$report/state.json"

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

missing=0
: >"$report/missing-summaries.log"
for raw in "$target"/*.md; do
    [ -f "$raw" ] || continue
    summary="$target/summaries/$(basename "$raw")"
    if [ ! -s "$summary" ]; then
        printf 'missing summary: %s\n' "$(basename "$raw")" >>"$report/missing-summaries.log"
        missing=$((missing + 1))
    fi
done

hashes >"$report/raw-after.sha256"
if ! diff -u "$report/raw-before.sha256" "$report/raw-after.sha256" >"$report/raw-hash-diff.log"; then
    printf '%s\n' "raw hashes changed" >>"$report/missing-summaries.log"
    missing=$((missing + 1))
fi

printf 'snap status: %s\nagent status: %s\nmissing summaries or hash changes: %s\nexpected failures: %s\nreport: %s\n' \
    "$snap_status" "$agent_status" "$missing" "$(wc -l <"$report/expected-failures.log" | tr -d ' ')" "$report"
[ "$agent_status" -eq 0 ] && [ "$missing" -eq 0 ]
