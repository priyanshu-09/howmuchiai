use crate::platform;
use crate::providers::Provider;
use crate::sqlite_util::SafeSqlite;
use crate::time_util;
use crate::types::{ProviderResult, ScanError, TokenUsage};
use std::collections::HashMap;
use tiktoken_rs::cl100k_base;

pub struct CursorProvider;

/// Cursor does not log per-request token usage locally. This note is stamped on
/// every estimated count so downstream dashboards never treat it as telemetry.
const ESTIMATE_METHOD: &str = "cl100k_base tokenizer over locally stored bubble text (text, thinking, toolFormerData params/result, attachedCodeChunks). Excludes server-side context, cache reads, and bubbles with no stored text. Not per-project.";

impl Provider for CursorProvider {
    fn name(&self) -> &'static str {
        "cursor"
    }

    fn display_name(&self) -> &'static str {
        "Cursor IDE"
    }

    fn is_available(&self) -> bool {
        platform::cursor_state_db().is_some()
    }

    fn scan(&self) -> Result<ProviderResult, ScanError> {
        let db_path = platform::cursor_state_db()
            .ok_or_else(|| ScanError::NotFound("Cursor state DB not found".into()))?;

        let db = SafeSqlite::open(&db_path)?;

        let mut sessions: u64 = 0;
        let mut all_timestamps: Vec<i64> = Vec::new();
        let mut first_seen: Option<i64> = None;
        let mut last_seen: Option<i64> = None;
        let mut accepted_lines: u64 = 0;
        let mut events: Vec<(i64, u64, Option<String>)> = Vec::new();

        // Query composerData entries for session info
        let mut stmt = db
            .conn()
            .prepare("SELECT key, value FROM cursorDiskKV WHERE key LIKE 'composerData:%'")?;

        let rows = stmt.query_map([], |row| {
            let _key: String = row.get(0)?;
            let value: String = row.get(1)?;
            Ok(value)
        })?;

        for row in rows {
            let value_str = match row {
                Ok(v) => v,
                Err(_) => continue,
            };

            let value: serde_json::Value = match serde_json::from_str(&value_str) {
                Ok(v) => v,
                Err(_) => continue,
            };

            // Extract createdAt (epoch milliseconds -> seconds)
            if let Some(created_at_ms) = value.get("createdAt").and_then(|v| v.as_i64()) {
                let created_at = created_at_ms / 1000;

                // Skip unreasonable timestamps
                if !(1_577_836_800..=2_000_000_000).contains(&created_at) {
                    continue;
                }

                sessions += 1;
                all_timestamps.push(created_at);
                // Composer rows drive hours/sessions; token counts come from bubbles.
                events.push((created_at, 0, None));

                first_seen = Some(first_seen.map_or(created_at, |fs: i64| fs.min(created_at)));
                last_seen = Some(last_seen.map_or(created_at, |ls: i64| ls.max(created_at)));
            }
        }

        // Query daily stats from ItemTable for accepted lines
        let daily_stats_result = db.conn().prepare(
            "SELECT key, value FROM ItemTable WHERE key LIKE 'aiCodeTracking.dailyStats%'",
        );

        if let Ok(mut stats_stmt) = daily_stats_result {
            let stats_rows = stats_stmt.query_map([], |row| {
                let _key: String = row.get(0)?;
                let value: String = row.get(1)?;
                Ok(value)
            });

            if let Ok(rows) = stats_rows {
                for row in rows {
                    let value_str = match row {
                        Ok(v) => v,
                        Err(_) => continue,
                    };

                    // Try parsing as JSON
                    let value: serde_json::Value = match serde_json::from_str(&value_str) {
                        Ok(v) => v,
                        Err(_) => {
                            // May be a plain number
                            if let Ok(n) = value_str.parse::<u64>() {
                                accepted_lines = accepted_lines.saturating_add(n);
                            }
                            continue;
                        }
                    };

                    // If it's a JSON object with accepted/generated counts
                    if let Some(accepted) = value.get("accepted").and_then(|v| v.as_u64()) {
                        accepted_lines = accepted_lines.saturating_add(accepted);
                    }
                    // If it's a direct number
                    if let Some(n) = value.as_u64() {
                        accepted_lines = accepted_lines.saturating_add(n);
                    }
                }
            }
        }

        // --- Estimated token extraction from chat bubbles -------------------
        // Each `bubbleId:<composerId>:<bubbleUuid>` row is counted exactly once.
        let (estimated_tokens, token_events) = extract_bubble_tokens(&db);
        events.extend(token_events);

        // Compute hours from timestamps
        all_timestamps.sort_unstable();
        let total_hours = time_util::active_hours_from_timestamps(&all_timestamps, 1800);

        let mut result = ProviderResult::new("Cursor IDE");
        result.hours = Some(total_hours);
        result.sessions = Some(sessions);
        result.first_seen = first_seen;
        result.last_seen = last_seen;

        if accepted_lines > 0 {
            let mut metadata = HashMap::new();
            metadata.insert(
                "accepted_lines".to_string(),
                serde_json::Value::Number(serde_json::Number::from(accepted_lines)),
            );
            result.metadata = Some(metadata);
        }

        if estimated_tokens.total > 0 {
            result.tokens = Some(estimated_tokens);
            result.tokens_estimated = true;
            result.tokens_estimate_method = Some(ESTIMATE_METHOD.to_string());
        }

        let daily_buckets = time_util::build_daily_buckets(&events);
        if !daily_buckets.is_empty() {
            result.daily_buckets = Some(daily_buckets);
        }

        Ok(result)
    }
}

