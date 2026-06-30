# Architecture

bo's architecture is guided by one tenet: **code that a human can trace top-to-bottom, and a machine can enforce with types.** Like an agent skill, each layer discloses what it needs for the reader to understand the next — no holding context in your head, no external map.

`src/AGENTS.md` captures the *rules* this philosophy implies. This file captures the *decisions* — the shape of the code distilled into a few paragraphs.

---

## Manifest as single source of truth

The manifest (`{tree}/.bo/manifest.json`) is the one topology record. Branches reference their leaves by slug; there is no inverse persisted. `Manifest::branches_for_leaf` computes the inverse in-memory on demand rather than storing redundant state. This avoids bidirectional-write consistency problems: only branches list their leaves, so adding a leaf to a branch touches one record.

---

## Deterministic validation gate (never trust the LLM)

Every structured LLM response is deserialized, validated against the known set of leaves and branches, and **rejected without writing anything** on a single failure. The tree is never partially corrupted by a bad model output.

This is the single most important compile abstraction. The gate stopped three different failure modes during dogfooding — invented filenames (Gemini), malformed JSON (DeepSeek), ambiguous title refs — every time refusing to write. Without it, one bad response would have silently corrupted the manifest.

## Engine vs CLI boundary

`src/engine/` holds reusable primitives shared across all commands: `fetch`, `extract`, `llm`, `pending`, `retrieval`, `quality`, `summary`. These know nothing about specific commands — they are the toolbox.

`src/cli/<cmd>/*` holds command-specific orchestration. A command whose work is a multi-stage pipeline with non-trivial intermediate contracts is split into one module per stage, each owning its stage's types. The type system then enforces the contracts: a validated `CompilePlan` is a different type than a raw deserialized response, so it's impossible to execute an unvalidated plan.

The discipline runs both ways: don't push command-specific logic down into the engine (e.g. "branch staleness" is compile-domain knowledge, so `repair_stale_branches` lives in `cli/compile/`, not `engine/`), and don't carve the engine up per-command (the toolbox stays shared).

---

## Pipeline-stage modularity

Commands that are multi-stage transformations are split into named modules per stage. Compile is the exemplar: `plan` (what to do), `prompt` (build LLM input), `parse` (deserialize), `validation` (enforce the contract), `execute` (commit), `schema` (the shape the LLM sees), `repair` (self-heal), `render` (output). Query and collect follow the same pattern.

A single-stage command stays as one file. The transfer test: "does this command have ≥2 stages with a contract between them that could be violated?" If yes, split. If no, one module.
