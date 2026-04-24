use crate::platform;
use crate::providers::amp::accumulate_model;
use crate::providers::opencode::build_daily_buckets;
use crate::providers::Provider;
use crate::time_util;
use crate::types::{ModelUsage, ProviderResult, ScanError, TokenUsage};
use std::collections::{HashMap, HashSet};
use std::io::BufRead;
use std::path::{Path, PathBuf};

pub struct KimiProvider;

impl Provider for KimiProvider {
    fn name(&self) -> &'static str {
        "kimi_cli"
    }

    fn display_name(&self) -> &'static str {
        "Kimi CLI"
    }

    fn is_available(&self) -> bool {
        platform::kimi_sessions_dir().is_some()
    }

    fn scan(&self) -> Result<ProviderResult, ScanError> {
        let root = platform::kimi_sessions_dir()
            .ok_or_else(|| ScanError::NotFound("Kimi sessions dir not found".into()))?;

        // Read model override from ~/.kimi/config.json if present
        let config_model = platform::kimi_config_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .and_then(|v| {
                v.get("model")
                    .and_then(|m| m.as_str())
                    .map(|s| s.to_string())
            })
            .unwrap_or_else(|| "kimi-for-coding".to_string());

        let pattern = format!("{}/**/wire.jsonl", root.display());
        let paths: Vec<PathBuf> = glob::glob(&pattern)
            .map(|p| p.filter_map(|x| x.ok()).collect())
            .unwrap_or_default();

        if paths.is_empty() {
            return Err(ScanError::NotFound("No Kimi wire.jsonl files found".into()));
        }

        let mut models: HashMap<String, ModelUsage> = HashMap::new();
        let mut session_ids: HashSet<String> = HashSet::new();
        let mut session_timestamps: HashMap<String, Vec<i64>> = HashMap::new();
        let mut assistant_rows: Vec<(i64, u64, String)> = Vec::new();
        let mut first_seen: Option<i64> = None;
        let mut last_seen: Option<i64> = None;
        let mut seen_msg_ids: HashSet<String> = HashSet::new();

        for path in &paths {
            scan_wire_jsonl(
                path,
                &config_model,
                &mut models,
                &mut session_ids,
                &mut session_timestamps,
                &mut assistant_rows,
                &mut first_seen,
                &mut last_seen,
                &mut seen_msg_ids,
            );
        }

        if session_ids.is_empty() && models.is_empty() {
            return Err(ScanError::NotFound("No Kimi token data found".into()));
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

        let mut result = ProviderResult::new("Kimi CLI");
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
fn scan_wire_jsonl(
    path: &Path,
    config_model: &str,
    models: &mut HashMap<String, ModelUsage>,
    session_ids: &mut HashSet<String>,
    session_timestamps: &mut HashMap<String, Vec<i64>>,
    assistant_rows: &mut Vec<(i64, u64, String)>,
    first_seen: &mut Option<i64>,
    last_seen: &mut Option<i64>,
    seen_msg_ids: &mut HashSet<String>,
) {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return,
    };
    let reader = std::io::BufReader::new(file);

    // Session id = the UUID segment (parent dir of wire.jsonl)
    let session_id = path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();

    let mtime_fallback = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64);

    for line in reader.lines().map_while(Result::ok) {
        if line.trim().is_empty() {
            continue;
        }
        let v: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        // Only process StatusUpdate messages with TokenUsage
        if v.get("type").and_then(|t| t.as_str()) != Some("StatusUpdate") {
            continue;
        }
        let tu = match v
            .get("payload")
            .and_then(|p| p.get("TokenUsage"))
            .or_else(|| v.get("payload").and_then(|p| p.get("token_usage")))
        {
            Some(t) => t,
            None => continue,
        };

        // Dedup by message_id
        if let Some(mid) = v.get("message_id").and_then(|x| x.as_str()) {
            if !seen_msg_ids.insert(format!("{}:{}", session_id, mid)) {
                continue;
            }
        }

        let input = tu.get("input_other").and_then(|x| x.as_u64()).unwrap_or(0);
        let output = tu.get("output").and_then(|x| x.as_u64()).unwrap_or(0);
        let cache_read = tu
            .get("input_cache_read")
            .and_then(|x| x.as_u64())
            .unwrap_or(0);
        let cache_write = tu
            .get("input_cache_creation")
            .and_then(|x| x.as_u64())
            .unwrap_or(0);

        if input + output + cache_read + cache_write == 0 {
            continue;
        }

        // Timestamp is float seconds
        let ts = v
            .get("timestamp")
            .and_then(|x| x.as_f64())
            .map(|s| s as i64)
            .or(mtime_fallback);

        session_ids.insert(session_id.clone());
        accumulate_model(
            models,
            config_model,
            input,
            output,
            0,
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
            assistant_rows.push((ts, input.saturating_add(output), session_id.clone()));
        }
    }
}
