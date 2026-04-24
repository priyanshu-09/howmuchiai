use crate::platform;
use crate::providers::Provider;
use crate::providers::opencode::{build_daily_buckets, ms_to_secs};
use crate::time_util;
use crate::types::{ModelUsage, ProviderResult, ScanError, TokenUsage};
use std::collections::{HashMap, HashSet};
use std::path::Path;

pub struct AmpProvider;

impl Provider for AmpProvider {
    fn name(&self) -> &'static str {
        "amp"
    }

    fn display_name(&self) -> &'static str {
        "Amp"
    }

    fn is_available(&self) -> bool {
        !platform::amp_threads_dirs().is_empty()
    }

    fn scan(&self) -> Result<ProviderResult, ScanError> {
        let roots = platform::amp_threads_dirs();
        if roots.is_empty() {
            return Err(ScanError::NotFound("Amp threads dir not found".into()));
        }

        let mut models: HashMap<String, ModelUsage> = HashMap::new();
        let mut session_ids: HashSet<String> = HashSet::new();
        let mut session_timestamps: HashMap<String, Vec<i64>> = HashMap::new();
        let mut assistant_rows: Vec<(i64, u64, String)> = Vec::new();
        let mut first_seen: Option<i64> = None;
        let mut last_seen: Option<i64> = None;

        for root in &roots {
            let pattern = format!("{}/*.json", root.display());
            let Ok(paths) = glob::glob(&pattern) else { continue };
            for path in paths.filter_map(|p| p.ok()) {
                scan_thread(
                    &path,
                    &mut models,
                    &mut session_ids,
                    &mut session_timestamps,
                    &mut assistant_rows,
                    &mut first_seen,
                    &mut last_seen,
                );
            }
        }

        if session_ids.is_empty() && models.is_empty() {
            return Err(ScanError::NotFound("No Amp thread data found".into()));
        }

        let mut total_hours = 0.0_f64;
        for timestamps in session_timestamps.values() {
            let mut sorted = timestamps.clone();
            sorted.sort_unstable();
            total_hours += time_util::active_hours_from_timestamps(&sorted, 1800);
        }

        let mut total_tokens = TokenUsage::default();
        for mu in models.values() {
            total_tokens.merge(&mu.tokens);
        }

        let daily_buckets = build_daily_buckets(&assistant_rows);

        let mut result = ProviderResult::new("Amp");
        result.hours = Some(total_hours);
        result.tokens = Some(total_tokens);
        result.sessions = Some(session_ids.len() as u64);
        result.first_seen = first_seen;
        result.last_seen = last_seen;
        if !models.is_empty() {
            result.models = Some(models);
        }
        if !daily_buckets.is_empty() {
            result.daily_buckets = Some(daily_buckets);
        }

        Ok(result)
    }
}

