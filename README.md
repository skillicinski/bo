# bo

Collect web content that you don't have time to read as a local markdown knowledge tree. Compile the content into common themes and topics using an LLM. Your collection can now be queried for the information you need and answer questions that span every raw document.

Your collection stays legible and can be viewed with any markdown reader. The idea is to skip RAG and vector databases altogether and just rely on local files and let LLMs do what they're best at. The CLI is intended to be machine-friendly for use via any coding assistant/harness.

All you need is to bring your own API key!

## Who is this for?

**Habitual tab and link hoarders. People who "save it for later". Folks that already forgot the takeaways.**

If you ever used [Pocket](https://getpocket.com/home) or similar products then then the idea is similarly straightforward; capture links to stuff that looks interesting and useful for you to catch up on when you have time. The added value in this tool is that you can easily derive the core claims and insights from the knowledge housed in all that stuff. Taking inspiration from other OSS that tries to "do good" with consumer-grade AI, the intent is for bo to stay model-agnostic, local and free to use.

The goal is not to replace journaling tools like Obsidian, automated cloud research tools like NotebookLM or vector-based RAG technologies. It is a pretty niche package but hopefully specialised at doing a few things very well. If anything, bo is meant to be valuable as a companion to way more advanced tools.

## Install

Pre-built binaries for macOS (Intel + Apple Silicon) and Linux x86_64.

```bash
npm install -g @skillicinski/bo
```

Alternatively, build from source:

```bash
cargo install --git https://github.com/skillicinski/bo --tag v0.0.2
```

## Quickstart

```bash
# Seed a tree and choose the provider/model
bo seed --path ~/bo-tree --name bo-tree --provider openai --model gpt-4.1-mini

# Make your API key available (either env var or ~/.bo/auth.json — see below)
export OPENAI_API_KEY=sk-...

# Collect some pages — single URL, many URLs, or a .txt file with one URL per line
bo collect https://example.com/blog/intro-to-knowledge-graphs
bo collect https://example.com/a https://example.com/b
bo collect urls.txt

# Inspect what you have stored
bo list
bo status

# Read some collected content
bo show "Intro to Knowledge Graphs"

# Identify themes/topics across documents
bo compile

# Ask a question
bo query "Can a Knowledge Graph be explained in three sentences?"
```

## Commands

See [docs/usage.md](docs/usage.md) for a detailed walkthrough with examples and workflow loops.

Commands other than `seed` support `--json` for machine-readable output, intended for use by coding assistants and scripts. `bo seed` stays human-readable and rejects `--json`.

## Provider setup

bo currently supports **OpenAI**, **DeepSeek**, and **Google**.

### 1. Choose provider and model

```bash
bo config --provider openai --model gpt-4.1-mini
# or
bo config --provider deepseek --model deepseek-v4-flash
# or
bo config --provider google --model gemini-2.5-flash
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
export GEMINI_API_KEY=...
```

**Option B — `~/.bo/auth.json`** (chmod 600):

```json
{
  "openai_api_key": "sk-...",
  "deepseek_api_key": "sk-...",
  "google_api_key": "..."
}
```

See [docs/providers.md](docs/providers.md) for per-provider model tables and API-specific behaviour.

## License

MIT
