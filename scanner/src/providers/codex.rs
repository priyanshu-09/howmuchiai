use crate::platform;
use crate::providers::Provider;
use crate::sqlite_util::SafeSqlite;
use crate::time_util;
use crate::types::{ModelUsage, ProviderResult, ScanError, TokenUsage};
use std::collections::HashMap;
use std::io::BufRead;

pub struct CodexProvider;

impl Provider for CodexProvider {
    fn name(&self) -> &'static str {
        "codex"
    }

    fn display_name(&self) -> &'static str {
        "Codex (OpenAI)"
    }

    fn is_available(&self) -> bool {
        platform::codex_sqlite().exists()
    }

    fn scan(&self) -> Result<ProviderResult, ScanError> {
        let db_path = platform::codex_sqlite();
        let db = SafeSqlite::open(&db_path)?;

        let mut stmt = db
            .conn()
            .prepare("SELECT id, created_at, updated_at, model, tokens_used FROM threads")?;

        let mut total_tokens: u64 = 0;
        let mut total_hours: f64 = 0.0;
        let mut sessions: u64 = 0;
        let mut all_timestamps: Vec<i64> = Vec::new();
        let mut first_seen: Option<i64> = None;
        let mut last_seen: Option<i64> = None;
        let mut model_tokens_from_db: HashMap<String, u64> = HashMap::new();
        let mut events: Vec<(i64, u64, Option<String>)> = Vec::new();

        let rows = stmt.query_map([], |row| {
            let id: String = row.get(0)?;
            let created_at: i64 = row.get(1)?;
            let updated_at: i64 = row.get(2)?;
            let model: Option<String> = row.get(3)?;
            let tokens_used: Option<i64> = row.get(4)?;
            Ok((id, created_at, updated_at, model, tokens_used))
        })?;

        for row in rows {
            let (id, created_at, updated_at, model, tokens_used) = match row {
                Ok(r) => r,
                Err(_) => continue,
            };

            sessions += 1;

            // Track timestamps
            all_timestamps.push(created_at);
            all_timestamps.push(updated_at);
            first_seen = Some(first_seen.map_or(created_at, |fs: i64| fs.min(created_at)));
            last_seen = Some(last_seen.map_or(updated_at, |ls: i64| ls.max(updated_at)));

            // Sum tokens
            let tok = tokens_used.unwrap_or(0).max(0) as u64;
            total_tokens = total_tokens.saturating_add(tok);

            if let Some(ref model_name) = model {
                *model_tokens_from_db.entry(model_name.clone()).or_insert(0) += tok;
            }

            // Hours: duration of each thread, capped at 24h
            let duration = (updated_at - created_at).clamp(0, 86400);
            total_hours += duration as f64 / 3600.0;

            // Daily-bucket events: attribute tokens to the thread's created_at day,
            // but also record updated_at so per-day `hours` reflects thread spans that
            // cross midnight UTC. The thread `id` serves as the session key.
            events.push((created_at, tok, Some(id.clone())));
            if updated_at != created_at {
                events.push((updated_at, 0, Some(id)));
            }
        }

        // Also scan session JSONL files for per-model token breakdown
        let mut models: HashMap<String, ModelUsage> = HashMap::new();
        let sessions_dir = platform::codex_sessions_dir();
        if sessions_dir.exists() {
            let pattern = format!("{}/**/*.jsonl", sessions_dir.display());
            if let Ok(paths) = glob::glob(&pattern) {
                for path in paths.filter_map(|p| p.ok()) {
                    scan_codex_jsonl(&path, &mut models);
                }
            }
        }

        // If JSONL scanning didn't find model data, use DB data
        if models.is_empty() && !model_tokens_from_db.is_empty() {
            for (model_name, tok) in &model_tokens_from_db {
                // We only have total tokens from DB, attribute to output
                let mut usage = TokenUsage {
                    output_tokens: *tok,
                    ..Default::default()
                };
                usage.compute_total();
                models.insert(
                    model_name.clone(),
                    ModelUsage {
                        tokens: usage,
                        sessions: 0,
                        hours: 0.0,
                    },
                );
            }
        }

        // Build aggregate token usage
        let mut aggregate_tokens = TokenUsage::default();
        if !models.is_empty() {
            for model_usage in models.values() {
                aggregate_tokens.merge(&model_usage.tokens);
            }
        } else {
            // Fall back to DB total
            aggregate_tokens.output_tokens = total_tokens;
            aggregate_tokens.compute_total();
        }

        let mut result = ProviderResult::new("Codex (OpenAI)");
        result.hours = Some(if total_hours > 0.0 {
            total_hours
        } else {
            // Estimate from timestamps
            all_timestamps.sort_unstable();
            time_util::active_hours_from_timestamps(&all_timestamps, 1800)
        });
        result.tokens = Some(aggregate_tokens);
        result.sessions = Some(sessions);
        result.first_seen = first_seen;
        result.last_seen = last_seen;

        if !models.is_empty() {
            result.models = Some(models);
        }

        let daily_buckets = time_util::build_daily_buckets(&events);
        if !daily_buckets.is_empty() {
            result.daily_buckets = Some(daily_buckets);
        }

        Ok(result)
    }
}

