use crate::platform;
use crate::providers::Provider;
use crate::sqlite_util::SafeSqlite;
use crate::time_util;
use crate::types::{DailyBucket, ModelUsage, ProviderResult, ScanError, TokenUsage};
use std::collections::{HashMap, HashSet};
use std::path::Path;

pub struct OpenCodeProvider;

impl Provider for OpenCodeProvider {
    fn name(&self) -> &'static str {
        "opencode"
    }

    fn display_name(&self) -> &'static str {
        "OpenCode"
    }

    fn is_available(&self) -> bool {
        !platform::opencode_data_dirs().is_empty()
            || !platform::opencode_sqlite_paths().is_empty()
    }

    fn scan(&self) -> Result<ProviderResult, ScanError> {
        let data_dirs = platform::opencode_data_dirs();
        let db_paths = platform::opencode_sqlite_paths();

        if data_dirs.is_empty() && db_paths.is_empty() {
            return Err(ScanError::NotFound("OpenCode data dir not found".into()));
        }

        let mut models: HashMap<String, ModelUsage> = HashMap::new();
        let mut session_ids: HashSet<String> = HashSet::new();
        let mut assistant_ts: Vec<(i64, u64, String)> = Vec::new(); // (ts_secs, tokens_for_day, session_id)
        let mut session_timestamps: HashMap<String, Vec<i64>> = HashMap::new();
        let mut first_seen: Option<i64> = None;
        let mut last_seen: Option<i64> = None;
        let mut dedup_keys: HashSet<String> = HashSet::new();

        // 1) SQLite primary (opencode v1.2+)
        for db_path in &db_paths {
            if let Err(e) = scan_sqlite(
                db_path,
                &mut models,
                &mut session_ids,
                &mut assistant_ts,
                &mut session_timestamps,
                &mut first_seen,
                &mut last_seen,
                &mut dedup_keys,
            ) {
                // Non-fatal: fall through to JSON
                eprintln!("opencode: sqlite scan failed at {:?}: {}", db_path, e);
            }
        }

        // 2) Legacy JSON layout (sst/opencode pre-1.2)
        for root in &data_dirs {
            // Session info files: storage/session/info/*.json
            let info_pattern = format!("{}/storage/session/info/*.json", root.display());
            if let Ok(paths) = glob::glob(&info_pattern) {
                for path in paths.filter_map(|p| p.ok()) {
                    if let Some((id, created, updated)) = parse_session_info(&path) {
                        session_ids.insert(id.clone());
                        for ts in [created, updated].iter().copied().flatten() {
                            session_timestamps.entry(id.clone()).or_default().push(ts);
                            first_seen = Some(first_seen.map_or(ts, |fs: i64| fs.min(ts)));
                            last_seen = Some(last_seen.map_or(ts, |ls: i64| ls.max(ts)));
                        }
                    }
                }
            }

            // Message files: storage/message/**/*.json (flat tokscale-confirmed path)
            let msg_pattern = format!("{}/storage/message/**/*.json", root.display());
            if let Ok(paths) = glob::glob(&msg_pattern) {
                for path in paths.filter_map(|p| p.ok()) {
                    scan_message_file(
                        &path,
                        &mut models,
                        &mut session_ids,
                        &mut assistant_ts,
                        &mut session_timestamps,
                        &mut first_seen,
                        &mut last_seen,
                        &mut dedup_keys,
                    );
                }
            }
        }

        if session_ids.is_empty() && models.is_empty() && assistant_ts.is_empty() {
            return Err(ScanError::NotFound(
                "No OpenCode session data found".into(),
            ));
        }

        // Hours from per-session timestamps (30-min gap threshold)
        let mut total_hours = 0.0_f64;
        for timestamps in session_timestamps.values() {
            let mut sorted = timestamps.clone();
            sorted.sort_unstable();
            total_hours += time_util::active_hours_from_timestamps(&sorted, 1800);
        }

        // Aggregate tokens
        let mut total_tokens = TokenUsage::default();
        for mu in models.values() {
            total_tokens.merge(&mu.tokens);
        }

        // Daily buckets
        let daily_buckets = build_daily_buckets(&assistant_ts);

        let mut result = ProviderResult::new("OpenCode");
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
fn scan_sqlite(
    db_path: &Path,
    models: &mut HashMap<String, ModelUsage>,
    session_ids: &mut HashSet<String>,
    assistant_ts: &mut Vec<(i64, u64, String)>,
    session_timestamps: &mut HashMap<String, Vec<i64>>,
    first_seen: &mut Option<i64>,
    last_seen: &mut Option<i64>,
    dedup_keys: &mut HashSet<String>,
) -> Result<(), ScanError> {
    let db = SafeSqlite::open(db_path)?;
    let mut stmt = db.conn().prepare(
        "SELECT id, session_id, data FROM message \
         WHERE json_extract(data, '$.role') = 'assistant' \
           AND json_extract(data, '$.tokens') IS NOT NULL",
    )?;

    let rows = stmt.query_map([], |row| {
        let id: String = row.get(0)?;
        let session_id: String = row.get::<_, Option<String>>(1)?.unwrap_or_default();
        let data: String = row.get(2)?;
        Ok((id, session_id, data))
    })?;

    for row in rows {
        let (id, sess, data) = match row {
            Ok(r) => r,
            Err(_) => continue,
        };
        let v: serde_json::Value = match serde_json::from_str(&data) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let dedup = v
            .get("id")
            .and_then(|x| x.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| id.clone());
        if !dedup_keys.insert(dedup) {
            continue;
        }
        let effective_sess = v
            .get("sessionID")
            .and_then(|x| x.as_str())
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or(sess);

        process_assistant_msg(
            &v,
            &effective_sess,
            models,
            session_ids,
            assistant_ts,
            session_timestamps,
            first_seen,
            last_seen,
        );
    }

    Ok(())
}

fn parse_session_info(path: &Path) -> Option<(String, Option<i64>, Option<i64>)> {
    let content = std::fs::read_to_string(path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&content).ok()?;

    let id = v
        .get("id")
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
        .or_else(|| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s.to_string())
        })?;

    let created = v
        .get("time")
        .and_then(|t| t.get("created"))
        .and_then(|x| x.as_i64())
        .map(ms_to_secs);
    let updated = v
        .get("time")
        .and_then(|t| t.get("updated"))
        .and_then(|x| x.as_i64())
        .map(ms_to_secs);

    Some((id, created, updated))
}

