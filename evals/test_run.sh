#!/bin/sh
set -eu

repo=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
corpus=$(mktemp "${TMPDIR:-/tmp}/bo-eval-corpus.XXXXXX")
trap 'rm -f "$corpus"' EXIT HUP INT TERM

assert_failure() {
    expected_status=$1
    expected_text=$2
    shift 2
    set +e
    output=$(env -u DEEPSEEK_API_KEY "$repo/evals/run.sh" "$@" 2>&1)
    status=$?
    set -e
    [ "$status" -eq "$expected_status" ] || {
        printf 'status %s, want %s: %s\n' "$status" "$expected_status" "$output" >&2
        exit 1
    }
    case "$output" in
        *"$expected_text"*) ;;
        *) printf 'missing %s in: %s\n' "$expected_text" "$output" >&2; exit 1 ;;
    esac
}

printf '%s\n' 'https://example.com' >"$corpus"
assert_failure 1 'DEEPSEEK_API_KEY is required' --corpus "$corpus"

: >"$corpus"
assert_failure 2 'corpus has no sources' --corpus "$corpus"

assert_failure 2 "$repo/evals/corpora/missing.txt" --corpus missing.txt
