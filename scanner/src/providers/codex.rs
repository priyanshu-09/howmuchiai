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

/// Parse a Codex session JSONL file for per-model token usage
fn scan_codex_jsonl(path: &std::path::Path, models: &mut HashMap<String, ModelUsage>) {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return,
    };
    let reader = std::io::BufReader::new(file);

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
        if entry_type != "response_item" {
            continue;
        }

        // Extract model from payload
        let payload = match value.get("payload") {
            Some(p) => p,
            None => continue,
        };

        let model_name = payload
            .get("model")
            .and_then(|m| m.as_str())
            .unwrap_or("unknown")
            .to_string();

        // Look for total_token_usage in payload.info
        let info = match payload.get("info") {
            Some(i) => i,
            None => continue,
        };

        let token_usage = match info.get("total_token_usage") {
            Some(tu) => tu,
            None => continue,
        };

        let input_tokens = token_usage
            .get("input_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let output_tokens = token_usage
            .get("output_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let cache_read = token_usage
            .get("cache_read_input_tokens")
            .or_else(|| token_usage.get("cached_tokens"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        let entry = models.entry(model_name).or_insert_with(|| ModelUsage {
            tokens: TokenUsage::default(),
            sessions: 0,
            hours: 0.0,
        });

        entry.tokens.input_tokens = entry.tokens.input_tokens.saturating_add(input_tokens);
        entry.tokens.output_tokens = entry.tokens.output_tokens.saturating_add(output_tokens);
        entry.tokens.cache_read_tokens = entry.tokens.cache_read_tokens.saturating_add(cache_read);
        entry.tokens.compute_total();
    }
}
