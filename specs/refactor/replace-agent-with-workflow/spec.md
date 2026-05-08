# Spec: Replace Compile Agent Loop with Structured-Output Pipeline

## Problem Statement

`bo compile` currently uses an internal agent loop where the LLM iterates through tool calls (list_index, read_leaf, write_branch, update_leaf_frontmatter) over ≤50 steps. The workflow is entirely predetermined — the LLM never makes meaningful decisions about what to do next — yet pays the cost of agent-style execution: token waste from replayed history, fragility from skipped steps, a step-limit ceiling on collection size, and ~20x more API calls than necessary.

This refactor replaces the agent loop with a deterministic pipeline that makes a single structured-output LLM call, while preserving identical external behavior.

## User-Facing Requirements

1. `bo compile` produces the same observable result: branch files written to the branches directory, leaf frontmatter updated with branch associations, and a summary printed to stdout.

2. The existing guards are preserved:
   - Empty collection → "bo is empty!" and clean exit
   - Single leaf → "bo only has 1 leaf!" and clean exit
   - Missing OPENAI_API_KEY → clear error message

3. The summary output format is unchanged:
   ```
   compiled: N branches across M leaves
     ✓ slug-name (K leaves)
     ...
   ```

4. Compile runs faster and cheaper (single API call instead of ≤50).

5. If the collection's content exceeds the LLM's context window, compile fails with a clear error rather than silently truncating or producing degraded output.

## Success Criteria

- All existing compile tests pass (empty collection, single leaf, missing API key).
- A compile against a multi-leaf test collection produces valid branches and updated frontmatter identical in structure to the current agent-based output.
- The `engine/agent/` module (agent loop, Tool trait, tool structs) is fully removed.
- The new `engine/llm/` module exposes `LlmProvider` trait and provider implementations with no agent/tool-calling concepts.
- `cli/compile.rs` is a flat module (not a directory) containing the pipeline and private write helpers.
- Exactly one LLM API call is made per compile invocation (verifiable via provider call count or mock).

## Out of Scope

- Chunking/parallelization for collections that exceed context window limits (future enhancement).
- `--json` output flag (separate feature).
- `--dry-run` mode (separate feature).
- Changes to `bo collect`, `bo list`, or any other command.
- New LLM providers beyond OpenAI.
- Prompt quality tuning beyond structural correctness (prompt refinement is iterative post-merge work).

## Open Questions

None.
