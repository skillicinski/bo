# bo

Collect web pages into a local markdown tree.

## Run locally

```bash
cargo run -- <command>
```

Or install the binary:

```bash
cargo install --path .
```

## Commands

```
bo seed <output-dir>   # Initialise a tree
bo collect <url>       # Fetch a URL and collect it
bo list [--recent] [--branch <branch>] [--limit <n>] [--json]  # List collected leaves
OPENAI_API_KEY=sk-... bo compile  # Build linked knowledge graph from collected docs
bo raze                # Delete all bo-managed files and config
```
