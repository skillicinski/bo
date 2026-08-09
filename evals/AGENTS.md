# Evaluation workflow

- Keep the corpus manifest in `manifest.txt` and the human scoring guidance in `RUBRIC.md`.
- Rebuild `target/debug/bo` after source changes before running the evaluator.
- Run `./evals/run.sh` from the repository root with a non-empty `BO_API_KEY`. Never put API keys in files or logs.
- `bo snap` writes successful raw snapshots into the seeded target directory under the temporary eval home. The runner copies them to `evals/results/<run-id>/raw/`.
- Summaries remain in `evals/work/<run-id>/home/.bo/<run-id>/summaries/`; logs, state, hashes, and expected fetch failures are in `evals/results/<run-id>/`.
- Treat `evals/work/` and `evals/results/` as generated output. Do not commit them.
- Read `RUBRIC.md` before judging summaries. The rubric is for evaluation; it is not automatically sent to `bo agent`.
- A nonzero `snap status` may only reflect reported fetch failures. Require `agent status: 0`, no missing summaries, and no raw hash changes.
