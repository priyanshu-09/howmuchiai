use crate::platform;
use crate::providers::Provider;
use crate::time_util;
use crate::types::{ModelUsage, ProviderResult, ScanError, TokenUsage};
use std::collections::{HashMap, HashSet};

pub struct GeminiProvider;

impl Provider for GeminiProvider {
    fn name(&self) -> &'static str {
        "gemini_cli"
    }

    fn display_name(&self) -> &'static str {
        "Gemini CLI"
    }

    fn is_available(&self) -> bool {
        platform::gemini_dir().join("tmp").exists()
    }

    fn scan(&self) -> Result<ProviderResult, ScanError> {
        let tmp_dir = platform::gemini_dir().join("tmp");
        if !tmp_dir.exists() {
            return Err(ScanError::NotFound("Gemini tmp directory not found".into()));
        }

        let pattern = format!("{}/*/logs.json", tmp_dir.to_string_lossy());
        let log_files: Vec<_> = glob::glob(&pattern)
            .map(|paths| paths.filter_map(|p| p.ok()).collect())
            .unwrap_or_default();

        if log_files.is_empty() {
            let mut result = ProviderResult::new("Gemini CLI");
            result.sessions = Some(0);
            result.invocations = Some(0);
            return Ok(result);
        }

        let mut sessions_set: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut total_messages: u64 = 0;
        let mut all_timestamps: Vec<i64> = Vec::new();
        let mut events: Vec<(i64, u64, Option<String>)> = Vec::new();

        for log_file in &log_files {
            let content = match std::fs::read_to_string(log_file) {
                Ok(c) => c,
                Err(_) => continue,
            };

            // Each logs.json can be a JSON array of log entries
            let entries: Vec<serde_json::Value> = match serde_json::from_str(&content) {
                Ok(arr) => arr,
                Err(_) => {
                    // Try parsing as newline-delimited JSON
                    content
                        .lines()
                        .filter_map(|line| serde_json::from_str(line).ok())
                        .collect()
                }
            };

            for entry in &entries {
                let session_id = entry
                    .get("sessionId")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                if let Some(ref sid) = session_id {
                    sessions_set.insert(sid.clone());
                }

                total_messages += 1;

                // Parse timestamp -- could be ISO 8601 string or numeric
                let parsed_ts: Option<i64> = entry.get("timestamp").and_then(|ts| {
                    if let Some(ts_str) = ts.as_str() {
                        time_util::iso8601_to_unix(ts_str)
                    } else if let Some(ts_num) = ts.as_i64() {
                        // Assume milliseconds if value is large enough
                        if ts_num > 1_000_000_000_000 {
                            Some(ts_num / 1000)
                        } else {
                            Some(ts_num)
                        }
                    } else {
                        ts.as_f64().map(|v| v as i64)
                    }
                });
                if let Some(ts) = parsed_ts {
                    all_timestamps.push(ts);
                    // logs.json rows carry no token counts; per-turn output tokens are
                    // appended below from the sibling chat transcripts. These events
                    // still drive per-day session counts via their sessionId.
                    events.push((ts, 0, session_id.clone()));
                }
            }
        }

        // --- Token extraction from chat transcripts -------------------------
        //
        // logs.json carries no token counts. The real per-turn token usage lives
        // in the sibling chat transcripts at two glob depths:
        //   {tmp}/*/chats/*.jsonl               (header + inline turns)
        //   {tmp}/*/chats/<sessionId>/*.jsonl   (checkpointed turns)
        // Each transcript is newline-delimited JSON; token-bearing lines have
        // `type == "gemini"` and a `.tokens` object. See `extract_chat_tokens`
        // for the (verified) aggregation rules that avoid double-counting the
        // cumulative context snapshots Gemini records.
        let chat_patterns = [
            format!("{}/*/chats/*.jsonl", tmp_dir.to_string_lossy()),
            format!("{}/*/chats/*/*.jsonl", tmp_dir.to_string_lossy()),
        ];
        let (tokens, chat_models, token_events) = extract_chat_tokens(&chat_patterns);
        // Fold per-turn output tokens into the daily-bucket event stream so the
        // heatmap/streak widgets reflect real token volume per day.
        events.extend(token_events);

        all_timestamps.sort_unstable();

        let hours = time_util::active_hours_from_timestamps(&all_timestamps, 1800);
        let first_seen = all_timestamps.first().copied();
        let last_seen = all_timestamps.last().copied();

        let mut metadata = HashMap::new();
        metadata.insert(
            "log_files_scanned".to_string(),
            serde_json::Value::from(log_files.len() as u64),
        );

        let mut result = ProviderResult::new("Gemini CLI");
        result.sessions = Some(sessions_set.len() as u64);
        result.invocations = Some(total_messages);
        // Only attach tokens when the chat transcripts actually contained some;
        // a user with logs but no token-bearing chats stays at the prior None.
        if tokens.total > 0 {
            result.tokens = Some(tokens);
        }
        if !chat_models.is_empty() {
            result.models = Some(chat_models);
        }
        result.hours = Some(hours);
        result.first_seen = first_seen;
        result.last_seen = last_seen;
        result.metadata = Some(metadata);

        let daily_buckets = time_util::build_daily_buckets(&events);
        if !daily_buckets.is_empty() {
            result.daily_buckets = Some(daily_buckets);
        }

        Ok(result)
    }
}

