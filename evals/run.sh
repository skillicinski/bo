#!/bin/sh
set -eu

repo=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
bo=${BO_BIN:-$repo/bin/bo}
bo_eval=${BO_EVAL_BIN:-$repo/bin/bo-eval}
usage='usage: ./evals/run.sh [--workflow summarize|distill|end-to-end] [--corpus name.txt|path] [--tools all|name,name,...]'

workflow=end-to-end
corpus=$repo/evals/corpora/default.txt
toolset=all
while [ "$#" -gt 0 ]; do
	case "$1" in
		--workflow)
			[ "$#" -eq 1 ] && { printf '%s\n' "$usage" >&2; exit 2; }
			case "$2" in
				summarize|distill|end-to-end) workflow=$2 ;;
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

if [ "$workflow" = end-to-end ] && [ "$toolset" != all ]; then
	printf '%s\n' 'custom tools are only supported for focused workflows' >&2
	exit 2
fi
[ -f "$corpus" ] || { printf 'corpus not found: %s\n' "$corpus" >&2; exit 2; }
sources=$(awk 'NF && $1 !~ /^#/ { sub(/^[[:space:]]+/, ""); sub(/[[:space:]]+$/, ""); print }' "$corpus")
[ -n "$sources" ] || { printf 'corpus has no sources: %s\n' "$corpus" >&2; exit 2; }
if [ -z "${DEEPSEEK_API_KEY:-}" ]; then
	printf '%s\n' 'DEEPSEEK_API_KEY is required' >&2
	exit 1
fi

old_ifs=$IFS
IFS='
'
set -f
set -- $sources
set +f
IFS=$old_ifs

cd "$repo"
if [ -z "${BO_BIN:-}" ]; then
	mkdir -p "$(dirname "$bo")"
	go build -o "$bo" "$repo/cmd/bo"
fi
if [ -z "${BO_EVAL_BIN:-}" ]; then
	mkdir -p "$(dirname "$bo_eval")"
	go build -o "$bo_eval" "$repo/evals/cmd/bo-eval"
fi

run_id="$workflow-$(date +%s)-$$"
work="$repo/evals/work/$run_id"
report="$repo/evals/results/$run_id"
home="$work/home"
target="$home/.bo/$run_id"
raw="$report/raw"
mkdir -p "$home" "$report" "$raw"
cp "$corpus" "$report/corpus.txt"
printf '%s\n' "$toolset" >"$report/tools.txt"
printf '%s\n' "$workflow" >"$report/workflow.txt"

HOME="$home" "$bo" seed --name "$run_id" >"$report/seed.log"
set +e
HOME="$home" "$bo" snap "$run_id" "$@" >"$report/snap.log" 2>&1
snap_status=$?
set -e

copy_state() {
	if [ -f "$target/state.json" ]; then
		cp "$target/state.json" "$report/state.json"
	fi
}

