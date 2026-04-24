use crate::platform;
use crate::providers::Provider;
use crate::providers::amp::accumulate_model;
use crate::providers::opencode::build_daily_buckets;
use crate::time_util;
use crate::types::{ModelUsage, ProviderResult, ScanError, TokenUsage};
use std::collections::{HashMap, HashSet};
use std::io::BufRead;
use std::path::{Path, PathBuf};

pub struct OpenClawProvider;

impl Provider for OpenClawProvider {
    fn name(&self) -> &'static str {
        "openclaw"
    }

    fn display_name(&self) -> &'static str {
        "OpenClaw"
    }

    fn is_available(&self) -> bool {
        !platform::openclaw_dirs().is_empty()
    }

    fn scan(&self) -> Result<ProviderResult, ScanError> {
        let roots = platform::openclaw_dirs();
        if roots.is_empty() {
            return Err(ScanError::NotFound("OpenClaw dir not found".into()));
        }

        let mut models: HashMap<String, ModelUsage> = HashMap::new();
        let mut session_ids: HashSet<String> = HashSet::new();
        let mut session_timestamps: HashMap<String, Vec<i64>> = HashMap::new();
        let mut assistant_rows: Vec<(i64, u64, String)> = Vec::new();
        let mut first_seen: Option<i64> = None;
        let mut last_seen: Option<i64> = None;

        let mut all_jsonl: Vec<PathBuf> = Vec::new();
        for root in &roots {
            // Direct .jsonl discovery
            let pattern = format!("{}/**/*.jsonl", root.display());
            if let Ok(paths) = glob::glob(&pattern) {
                all_jsonl.extend(paths.filter_map(|p| p.ok()));
            }

            // Legacy sessions.json index
            for idx_rel in ["agents/sessions.json", "sessions.json"] {
                let idx = root.join(idx_rel);
                if idx.exists() {
                    if let Some(mut refs) = read_sessions_index(&idx, root) {
                        all_jsonl.append(&mut refs);
                    }
                }
            }
        }

        // Dedup
        let mut seen: HashSet<PathBuf> = HashSet::new();
        all_jsonl.retain(|p| seen.insert(p.clone()));

        for path in &all_jsonl {
            scan_transcript(
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
            return Err(ScanError::NotFound("No OpenClaw transcript data found".into()));
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

        let mut result = ProviderResult::new("OpenClaw");
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

fn read_sessions_index(idx_path: &Path, root: &Path) -> Option<Vec<PathBuf>> {
    let content = std::fs::read_to_string(idx_path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&content).ok()?;

    let entries = v.as_array().cloned().or_else(|| {
        v.get("sessions")
            .and_then(|s| s.as_array())
            .cloned()
    })?;

    let mut out = Vec::new();
    for entry in entries {
        let (session_id, explicit_path) = match &entry {
            serde_json::Value::String(s) => (Some(s.clone()), None),
            serde_json::Value::Object(_) => (
                entry
                    .get("sessionId")
                    .or_else(|| entry.get("id"))
                    .and_then(|x| x.as_str())
                    .map(|s| s.to_string()),
                entry
                    .get("path")
                    .or_else(|| entry.get("file"))
                    .and_then(|x| x.as_str())
                    .map(|s| s.to_string()),
            ),
            _ => (None, None),
        };

        if let Some(p) = explicit_path {
            let path = if Path::new(&p).is_absolute() {
                PathBuf::from(p)
            } else {
                root.join(p)
            };
            if path.exists() {
                out.push(path);
            }
        } else if let Some(sid) = session_id {
            let candidate = root.join("agents").join(format!("{}.jsonl", sid));
            if candidate.exists() {
                out.push(candidate);
            }
        }
    }
    Some(out)
}

fn scan_transcript(
    path: &Path,
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

    let session_id = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();

    let mtime_fallback = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64);

    let mut current_model: Option<String> = None;
    let mut current_provider: Option<String> = None;

    for line in reader.lines().map_while(Result::ok) {
        if line.trim().is_empty() {
            continue;
        }
        let v: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let entry_type = v.get("type").and_then(|t| t.as_str()).unwrap_or("");

        match entry_type {
            "model_change" => {
                if let Some(m) = v.get("model").and_then(|x| x.as_str()) {
                    current_model = Some(m.to_string());
                }
                if let Some(p) = v.get("provider").and_then(|x| x.as_str()) {
                    current_provider = Some(p.to_string());
                }
            }
            "custom" => {
                // model-snapshot — check if it contains model info
                if let Some(snap) = v.get("data").or_else(|| v.get("snapshot")) {
                    if let Some(m) = snap.get("model").and_then(|x| x.as_str()) {
                        current_model = Some(m.to_string());
                    }
                    if let Some(p) = snap.get("provider").and_then(|x| x.as_str()) {
                        current_provider = Some(p.to_string());
                    }
                }
            }
            "message" => {
                let msg = match v.get("message") {
                    Some(m) => m,
                    None => continue,
                };
                if msg.get("role").and_then(|r| r.as_str()) != Some("assistant") {
                    continue;
                }
                let usage = match msg.get("usage") {
                    Some(u) => u,
                    None => continue,
                };
                let input = usage.get("input").and_then(|x| x.as_u64()).unwrap_or(0);
                let output = usage.get("output").and_then(|x| x.as_u64()).unwrap_or(0);
                let cache_read = usage
                    .get("cache_read")
                    .and_then(|x| x.as_u64())
                    .unwrap_or(0);
                let cache_write = usage
                    .get("cache_write")
                    .and_then(|x| x.as_u64())
                    .unwrap_or(0);

                if input + output + cache_read + cache_write == 0 {
                    continue;
                }

                let model = msg
                    .get("model")
                    .and_then(|x| x.as_str())
                    .map(|s| s.to_string())
                    .or_else(|| current_model.clone())
                    .unwrap_or_else(|| "unknown".into());
                let _provider = msg
                    .get("provider")
                    .and_then(|x| x.as_str())
                    .map(|s| s.to_string())
                    .or_else(|| current_provider.clone())
                    .unwrap_or_else(|| "unknown".into());

                let ts = msg
                    .get("timestamp")
                    .and_then(|x| x.as_str())
                    .and_then(time_util::iso8601_to_unix)
                    .or(mtime_fallback);

                session_ids.insert(session_id.clone());
                accumulate_model(models, &model, input, output, 0, cache_read, cache_write);

                if let Some(ts) = ts {
                    session_timestamps
                        .entry(session_id.clone())
                        .or_default()
                        .push(ts);
                    *first_seen = Some(first_seen.map_or(ts, |fs| fs.min(ts)));
                    *last_seen = Some(last_seen.map_or(ts, |ls| ls.max(ts)));
                    assistant_rows.push((
                        ts,
                        input.saturating_add(output),
                        session_id.clone(),
                    ));
                }
            }
            _ => {}
        }
    }
}
