# Browser web-tool token estimation

How Much AI estimates tokens for browser-based AI tools (ChatGPT, Gemini,
Perplexity, NotebookLM) because **provider-logged usage does not exist on disk**.
This document is the proof matrix: what we scan, what we find, and why Tier 2/3
is the closest local estimate for tokens and derived cost (Rs on the dashboard).

## Architecture

```
Browser history (all browsers)     → active hours (accurate)
Chromium IndexedDB (Tier 1)        → message JSON → cl100k_base (rare)
Chromium Local Storage (Tier 1.5/2)→ snippets + session IDs
Active hours × benchmark (Tier 3)  → floor when metadata is thin
```

Displayed tokens = `max(Tier1, Tier1.5, Tier2, Tier3)`. Every estimate ships
`tokens_estimate_evidence` (schema v6) documenting which sources were checked and
why the winner tier was chosen.

## Per-platform proof matrix

| Platform | Local paths scanned | What exists on disk | Why accurate tokens are impossible locally | Typical winner |
|----------|--------------------|--------------------|---------------------------------------------|----------------|
| **ChatGPT** | Chromium IDB `https_chatgpt.com_0.indexeddb.leveldb`; LS keys `conversation-history*`, `starred-conversations`; Safari WebKit WebsiteData | Sidebar UUIDs, titles, snippets; IDB often PEM/certs/metadata | Full chat threads are server-side; local cache is a partial sidebar subset | Tier 3 (hours floor) or Tier 2 when sessions dominate |
| **Gemini** | Chromium LS origin `gemini.google.com`; Safari WebKit | Session/auth prefs; no structured thread list | No IndexedDB folder mapped; no message bodies cached | Tier 3 |
| **Perplexity** | Chromium LS `threads-v2`, `threadId` | Thread IDs and names | No message bodies in Local Storage | Tier 2 or Tier 3 |
| **NotebookLM** | Chromium LS `notebooks[]` | Notebook IDs | Notes and chats are server-side | Tier 2 or Tier 3 |
| **Grok / Lovable** | Chromium IDB only | Rare message JSON; often unavailable | Non-priority: no Tier 2/3 fallback | Tier 1 or unavailable |

## Sources explicitly checked (exhaustion audit)

| Source | Scanned? | Token signal? |
|--------|----------|---------------|
| Chrome/Arc/Brave/Edge/Dia **History** | Yes | Hours only |
| Chromium **IndexedDB** | Yes (ChatGPT, Grok, Lovable) | Message JSON when quality gate passes |
| Chromium **Local Storage** | Yes (priority domains) | Session counts + snippets |
| Safari **History** | Yes | Hours only |
| Safari **WebKit WebsiteData** | Yes (listing probe) | No message bodies found for AI origins |
| Firefox **History** | Yes | Hours only |
| Firefox / Safari web storage | No parser (probe confirms absence) | None |
| Service Worker / Cache Storage | Not scanned | Providers do not persist usage here |
| Cookies / HAR / network logs | Not scanned | Not persisted locally by default |
| Provider billing / usage API | Out of scope | Requires OAuth; not local scanner |

## Tier definitions

### Tier 1 — IndexedDB transcripts
- **Method:** `cl100k_base` over message fields in LevelDB JSON
- **Accurate when:** `message_chars >= 200` and sparse-vs-hours quality gate passes
- **`accurate_count_possible: true`** only in this case

### Tier 1.5 — Local Storage snippets
- **Method:** `cl100k_base` over `title`, `snippet`, `content` fields
- **Limitation:** Titles/snippets only — not full conversations

### Tier 2 — Session metadata
- **Constants:** ChatGPT 35k/conv, Perplexity 12k/thread, NotebookLM 80k/notebook
- **Method:** deduped session count × median prior (not fitted to user data)
- **`accurate_count_possible: false`** — metadata multiplication, not transcript sum

### Tier 3 — Active hours benchmark
- **Constants:** ChatGPT 1.2M/h, Gemini 900k/h, Perplexity 700k/h, NotebookLM 500k/h
- **Method:** `hours × benchmark` (min 500 tokens)
- **`accurate_count_possible: false`** — engineering prior anchored below Cursor
  observed ratio (~5M tokens/h) to avoid wild overcount for casual web chat

## Rs (cost) linkage

The scanner does **not** compute Rs. The dashboard applies per-model rates from
`pricing.ts` to token fields. When `tokens_estimated: true` and
`tokens_estimate_evidence.accurate_count_possible: false`, cost must display as
approximate (`≈`) with the same tier tooltip as tokens.

## Verification

```bash
# Unit tests
cd scanner && cargo test --lib browser_data_audit

# Live exhaustion audit (dev machine with Chrome)
cargo test --lib live_browser_audit_exhaustion -- --ignored --nocapture

# Full scan evidence check
./scripts/verify-local.sh --full-scan
```

## What we do not claim

- Tier 2/3 numbers are not provider-logged usage
- `max()` may select Tier 3 over Tier 2 when hours imply higher usage than cached sidebar counts
- Cloud account history (OpenAI/Google APIs) is out of scope for the local scanner