/// Per-turn `(unix_ts, output_tokens, session_id)` event, matching the tuple
/// `time_util::build_daily_buckets` consumes.
type TokenEvent = (i64, u64, Option<String>);

/// Aggregated token usage extracted from the Gemini chat transcripts.
///
/// Returns `(tokens, models, events)` where `events` is the per-turn
/// `(unix_ts, output_tokens, session_id)` stream for daily-bucket folding.
///
/// AGGREGATION RULES (verified against the on-disk store — see the provider RCA).
/// Naively summing raw rows fabricates numbers, so:
///   * Rows are deduplicated by `.id`: checkpoint `$set` rewrites re-emit the
///     same turn up to ~6x. Only the first occurrence of an `.id` is counted.
///   * `output_tokens` = SUM over unique turns of `.tokens.output` — the only
///     true per-turn delta.
///   * `input_tokens` = SUM over sessions of the per-session MAX of
///     `.tokens.input`; `cache_read_tokens` likewise from `.tokens.cached`.
///     These two fields are CUMULATIVE running context-window snapshots, so the
///     session max is the peak/final context size — summing raw rows is
///     meaningless (it grows monotonically then plateaus at the model limit).
///   * `cache_creation_tokens` = 0 (Gemini has no equivalent).
///   * `.tokens.total` is ignored; `TokenUsage::compute_total()` recomputes it
///     (the raw field is cumulative and double-counts input+cached+output).
fn extract_chat_tokens(
    patterns: &[String],
) -> (TokenUsage, HashMap<String, ModelUsage>, Vec<TokenEvent>) {
    // Per-session peak of the cumulative input / cached snapshots.
    let mut session_max_input: HashMap<String, u64> = HashMap::new();
    let mut session_max_cached: HashMap<String, u64> = HashMap::new();
    // Global dedup of turn ids across every transcript file.
    let mut seen_ids: HashSet<String> = HashSet::new();
    let mut models: HashMap<String, ModelUsage> = HashMap::new();
    let mut output_sum: u64 = 0;
    let mut token_events: Vec<TokenEvent> = Vec::new();

    for pattern in patterns {
        let files: Vec<_> = glob::glob(pattern)
            .map(|paths| paths.filter_map(|p| p.ok()).collect())
            .unwrap_or_default();

        for file in &files {
            let content = match std::fs::read_to_string(file) {
                Ok(c) => c,
                Err(_) => continue,
            };

            // Session key: header transcripts ({chats}/*.jsonl) carry the
            // sessionId on their first line; checkpoint transcripts live under
            // {chats}/<sessionId>/*.jsonl, so the parent directory name is the
            // sessionId. Either way the key groups all rows of one session so
            // the per-session max(input)/max(cached) is computed correctly.
            let session_key = chat_session_key(file, &content);

            for line in content.lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let value: serde_json::Value = match serde_json::from_str(line) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                // Only token-bearing model turns contribute.
                if value.get("type").and_then(|v| v.as_str()) != Some("gemini") {
                    continue;
                }
                let Some(tokens_obj) = value.get("tokens") else {
                    continue;
                };

                let input = tokens_obj
                    .get("input")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let cached = tokens_obj
                    .get("cached")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let output = tokens_obj
                    .get("output")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);

                // Cumulative snapshots: take the running max across ALL rows
                // (including duplicates) so the peak context size is exact.
                let mi = session_max_input.entry(session_key.clone()).or_insert(0);
                *mi = (*mi).max(input);
                let mc = session_max_cached.entry(session_key.clone()).or_insert(0);
                *mc = (*mc).max(cached);

                // Per-turn deltas (output) and per-day events only count each
                // turn once — skip checkpoint rewrites of an already-seen id.
                let id = match value.get("id").and_then(|v| v.as_str()) {
                    Some(id) => id.to_string(),
                    // A token row without an id can't be deduplicated reliably;
                    // skip it rather than risk inflating the output sum.
                    None => continue,
                };
                if !seen_ids.insert(id) {
                    continue;
                }

                output_sum = output_sum.saturating_add(output);

                if let Some(model) = value.get("model").and_then(|v| v.as_str()) {
                    let entry = models
                        .entry(model.to_string())
                        .or_insert_with(|| ModelUsage {
                            tokens: TokenUsage::default(),
                            sessions: 0,
                            hours: 0.0,
                        });
                    entry.tokens.output_tokens = entry.tokens.output_tokens.saturating_add(output);
                }

                // Bucket this unique turn's output tokens by its timestamp date.
                if let Some(ts) = value
                    .get("timestamp")
                    .and_then(|v| v.as_str())
                    .and_then(time_util::iso8601_to_unix)
                {
                    token_events.push((ts, output, Some(session_key.clone())));
                }
            }
        }
    }

    let input_total: u64 = session_max_input
        .values()
        .fold(0u64, |acc, v| acc.saturating_add(*v));
    let cached_total: u64 = session_max_cached
        .values()
        .fold(0u64, |acc, v| acc.saturating_add(*v));

    let mut tokens = TokenUsage {
        input_tokens: input_total,
        output_tokens: output_sum,
        cache_read_tokens: cached_total,
        cache_creation_tokens: 0,
        total: 0,
    };
    tokens.compute_total();

    for usage in models.values_mut() {
        usage.tokens.compute_total();
    }

    (tokens, models, token_events)
}

