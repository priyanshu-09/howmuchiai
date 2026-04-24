use crate::platform;
use crate::providers::Provider;
use crate::time_util;
use crate::types::{DailyBucket, ProviderResult, ScanError};
use regex::Regex;
use std::collections::HashMap;
use std::io::BufRead;

/// AI CLI tool patterns to match in shell history
const AI_PATTERNS: &[(&str, &str)] = &[
    ("claude", "Claude Code"),
    ("codex", "Codex CLI"),
    ("cursor", "Cursor"),
    ("copilot", "Copilot"),
    ("aider", "Aider"),
    ("sgpt", "ShellGPT"),
    ("ollama", "Ollama"),
    ("gemini", "Gemini CLI"),
    ("chatgpt", "ChatGPT CLI"),
    ("cody", "Cody"),
    ("gh copilot", "GitHub Copilot CLI"),
];

pub struct ShellHistoryProvider;

impl Provider for ShellHistoryProvider {
    fn name(&self) -> &'static str {
        "shell_history"
    }

    fn display_name(&self) -> &'static str {
        "Shell History"
    }

    fn is_available(&self) -> bool {
        platform::zsh_history_path().is_some()
            || platform::bash_history_path().is_some()
            || platform::fish_history_path().is_some()
    }

    fn scan(&self) -> Result<ProviderResult, ScanError> {
        let mut tool_counts: HashMap<String, u64> = HashMap::new();
        let mut all_timestamps: Vec<i64> = Vec::new();
        let mut total_invocations: u64 = 0;

        // Parse zsh history (timestamped)
        if let Some(path) = platform::zsh_history_path() {
            parse_zsh_history(
                &path,
                &mut tool_counts,
                &mut all_timestamps,
                &mut total_invocations,
            );
        }

        // Parse bash history (no timestamps)
        if let Some(path) = platform::bash_history_path() {
            parse_bash_history(&path, &mut tool_counts, &mut total_invocations);
        }

        // Parse fish history
        if let Some(path) = platform::fish_history_path() {
            parse_fish_history(&path, &mut tool_counts, &mut total_invocations);
        }

        if total_invocations == 0 {
            return Err(ScanError::NotFound(
                "No AI tool invocations found in shell history".into(),
            ));
        }

        // Compute first_seen / last_seen from zsh timestamps
        all_timestamps.sort_unstable();
        let first_seen = all_timestamps.first().copied();
        let last_seen = all_timestamps.last().copied();

        let mut result = ProviderResult::new("Shell History");
        result.invocations = Some(total_invocations);
        result.first_seen = first_seen;
        result.last_seen = last_seen;

        // Build daily buckets: each invocation is instantaneous (hours=0.0).
        if !all_timestamps.is_empty() {
            let by_day = time_util::group_by_day(&all_timestamps);
            let mut buckets: HashMap<String, DailyBucket> = HashMap::new();
            for (day, ts_list) in by_day {
                buckets.insert(
                    day,
                    DailyBucket {
                        hours: 0.0,
                        tokens: 0,
                        sessions: 0,
                        invocations: Some(ts_list.len() as u64),
                    },
                );
            }
            if !buckets.is_empty() {
                result.daily_buckets = Some(buckets);
            }
        }

        // Store per-tool breakdown in metadata
        if !tool_counts.is_empty() {
            let mut metadata = HashMap::new();
            let tools_value: HashMap<String, serde_json::Value> = tool_counts
                .iter()
                .map(|(k, v)| {
                    (
                        k.clone(),
                        serde_json::Value::Number(serde_json::Number::from(*v)),
                    )
                })
                .collect();
            metadata.insert(
                "tools".to_string(),
                serde_json::to_value(&tools_value).unwrap_or_default(),
            );
            result.metadata = Some(metadata);
        }

        Ok(result)
    }
}

