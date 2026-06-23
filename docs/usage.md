# Usage

bo models a cyclical knowledge workflow: **collect** material → **compile** it into topics → **query** for insights → repeat.

The tree grows with you. New leaves feed existing branches; new questions surface gaps you fill by collecting more.

---

## Seeding a tree

```bash
bo seed --path ~/my-knowledge-tree --name my-knowledge-tree --provider openai --model gpt-4.1-mini
```

Creates a single active local tree and writes `~/.bo/config.json`. `bo seed` does not create `manifest.json`; collection creates the manifest when tree runtime state first exists. Run bare `bo seed` in a terminal for guided prompts.

---

## Configuring a provider and model

```bash
bo config --provider openai --model gpt-4.1-mini
```

Writes to `~/.bo/config.json`. You can also pin a heavier model for the compile step:

```bash
bo config --compile-model gpt-4.1
```

**API key** — resolution order: environment variable → `~/.bo/auth.json` → error.

```bash
export OPENAI_API_KEY=sk-...
```

DeepSeek works the same way with `--provider deepseek` and `DEEPSEEK_API_KEY`; Google uses `--provider google` and `GEMINI_API_KEY`.

---

## Collecting material

```bash
# Single URL
bo collect https://example.com/blog/intro-to-knowledge-graphs

# Multiple URLs
bo collect https://example.com/a https://example.com/b

# A text file with one URL per non-empty line
bo collect urls.txt
```

Each URL is fetched, converted to markdown, and saved as a **leaf** in your tree. Duplicates (URLs already collected) are reported and skipped.

---

## Inspecting a tree

```bash
# Tree health at a glance
bo status

# Branch-centric tree view (default)
bo list

# Flat leaf list
bo list --leaves

# Search by title keywords
bo list --terms "knowledge graphs"

# Filter to a single branch
bo list --branch "Knowledge Graphs"

# Most recently collected first
bo list --leaves --recent --limit 10
```

---

## Inspecting a leaf

```bash
# Card view — frontmatter only (title, URL, date, word count, branches)
bo show "Intro to Knowledge Graphs"

# Full content
bo show "Intro to Knowledge Graphs" --full
```

---

## Compiling into branches

```bash
bo compile
```

This is where bo does the heavy lifting. All collected leaves are sent to the configured LLM, which:

1. Identifies common themes and topics across documents
2. Assigns each leaf to one or more topic branches
3. Writes a summary for each branch synthesising its leaves

**Incremental by default** — only leaves collected since the last compile are processed. Existing branches are preserved; new leaves are fitted to them.

```bash
# Recompile the full corpus (allow complete branch graph rewrite)
bo compile --all
```

**Validation gate** — if the LLM response is malformed (missing fields, phantom leaf references, empty branches), bo rejects it and writes nothing. No partial state.

**Trees grow in size over time — know your model's limits.** Smaller models (e.g. `gpt-4.1-nano`) have tighter context windows and will throw compile-time _context overflow_ errors once your collection passes a certain size. If this happens, try compiling with a larger model via `bo config --compile-model gpt-4.1`.

---

## Querying your tree

```bash
bo query "What are the core principles of knowledge graphs?"
```

Behind the scenes:
1. Your question is matched against leaf titles and content using lexical (keyword-based) retrieval
2. The most relevant leaves and their parent branches are gathered as context
3. The LLM generates an answer with citations back to source documents

**No-answer hardening** — if no relevant sources are found, bo reports "no answer from collected sources" rather than hallucinating.

---

## Tearing down

```bash
# Delete the tree and config, preserving API credentials
bo raze

# Full wipe including credentials
bo raze --include-auth
```

`bo raze` requires interactive confirmation (`yes`). There is no `--force` or `--yes` flag.

---

## JSON output

Commands other than `seed` support `--json` for machine consumption:

```bash
bo status --json
bo list --json --leaves
bo query "What is RDF?" --json
```

Intended for use by coding assistants and scripts.

---

## Loops

The commands aren't a linear pipeline — they form loops at different grain sizes:

**Inner loop (collect → compile):** gather a batch of related URLs, compile to find out what themes emerged, then collect more to deepen interesting branches.

**Full loop (collect → compile → query → collect):** compile your tree, ask a question, discover a gap, collect material to fill it, repeat.

**Browse loop (list → show → query → list):** scan your tree, read a few leaves, query for connections across branches, then search for more leaves to feed your new questions.
