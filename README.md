# howmuchiai

Spotify Wrapped for your AI usage. Scan your machine, see how much you AI'd, share a beautiful card.

## What it does

`howmuchiai` scans your local machine for AI tool usage across every platform — Claude Code, ChatGPT, Cursor, Copilot, Gemini, Codex, and more — then generates a shareable usage card with your stats.

**No data leaves your machine.** The CLI reads local files only. Zero network calls.

## Quick start

```bash
# Build from source (requires Rust)
git clone https://github.com/priyanshu-09/howmuchiai.git
cd howmuchiai
cargo build --release
./target/release/howmuchiai
```

## What it scans

### Tier 1 — Rich usage data (hours, tokens, sessions)

| Source | What's collected |
|--------|-----------------|
| **Claude Code** | Tokens (input/output/cache), sessions, hours, per-model breakdown |
| **Codex (OpenAI)** | Token counts, sessions, hours, models used |
| **Browser History** | Hours spent on AI web apps (Claude.ai, ChatGPT, Gemini, Perplexity, DeepSeek, etc.) |
| **Cursor IDE** | Composer sessions, daily AI-generated lines |
| **Shell History** | CLI invocation counts (claude, codex, aider, ollama, etc.) |

### Tier 2 — Moderate data

| Source | What's collected |
|--------|-----------------|
| **Warp Terminal AI** | Query count, conversations |
| **Gemini CLI** | Sessions, messages, hours |
| **VS Code Copilot** | Active workspace detection |
| **Aider** | Command count |
| **Continue** | Session count |
| **Ollama** | Installed models |
| **JetBrains AI** | IDE AI activation |

### Tier 3 — Detection only

Codeium, Tabnine, Amazon Q, Supermaven, Windsurf, Roo Code

### Browser AI apps detected

Claude.ai, ChatGPT, Gemini, Perplexity, Poe, Phind, DeepSeek, Groq, HuggingFace Chat, AI Studio, You.com, Google Labs, Together AI, Copilot Web

**Browsers scanned:** Chrome, Arc, Brave, Edge, Safari, Firefox

## Output formats

```bash
# Pretty terminal output (default)
howmuchiai

# JSON output
howmuchiai --format json

# Base64url card URL
howmuchiai --format card-url
```

## Tier system

| Threshold | Title |
|-----------|-------|
| 1000+ hours or 10M+ tokens | The Singularity |
| 500+ hours or 5M+ tokens | Neural Link |
| 200+ hours or 2M+ tokens | The Architect |
| 100+ hours or 1M+ tokens | Prompt Native |
| 50+ hours | Vibe Coder |
| 10+ hours | The Explorer |
| < 10 hours | The Purist |

Every tier sounds cool. Nobody gets shamed.

## Privacy

- **Zero network calls.** The binary has no HTTP dependencies. It cannot phone home.
- **Read-only.** The scanner never writes to your files — only to a temporary directory for SQLite copies (auto-cleaned).
- **No URLs stored.** Browser history is matched to domain names in SQL — full URLs never enter Rust memory.
- **No command text.** Shell history matching returns only tool names and counts, never your actual commands.
- **No secrets read.** The scanner never touches `.env` files, API keys, cookies, or credentials.

## How it works

1. **Claude Code**: Parses `~/.claude/projects/**/*.jsonl` for token usage, session IDs, timestamps
2. **Browser History**: Copies locked SQLite DBs to temp, queries for AI domain visits with time-on-site
3. **Codex**: Reads `~/.codex/state_5.sqlite` threads table + session JSONL for token breakdowns
4. **Cursor**: Parses `state.vscdb` for composer sessions and daily AI code stats
5. **Shell History**: Regex-matches `~/.zsh_history` / `~/.bash_history` for AI tool commands

All providers run in parallel via [rayon](https://github.com/rayon-rs/rayon) for fast scanning.

## Supported platforms

- macOS (primary, fully tested)
- Linux (paths supported, needs testing)

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

## License

MIT