/// Check if a command line matches any AI tool pattern.
/// Uses word-boundary-start matching: the pattern must appear at the start
/// of the command or after a whitespace/pipe/semicolon character.
fn match_ai_tool(command: &str) -> Option<&'static str> {
    let cmd_lower = command.to_lowercase();

    // Check multi-word patterns first (e.g., "gh copilot")
    for (pattern, display_name) in AI_PATTERNS {
        if pattern.contains(' ') {
            // Multi-word: just check contains
            if cmd_lower.contains(pattern) {
                return Some(display_name);
            }
            continue;
        }

        // Single-word: check at word boundary start
        if let Some(pos) = cmd_lower.find(pattern) {
            // Must be at start of line or preceded by whitespace/pipe/semicolon/backtick
            let at_boundary = pos == 0 || {
                let prev_byte = cmd_lower.as_bytes()[pos - 1];
                matches!(
                    prev_byte,
                    b' ' | b'\t' | b'|' | b';' | b'`' | b'$' | b'(' | b'/'
                )
            };

            if at_boundary {
                return Some(display_name);
            }
        }
    }

    None
}

/// Parse zsh history format: `: 1234567890:0;command`
fn parse_zsh_history(
    path: &std::path::Path,
    tool_counts: &mut HashMap<String, u64>,
    timestamps: &mut Vec<i64>,
    total_invocations: &mut u64,
) {
    // Read as bytes to handle mixed encodings common in zsh history
    let content = match std::fs::read(path) {
        Ok(c) => c,
        Err(_) => return,
    };

    let content_str = String::from_utf8_lossy(&content);
    let zsh_re = Regex::new(r"^: (\d+):\d+;(.*)$").unwrap();

    for line in content_str.lines() {
        if let Some(caps) = zsh_re.captures(line) {
            let timestamp: i64 = match caps[1].parse() {
                Ok(ts) => ts,
                Err(_) => continue,
            };
            let command = &caps[2];

            if let Some(tool_name) = match_ai_tool(command) {
                *tool_counts.entry(tool_name.to_string()).or_insert(0) += 1;
                timestamps.push(timestamp);
                *total_invocations += 1;
            }
        } else {
            // Some zsh history lines don't have timestamps (continuation lines)
            // Still check for AI tool usage
            if let Some(tool_name) = match_ai_tool(line) {
                *tool_counts.entry(tool_name.to_string()).or_insert(0) += 1;
                *total_invocations += 1;
            }
        }
    }
}

/// Parse bash history: plain command lines (no timestamps)
fn parse_bash_history(
    path: &std::path::Path,
    tool_counts: &mut HashMap<String, u64>,
    total_invocations: &mut u64,
) {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return,
    };
    let reader = std::io::BufReader::new(file);

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };

        if let Some(tool_name) = match_ai_tool(&line) {
            *tool_counts.entry(tool_name.to_string()).or_insert(0) += 1;
            *total_invocations += 1;
        }
    }
}

/// Parse fish history format:
/// ```text
/// - cmd: some_command
///   when: 1234567890
/// ```
fn parse_fish_history(
    path: &std::path::Path,
    tool_counts: &mut HashMap<String, u64>,
    total_invocations: &mut u64,
) {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return,
    };
    let reader = std::io::BufReader::new(file);

    let mut current_cmd: Option<String> = None;

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };

        let trimmed = line.trim();

        if let Some(cmd) = trimmed.strip_prefix("- cmd: ") {
            // If we had a previous command pending, process it
            if let Some(ref prev_cmd) = current_cmd {
                if let Some(tool_name) = match_ai_tool(prev_cmd) {
                    *tool_counts.entry(tool_name.to_string()).or_insert(0) += 1;
                    *total_invocations += 1;
                }
            }
            current_cmd = Some(cmd.to_string());
        } else if trimmed.starts_with("when: ") {
            // The "when" line follows a "cmd" line; we already captured the command
            // Nothing extra needed since we process the cmd on the next "- cmd:" line
        }
    }

    // Process the last command if present
    if let Some(ref cmd) = current_cmd {
        if let Some(tool_name) = match_ai_tool(cmd) {
            *tool_counts.entry(tool_name.to_string()).or_insert(0) += 1;
            *total_invocations += 1;
        }
    }
}