/// Parse a Codex session JSONL file for per-model token usage.
///
/// Codex CLI ≥ 0.114 emits per-turn token deltas in `event_msg` events with
/// `payload.type == "token_count"`, not in `response_item`. Each event carries
/// `info.last_token_usage` (per-turn delta) and `info.total_token_usage`
/// (session-cumulative). We accumulate `last_token_usage` so multiple events
/// in one session sum without double-counting.
///
/// The active model lives in `turn_context.payload.model` and can change
/// mid-session (e.g. switching from gpt-5.4 to gpt-5.3-codex). Each
/// `token_count` event is attributed to whichever model was last announced.
///
/// Field mapping into TokenUsage. Note OpenAI reports `input_tokens` as the
/// TOTAL input including cache hits, with `cached_input_tokens` as a subset.
/// Anthropic reports them disjoint. To normalize, we subtract:
///   input_tokens − cached_input_tokens → input_tokens   (uncached input)
///   cached_input_tokens                → cache_read_tokens
///   output_tokens                      → output_tokens  (regular completion)
///   reasoning_output_tokens            → output_tokens  (billed at output rate)
fn scan_codex_jsonl(path: &std::path::Path, models: &mut HashMap<String, ModelUsage>) {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return,
    };
    let reader = std::io::BufReader::new(file);

    let mut current_model: Option<String> = None;
    // Track the previous cumulative `total_token_usage` per model so we can
    // synthesise a per-turn delta when `last_token_usage` is null (older
    // Codex CLI versions, aborted turns). Mirrors ccusage's
    // `subtractRawUsage(totalUsage, previousTotals)` fallback.
    let mut prev_total_per_model: HashMap<String, (u64, u64, u64, u64)> = HashMap::new();

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };

        if line.trim().is_empty() {
            continue;
        }

        let value: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let entry_type = value.get("type").and_then(|t| t.as_str()).unwrap_or("");
        let payload = match value.get("payload") {
            Some(p) => p,
            None => continue,
        };

        // Track active model from turn_context events.
        if entry_type == "turn_context" {
            if let Some(m) = payload.get("model").and_then(|v| v.as_str()) {
                let trimmed = m.trim();
                if !trimmed.is_empty() {
                    current_model = Some(trimmed.to_string());
                }
            }
            continue;
        }

        // The token_count event is wrapped inside event_msg.
        if entry_type != "event_msg" {
            continue;
        }
        if payload.get("type").and_then(|t| t.as_str()) != Some("token_count") {
            continue;
        }

        let info = match payload.get("info") {
            Some(i) => i,
            None => continue,
        };

        let model_name = current_model
            .clone()
            .unwrap_or_else(|| "unknown".to_string());

        // Prefer `last_token_usage` (per-turn delta). When it's null —
        // older Codex CLI versions and aborted turns — synthesise a delta
        // by subtracting the previous cumulative `total_token_usage` for
        // this model from the current one. ccusage does the same so we
        // don't undercount on those events.
        let last_obj = info.get("last_token_usage");
        let total_obj = info.get("total_token_usage");

        let (raw_input, output_tokens, reasoning_tokens, cached_input) =
            if let Some(last) = last_obj.filter(|v| !v.is_null()) {
                (
                    last.get("input_tokens")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0),
                    last.get("output_tokens")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0),
                    last.get("reasoning_output_tokens")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0),
                    last.get("cached_input_tokens")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0),
                )
            } else if let Some(total) = total_obj.filter(|v| !v.is_null()) {
                let cur_in = total
                    .get("input_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let cur_out = total
                    .get("output_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let cur_reasoning = total
                    .get("reasoning_output_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let cur_cached = total
                    .get("cached_input_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let (prev_in, prev_out, prev_reasoning, prev_cached) = prev_total_per_model
                    .get(&model_name)
                    .copied()
                    .unwrap_or((0, 0, 0, 0));
                prev_total_per_model.insert(
                    model_name.clone(),
                    (cur_in, cur_out, cur_reasoning, cur_cached),
                );
                (
                    cur_in.saturating_sub(prev_in),
                    cur_out.saturating_sub(prev_out),
                    cur_reasoning.saturating_sub(prev_reasoning),
                    cur_cached.saturating_sub(prev_cached),
                )
            } else {
                continue;
            };

        // If we used `last_token_usage`, also keep `prev_total_per_model`
        // up to date so a subsequent null-last event can fall back cleanly.
        if last_obj.map(|v| !v.is_null()).unwrap_or(false) {
            if let Some(total) = total_obj.filter(|v| !v.is_null()) {
                prev_total_per_model.insert(
                    model_name.clone(),
                    (
                        total
                            .get("input_tokens")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0),
                        total
                            .get("output_tokens")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0),
                        total
                            .get("reasoning_output_tokens")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0),
                        total
                            .get("cached_input_tokens")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0),
                    ),
                );
            }
        }

        // Strip cache hits out of `input_tokens` so the field carries the
        // same "uncached input" meaning as Anthropic's. Saturating sub
        // protects against malformed events where cached > input.
        let uncached_input = raw_input.saturating_sub(cached_input);

        if uncached_input == 0 && output_tokens == 0 && reasoning_tokens == 0 && cached_input == 0 {
            continue;
        }

        let entry = models.entry(model_name).or_insert_with(|| ModelUsage {
            tokens: TokenUsage::default(),
            sessions: 0,
            hours: 0.0,
        });

        entry.tokens.input_tokens = entry.tokens.input_tokens.saturating_add(uncached_input);
        entry.tokens.output_tokens = entry
            .tokens
            .output_tokens
            .saturating_add(output_tokens)
            .saturating_add(reasoning_tokens);
        entry.tokens.cache_read_tokens =
            entry.tokens.cache_read_tokens.saturating_add(cached_input);
        entry.tokens.compute_total();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn write_jsonl(lines: &[&str]) -> NamedTempFile {
        let mut f = NamedTempFile::new().expect("tempfile");
        for line in lines {
            writeln!(f, "{}", line).unwrap();
        }
        f.flush().unwrap();
        f
    }

    #[test]
    fn parses_per_turn_deltas_and_subtracts_cache_from_input() {
        // Single-model session: three token_count events. last_token_usage
        // values should accumulate (no cumulative double count). Cached input
        // should be split out of `input_tokens`.
        let lines = [
            r#"{"type":"turn_context","payload":{"model":"gpt-5.3-codex"}}"#,
            r#"{"type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":10000,"cached_input_tokens":2000,"output_tokens":500,"reasoning_output_tokens":100}}}}"#,
            r#"{"type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":15000,"cached_input_tokens":12000,"output_tokens":300,"reasoning_output_tokens":50}}}}"#,
            r#"{"type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":20000,"cached_input_tokens":18000,"output_tokens":200,"reasoning_output_tokens":0}}}}"#,
        ];
        let f = write_jsonl(&lines);
        let mut models = HashMap::new();
        scan_codex_jsonl(f.path(), &mut models);

        let codex = models.get("gpt-5.3-codex").expect("model populated");
        // uncached input = (10000-2000) + (15000-12000) + (20000-18000) = 13000
        assert_eq!(codex.tokens.input_tokens, 13_000);
        // output = 500+300+200 + reasoning 100+50+0 = 1150
        assert_eq!(codex.tokens.output_tokens, 1_150);
        // cache_read = sum of cached_input = 2000+12000+18000 = 32000
        assert_eq!(codex.tokens.cache_read_tokens, 32_000);
        // total = input + output + cache_read + cache_creation per launch-ready
        // compute_total (matches openusage / ccusage / CodexBar conventions).
        assert_eq!(codex.tokens.total, 46_150);
    }

    #[test]
    fn attributes_to_active_model_through_mid_session_switch() {
        // turn_context flips from gpt-5.4 to gpt-5.3-codex; tokens after the
        // switch belong to gpt-5.3-codex, not the earlier model.
        let lines = [
            r#"{"type":"turn_context","payload":{"model":"gpt-5.4"}}"#,
            r#"{"type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":1000,"cached_input_tokens":0,"output_tokens":100}}}}"#,
            r#"{"type":"turn_context","payload":{"model":"gpt-5.3-codex"}}"#,
            r#"{"type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":2000,"cached_input_tokens":500,"output_tokens":200}}}}"#,
        ];
        let f = write_jsonl(&lines);
        let mut models = HashMap::new();
        scan_codex_jsonl(f.path(), &mut models);

        let a = models.get("gpt-5.4").expect("first model populated");
        assert_eq!(a.tokens.input_tokens, 1_000);
        assert_eq!(a.tokens.output_tokens, 100);

        let b = models.get("gpt-5.3-codex").expect("second model populated");
        assert_eq!(b.tokens.input_tokens, 1_500); // 2000 - 500 cached
        assert_eq!(b.tokens.output_tokens, 200);
        assert_eq!(b.tokens.cache_read_tokens, 500);
    }

    #[test]
    fn falls_back_to_total_token_usage_when_last_is_null() {
        // Older Codex CLI versions and aborted-turn events emit a
        // token_count with `last_token_usage: null` but a populated
        // `total_token_usage`. The parser should subtract the previous
        // cumulative to synthesise a per-turn delta — same behaviour as
        // ccusage's subtractRawUsage fallback.
        let lines = [
            r#"{"type":"turn_context","payload":{"model":"gpt-5.3-codex"}}"#,
            // First event: cumulative 1000 in / 100 out — no prior, so the full
            // amount becomes the delta.
            r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1000,"output_tokens":100,"cached_input_tokens":0,"reasoning_output_tokens":0},"last_token_usage":null}}}"#,
            // Second event: cumulative 3000 in / 250 out — delta is 2000 / 150.
            r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":3000,"output_tokens":250,"cached_input_tokens":500,"reasoning_output_tokens":0},"last_token_usage":null}}}"#,
        ];
        let f = write_jsonl(&lines);
        let mut models = HashMap::new();
        scan_codex_jsonl(f.path(), &mut models);

        let codex = models.get("gpt-5.3-codex").expect("model populated");
        // Synthesised deltas: input=1000+2000=3000 (raw), cached=0+500=500,
        // uncached input = 3000 - 500 = 2500. output = 100 + 150 = 250.
        assert_eq!(codex.tokens.input_tokens, 2_500);
        assert_eq!(codex.tokens.output_tokens, 250);
        assert_eq!(codex.tokens.cache_read_tokens, 500);
    }

    #[test]
    fn ignores_legacy_response_item_path() {
        // `response_item` events used to carry token usage on older Codex
        // builds; the new parser strictly looks at event_msg/token_count and
        // should not accidentally pick up these legacy fields.
        let lines = [
            r#"{"type":"turn_context","payload":{"model":"gpt-5.3-codex"}}"#,
            r#"{"type":"response_item","payload":{"model":"gpt-5.3-codex","info":{"total_token_usage":{"input_tokens":99999,"output_tokens":99999}}}}"#,
            r#"{"type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":1000,"cached_input_tokens":0,"output_tokens":100}}}}"#,
        ];
        let f = write_jsonl(&lines);
        let mut models = HashMap::new();
        scan_codex_jsonl(f.path(), &mut models);

        let codex = models.get("gpt-5.3-codex").expect("model populated");
        // Only the event_msg/token_count event contributes; response_item is ignored.
        assert_eq!(codex.tokens.input_tokens, 1_000);
        assert_eq!(codex.tokens.output_tokens, 100);
    }

    #[test]
    fn falls_back_to_unknown_when_no_turn_context_yet() {
        // Some sessions emit token_count before the first turn_context.
        // Those tokens are real and should be tracked under "unknown".
        let lines = [
            r#"{"type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":500,"cached_input_tokens":0,"output_tokens":50}}}}"#,
        ];
        let f = write_jsonl(&lines);
        let mut models = HashMap::new();
        scan_codex_jsonl(f.path(), &mut models);

        let unknown = models.get("unknown").expect("fallback model");
        assert_eq!(unknown.tokens.input_tokens, 500);
        assert_eq!(unknown.tokens.output_tokens, 50);
    }
}
