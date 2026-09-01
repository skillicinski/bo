# Evaluation fixtures

Fixtures are seeded workspaces for fast, repeatable trials. They contain no
network fetches during a run.

The harness owns the `uv` project in `evals/pyproject.toml`; `uv run` creates
the local environment and keeps Python packages out of the global interpreter.

Create or refresh one from a corpus with:

```text
evals/harness capture --corpus default.txt --fixture default
```

Run the package API against it with:

```text
evals/harness run --fixture default --workflow end-to-end --provider deepseek
```

Each fixture has this local layout:

```text
<fixture>/
  corpus.txt
  task.json
  expected/
    distillations/
  workspace/
```

`expected/distillations/` contains hand-curated reference outputs. The harness
checks the topic and source identities named by `task.json`, but does not seed
the reference documents into a trial.

The workspace is copied for every trial. The harness records stdout, stderr,
the public operation log, final state, deterministic checks, and pass@k and
pass^k. Fixture contents are local evaluation data and are ignored by Git.

The `command` runtime receives only `BO_EVAL_TASK` and `BO_EVAL_WORKSPACE`.
This is the adapter boundary for a Codex-style or other external agent
runtime. The per-trial task JSON contains the selected workflow and provider.
