use crate::platform;
use crate::providers::Provider;
use crate::providers::amp::accumulate_model;
use crate::providers::opencode::build_daily_buckets;
use crate::time_util;
use crate::types::{ModelUsage, ProviderResult, ScanError, TokenUsage};
use std::collections::{HashMap, HashSet};
use std::path::Path;

pub struct DroidProvider;

impl Provider for DroidProvider {
    fn name(&self) -> &'static str {
        "droid"
    }

    fn display_name(&self) -> &'static str {
        "Droid (Factory)"
    }

    fn is_available(&self) -> bool {
        platform::droid_sessions_dir().is_some()
    }

    fn scan(&self) -> Result<ProviderResult, ScanError> {
        let dir = platform::droid_sessions_dir()
            .ok_or_else(|| ScanError::NotFound("Droid sessions dir not found".into()))?;

        let pattern = format!("{}/*.json", dir.display());
        let paths: Vec<_> = glob::glob(&pattern)
            .map(|p| p.filter_map(|x| x.ok()).collect())
            .unwrap_or_default();

        let mut models: HashMap<String, ModelUsage> = HashMap::new();
        let mut session_ids: HashSet<String> = HashSet::new();
        let mut session_timestamps: HashMap<String, Vec<i64>> = HashMap::new();
        let mut assistant_rows: Vec<(i64, u64, String)> = Vec::new();
        let mut first_seen: Option<i64> = None;
        let mut last_seen: Option<i64> = None;

        for path in &paths {
            scan_session(
                path,
                &mut models,
                &mut session_ids,
                &mut session_timestamps,
                &mut assistant_rows,
                &mut first_seen,
                &mut last_seen,
            );
        }

        if session_ids.is_empty() && models.is_empty() {
            return Err(ScanError::NotFound("No Droid session data found".into()));
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

        let mut result = ProviderResult::new("Droid (Factory)");
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

fn scan_session(
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

    let session_id = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();

    let usage = match v.get("tokenUsage") {
        Some(u) => u,
        None => return,
    };

    let input = usage
        .get("inputTokens")
        .and_then(|x| x.as_u64())
        .unwrap_or(0);
    let output = usage
        .get("outputTokens")
        .and_then(|x| x.as_u64())
        .unwrap_or(0);
    let thinking = usage
        .get("thinkingTokens")
        .and_then(|x| x.as_u64())
        .unwrap_or(0);
    let cache_read = usage
        .get("cacheReadTokens")
        .and_then(|x| x.as_u64())
        .unwrap_or(0);
    let cache_write = usage
        .get("cacheCreationTokens")
        .and_then(|x| x.as_u64())
        .unwrap_or(0);

    if input + output + thinking + cache_read + cache_write == 0 {
        return;
    }

    let raw_model = v
        .get("model")
        .and_then(|m| m.as_str())
        .unwrap_or("unknown");
    let model = normalize_model_name(raw_model);

    let ts = v
        .get("providerLockTimestamp")
        .and_then(|t| t.as_str())
        .and_then(time_util::iso8601_to_unix)
        .or_else(|| {
            std::fs::metadata(path)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64)
        });

    session_ids.insert(session_id.clone());
    accumulate_model(
        models,
        &model,
        input,
        output,
        thinking,
        cache_read,
        cache_write,
    );

    if let Some(ts) = ts {
        session_timestamps
            .entry(session_id.clone())
            .or_default()
            .push(ts);
        *first_seen = Some(first_seen.map_or(ts, |fs| fs.min(ts)));
        *last_seen = Some(last_seen.map_or(ts, |ls| ls.max(ts)));
        assistant_rows.push((
            ts,
            input.saturating_add(output).saturating_add(thinking),
            session_id,
        ));
    }
}

/// Droid model names like `custom:Claude-Opus-4.5-Thinking-[Anthropic]-0` →
/// `claude-opus-4-5-thinking-0`.
pub(crate) fn normalize_model_name(raw: &str) -> String {
    let mut s = raw.to_string();
    if let Some(rest) = s.strip_prefix("custom:") {
        s = rest.to_string();
    }
    // Strip bracketed content like [Anthropic]
    let mut out = String::with_capacity(s.len());
    let mut depth = 0;
    for ch in s.chars() {
        match ch {
            '[' | '(' => depth += 1,
            ']' | ')' => {
                if depth > 0 {
                    depth -= 1;
                }
            }
            _ => {
                if depth == 0 {
                    out.push(ch);
                }
            }
        }
    }
    let lower = out.to_lowercase();
    let hyphenated: String = lower
        .chars()
        .map(|c| if c == '.' { '-' } else { c })
        .collect();

    // Collapse consecutive hyphens
    let mut collapsed = String::with_capacity(hyphenated.len());
    let mut prev_hyphen = false;
    for ch in hyphenated.chars() {
        if ch == '-' {
            if !prev_hyphen {
                collapsed.push('-');
            }
            prev_hyphen = true;
        } else {
            collapsed.push(ch);
            prev_hyphen = false;
        }
    }
    collapsed.trim_matches('-').to_string()
}
