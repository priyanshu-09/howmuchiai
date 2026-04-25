use crate::platform;
use crate::providers::Provider;
use crate::time_util;
use crate::types::{ModelUsage, ProviderResult, ScanError, TokenUsage};
use std::collections::{HashMap, HashSet};
use std::io::BufRead;

pub struct ClaudeCodeProvider;

impl Provider for ClaudeCodeProvider {
    fn name(&self) -> &'static str {
        "claude_code"
    }

    fn display_name(&self) -> &'static str {
        "Claude Code"
    }

    fn is_available(&self) -> bool {
        platform::claude_projects_dir().exists()
            || platform::claude_desktop_sessions_dir().is_some()
    }

    fn scan(&self) -> Result<ProviderResult, ScanError> {
        let mut paths: Vec<std::path::PathBuf> = Vec::new();

        // Claude Code CLI sessions
        let projects_dir = platform::claude_projects_dir();
        if projects_dir.exists() {
            let pattern = format!("{}/**/*.jsonl", projects_dir.display());
            if let Ok(glob_paths) = glob::glob(&pattern) {
                paths.extend(glob_paths.filter_map(|p| p.ok()));
            }
        }

        // Claude Desktop local agent mode sessions (same JSONL format)
        if let Some(desktop_dir) = platform::claude_desktop_sessions_dir() {
            let pattern = format!("{}/**/*.jsonl", desktop_dir.display());
            if let Ok(glob_paths) = glob::glob(&pattern) {
                // Skip audit.jsonl files — they have a different format
                paths.extend(glob_paths.filter_map(|p| p.ok()).filter(|p| {
                    !p.file_name()
                        .is_some_and(|n| n.to_string_lossy().starts_with("audit"))
                }));
            }
        }

        if paths.is_empty() {
            return Err(ScanError::NotFound(
                "No Claude Code JSONL files found".into(),
            ));
        }

        let mut models: HashMap<String, ModelUsage> = HashMap::new();
        let mut all_timestamps: Vec<i64> = Vec::new();
        let mut session_timestamps: HashMap<String, Vec<i64>> = HashMap::new();
        let mut non_subagent_session_ids: HashSet<String> = HashSet::new();
        let mut first_seen: Option<i64> = None;
        let mut last_seen: Option<i64> = None;
        let mut events: Vec<(i64, u64, Option<String>)> = Vec::new();

        // Cross-file dedup of assistant messages by (message.id, requestId).
        // Resumed Claude Code sessions copy the entire prior turn-list into the
        // new JSONL file, so the same assistant message commonly appears in
        // multiple files. Without this, totals inflate roughly with the number
        // of resumes per conversation. Falls through (no dedup) when either id
        // is missing — older logs predate these fields.
        let mut seen_assistant_msgs: HashSet<String> = HashSet::new();

        for path in &paths {
            let is_subagent = path.to_string_lossy().contains("/subagents/");

            let file = match std::fs::File::open(path) {
                Ok(f) => f,
                Err(_) => continue,
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

                // Extract timestamp from every line
                let mut msg_ts: Option<i64> = None;
                let mut msg_session_id: Option<String> = None;
                if let Some(ts_str) = value.get("timestamp").and_then(|t| t.as_str()) {
                    if let Some(unix_ts) = time_util::iso8601_to_unix(ts_str) {
                        all_timestamps.push(unix_ts);
                        msg_ts = Some(unix_ts);

                        // Update first_seen / last_seen
                        first_seen = Some(first_seen.map_or(unix_ts, |fs: i64| fs.min(unix_ts)));
                        last_seen = Some(last_seen.map_or(unix_ts, |ls: i64| ls.max(unix_ts)));

                        // Track timestamps by sessionId
                        if let Some(session_id) = value.get("sessionId").and_then(|s| s.as_str()) {
                            session_timestamps
                                .entry(session_id.to_string())
                                .or_default()
                                .push(unix_ts);
                            msg_session_id = Some(session_id.to_string());

                            if !is_subagent {
                                non_subagent_session_ids.insert(session_id.to_string());
                            }
                        }
                    }
                }

                // Extract token usage from assistant messages
                let msg_type = value.get("type").and_then(|t| t.as_str()).unwrap_or("");
                if msg_type == "assistant" {
                    if let Some(message) = value.get("message") {
                        // Dedup before accumulating. Same (message.id, requestId)
                        // appearing in another resumed-session JSONL is the same
                        // billed turn, not a new one.
                        let msg_id = message.get("id").and_then(|v| v.as_str());
                        let req_id = value.get("requestId").and_then(|v| v.as_str());
                        if let (Some(mid), Some(rid)) = (msg_id, req_id) {
                            if !seen_assistant_msgs.insert(format!("{}:{}", mid, rid)) {
                                continue;
                            }
                        }

                        if let Some(usage) = message.get("usage") {
                            let input_tokens = usage
                                .get("input_tokens")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0);
                            let output_tokens = usage
                                .get("output_tokens")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0);
                            let cache_creation = usage
                                .get("cache_creation_input_tokens")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0);
                            let cache_read = usage
                                .get("cache_read_input_tokens")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0);

                            let model_name = message
                                .get("model")
                                .and_then(|m| m.as_str())
                                .unwrap_or("unknown")
                                .to_string();

                            let entry = models.entry(model_name).or_insert_with(|| ModelUsage {
                                tokens: TokenUsage::default(),
                                sessions: 0,
                                hours: 0.0,
                            });

                            entry.tokens.input_tokens =
                                entry.tokens.input_tokens.saturating_add(input_tokens);
                            entry.tokens.output_tokens =
                                entry.tokens.output_tokens.saturating_add(output_tokens);
                            entry.tokens.cache_creation_tokens = entry
                                .tokens
                                .cache_creation_tokens
                                .saturating_add(cache_creation);
                            entry.tokens.cache_read_tokens =
                                entry.tokens.cache_read_tokens.saturating_add(cache_read);
                            entry.tokens.compute_total();

                            // Emit daily-bucket event. We sum all four fields so the heatmap
                            // intensity matches the headline tokens number (which now
                            // includes cache, post-launch-ready compute_total change).
                            if let Some(ts) = msg_ts {
                                let event_tokens = input_tokens
                                    .saturating_add(output_tokens)
                                    .saturating_add(cache_creation)
                                    .saturating_add(cache_read);
                                events.push((ts, event_tokens, msg_session_id.clone()));
                            }
                        }
                    }
                } else if let Some(ts) = msg_ts {
                    // Non-assistant events still contribute to daily activity timestamps
                    // so per-day `hours` reflects real session work, not just token-emitting
                    // assistant replies.
                    events.push((ts, 0, msg_session_id.clone()));
                }
            }
        }

        // Compute hours from per-session timestamps
        let mut total_hours = 0.0_f64;
        for timestamps in session_timestamps.values() {
            let mut sorted = timestamps.clone();
            sorted.sort_unstable();
            total_hours += time_util::active_hours_from_timestamps(&sorted, 1800);
        }

        // Sessions: count distinct non-subagent sessionIds
        let sessions = non_subagent_session_ids.len() as u64;

        // Aggregate total tokens across all models
        let mut total_tokens = TokenUsage::default();
        for model_usage in models.values() {
            total_tokens.merge(&model_usage.tokens);
        }

        let mut result = ProviderResult::new("Claude Code");
        result.hours = Some(total_hours);
        result.tokens = Some(total_tokens);
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