#[allow(clippy::too_many_arguments)]
fn scan_thread(
    path: &Path,
    models: &mut HashMap<String, ModelUsage>,
    session_ids: &mut HashSet<String>,
    session_timestamps: &mut HashMap<String, Vec<i64>>,
    assistant_rows: &mut Vec<(i64, u64, String)>,
    first_seen: &mut Option<i64>,
    last_seen: &mut Option<i64>,
) {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return,
    };
    let v: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return,
    };

    let session_id = v
        .get("id")
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
        .or_else(|| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| "unknown".into());
    session_ids.insert(session_id.clone());

    let thread_created = v
        .get("created")
        .and_then(|x| x.as_i64())
        .map(ms_to_secs);

    // Dedup keys across ledger + messages: prefer messageId, fall back to (model, tokens) tuple
    let mut seen_msg_ids: HashSet<String> = HashSet::new();
    let mut seen_fingerprint: HashSet<String> = HashSet::new();

    // 1) Ledger events (preferred for per-model token data)
    if let Some(events) = v
        .get("usageLedger")
        .and_then(|l| l.get("events"))
        .and_then(|e| e.as_array())
    {
        for ev in events {
            let ts = ev
                .get("timestamp")
                .and_then(|t| t.as_str())
                .and_then(time_util::iso8601_to_unix)
                .or(thread_created)
                .unwrap_or_else(|| file_mtime(path).unwrap_or(0));

            let model = ev
                .get("model")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown")
                .to_string();

            let t = match ev.get("tokens") {
                Some(t) => t,
                None => continue,
            };

            let input = t.get("input").and_then(|x| x.as_u64()).unwrap_or(0);
            let output = t.get("output").and_then(|x| x.as_u64()).unwrap_or(0);
            let cache_read = t
                .get("cache_read_input")
                .and_then(|x| x.as_u64())
                .unwrap_or(0);
            let cache_write = t
                .get("cache_creation_input")
                .and_then(|x| x.as_u64())
                .unwrap_or(0);

            if input + output + cache_read + cache_write == 0 {
                continue;
            }

            let fp = format!("{}|{}|{}|{}|{}", model, input, output, cache_read, cache_write);
            if !seen_fingerprint.insert(fp) {
                continue;
            }

            accumulate_model(
                models,
                &model,
                input,
                output,
                0,
                cache_read,
                cache_write,
            );
            push_ts(
                session_timestamps,
                &session_id,
                ts,
                first_seen,
                last_seen,
            );
            assistant_rows.push((ts, input.saturating_add(output), session_id.clone()));
        }
    }

    // 2) Per-message usage (fallback / supplement if ledger missing or message-only)
    if let Some(messages) = v.get("messages").and_then(|m| m.as_array()) {
        for msg in messages {
            let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");
            if role != "assistant" {
                continue;
            }
            let mid = msg
                .get("messageId")
                .and_then(|x| x.as_i64())
                .map(|n| n.to_string())
                .or_else(|| {
                    msg.get("id")
                        .and_then(|x| x.as_str())
                        .map(|s| s.to_string())
                });
            if let Some(mid_str) = &mid {
                if !seen_msg_ids.insert(mid_str.clone()) {
                    continue;
                }
            }
            let usage = match msg.get("usage") {
                Some(u) => u,
                None => continue,
            };
            let model = msg
                .get("model")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown")
                .to_string();

            let input = usage
                .get("input_tokens")
                .or_else(|| usage.get("input"))
                .and_then(|x| x.as_u64())
                .unwrap_or(0);
            let output = usage
                .get("output_tokens")
                .or_else(|| usage.get("output"))
                .and_then(|x| x.as_u64())
                .unwrap_or(0);
            let cache_read = usage
                .get("cache_read_input_tokens")
                .or_else(|| usage.get("cache_read_input"))
                .and_then(|x| x.as_u64())
                .unwrap_or(0);
            let cache_write = usage
                .get("cache_creation_input_tokens")
                .or_else(|| usage.get("cache_creation_input"))
                .and_then(|x| x.as_u64())
                .unwrap_or(0);

            if input + output + cache_read + cache_write == 0 {
                continue;
            }

            let fp = format!("{}|{}|{}|{}|{}", model, input, output, cache_read, cache_write);
            if !seen_fingerprint.insert(fp) {
                continue;
            }

            accumulate_model(models, &model, input, output, 0, cache_read, cache_write);

            let ts = thread_created
                .or_else(|| file_mtime(path))
                .unwrap_or(0);
            push_ts(session_timestamps, &session_id, ts, first_seen, last_seen);
            assistant_rows.push((ts, input.saturating_add(output), session_id.clone()));
        }
    }

    // If we have no token data, still record thread_created for hours
    if let Some(ts) = thread_created {
        push_ts(session_timestamps, &session_id, ts, first_seen, last_seen);
    }
}

fn push_ts(
    session_timestamps: &mut HashMap<String, Vec<i64>>,
    session_id: &str,
    ts: i64,
    first_seen: &mut Option<i64>,
    last_seen: &mut Option<i64>,
) {
    if ts <= 0 {
        return;
    }
    session_timestamps
        .entry(session_id.to_string())
        .or_default()
        .push(ts);
    *first_seen = Some(first_seen.map_or(ts, |fs| fs.min(ts)));
    *last_seen = Some(last_seen.map_or(ts, |ls| ls.max(ts)));
}

pub(crate) fn accumulate_model(
    models: &mut HashMap<String, ModelUsage>,
    model: &str,
    input: u64,
    output: u64,
    reasoning: u64,
    cache_read: u64,
    cache_write: u64,
) {
    let entry = models
        .entry(model.to_string())
        .or_insert_with(|| ModelUsage {
            tokens: TokenUsage::default(),
            sessions: 0,
            hours: 0.0,
        });
    entry.tokens.input_tokens = entry.tokens.input_tokens.saturating_add(input);
    entry.tokens.output_tokens = entry
        .tokens
        .output_tokens
        .saturating_add(output.saturating_add(reasoning));
    entry.tokens.cache_read_tokens = entry.tokens.cache_read_tokens.saturating_add(cache_read);
    entry.tokens.cache_creation_tokens = entry
        .tokens
        .cache_creation_tokens
        .saturating_add(cache_write);
    entry.tokens.compute_total();
}

fn file_mtime(path: &Path) -> Option<i64> {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
}
