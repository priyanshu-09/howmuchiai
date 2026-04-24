# Changelog

All notable changes to this project are documented here.

## [Unreleased]

## [0.3.0]
### Added
- **OpenCode provider** — reads SQLite `opencode.db` (v1.2+) as primary source, falls back to legacy `storage/message/**` JSON layout. Emits tokens, sessions, hours, per-model breakdown, and daily_buckets.
- **Amp (AmpCode) provider** — reads `~/.local/share/amp/threads/*.json`, prefers `usageLedger.events` with per-message fallback and `(model, tokens)` fingerprint dedup.
- **Droid (Factory) provider** — reads `~/.factory/sessions/*.json` with `normalize_model_name()` (strips `custom:` prefix, `[Provider]` brackets, lowercases, dots→hyphens, collapses consecutive hyphens).
- **Qwen CLI provider** — reads `~/.qwen/projects/**/chats/*.jsonl`, aggregates `promptTokenCount`/`candidatesTokenCount`/`thoughtsTokenCount`/`cachedContentTokenCount` from `usageMetadata`.
- **Kimi CLI provider** — reads `~/.kimi/sessions/*/*/wire.jsonl` `StatusUpdate` frames, honors model override from `~/.kimi/config.json` (defaults to `kimi-for-coding`), dedups by `message_id`.
- **OpenClaw provider** — reads `~/.openclaw/agents/**/*.jsonl` and legacy `.clawdbot`/`.moltbot`/`.moldbot` dirs; state-machine over `model_change`/`custom`/`message` entries to attribute tokens when messages lack inline model info; supports legacy `sessions.json` index.
- **Shell history patterns**: `opencode`, `devin`, `amp`, `droid`, `qwen`, `kimi` CLIs now counted in invocation totals.
- `daily_buckets` emission for all new providers (per-day tokens + sessions) for heatmap/streak parity with shell_history and browser providers.

### Changed
- OpenCode legacy JSON glob fixed: `storage/message/**` (was incorrectly `storage/session/message/**`).
- Shared helpers `build_daily_buckets`, `ms_to_secs`, `accumulate_model` extracted so future providers get per-day bucketing + model aggregation for free.

## [0.2.0]
### Added
- `ScanResult.schema_version` (u32, defaults to 2) so the web app can detect schema epochs.
- `ProviderResult.daily_buckets` (optional map of YYYY-MM-DD → hours/tokens/sessions/invocations) for real streak + heatmap widgets. Implemented for shell_history and browser providers; other providers may opt in later.
- Windows release binary (x86_64-pc-windows-msvc).
