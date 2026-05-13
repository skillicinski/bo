# bo

Collect web pages into a local markdown knowledge tree. Compile topic branches via LLM. Query your collection and get answers with citations back to source material. No cloud service, no vector database — just local files and your own API key.

> **Experimental** — v0.0.1. Expect rough edges and breaking changes.

## Install

```bash
cargo install --git https://github.com/jellysnake/bo --tag v0.0.1
```

## Quickstart

```bash
# Configure your OpenAI API key
bo config auth --provider openai

# Seed a tree
bo seed ~/bo-tree

# Collect some pages
bo collect https://example.com/blog/intro-to-knowledge-graphs
bo collect https://example.com/blog/linked-data-fundamentals

# See what you have
bo list

# Compile into a linked knowledge graph
bo compile

# Ask a question
bo query "How do these concepts relate to each other?"
```

## Commands

| Command | Description |
|---------|-------------|
| `bo seed <dir>` | Initialise a new tree |
| `bo collect <url>` | Fetch and store a web page as a markdown leaf |
| `bo compile` | Build topic branches from collected leaves via LLM |
| `bo query <question>` | Answer a question with citations from your tree |
| `bo list` | List collected leaves |
| `bo search <terms>` | Search leaves by content |
| `bo show <title>` | Display a single leaf |
| `bo config auth --provider openai` | Store your API key |
| `bo config get model` | Show the active model |
| `bo config set model <id>` | Change the LLM model |
| `bo raze` | Delete all bo-managed files and config |

All commands support `--json` for machine-readable output.

## Provider setup

```bash
# Interactive key entry (recommended)
bo config auth --provider openai

# Or use an environment variable
export OPENAI_API_KEY=sk-...
```

Change the model (default `gpt-4o`):

```bash
bo config set model gpt-4.1-mini
```

Supported: `gpt-4o`, `gpt-4o-mini`, `gpt-4.1`, `gpt-4.1-mini`, `gpt-4.1-nano`.

## Storage

```
~/.bo/
├── config.json          # Tree path + model setting
└── auth.json            # Provider credentials (0600 permissions)

~/bo-tree/               # Your tree (location chosen at seed)
├── index.jsonl          # Ledger of collected leaves
├── intro-to-knowledge-graphs.md
├── linked-data-fundamentals.md
└── branch-knowledge-graphs.md   # Branch: compiled topic summary
```

## Limitations

- **Lexical retrieval only** — no embeddings. Keyword overlap can surface irrelevant results.
- **OpenAI-compatible only** — no local/offline model support yet.
- **No incremental compile** — recompiles all branches each run.
- **Tree size ceiling** — depends on model context window (~50 leaves with gpt-4o, ~200+ with gpt-4.1).

## Contributing

PRs welcome.

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

## License

MIT