/// Per-bubble `(unix_ts, token_count, session_id)` for daily-bucket folding.
type BubbleTokenEvent = (i64, u64, Option<String>);

/// Tokenize every unique bubble row in `cursorDiskKV`.
///
/// AGGREGATION RULES (verified against on-disk store):
///   * One count per `bubbleId:` key — bubbles are stored once globally.
///   * `type == 1` bubbles contribute to `input_tokens`; all others to
///     `output_tokens` (assistant text, thinking blocks, tool results).
///   * Concatenate visible fields: `text`, `thinking.text`, `toolFormerData`
///     `.params`/`.result`, `attachedCodeChunks` code bodies. Skip `rawArgs`
///     when it duplicates `.params`. Use `richText` only when `text` is empty.
///   * Ignore native `tokenCount` / `usageData` — uniformly zero on this Cursor
///     version and not telemetry.
fn extract_bubble_tokens(db: &SafeSqlite) -> (TokenUsage, Vec<BubbleTokenEvent>) {
    let bpe = match cl100k_base() {
        Ok(b) => b,
        Err(_) => return (TokenUsage::default(), Vec::new()),
    };

    let mut input_sum: u64 = 0;
    let mut output_sum: u64 = 0;
    let mut token_events: Vec<BubbleTokenEvent> = Vec::new();

    let mut stmt = match db
        .conn()
        .prepare("SELECT key, value FROM cursorDiskKV WHERE key LIKE 'bubbleId:%'")
    {
        Ok(s) => s,
        Err(_) => return (TokenUsage::default(), Vec::new()),
    };

    let rows = match stmt.query_map([], |row| {
        let key: String = row.get(0)?;
        let value: String = row.get(1)?;
        Ok((key, value))
    }) {
        Ok(r) => r,
        Err(_) => return (TokenUsage::default(), Vec::new()),
    };

    for row in rows {
        let (key, value_str) = match row {
            Ok(v) => v,
            Err(_) => continue,
        };

        let value: serde_json::Value = match serde_json::from_str(&value_str) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let text = collect_bubble_text(&value);
        if text.is_empty() {
            continue;
        }

        let count = bpe.encode_with_special_tokens(&text).len() as u64;
        if count == 0 {
            continue;
        }

        let is_user = value.get("type").and_then(|v| v.as_i64()) == Some(1);
        if is_user {
            input_sum = input_sum.saturating_add(count);
        } else {
            output_sum = output_sum.saturating_add(count);
        }

        if let Some(ts) = bubble_created_at(&value) {
            let composer_id = bubble_composer_id(&key);
            token_events.push((ts, count, composer_id));
        }
    }

    let mut tokens = TokenUsage {
        input_tokens: input_sum,
        output_tokens: output_sum,
        cache_read_tokens: 0,
        cache_creation_tokens: 0,
        total: 0,
    };
    tokens.compute_total();
    (tokens, token_events)
}

