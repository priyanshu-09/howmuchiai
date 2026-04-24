use crate::platform;
use crate::providers::Provider;
use crate::sqlite_util::SafeSqlite;
use crate::time_util;
use crate::types::{ProviderResult, ScanError};
use std::collections::HashMap;

pub struct CursorProvider;

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
                // Per-event tokens are not recorded by Cursor; emit 0 and no session id
                // so each day shows accurate `hours` but tokens/sessions stay zero.
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

        let daily_buckets = time_util::build_daily_buckets(&events);
        if !daily_buckets.is_empty() {
            result.daily_buckets = Some(daily_buckets);
        }

        Ok(result)
    }
}
