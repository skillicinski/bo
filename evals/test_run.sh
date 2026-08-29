#!/bin/sh
set -eu

repo=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
corpus=$(mktemp "${TMPDIR:-/tmp}/bo-eval-corpus.XXXXXX")
test_root=$(mktemp -d "${TMPDIR:-/tmp}/bo-eval-run.XXXXXX")
fake_bo=$test_root/bo
fake_eval=$test_root/bo-eval
log=$test_root/log
trap 'rm -rf "$test_root"; rm -f "$corpus"' EXIT HUP INT TERM

make_fake_commands() {
    cat >"$fake_bo" <<'SH'
#!/bin/sh
set -eu

command=$1
shift
printf 'bo:%s\n' "$command" >>"$FAKE_LOG"

case "$command" in
    seed)
        name=$2
        target="$HOME/.bo/$name"
        mkdir -p "$target"
        printf '%s\n' '{"sources":[]}' >"$target/state.json"
        ;;
    snap)
        name=$1
        target="$HOME/.bo/$name"
        mkdir -p "$target/summaries" "$target/distillations"
        printf '%s\n' 'one source' >"$target/one.md"
        printf '%s\n' 'two source' >"$target/two.md"
        cat >"$target/state.json" <<'JSON'
{"sources":[{"source_key":"https://example.test/one","snapshots":[{"filename":"one.md","written_at":"2026-08-23T00:00:01Z"}]},{"source_key":"https://example.test/two","snapshots":[{"filename":"two.md","written_at":"2026-08-23T00:00:02Z"}]}]}
JSON
        ;;
    *)
        exit 2
        ;;
esac
SH
    chmod +x "$fake_bo"

    cat >"$fake_eval" <<'SH'
#!/bin/sh
set -eu

command=$1
name=$2
printf 'eval:%s\n' "$command" >>"$FAKE_LOG"
target="$HOME/.bo/$name"

case "$command" in
    synth)
        [ "${FAKE_SUMMARIZE_FAIL:-0}" -eq 0 ] || exit 7
        [ "${FAKE_MUTATE_RAW:-0}" -eq 0 ] || printf '%s\n' 'changed' >>"$target/one.md"
        printf '%s\n' 'one summary' >"$target/summaries/one-summary.md"
        printf '%s\n' 'two summary' >"$target/summaries/two-summary.md"
        cat >"$target/state.json" <<'JSON'
{"sources":[{"source_key":"https://example.test/one","snapshots":[{"filename":"one.md","written_at":"2026-08-23T00:00:01Z"}],"summary":{"filename":"one-summary.md","derived_from":"one.md"}},{"source_key":"https://example.test/two","snapshots":[{"filename":"two.md","written_at":"2026-08-23T00:00:02Z"}],"summary":{"filename":"two-summary.md","derived_from":"two.md"}}]}
JSON
        ;;
    distill)
        [ "${FAKE_DISTILL_SKIP:-0}" -eq 0 ] || exit 0
        [ "${FAKE_DISTILL_FAIL:-0}" -eq 0 ] || exit 8
        [ "${FAKE_MUTATE_SUMMARY:-0}" -eq 0 ] || printf '%s\n' 'changed' >>"$target/summaries/one-summary.md"
        one_digest=$(shasum -a 256 "$target/one.md" | awk '{print $1}')
        two_digest=$(shasum -a 256 "$target/two.md" | awk '{print $1}')
        printf '%s\n' '# Shared' >"$target/distillations/shared.md"
        cat >"$target/state.json" <<JSON
{"sources":[{"source_key":"https://example.test/one","snapshots":[{"filename":"one.md","written_at":"2026-08-23T00:00:01Z"}],"summary":{"filename":"one-summary.md","derived_from":"one.md"}},{"source_key":"https://example.test/two","snapshots":[{"filename":"two.md","written_at":"2026-08-23T00:00:02Z"}],"summary":{"filename":"two-summary.md","derived_from":"two.md"}}],"distillation_documents":[{"filename":"shared.md","kind":"distillation","derived_from":[{"source_key":"https://example.test/one","kind":"raw","filename":"one.md","content_digest":"$one_digest"},{"source_key":"https://example.test/two","kind":"raw","filename":"two.md","content_digest":"$two_digest"}]}]}
JSON
        ;;
    *)
        exit 2
        ;;
