# Evaluation harness

The harness runs `bo` against a seeded fixture. A run does not fetch source
URLs. Capture is the only network step.

`evals/corpora/` contains capture manifests and local source files. `evals/fixtures/`
contains seeded snapshots for repeatable runs. Keep both when the corpus must
be recaptured or extended.

```text
evals/harness capture --corpus default.txt --fixture default
evals/harness run --fixture default --workflow end-to-end --provider deepseek
evals/harness grade evals/results/<run-id>
```

Use `--runtime package` for the Go public API adapter, `--runtime cli` for the
public CLI, or `--runtime command -- COMMAND ...` for an external agent
runtime. The command runtime contract has two values: `BO_EVAL_TASK` points to
the resolved per-trial task JSON, and `BO_EVAL_WORKSPACE` points to its
isolated writable workspace. It starts in the repository root with an isolated
`HOME`; the harness owns the trial report and captures stdout and stderr.

The task contract contains `schema_version`, `name`, `instructions`,
`workflow`, and `success` with `min_source_identities` and
`require_distillation`. Optional `expected_distillations` entries name a
canonical topic, a non-seeded reference Markdown document, and the source
identities that the produced document must cover. A run resolves `workflow`
and adds `provider` before it starts a trial.

Package and CLI trials use `DEEPSEEK_API_KEY` or `GEMINI_API_KEY` (with
`GOOGLE_API_KEY` as the Gemini fallback). The evaluator uses only
`BO_EVAL_API_KEY`. Set `GEMINI_THINKING_BUDGET=0` to disable Gemini thinking
for a run, `-1` for dynamic thinking, or leave it unset for the provider
default.

Each run writes `run.json`, one isolated `trials/trial-*/` directory per
trial, and deterministic checks for process exit, workspace state, raw-document
integrity, summaries, distillations, and the operation log. Built-in
end-to-end package and CLI trials run the public summarize and distill stages
separately so the harness can compare summary hashes across the stage boundary.
External commands must record public operation events for the work they claim;
an unseeded summarize run needs a committed `write_summary`, and a required
end-to-end distill needs a committed `write_distillation`. The harness also
checks that no summary write follows the first distillation write and that each
configured expected topic has the required source identities. Expected
distillation documents live under a fixture's `expected/` directory and are
not copied into trial workspaces.
`pass_at_k` means any trial passed; `pass_caret_k` means every trial passed.
The run status is failed unless every trial passed.

The package adapter exposes runtime telemetry through the public `bo.Synth`
result. The harness copies telemetry emitted by a runtime to `trajectory.json`
and `trial.json`. Each stage records tool names, turns, bounded previews for
read and terminal calls, argument and output sizes and hashes, tool errors, and
the runtime terminal reason and detail. The telemetry fields are `workflow`,
`source_key`, `terminal_reason`, `terminal_detail`, `provider_retries`,
`provider_retry_reasons`, and `tool_calls`; tool calls
include `turn`, `index`, `name`, bounded `arguments_preview`, byte and hash
fields, and `error`. Write payloads are not copied into telemetry.

`grade` writes `evaluation/aggregate.json`, stage aggregates, and one score
file per graded document. It uses only `BO_EVAL_API_KEY`; set
`BO_EVAL_API_URL` and `BO_EVAL_MODEL` to select another OpenAI-compatible
judge. Reports remain under the ignored `evals/results/` directory for review.
A score below 4 or a criterion mean below 4.6 fails the quality gate.

The summary and distillation rubrics are in `evals/rubrics/SUMMARY.md` and
`evals/rubrics/DISTILLATION.md`.

The Python project has no dependencies. `evals/harness`, `evals/run.sh`, and
`evals/test_run.sh` run through `uv` so the project does not install packages
into the global interpreter.