copy_raw() {
	for file in "$target"/*.md; do
		[ -f "$file" ] || continue
		cp "$file" "$raw/$(basename "$file")"
	done
}

copy_summaries() {
	if [ ! -d "$target/summaries" ]; then
		return
	fi
	mkdir -p "$report/summaries"
	for file in "$target"/summaries/*.md; do
		[ -f "$file" ] || continue
		cp "$file" "$report/summaries/$(basename "$file")"
	done
}

copy_distillations() {
	if [ ! -d "$target/distillations" ]; then
		return
	fi
	mkdir -p "$report/distillations"
	for file in "$target"/distillations/*.md; do
		[ -f "$file" ] || continue
		cp "$file" "$report/distillations/$(basename "$file")"
	done
}

hashes() {
	for file in "$target"/*.md; do
		[ -f "$file" ] || continue
		shasum -a 256 "$file"
	done | sort
}

summary_hashes() {
	for file in "$target"/summaries/*.md; do
		[ -f "$file" ] || continue
		shasum -a 256 "$file"
	done | sort
}

copy_raw
hashes >"$report/raw-before.sha256"
failed=$(awk '/^failed:/{print}' "$report/snap.log" || true)
if [ -n "$failed" ]; then
	printf '%s\n' "$failed" >"$report/expected-failures.log"
else
	: >"$report/expected-failures.log"
fi

successful_source_count() {
	python3 - "$target/state.json" "$target" <<'PY'
import json
import os
import sys

state_path, target = sys.argv[1:]
with open(state_path, encoding="utf-8") as file:
    state = json.load(file)
sources = state.get("sources")
if not isinstance(sources, list):
    raise ValueError("state sources must be an array")
keys = set()
for source in sources:
    if not isinstance(source, dict):
        raise ValueError("state source records must be objects")
    source_key = source.get("source_key")
    snapshots = source.get("snapshots")
    if not isinstance(source_key, str) or not isinstance(snapshots, list):
        raise ValueError("state source record is invalid")
    for snapshot in snapshots:
        if not isinstance(snapshot, dict):
            continue
        filename = snapshot.get("filename")
        if not isinstance(filename, str) or os.path.basename(filename) != filename:
            continue
        path = os.path.join(target, filename)
        if os.path.isfile(path) and not os.path.islink(path) and os.path.getsize(path) > 0:
            keys.add(source_key)
            break
print(len(keys))
PY
}

validate_summaries() {
	python3 - "$target/state.json" "$target" <<'PY'
import json
import os
import sys
from datetime import datetime, timezone

def parse_timestamp(value):
    if not isinstance(value, str) or not value:
        raise ValueError("snapshot written_at must be an RFC 3339 timestamp")
    normalized = value[:-1] + "+00:00" if value.endswith("Z") else value
    fraction = normalized.rsplit(".", 1)
    if len(fraction) == 2 and ("+" in fraction[1] or "-" in fraction[1]):
        number, offset = fraction[1].split("+", 1) if "+" in fraction[1] else fraction[1].split("-", 1)
        sign = "+" if "+" in fraction[1] else "-"
        normalized = f"{fraction[0]}.{number[:6].ljust(6, '0')}{sign}{offset}"
    try:
        parsed = datetime.fromisoformat(normalized)
    except ValueError as error:
        raise ValueError("snapshot written_at must be an RFC 3339 timestamp") from error
    if parsed.tzinfo is None or parsed.utcoffset() is None:
        raise ValueError("snapshot written_at must include a timezone")
    return parsed.astimezone(timezone.utc)

state_path, target = sys.argv[1:]
with open(state_path, encoding="utf-8") as file:
    state = json.load(file)
sources = state.get("sources")
if not isinstance(sources, list):
    raise ValueError("state sources must be an array")
missing = []
for source in sources:
    if not isinstance(source, dict):
        raise ValueError("state source records must be objects")
    source_key = source.get("source_key")
    snapshots = source.get("snapshots")
    if not isinstance(source_key, str) or not isinstance(snapshots, list):
        raise ValueError("state source record is invalid")
    latest = None
    for snapshot_index, snapshot in enumerate(snapshots):
        if not isinstance(snapshot, dict):
            raise ValueError("state snapshot record is invalid")
        filename = snapshot.get("filename")
        if not isinstance(filename, str) or os.path.basename(filename) != filename:
            raise ValueError("state snapshot filename is invalid")
        candidate = (parse_timestamp(snapshot.get("written_at")), snapshot_index)
        if latest is None or candidate > latest[0]:
            latest = (candidate, filename)
    if latest is None:
        continue
    latest_filename = latest[1]
    raw_path = os.path.join(target, latest_filename)
    if not os.path.isfile(raw_path) or os.path.islink(raw_path) or os.path.getsize(raw_path) == 0:
        missing.append(f"missing current raw: {source_key}")
        continue
    summary = source.get("summary")
    if not isinstance(summary, dict):
        missing.append(f"missing summary: {source_key}")
        continue
    filename = summary.get("filename")
    if not isinstance(filename, str) or os.path.basename(filename) != filename:
        missing.append(f"invalid summary filename: {source_key}")
        continue
    if summary.get("derived_from") != latest_filename:
        missing.append(f"stale summary: {source_key}")
        continue
    path = os.path.join(target, "summaries", filename)
    if not os.path.isfile(path) or os.path.islink(path) or os.path.getsize(path) == 0:
        missing.append(f"missing summary: {source_key}")
if missing:
    print("\n".join(missing))
    raise SystemExit(1)
PY
}

validate_distillation() {
	require_one=$1
	python3 - "$target/state.json" "$target" "$require_one" <<'PY'
import hashlib
import json
import os
import re
import sys
from datetime import datetime, timezone

def parse_timestamp(value):
    if not isinstance(value, str) or not value:
        raise ValueError("snapshot written_at must be an RFC 3339 timestamp")
    normalized = value[:-1] + "+00:00" if value.endswith("Z") else value
    fraction = normalized.rsplit(".", 1)
    if len(fraction) == 2 and ("+" in fraction[1] or "-" in fraction[1]):
        number, offset = fraction[1].split("+", 1) if "+" in fraction[1] else fraction[1].split("-", 1)
        sign = "+" if "+" in fraction[1] else "-"
        normalized = f"{fraction[0]}.{number[:6].ljust(6, '0')}{sign}{offset}"
    try:
        parsed = datetime.fromisoformat(normalized)
    except ValueError as error:
        raise ValueError("snapshot written_at must be an RFC 3339 timestamp") from error
    if parsed.tzinfo is None or parsed.utcoffset() is None:
        raise ValueError("snapshot written_at must include a timezone")
    return parsed.astimezone(timezone.utc)

state_path, target, require_one = sys.argv[1:]
require_one = require_one == "1"
with open(state_path, encoding="utf-8") as file:
    state = json.load(file)
sources = state.get("sources")
records = state.get("distillation_documents", [])
if not isinstance(sources, list) or not isinstance(records, list):
    raise ValueError("state source or distillation records are invalid")
artifacts_dir = os.path.join(target, "distillations")
artifact_names = []
if os.path.isdir(artifacts_dir):
    for name in os.listdir(artifacts_dir):
        path = os.path.join(artifacts_dir, name)
        if name.endswith(".md") and os.path.isfile(path) and not os.path.islink(path):
            artifact_names.append(name)
if require_one and len(artifact_names) != 1:
    raise ValueError(f"end-to-end requires exactly one distillation document, found {len(artifact_names)}")
if not require_one and len(artifact_names) > 1:
    raise ValueError(f"distill produced more than one document: {len(artifact_names)}")
if len(records) != len(artifact_names):
    raise ValueError("distillation state and files do not match")

raw_owners = {}
summary_owners = {}
current_raw = {}
current_summaries = {}
for source in sources:
    source_key = source.get("source_key")
    snapshots = source.get("snapshots")
    if not isinstance(source_key, str) or not isinstance(snapshots, list):
        raise ValueError("state source record is invalid")
    latest = None
    for snapshot_index, snapshot in enumerate(snapshots):
        if not isinstance(snapshot, dict):
            raise ValueError("state snapshot record is invalid")
        filename = snapshot.get("filename")
        if not isinstance(filename, str) or os.path.basename(filename) != filename:
            raise ValueError("state snapshot filename is invalid")
        if filename in raw_owners:
            raise ValueError(f"duplicate raw filename: {filename}")
        raw_owners[filename] = source_key
        candidate = (parse_timestamp(snapshot.get("written_at")), snapshot_index)
        if latest is None or candidate > latest[0]:
            latest = (candidate, filename)
    if latest is not None:
        current_raw[source_key] = latest[1]
    summary = source.get("summary")
    if summary is not None:
        if not isinstance(summary, dict):
            raise ValueError("state summary record is invalid")
        filename = summary.get("filename")
        if not isinstance(filename, str) or os.path.basename(filename) != filename:
            raise ValueError("state summary filename is invalid")
        if filename in summary_owners:
            raise ValueError(f"duplicate summary filename: {filename}")
        summary_owners[filename] = source_key
        if summary.get("derived_from") == current_raw.get(source_key):
            current_summaries[filename] = source_key

for record in records:
    if record.get("kind") != "distillation":
        raise ValueError("distillation record has an invalid kind")
    filename = record.get("filename")
    if not isinstance(filename, str) or os.path.basename(filename) != filename:
        raise ValueError("distillation record has an invalid filename")
    artifact_path = os.path.join(artifacts_dir, filename)
    if not os.path.isfile(artifact_path) or os.path.islink(artifact_path) or os.path.getsize(artifact_path) == 0:
        raise ValueError(f"distillation document is missing or empty: {filename}")
    inputs = record.get("derived_from")
    if not isinstance(inputs, list) or not inputs:
        raise ValueError(f"distillation record has no provenance: {filename}")
    source_keys = set()
    for item in inputs:
        if not isinstance(item, dict):
            raise ValueError("distillation provenance entry is invalid")
        source_key = item.get("source_key")
        kind = item.get("kind")
        input_filename = item.get("filename")
        digest = item.get("content_digest")
        if not isinstance(source_key, str) or kind not in ("raw", "summary"):
            raise ValueError("distillation provenance entry is invalid")
        if not isinstance(input_filename, str) or os.path.basename(input_filename) != input_filename:
            raise ValueError("distillation provenance filename is invalid")
        if not isinstance(digest, str) or not re.fullmatch(r"[0-9a-fA-F]{64}", digest):
            raise ValueError("distillation provenance digest is invalid")
        owners = raw_owners if kind == "raw" else summary_owners
        if owners.get(input_filename) != source_key:
            raise ValueError(f"distillation input does not belong to source: {input_filename}")
        if kind == "raw" and current_raw.get(source_key) != input_filename:
            raise ValueError(f"distillation input is not the current raw document: {input_filename}")
        if kind == "summary" and current_summaries.get(input_filename) != source_key:
            raise ValueError(f"distillation input is not the current summary: {input_filename}")
        directory = target if kind == "raw" else os.path.join(target, "summaries")
        path = os.path.join(directory, input_filename)
        if not os.path.isfile(path) or os.path.islink(path):
            raise ValueError(f"missing distillation input: {kind}/{input_filename}")
        with open(path, "rb") as input_file:
            actual = hashlib.sha256(input_file.read()).hexdigest()
        if actual.lower() != digest.lower():
            raise ValueError(f"distillation input digest changed: {kind}/{input_filename}")
        source_keys.add(source_key)
    if len(source_keys) < 2:
        raise ValueError(f"distillation record has fewer than two source identities: {filename}")
PY
}

raw_after_summarize_status=0
raw_unchanged=1
summary_unchanged=1
summarize_status=not-run
distill_status=not-run
summarize_ok=0
distill_ok=0

run_summarize() {
	set +e
	HOME="$home" "$bo_eval" synth "$run_id" --tools "$toolset" >"$report/summarize.log" 2>&1
	summarize_status=$?
	set -e
	copy_state
	if [ "$summarize_status" -eq 0 ]; then
		set +e
		validate_summaries >"$report/missing-summaries.log" 2>&1
		summary_validation_status=$?
		set -e
	else
		summary_validation_status=1
		printf '%s\n' 'summarize command failed' >"$report/missing-summaries.log"
	fi
	hashes >"$report/raw-after-summarize.sha256"
	if diff -u "$report/raw-before.sha256" "$report/raw-after-summarize.sha256" >"$report/raw-hash-diff-after-summarize.log"; then
		raw_after_summarize_status=0
	else
		raw_after_summarize_status=1
		raw_unchanged=0
		printf '%s\n' 'raw hashes changed after summarize' >>"$report/missing-summaries.log"
	fi
	copy_summaries
	if [ "$summarize_status" -eq 0 ] && [ "$summary_validation_status" -eq 0 ] && [ "$raw_after_summarize_status" -eq 0 ]; then
		summarize_ok=1
	fi
}

run_distill() {
	set +e
	HOME="$home" "$bo_eval" distill "$run_id" --tools "$toolset" >"$report/distill.log" 2>&1
	distill_status=$?
	set -e
	copy_state
	set +e
	if [ "$workflow" = end-to-end ]; then
		validate_distillation 1 >"$report/distillation-validation.log" 2>&1
	else
		validate_distillation 0 >"$report/distillation-validation.log" 2>&1
	fi
	distillation_validation_status=$?
	set -e
	hashes >"$report/raw-after.sha256"
	if diff -u "$report/raw-before.sha256" "$report/raw-after.sha256" >"$report/raw-hash-diff.log"; then
		raw_after_distill_status=0
	else
		raw_after_distill_status=1
		raw_unchanged=0
	fi
	summary_hashes >"$report/summary-after-distill.sha256"
	if diff -u "$report/summary-before-distill.sha256" "$report/summary-after-distill.sha256" >"$report/summary-hash-diff.log"; then
		summary_after_distill_status=0
	else
		summary_after_distill_status=1
		summary_unchanged=0
	fi
	copy_summaries
	copy_distillations
	if [ "$distill_status" -eq 0 ] && [ "$distillation_validation_status" -eq 0 ] && [ "$raw_after_distill_status" -eq 0 ] && [ "$summary_after_distill_status" -eq 0 ]; then
		distill_ok=1
	fi
}

set +e
source_count=$(successful_source_count 2>"$report/source-count-error.log")
source_count_status=$?
set -e
if [ "$source_count_status" -ne 0 ]; then
	source_count=0
fi

case "$workflow" in
	summarize)
		run_summarize
		: >"$report/summary-before-distill.sha256"
		;;
	distill)
		: >"$report/summary-before-distill.sha256"
		run_distill
		;;
	end-to-end)
		if [ "$source_count" -ge 2 ]; then
			run_summarize
			if [ "$summarize_ok" -eq 1 ]; then
				summary_hashes >"$report/summary-before-distill.sha256"
				run_distill
			else
				: >"$report/summary-before-distill.sha256"
			fi
		else
			printf '%s\n' "end-to-end requires at least two successfully snapped source identities; found $source_count" >"$report/source-count-error.log"
			: >"$report/missing-summaries.log"
			: >"$report/summary-before-distill.sha256"
		fi
		;;
esac

copy_state
hashes >"$report/raw-after.sha256"
if [ "$workflow" != summarize ] && [ ! -f "$report/summary-after-distill.sha256" ]; then
	summary_hashes >"$report/summary-after-distill.sha256"
fi

result=0
case "$workflow" in
	summarize)
		if [ "$summarize_ok" -ne 1 ]; then result=1; fi
		;;
	distill)
		if [ "$distill_ok" -ne 1 ]; then result=1; fi
		;;
	end-to-end)
		if [ "$source_count" -lt 2 ] || [ "$summarize_ok" -ne 1 ] || [ "$distill_ok" -ne 1 ]; then result=1; fi
		;;
esac

printf 'workflow: %s\nsnap status: %s\nsummarize status: %s\ndistill status: %s\nsuccessful source identities: %s\nraw unchanged: %s\nsummary unchanged: %s\nexpected failures: %s\nreport: %s\n' \
	"$workflow" "$snap_status" "$summarize_status" "$distill_status" "$source_count" "$raw_unchanged" "$summary_unchanged" "$(wc -l <"$report/expected-failures.log" | tr -d ' ')" "$report"
exit "$result"
