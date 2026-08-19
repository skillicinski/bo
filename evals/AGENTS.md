# Evaluation workflow

- Keep the corpus manifest in `manifest.txt` and the human scoring guidance in `RUBRIC.md`.
- Rebuild `bin/bo` after source changes with `go build -o bin/bo ./cmd/bo` before running the evaluator.
- Run `./evals/run.sh` from the repository root with a non-empty `DEEPSEEK_API_KEY`. On macOS, use `DEEPSEEK_API_KEY="$(security find-generic-password -s deepseek-api-key -w)" ./evals/run.sh` to read the generic-password item without writing the key to a file. Never put API keys in files or logs; `BO_API_KEY` is not accepted by the local agent.
- `bo snap` writes successful raw snapshots into the seeded target directory under the temporary eval home. The runner copies them to `evals/results/<run-id>/raw/`.
- The runner copies final state and summaries to `evals/results/<run-id>/`; logs, hashes, and expected fetch failures are stored there too.
- Evaluate a completed run with `BO_EVAL_API_KEY=... python3 evals/evaluate.py evals/results/<run-id>`. The evaluator uses only `BO_EVAL_API_KEY`.
- Evaluation output is atomically published under `evals/results/<run-id>/evaluation/aggregate.json` and `evaluation/documents/*.json`.
- Treat `evals/work/` and `evals/results/` as generated output. Do not commit them.
- Read `RUBRIC.md` before judging summaries. The rubric is for evaluation; it is not automatically sent to `bo synth`.
- A nonzero `snap status` may only reflect reported fetch failures. Require `synth status: 0`, no missing summaries, and no raw hash changes.