esac
SH
    chmod +x "$fake_eval"
}

make_fake_commands

run_fake() {
    : >"$log"
    set +e
    run_output=$(FAKE_LOG="$log" \
        DEEPSEEK_API_KEY=test \
        BO_BIN="$fake_bo" \
        BO_EVAL_BIN="$fake_eval" \
        FAKE_SUMMARIZE_FAIL="${FAKE_SUMMARIZE_FAIL:-0}" \
        FAKE_MUTATE_RAW="${FAKE_MUTATE_RAW:-0}" \
        FAKE_MUTATE_SUMMARY="${FAKE_MUTATE_SUMMARY:-0}" \
        FAKE_DISTILL_SKIP="${FAKE_DISTILL_SKIP:-0}" \
        FAKE_DISTILL_FAIL="${FAKE_DISTILL_FAIL:-0}" \
        "$repo/evals/run.sh" "$@" 2>&1)
    run_status=$?
    set -e
    run_report=$(printf '%s\n' "$run_output" | awk -F': ' '$1 == "report" { print $2 }')
}

cleanup_report() {
    [ -n "${run_report:-}" ] || return
    rm -rf "$run_report" "$repo/evals/work/$(basename "$run_report")"
}

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

assert_failure 2 'usage:' --workflow invalid --corpus "$corpus"
assert_failure 2 'custom tools are only supported' --workflow end-to-end --tools read_document --corpus "$corpus"

printf '%s\n' 'https://example.test/one' 'https://example.test/two' >"$corpus"

run_fake --corpus "$corpus"
[ "$run_status" -eq 0 ] || { printf '%s\n' "$run_output" >&2; exit 1; }
case "$run_output" in
    *'workflow: end-to-end'*'summarize status: 0'*'distill status: 0'*'raw unchanged: 1'*'summary unchanged: 1'*) ;;
    *) printf 'unexpected successful output: %s\n' "$run_output" >&2; exit 1 ;;
esac
order=$(sed 's/^[^:]*://' "$log" | tr '\n' ' ')
[ "$order" = 'seed snap synth distill ' ] || { printf 'wrong execution order: %s\n' "$order" >&2; exit 1; }
[ "$(cat "$run_report/workflow.txt")" = end-to-end ]
cleanup_report

FAKE_SUMMARIZE_FAIL=1 run_fake --corpus "$corpus"
[ "$run_status" -ne 0 ] || { printf 'summarize failure was accepted\n' >&2; exit 1; }
case "$run_output" in
    *'distill status: not-run'*) ;;
    *) printf 'distill was not gated: %s\n' "$run_output" >&2; exit 1 ;;
esac
if grep -q 'eval:distill' "$log"; then
    printf 'distill ran after summarize failure\n' >&2
    exit 1
fi
cleanup_report
unset FAKE_SUMMARIZE_FAIL

FAKE_MUTATE_SUMMARY=1 run_fake --corpus "$corpus"
[ "$run_status" -ne 0 ] || { printf 'summary mutation was accepted\n' >&2; exit 1; }
case "$run_output" in
    *'summary unchanged: 0'*) ;;
    *) printf 'summary mutation was not reported: %s\n' "$run_output" >&2; exit 1 ;;
esac
cleanup_report
unset FAKE_MUTATE_SUMMARY

FAKE_MUTATE_RAW=1 run_fake --corpus "$corpus"
[ "$run_status" -ne 0 ] || { printf 'raw mutation was accepted\n' >&2; exit 1; }
case "$run_output" in
    *'raw unchanged: 0'*) ;;
    *) printf 'raw mutation was not reported: %s\n' "$run_output" >&2; exit 1 ;;
esac
cleanup_report
unset FAKE_MUTATE_RAW

FAKE_DISTILL_SKIP=1 run_fake --workflow distill --tools read_document --corpus "$corpus"
[ "$run_status" -eq 0 ] || { printf '%s\n' "$run_output" >&2; exit 1; }
case "$run_output" in
    *'workflow: distill'*'distill status: 0'*) ;;
    *) printf 'unexpected focused distill output: %s\n' "$run_output" >&2; exit 1 ;;
esac
order=$(sed 's/^[^:]*://' "$log" | tr '\n' ' ')
[ "$order" = 'seed snap distill ' ] || { printf 'wrong focused execution order: %s\n' "$order" >&2; exit 1; }
cleanup_report
