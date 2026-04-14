use crate::platform;
use crate::providers::Provider;
use crate::time_util;
use crate::types::{ProviderResult, ScanError};
use std::collections::HashMap;

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
                if let Some(session_id) = entry.get("sessionId").and_then(|v| v.as_str()) {
                    sessions_set.insert(session_id.to_string());
                }

                total_messages += 1;

                // Parse timestamp -- could be ISO 8601 string or numeric
                if let Some(ts) = entry.get("timestamp") {
                    if let Some(ts_str) = ts.as_str() {
                        if let Some(unix) = time_util::iso8601_to_unix(ts_str) {
                            all_timestamps.push(unix);
                        }
                    } else if let Some(ts_num) = ts.as_i64() {
                        // Assume milliseconds if value is large enough
                        if ts_num > 1_000_000_000_000 {
                            all_timestamps.push(ts_num / 1000);
                        } else {
                            all_timestamps.push(ts_num);
                        }
                    } else if let Some(ts_f) = ts.as_f64() {
                        all_timestamps.push(ts_f as i64);
                    }
                }
            }
        }

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
        result.hours = Some(hours);
        result.first_seen = first_seen;
        result.last_seen = last_seen;
        result.metadata = Some(metadata);

        Ok(result)
    }
}