/// Derive the sessionId for a chat transcript. Header transcripts live directly
/// under `chats/` and store the sessionId on their first JSON line; checkpoint
/// transcripts live under `chats/<sessionId>/` so the parent dir name is the
/// sessionId. Falls back to the file's own path string when neither is present
/// (still groups all rows of that file together, which is the safe default).
fn chat_session_key(path: &std::path::Path, content: &str) -> String {
    let parent_name = path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str());

    if parent_name == Some("chats") {
        if let Some(first_line) = content.lines().find(|l| !l.trim().is_empty()) {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(first_line) {
                if let Some(sid) = value.get("sessionId").and_then(|v| v.as_str()) {
                    return sid.to_string();
                }
            }
        }
    } else if let Some(name) = parent_name {
        // Inside chats/<sessionId>/ — the directory name is the sessionId.
        return name.to_string();
    }

    path.to_string_lossy().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Build a temp `tmp/<project>/chats` layout with a header transcript and a
    /// checkpoint sub-transcript, then run the real `extract_chat_tokens`.
    ///
    /// The fixture deliberately encodes the two failure modes the RCA warns
    /// about so a regression to naive summing is caught:
    ///   * Duplicated `.id` rows ($set checkpoint rewrites) — must be counted
    ///     once for output, but still raise the cumulative input/cached max.
    ///   * Monotonically growing `.input` / `.cached` snapshots — must collapse
    ///     to the per-session MAX, never a sum of every row.
    #[test]
    fn extract_chat_tokens_dedups_and_uses_max_input_sum_output() {
        let dir = tempfile::tempdir().expect("tempdir");
        let tmp = dir.path().join("tmp");
        let project = tmp.join("proj");
        let chats = project.join("chats");
        let sub = chats.join("sess-1");
        fs::create_dir_all(&sub).expect("mkdir");

        // Header transcript: first line is the session header, then two unique
        // turns whose `input`/`cached` grow monotonically (cumulative snapshot).
        let header = chats.join("session-header.jsonl");
        let header_lines = [
            r#"{"kind":"chat","sessionId":"sess-1","startTime":"2026-05-24T16:45:00.000Z"}"#,
            r#"{"id":"a","type":"gemini","model":"gemini-3-flash-preview","timestamp":"2026-05-24T16:46:00.000Z","tokens":{"input":100,"output":10,"cached":5,"thoughts":1,"tool":0,"total":116}}"#,
            // Duplicate of id "a" (a $set rewrite): output must NOT be re-added,
            // but input/cached snapshots advance, so the session max climbs.
            r#"{"id":"a","type":"gemini","model":"gemini-3-flash-preview","timestamp":"2026-05-24T16:46:00.000Z","tokens":{"input":150,"output":10,"cached":8,"thoughts":1,"tool":0,"total":169}}"#,
            r#"{"id":"b","type":"gemini","model":"gemini-3-flash-preview","timestamp":"2026-05-24T16:47:00.000Z","tokens":{"input":200,"output":20,"cached":12,"thoughts":2,"tool":0,"total":234}}"#,
            // A non-gemini line and a token-less line — both ignored.
            r#"{"id":"u","type":"user","timestamp":"2026-05-24T16:47:30.000Z"}"#,
            r#"{"id":"x","type":"gemini","model":"gemini-3-flash-preview","timestamp":"2026-05-24T16:48:00.000Z"}"#,
        ]
        .join("\n");
        fs::write(&header, header_lines).expect("write header");

        // Checkpoint sub-transcript for the SAME session: the parent dir name
        // ("sess-1") is the session key, so its rows must share the same max.
        let checkpoint = sub.join("ckpt.jsonl");
        let checkpoint_lines = [
            r#"{"id":"c","type":"gemini","model":"gemini-3.1-pro-preview","timestamp":"2026-05-24T16:49:00.000Z","tokens":{"input":300,"output":30,"cached":20,"thoughts":3,"tool":0,"total":353}}"#,
            // Duplicate of "c" again — output stays summed once, input max rises.
            r#"{"id":"c","type":"gemini","model":"gemini-3.1-pro-preview","timestamp":"2026-05-24T16:49:00.000Z","tokens":{"input":320,"output":30,"cached":25,"thoughts":3,"tool":0,"total":378}}"#,
        ]
        .join("\n");
        fs::write(&checkpoint, checkpoint_lines).expect("write checkpoint");

        let patterns = [
            format!("{}/*/chats/*.jsonl", tmp.to_string_lossy()),
            format!("{}/*/chats/*/*.jsonl", tmp.to_string_lossy()),
        ];
        let (tokens, models, events) = extract_chat_tokens(&patterns);

        // Unique turns: a, b, c => output = 10 + 20 + 30 = 60. The duplicate
        // rows of a and c must NOT add another 10 / 30.
        assert_eq!(tokens.output_tokens, 60, "output is SUM over unique ids");

        // All rows belong to the single session "sess-1" (header sessionId ==
        // checkpoint parent dir name), so input/cached are ONE per-session max:
        // input max = 320, cached max = 25. Never the sum of every row.
        assert_eq!(
            tokens.input_tokens, 320,
            "input is per-session MAX snapshot"
        );
        assert_eq!(tokens.cache_read_tokens, 25, "cached is per-session MAX");
        assert_eq!(
            tokens.cache_creation_tokens, 0,
            "Gemini has no cache-create"
        );

        // compute_total = input + output + cached + 0 = 320 + 60 + 25 = 405.
        // Crucially NOT the cumulative raw `.total` sum (116+169+234+353+378).
        assert_eq!(tokens.total, 405, "total recomputed, not raw cumulative");

        // Model split is keyed by `.model` and only sums per-turn output.
        let flash = &models["gemini-3-flash-preview"].tokens;
        assert_eq!(flash.output_tokens, 30, "flash turns a+b => 10+20");
        let pro = &models["gemini-3.1-pro-preview"].tokens;
        assert_eq!(pro.output_tokens, 30, "pro turn c => 30");

        // One daily event per UNIQUE turn (3), each carrying its output tokens.
        assert_eq!(events.len(), 3, "one event per unique turn");
        let event_output: u64 = events.iter().map(|(_, t, _)| *t).sum();
        assert_eq!(event_output, 60, "event tokens match deduped output sum");
        assert!(
            events
                .iter()
                .all(|(_, _, sid)| sid.as_deref() == Some("sess-1")),
            "every event is attributed to the resolved sessionId"
        );
    }

    /// No transcripts at all (or only token-less lines) must yield zero tokens
    /// and no models — never a panic, never a fabricated default.
    #[test]
    fn extract_chat_tokens_empty_is_zero_not_panic() {
        let dir = tempfile::tempdir().expect("tempdir");
        let tmp = dir.path().join("tmp");
        let chats = tmp.join("proj").join("chats");
        fs::create_dir_all(&chats).expect("mkdir");
        // A transcript with only a header + a non-token gemini line.
        fs::write(
            chats.join("empty.jsonl"),
            "{\"kind\":\"chat\",\"sessionId\":\"s\"}\n{\"id\":\"z\",\"type\":\"gemini\",\"model\":\"m\"}\n",
        )
        .expect("write");

        let patterns = [
            format!("{}/*/chats/*.jsonl", tmp.to_string_lossy()),
            format!("{}/*/chats/*/*.jsonl", tmp.to_string_lossy()),
        ];
        let (tokens, models, events) = extract_chat_tokens(&patterns);
        assert_eq!(tokens.total, 0);
        assert_eq!(tokens.output_tokens, 0);
        assert!(models.is_empty());
        assert!(events.is_empty());
    }

    /// Malformed JSON lines and missing `tokens`/`id` fields must be skipped
    /// gracefully without crashing or inflating counts.
    #[test]
    fn extract_chat_tokens_skips_malformed_rows() {
        let dir = tempfile::tempdir().expect("tempdir");
        let tmp = dir.path().join("tmp");
        let chats = tmp.join("proj").join("chats");
        fs::create_dir_all(&chats).expect("mkdir");
        let lines = [
            "{ this is not json",
            r#"{"id":"ok","type":"gemini","model":"m","timestamp":"2026-05-24T16:46:00.000Z","tokens":{"input":50,"output":7,"cached":0}}"#,
            // token-bearing but no id => skipped from output sum (can't dedup).
            r#"{"type":"gemini","model":"m","tokens":{"input":999,"output":999,"cached":999}}"#,
        ]
        .join("\n");
        fs::write(chats.join("mixed.jsonl"), lines).expect("write");

        let patterns = [
            format!("{}/*/chats/*.jsonl", tmp.to_string_lossy()),
            format!("{}/*/chats/*/*.jsonl", tmp.to_string_lossy()),
        ];
        let (tokens, _models, _events) = extract_chat_tokens(&patterns);
        // Only the well-formed "ok" row contributes to the output sum...
        assert_eq!(tokens.output_tokens, 7, "only valid id-bearing row counts");
        // ...but the id-less row still raised the cumulative input max (it is a
        // real snapshot), per the max-over-all-rows rule.
        assert_eq!(tokens.input_tokens, 999, "max input includes id-less row");
    }
}
