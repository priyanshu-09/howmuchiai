# howmuchiai

Open-source scanner for [howmuchiai.xyz](https://howmuchiai.xyz). Scans your local machine for AI tool usage and generates a shareable card link.

**No data leaves your machine.** The CLI reads local files, encodes results into a URL hash, and opens your browser. Nothing is sent to any server.

## Install

```bash
curl -sSL https://raw.githubusercontent.com/priyanshu-09/howmuchiai/main/install.sh | sh
```

Or build from source:
```bash
git clone https://github.com/priyanshu-09/howmuchiai.git
cd howmuchiai
cargo build --release
./target/release/howmuchiai
```

## What it scans

| Source | What's collected |
|--------|-----------------|
| **Claude Code** | Tokens (input/output/cache), sessions, hours, per-model breakdown |
| **Claude Desktop** | Local agent mode sessions (same JSONL format) |
| **ChatGPT Desktop** | Conversation count |
| **Codex (OpenAI)** | Token counts, sessions, hours, models used |
| **OpenCode** | SQLite (`opencode.db`, v1.2+) + legacy JSON; tokens, sessions, hours, per-model, per-day buckets |
| **Amp (AmpCode)** | Thread JSON + usage ledger; tokens, sessions, hours, per-model, per-day buckets |
| **Droid (Factory)** | Session JSON; tokens (input/output/cache/thinking), sessions, hours, normalized model names |
| **Qwen CLI** | JSONL chats; prompt/candidates/thoughts/cache tokens, sessions, hours, per-model |
| **Kimi CLI** | `wire.jsonl` StatusUpdate; tokens, sessions, hours, model from `config.json` |
| **OpenClaw** | Agent transcripts + legacy Clawdbot/Moltbot/Moldbot; tokens, sessions, hours, per-model |
| **Browser History** | Hours on AI web apps (see list below) |
| **Cursor IDE** | Composer sessions, daily AI-generated lines |
| **Shell History** | CLI invocation counts (claude, codex, aider, ollama, etc.) |
| **Warp Terminal AI** | Query count, conversations |
| **Gemini CLI** | Sessions, messages, hours |
| **VS Code Copilot** | Active workspace detection |
| **Aider** | Command count |
| **Continue** | Session count |
| **Ollama** | Installed models |
| **JetBrains AI** | IDE AI activation |
| **Detection** | Codeium, Tabnine, Amazon Q, Supermaven, Windsurf, Roo Code |

### AI web apps detected via browser history

**Chat & research:** Claude.ai, ChatGPT, Gemini, Grok, Meta AI, Perplexity, Poe, DeepSeek, Le Chat (Mistral), Qwen, Kimi, Character.AI, Phind, You.com, HuggingFace Chat, Groq, AI Studio, NotebookLM, Google Labs, Copilot (Microsoft)

**Coding agents & app builders:** Lovable, Devin, v0, Bolt, Cursor Agents

### Browsers scanned

Chrome, Arc, Brave, Dia, Edge, Safari, Firefox

## Usage

```bash
# Default: scan + open card in browser
howmuchiai

# Don't auto-open browser (just print URL)
howmuchiai --no-open

# Raw JSON output (for piping/debugging)
howmuchiai --format json
```

## How it works

1. Scanner runs locally, reads data from each provider
2. Results are base64url-encoded into a URL: `howmuchiai.xyz/c/<hash>`
3. Browser opens the URL — the website decodes the hash client-side and renders your card
4. Nothing touches any server until you explicitly sign in

## Schema

The JSON output conforms to a versioned schema:

- `schema_version` (u32) — top-level field, currently `2`. Consumers should check this to handle breaking changes across scanner versions. If absent on older payloads, treat as `1`.
- `sources[<provider>].daily_buckets` — optional `{ "YYYY-MM-DD": { hours, tokens, sessions, invocations? } }` map. Populated today for `shell_history` (per-day invocation counts) and browser providers (per-day active hours). Other providers may populate it later; decoders MUST handle its absence gracefully.

## Privacy

- **Zero network calls.** The binary has no HTTP dependencies. It cannot phone home.
- **Read-only.** Never writes to your files — only to a temp directory for SQLite copies (auto-cleaned).
- **No URLs stored.** Browser history matches domains in SQL — full URLs never enter memory.
- **No command text.** Shell history returns only tool names and counts, never actual commands.
- **No secrets read.** Never touches `.env` files, API keys, cookies, or credentials.

## License

MIT
