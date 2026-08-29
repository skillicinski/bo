#!/bin/sh
set -eu

repo=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
bo=${BO_BIN:-$repo/bin/bo}
bo_eval=${BO_EVAL_BIN:-$repo/bin/bo-eval}
usage='usage: ./evals/run.sh [--task synth|distill] [--corpus name.txt|path] [--tools all|name,name,...]'

task=synth
corpus=$repo/evals/corpora/default.txt
toolset=all
while [ "$#" -gt 0 ]; do
	case "$1" in
		--task)
			[ "$#" -eq 1 ] && { printf '%s\n' "$usage" >&2; exit 2; }
			case "$2" in
				synth|distill) task=$2 ;;
				*) printf '%s\n' "$usage" >&2; exit 2 ;;
			esac
			shift 2
			;;
        --corpus)
            [ "$#" -eq 1 ] && { printf '%s\n' "$usage" >&2; exit 2; }
            case "$2" in
                */*) corpus=$2 ;;
                *) corpus=$repo/evals/corpora/$2 ;;
            esac
            shift 2
            ;;
        --tools)
            [ "$#" -eq 1 ] && { printf '%s\n' "$usage" >&2; exit 2; }
            toolset=$2
            shift 2
            ;;
        *)
            printf '%s\n' "$usage" >&2
            exit 2
            ;;
    esac
done

[ -f "$corpus" ] || { printf 'corpus not found: %s\n' "$corpus" >&2; exit 2; }
sources=$(awk 'NF && $1 !~ /^#/ { sub(/^[[:space:]]+/, ""); sub(/[[:space:]]+$/, ""); print }' "$corpus")
[ -n "$sources" ] || { printf 'corpus has no sources: %s\n' "$corpus" >&2; exit 2; }
: "${DEEPSEEK_API_KEY:?DEEPSEEK_API_KEY is required}"
old_ifs=$IFS
IFS='
'
set -f
set -- $sources
set +f
IFS=$old_ifs

cd "$repo"
mkdir -p "$(dirname "$bo")"
go build -o "$bo" "$repo/cmd/bo"
mkdir -p "$(dirname "$bo_eval")"
go build -o "$bo_eval" "$repo/evals/cmd/bo-eval"

run_id="$task-$(date +%s)-$$"
work="$repo/evals/work/$run_id"
report="$repo/evals/results/$run_id"
home="$work/home"
target="$home/.bo/$run_id"
raw="$report/raw"
mkdir -p "$home" "$report" "$raw"
cp "$corpus" "$report/corpus.txt"
printf '%s\n' "$toolset" >"$report/tools.txt"
printf '%s\n' "$task" >"$report/task.txt"

HOME="$home" "$bo" seed --name "$run_id" >"$report/seed.log"
set +e
HOME="$home" "$bo" snap "$run_id" "$@" >"$report/snap.log" 2>&1
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
HOME="$home" "$bo_eval" "$task" "$run_id" --tools "$toolset" >"$report/$task.log" 2>&1
task_status=$?
set -e

cp "$target/state.json" "$report/state.json"
if [ -d "$target/summaries" ]; then
    mkdir -p "$report/summaries"
    for file in "$target"/summaries/*.md; do
        [ -f "$file" ] || continue
        cp "$file" "$report/summaries/$(basename "$file")"
    done
fi

if [ "$task" = distill ] && [ -d "$target/synthesized" ]; then
	mkdir -p "$report/synthesized"
	for file in "$target"/synthesized/*.md; do
		[ -f "$file" ] || continue
		cp "$file" "$report/synthesized/$(basename "$file")"
	done
fi

missing=0
if [ "$task" = synth ]; then
: >"$report/missing-summaries.log"
python3 - "$target/state.json" "$target" >"$report/missing-summaries.log" <<'PY'
import json
import os
import sys

state_path, target = sys.argv[1:]
with open(state_path, encoding="utf-8") as file:
    state = json.load(file)

raw_records = {
    snapshot["filename"]: source["source_key"]
    for source in state["sources"]
    for snapshot in source["snapshots"]
}
source_keys = set()
for filename in os.listdir(target):
    if filename.lower().endswith(".md") and os.path.isfile(os.path.join(target, filename)):
        source_keys.add(raw_records.get(filename, f"raw:{filename}"))

summaries = {
    source["source_key"]: source["summary"]
    for source in state["sources"]
    if source.get("summary") is not None
}
for source_key in sorted(source_keys):
    record = summaries.get(source_key)
    path = os.path.join(target, "summaries", record["filename"]) if record else ""
    if not record or not os.path.isfile(path) or os.path.getsize(path) == 0:
        print(f"missing summary: {source_key}")
PY
missing=$(wc -l <"$report/missing-summaries.log" | tr -d ' ')
fi

hashes >"$report/raw-after.sha256"
if ! diff -u "$report/raw-before.sha256" "$report/raw-after.sha256" >"$report/raw-hash-diff.log"; then
	if [ "$task" = synth ]; then
		printf '%s\n' "raw hashes changed" >>"$report/missing-summaries.log"
	fi
	missing=$((missing + 1))
fi

printf 'task: %s\nsnap status: %s\n%s status: %s\nmissing summaries or hash changes: %s\nexpected failures: %s\nreport: %s\n' \
	"$task" "$snap_status" "$task" "$task_status" "$missing" "$(wc -l <"$report/expected-failures.log" | tr -d ' ')" "$report"
[ "$task_status" -eq 0 ] && [ "$missing" -eq 0 ]
