use crate::platform;
use crate::providers::amp::accumulate_model;
use crate::providers::opencode::build_daily_buckets;
use crate::providers::Provider;
use crate::time_util;
use crate::types::{ModelUsage, ProviderResult, ScanError, TokenUsage};
use std::collections::{HashMap, HashSet};
use std::io::BufRead;
use std::path::{Path, PathBuf};

pub struct QwenProvider;

impl Provider for QwenProvider {
    fn name(&self) -> &'static str {
        "qwen_cli"
    }

    fn display_name(&self) -> &'static str {
        "Qwen CLI"
    }

    fn is_available(&self) -> bool {
        platform::qwen_projects_dir().is_some()
    }

    fn scan(&self) -> Result<ProviderResult, ScanError> {
        let root = platform::qwen_projects_dir()
            .ok_or_else(|| ScanError::NotFound("Qwen projects dir not found".into()))?;

        let pattern = format!("{}/**/chats/*.jsonl", root.display());
        let paths: Vec<PathBuf> = glob::glob(&pattern)
            .map(|p| p.filter_map(|x| x.ok()).collect())
            .unwrap_or_default();

        if paths.is_empty() {
            return Err(ScanError::NotFound("No Qwen chat files found".into()));
        }

        let mut models: HashMap<String, ModelUsage> = HashMap::new();
        let mut session_ids: HashSet<String> = HashSet::new();
        let mut session_timestamps: HashMap<String, Vec<i64>> = HashMap::new();
        let mut assistant_rows: Vec<(i64, u64, String)> = Vec::new();
        let mut first_seen: Option<i64> = None;
        let mut last_seen: Option<i64> = None;

        for path in &paths {
            scan_jsonl(
                path,
                &root,
                &mut models,
                &mut session_ids,
                &mut session_timestamps,
                &mut assistant_rows,
                &mut first_seen,
                &mut last_seen,
            );
        }

        if session_ids.is_empty() && models.is_empty() {
            return Err(ScanError::NotFound(
                "No Qwen assistant messages found".into(),
            ));
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

        let mut result = ProviderResult::new("Qwen CLI");
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
fn scan_jsonl(
    path: &Path,
    root: &Path,
    models: &mut HashMap<String, ModelUsage>,
    session_ids: &mut HashSet<String>,
    session_timestamps: &mut HashMap<String, Vec<i64>>,
    assistant_rows: &mut Vec<(i64, u64, String)>,
    first_seen: &mut Option<i64>,
    last_seen: &mut Option<i64>,
) {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return,
    };
    let reader = std::io::BufReader::new(file);

    // Derive fallback session id: {project}-{chat-filename-stem}
    let fallback_session = {
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown");
        let project = path
            .strip_prefix(root)
            .ok()
            .and_then(|p| p.components().next())
            .and_then(|c| c.as_os_str().to_str())
            .unwrap_or("unknown");
        format!("{}-{}", project, stem)
    };
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
        if v.get("type").and_then(|t| t.as_str()) != Some("assistant") {
            continue;
        }
        let usage = match v.get("usageMetadata") {
            Some(u) => u,
            None => continue,
        };

        let prompt = usage
            .get("promptTokenCount")
            .and_then(|x| x.as_u64())
            .unwrap_or(0);
        let candidates = usage
            .get("candidatesTokenCount")
            .and_then(|x| x.as_u64())
            .unwrap_or(0);
        let thoughts = usage
            .get("thoughtsTokenCount")
            .and_then(|x| x.as_u64())
            .unwrap_or(0);
        let cached = usage
            .get("cachedContentTokenCount")
            .and_then(|x| x.as_u64())
            .unwrap_or(0);

        if prompt + candidates + thoughts + cached == 0 {
            continue;
        }

        let session_id = v
            .get("sessionId")
            .and_then(|x| x.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .unwrap_or_else(|| fallback_session.clone());

        let model = v
            .get("model")
            .and_then(|m| m.as_str())
            .unwrap_or("unknown")
            .to_string();

        let ts = v
            .get("timestamp")
            .and_then(|t| t.as_str())
            .and_then(time_util::iso8601_to_unix)
            .or(mtime_fallback);

        session_ids.insert(session_id.clone());
        accumulate_model(models, &model, prompt, candidates, thoughts, cached, 0);

        if let Some(ts) = ts {
            session_timestamps
                .entry(session_id.clone())
                .or_default()
                .push(ts);
            *first_seen = Some(first_seen.map_or(ts, |fs| fs.min(ts)));
            *last_seen = Some(last_seen.map_or(ts, |ls| ls.max(ts)));
            assistant_rows.push((
                ts,
                prompt.saturating_add(candidates).saturating_add(thoughts),
                session_id,
            ));
        }
    }
}
