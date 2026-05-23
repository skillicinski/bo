# bo

Collect web content that you don't have time to read as a local markdown knowledge tree. Compile the content into common themes and topics using an LLM. Then, query your collection with natural language and get answers with citations back to source material.

Your collection stays legible and can be viewed with any markdown reader. The idea is to skip RAG and vector databases altogether and just rely on local files and let LLMs do what they're best at. The CLI is intended to be machine-friendly for use via any coding assistant.

All you need is to bring your own API key!

## Install

```bash
cargo install --git https://github.com/skillicinski/bo --tag v0.0.1
```

## Quickstart

```bash
# Pick a provider and model (writes to ~/.bo/config.json)
bo config --provider openai --model gpt-4.1-mini

# Make your API key available (either env var or ~/.bo/auth.json — see below)
export OPENAI_API_KEY=sk-...

# Seed a tree
bo seed ~/bo-tree

# Collect some pages — single URL, many URLs, or a .txt file with one URL per line
bo collect https://example.com/blog/intro-to-knowledge-graphs
bo collect https://example.com/a https://example.com/b
bo collect urls.txt

# Inspect what you have
bo list
bo status

# Read a collected leaf
bo show "Intro to Knowledge Graphs"

# Compile into a linked knowledge graph
bo compile

# Ask a question
bo query "How do these concepts relate to each other?"
```

## Commands

| Command | Description |
|---------|-------------|
| `bo seed <dir>` | Initialise a new tree |
| `bo collect <url\|file>...` | Fetch and store one or more web pages as markdown leaves. `.txt` files are treated as URL lists (one URL per non-empty line) |
| `bo compile` | Build topic branches from collected leaves via LLM (incremental by default) |
| `bo compile --all` | Recompile the full corpus and allow complete branch graph rewrite |
| `bo query <question>` | Answer a question with citations from your tree |
| `bo list` | Inspect branches and leaves. Default: branch-centric tree view |
| `bo list --branches` | Flat branch list with leaf counts |
| `bo list --leaves` | Flat leaf list with branch counts |
| `bo list --terms <terms>...` | Filter by title/slug match (all terms must match) |
| `bo list --branch <name>` | Filter to a single branch by name or slug |
| `bo list --recent` | Sort leaves by collected date, newest first (`--leaves` mode only) |
| `bo list --limit <n>` | Cap the number of items shown |
| `bo show <title>` | Display a leaf's frontmatter card. `--full` for complete body |
| `bo status` | Show tree health and compile readiness |
| `bo config --provider <name>` | Set the active provider (`openai` or `deepseek`) |
| `bo config --model <id>` | Set the default model for all LLM stages |
| `bo config --compile-model <id>` | Set a separate model for `compile` (falls back to `--model`) |
| `bo raze` | Delete the seeded tree and config, preserving auth |
| `bo raze --include-auth` | Also delete stored provider credentials |

All commands support `--json` for machine-readable output, intended for use by coding assistants and scripts.

## Provider setup

bo currently supports two OpenAI-compatible providers: **OpenAI** and **DeepSeek**.

### 1. Choose provider and model

```bash
bo config --provider openai --model gpt-4.1-mini
# or
bo config --provider deepseek --model deepseek-v4-flash
```

You can also pin a heavier model just for the compile step:

```bash
bo config --compile-model gpt-4.1
```

### 2. Provide an API key

Resolution order: environment variable → `~/.bo/auth.json` → error.

**Option A — environment variable:**

```bash
export OPENAI_API_KEY=sk-...
export DEEPSEEK_API_KEY=sk-...
```

**Option B — `~/.bo/auth.json`** (chmod 600):

```json
{
  "openai_api_key": "sk-...",
  "deepseek_api_key": "sk-..."
}
```

### Supported models

| Provider | Models |
|----------|--------|
| `openai` | `gpt-4o`, `gpt-4o-mini`, `gpt-4.1`, `gpt-4.1-mini` (default), `gpt-4.1-nano` |
| `deepseek` | `deepseek-v4-flash`, `deepseek-v4-pro` |

## Storage

```
~/.bo/
├── config.json          # Provider, model, compile_model, active tree
└── auth.json            # Provider credentials (chmod 600)

~/bo-tree/               # Your tree (location chosen at seed)
├── index.jsonl          # Ledger of collected leaves
├── intro-to-knowledge-graphs.md   # Leaf
├── linked-data-fundamentals.md    # Leaf
└── branches/
    └── knowledge-graphs.md        # Compiled topic summary
```

## Limitations

- **Lexical retrieval only** — no embeddings. Keyword overlap can surface irrelevant results.
- **OpenAI-compatible only** — no local/offline model support yet.
- **Tree size ceiling** — depends on the model's context window. With `gpt-4.1-mini` (default) you can comfortably compile a few hundred leaves; smaller-context models will hit the ceiling sooner.

## Contributing

PRs welcome.

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

## License

MIT