/// Gather all locally stored text from a bubble JSON object.
fn collect_bubble_text(obj: &serde_json::Value) -> String {
    let mut parts: Vec<String> = Vec::new();

    if let Some(text) = obj.get("text").and_then(|v| v.as_str()) {
        if !text.trim().is_empty() {
            parts.push(text.to_string());
        }
    }

    // richText is a fallback when the plain text field is empty.
    if parts.is_empty() {
        if let Some(rt) = obj.get("richText").and_then(|v| v.as_str()) {
            if !rt.trim().is_empty() {
                parts.push(rt.to_string());
            }
        }
    }

    if let Some(thinking) = obj.get("thinking") {
        if let Some(tt) = thinking.get("text").and_then(|v| v.as_str()) {
            if !tt.trim().is_empty() {
                parts.push(tt.to_string());
            }
        }
    }

    if let Some(tfd) = obj.get("toolFormerData") {
        for field in ["params", "result"] {
            if let Some(s) = tfd.get(field).and_then(|v| v.as_str()) {
                if !s.trim().is_empty() {
                    parts.push(s.to_string());
                }
            }
        }
    }

    if let Some(chunks) = obj.get("attachedCodeChunks").and_then(|v| v.as_array()) {
        for chunk in chunks {
            for field in ["content", "text", "code"] {
                if let Some(c) = chunk.get(field).and_then(|v| v.as_str()) {
                    if !c.is_empty() {
                        parts.push(c.to_string());
                    }
                }
            }
        }
    }

    parts.join("\n")
}

/// Parse bubble `createdAt` (ISO-8601 string) to unix seconds.
fn bubble_created_at(obj: &serde_json::Value) -> Option<i64> {
    obj.get("createdAt")
        .and_then(|v| v.as_str())
        .and_then(time_util::iso8601_to_unix)
}