#[allow(clippy::too_many_arguments)]
fn scan_message_file(
    path: &Path,
    models: &mut HashMap<String, ModelUsage>,
    session_ids: &mut HashSet<String>,
    assistant_ts: &mut Vec<(i64, u64, String)>,
    session_timestamps: &mut HashMap<String, Vec<i64>>,
    first_seen: &mut Option<i64>,
    last_seen: &mut Option<i64>,
    dedup_keys: &mut HashSet<String>,
) {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return,
    };
    let v: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return,
    };

    // Dedup by embedded id, fallback to filename stem
    let dedup = v
        .get("id")
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
        .or_else(|| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_default();
    if !dedup.is_empty() && !dedup_keys.insert(dedup) {
        return;
    }

    // Session id: prefer embedded, fall back to parent dir name
    let session_id = v
        .get("sessionID")
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
        .or_else(|| {
            path.parent()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| "unknown".into());

    let role = v.get("role").and_then(|r| r.as_str()).unwrap_or("");
    if role != "assistant" {
        // Non-assistant: still track timestamp for session hours
        if let Some(ts) = v
            .get("time")
            .and_then(|t| t.get("created"))
            .and_then(|x| x.as_i64())
        {
            let secs = ms_to_secs(ts);
            session_ids.insert(session_id.clone());
            session_timestamps
                .entry(session_id)
                .or_default()
                .push(secs);
            *first_seen = Some(first_seen.map_or(secs, |fs| fs.min(secs)));
            *last_seen = Some(last_seen.map_or(secs, |ls| ls.max(secs)));
        }
        return;
    }

    process_assistant_msg(
        &v,
        &session_id,
        models,
        session_ids,
        assistant_ts,
        session_timestamps,
        first_seen,
        last_seen,
    );
}

#[allow(clippy::too_many_arguments)]
fn process_assistant_msg(
    v: &serde_json::Value,
    session_id: &str,
    models: &mut HashMap<String, ModelUsage>,
    session_ids: &mut HashSet<String>,
    assistant_ts: &mut Vec<(i64, u64, String)>,
    session_timestamps: &mut HashMap<String, Vec<i64>>,
    first_seen: &mut Option<i64>,
    last_seen: &mut Option<i64>,
) {
    session_ids.insert(session_id.to_string());

    // Timestamps (ms): time.created and time.completed
    let mut msg_ts: Option<i64> = None;
    for key in ["created", "completed", "updated"] {
        if let Some(ts) = v
            .get("time")
            .and_then(|t| t.get(key))
            .and_then(|x| x.as_i64())
        {
            let secs = ms_to_secs(ts);
            session_timestamps
                .entry(session_id.to_string())
                .or_default()
                .push(secs);
            *first_seen = Some(first_seen.map_or(secs, |fs| fs.min(secs)));
            *last_seen = Some(last_seen.map_or(secs, |ls| ls.max(secs)));
            if msg_ts.is_none() || key == "created" {
                msg_ts = Some(secs);
            }
        }
    }

    let model_name = v
        .get("modelID")
        .or_else(|| v.get("model"))
        .and_then(|m| m.as_str())
        .unwrap_or("unknown")
        .to_string();

    let t = match v
        .get("tokens")
        .or_else(|| v.get("metadata").and_then(|m| m.get("assistant")).and_then(|a| a.get("tokens")))
    {
        Some(t) => t,
        None => return,
    };

    let input = t.get("input").and_then(|x| x.as_u64()).unwrap_or(0);
    let output = t.get("output").and_then(|x| x.as_u64()).unwrap_or(0);
    let reasoning = t.get("reasoning").and_then(|x| x.as_u64()).unwrap_or(0);
    let cache_read = t
        .get("cache")
        .and_then(|c| c.get("read"))
        .and_then(|x| x.as_u64())
        .unwrap_or(0);
    let cache_write = t
        .get("cache")
        .and_then(|c| c.get("write"))
        .and_then(|x| x.as_u64())
        .unwrap_or(0);

    let entry = models.entry(model_name).or_insert_with(|| ModelUsage {
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

    if let Some(ts) = msg_ts {
        assistant_ts.push((ts, input.saturating_add(output).saturating_add(reasoning), session_id.to_string()));
    }
}

pub(crate) fn build_daily_buckets(
    rows: &[(i64, u64, String)],
) -> HashMap<String, DailyBucket> {
    let mut buckets: HashMap<String, DailyBucket> = HashMap::new();
    let mut day_sessions: HashMap<String, HashSet<String>> = HashMap::new();
    for (ts, tokens, session_id) in rows {
        let Some(dt) = chrono::DateTime::from_timestamp(*ts, 0) else {
            continue;
        };
        let day = dt.format("%Y-%m-%d").to_string();
        let b = buckets.entry(day.clone()).or_default();
        b.tokens = b.tokens.saturating_add(*tokens);
        if day_sessions
            .entry(day)
            .or_default()
            .insert(session_id.clone())
        {
            b.sessions = b.sessions.saturating_add(1);
        }
    }
    buckets
}

pub(crate) fn ms_to_secs(ms: i64) -> i64 {
    // OpenCode stores ms; older entries may already be seconds (< ~10^12).
    if ms > 10_000_000_000 { ms / 1000 } else { ms }
}
