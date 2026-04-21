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
                        .map_or(false, |n| n.to_string_lossy().starts_with("audit"))
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
                if let Some(ts_str) = value.get("timestamp").and_then(|t| t.as_str()) {
                    if let Some(unix_ts) = time_util::iso8601_to_unix(ts_str) {
                        all_timestamps.push(unix_ts);

                        // Update first_seen / last_seen
                        first_seen = Some(first_seen.map_or(unix_ts, |fs: i64| fs.min(unix_ts)));
                        last_seen = Some(last_seen.map_or(unix_ts, |ls: i64| ls.max(unix_ts)));

                        // Track timestamps by sessionId
                        if let Some(session_id) = value.get("sessionId").and_then(|s| s.as_str()) {
                            session_timestamps
                                .entry(session_id.to_string())
                                .or_default()
                                .push(unix_ts);

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
                        }
                    }
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

        Ok(result)
    }
}