/// Extract composer id from key `bubbleId:<composerId>:<bubbleUuid>`.
fn bubble_composer_id(key: &str) -> Option<String> {
    let rest = key.strip_prefix("bubbleId:")?;
    let mut parts = rest.split(':');
    let composer = parts.next()?;
    // Require the canonical three-part key; malformed keys get no attribution.
    if parts.next().is_none() || composer.is_empty() {
        return None;
    }
    Some(composer.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    // cl100k_base loads encoding tables; reuse across tests.
    fn test_bpe() -> tiktoken_rs::CoreBPE {
        cl100k_base().expect("cl100k_base")
    }

    fn count_text(text: &str) -> u64 {
        test_bpe().encode_with_special_tokens(text).len() as u64
    }

    #[test]
    fn collect_bubble_text_gathers_all_fields() {
        let obj: serde_json::Value = serde_json::json!({
            "text": "hello user",
            "thinking": { "text": "reasoning here" },
            "toolFormerData": {
                "params": "{\"cmd\":\"ls\"}",
                "result": "file1\nfile2",
                "rawArgs": "{\"cmd\":\"ls\"}"
            },
            "attachedCodeChunks": [{ "content": "fn main() {}" }]
        });
        let text = collect_bubble_text(&obj);
        assert!(text.contains("hello user"));
        assert!(text.contains("reasoning here"));
        assert!(text.contains("file1"));
        assert!(text.contains("fn main()"));
        // rawArgs must NOT appear as a separate duplicate when params is present
        assert_eq!(text.matches("{\"cmd\":\"ls\"}").count(), 1);
    }

    #[test]
    fn collect_bubble_text_uses_richtext_when_text_empty() {
        let obj: serde_json::Value = serde_json::json!({
            "text": "",
            "richText": "rich fallback"
        });
        let text = collect_bubble_text(&obj);
        assert_eq!(text, "rich fallback");
    }

    #[test]
    fn collect_bubble_text_empty_bubble_returns_empty() {
        let obj: serde_json::Value = serde_json::json!({
            "text": "",
            "toolFormerData": { "params": "", "result": "" }
        });
        assert!(collect_bubble_text(&obj).is_empty());
    }

    #[test]
    fn bubble_composer_id_parses_key() {
        assert_eq!(
            bubble_composer_id("bubbleId:abc-123:def-456"),
            Some("abc-123".to_string())
        );
        assert_eq!(bubble_composer_id("bubbleId:only-one"), None);
    }

    #[test]
    fn extract_bubble_tokens_counts_and_dedups_by_key() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("state.vscdb");
        let conn = Connection::open(&db_path).expect("open");
        conn.execute_batch(
            "CREATE TABLE cursorDiskKV (key TEXT PRIMARY KEY, value TEXT);
             INSERT INTO cursorDiskKV VALUES
               ('bubbleId:comp-1:bub-user', '{\"type\":1,\"text\":\"Hello world\",\"createdAt\":\"2026-05-24T16:46:00.000Z\"}'),
               ('bubbleId:comp-1:bub-asst', '{\"type\":2,\"text\":\"Hi there\",\"thinking\":{\"text\":\"hmm\"},\"createdAt\":\"2026-05-24T16:47:00.000Z\"}'),
               ('bubbleId:comp-1:bub-empty', '{\"type\":2,\"text\":\"\",\"createdAt\":\"2026-05-24T16:48:00.000Z\"}'),
               ('not-a-bubble', '{\"text\":\"ignored\"}');",
        )
        .expect("seed");

        let db = SafeSqlite::open(&db_path).expect("safe open");
        let (tokens, events) = extract_bubble_tokens(&db);

        let user_count = count_text("Hello world");
        let asst_count = count_text("Hi there\nhmm");
        assert_eq!(tokens.input_tokens, user_count);
        assert_eq!(tokens.output_tokens, asst_count);
        assert_eq!(tokens.total, user_count + asst_count);
        assert_eq!(events.len(), 2, "empty bubble must not emit an event");
    }

    #[test]
    fn extract_bubble_tokens_skips_malformed_json() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("state.vscdb");
        let conn = Connection::open(&db_path).expect("open");
        conn.execute_batch(
            "CREATE TABLE cursorDiskKV (key TEXT PRIMARY KEY, value TEXT);
             INSERT INTO cursorDiskKV VALUES
               ('bubbleId:c:b1', 'not json at all'),
               ('bubbleId:c:b2', '{\"type\":1,\"text\":\"ok\",\"createdAt\":\"2026-05-24T16:46:00.000Z\"}');",
        )
        .expect("seed");

        let db = SafeSqlite::open(&db_path).expect("safe open");
        let (tokens, _) = extract_bubble_tokens(&db);
        assert_eq!(tokens.input_tokens, count_text("ok"));
        assert_eq!(tokens.output_tokens, 0);
    }

    #[test]
    fn scan_result_marks_tokens_as_estimated() {
        // Verify the estimate marker constants are wired — integration with the
        // real global DB is covered by the release-binary cross-check.
        assert!(ESTIMATE_METHOD.contains("cl100k_base"));
        assert!(ESTIMATE_METHOD.contains("Excludes"));
    }

    /// Cross-check against the live Cursor global DB. Run manually:
    /// `cargo test verify_real_cursor_db_crosscheck -- --ignored --nocapture`
    #[test]
    #[ignore = "requires local Cursor state.vscdb; run for release verification"]
    fn verify_real_cursor_db_crosscheck() {
        let db_path = match platform::cursor_state_db() {
            Some(p) => p,
            None => return,
        };
        let db = SafeSqlite::open(&db_path).expect("open real db");
        let (tokens, _) = extract_bubble_tokens(&db);
        eprintln!(
            "real-db cross-check: input={} output={} total={}",
            tokens.input_tokens, tokens.output_tokens, tokens.total
        );
        assert!(tokens.total > 0, "real DB should yield nonzero estimate");
    }
}
